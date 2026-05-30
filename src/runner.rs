//! Runs installs and commands, streaming their output into a live tail window
//! that folds into a single summary line once the job finishes. Install path
//! follows the user's rule: use cargo-binstall if present; if absent, ask once
//! whether to install it, otherwise fall back to `cargo install`.

use crate::suggestion::{InstallSpec, RunSpec, Tag};
use crate::{system, ui};
use anyhow::{Context, Result, bail};
use cliclack::log;
use console::{Style, Term, strip_ansi_codes, style, truncate_str};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Instant;

/// How many recent output lines the live tail window keeps on screen.
const TAIL_LINES: usize = 6;
/// How many captured lines to replay when a job fails, so the error is visible.
const FAIL_TAIL: usize = 40;

#[derive(PartialEq, Eq)]
enum Binstall {
    Available,
    Fallback,
    Unknown,
}

pub struct Runner {
    binstall: Binstall,
}

impl Default for Runner {
    fn default() -> Self {
        Self::new()
    }
}

impl Runner {
    pub fn new() -> Self {
        let binstall = if system::have("cargo-binstall") {
            Binstall::Available
        } else {
            Binstall::Unknown
        };
        Runner { binstall }
    }

    pub fn install_method_label(&self) -> &'static str {
        match self.binstall {
            Binstall::Available => "cargo-binstall",
            Binstall::Fallback => "cargo install",
            Binstall::Unknown => "cargo-binstall if present, else cargo install",
        }
    }

    pub fn install(&mut self, spec: &InstallSpec, tag: Tag, dry_run: bool) -> Result<()> {
        if system::have(&spec.bin_name) {
            log::success(format!("{} already installed", spec.bin_name))?;
            return Ok(());
        }
        let use_binstall = self.ensure_binstall(dry_run)?;
        let args: Vec<String> = if use_binstall {
            vec!["binstall".into(), "-y".into(), spec.crate_name.clone()]
        } else {
            vec!["install".into(), spec.crate_name.clone()]
        };
        let job = Job {
            running: format!("Installing {}", spec.crate_name),
            done: format!("Installed {}", spec.crate_name),
            tag,
        };
        run_streaming("cargo", &args, None, &job, dry_run)
    }

    pub fn run(&self, spec: &RunSpec, tag: Tag, dry_run: bool) -> Result<()> {
        let job = Job {
            running: format!("Running {}", spec.display()),
            done: format!("Ran {}", spec.display()),
            tag,
        };
        run_streaming(
            &spec.program,
            &spec.args,
            spec.cwd.as_deref(),
            &job,
            dry_run,
        )
    }

    fn ensure_binstall(&mut self, dry_run: bool) -> Result<bool> {
        match self.binstall {
            Binstall::Available => Ok(true),
            Binstall::Fallback => Ok(false),
            Binstall::Unknown => {
                if dry_run {
                    return Ok(false);
                }
                let install_it = cliclack::confirm(
                    "cargo-binstall not found. Install it for fast prebuilt binaries? \
                     (otherwise fall back to cargo install)",
                )
                .initial_value(true)
                .interact()?;
                if install_it {
                    let job = Job {
                        running: "Installing cargo-binstall".into(),
                        done: "Installed cargo-binstall".into(),
                        tag: Tag::Both,
                    };
                    run_streaming(
                        "cargo",
                        &["install".into(), "cargo-binstall".into()],
                        None,
                        &job,
                        dry_run,
                    )?;
                    self.binstall = Binstall::Available;
                    Ok(true)
                } else {
                    self.binstall = Binstall::Fallback;
                    Ok(false)
                }
            }
        }
    }
}

/// Display labels and category color for one streamed job.
struct Job {
    running: String,
    done: String,
    tag: Tag,
}

fn run_streaming(
    prog: &str,
    args: &[String],
    cwd: Option<&Path>,
    job: &Job,
    dry_run: bool,
) -> Result<()> {
    let shown = format!("{prog} {}", args.join(" "));
    if dry_run {
        log::info(format!("would run {}", style(shown).dim()))?;
        return Ok(());
    }

    let mut cmd = Command::new(prog);
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(d) = cwd {
        cmd.current_dir(d);
    }
    let mut child = cmd.spawn().with_context(|| format!("spawn {prog}"))?;

    // Read stdout and stderr on separate threads so a full pipe never deadlocks,
    // funneling every line into one ordered channel for the tail window.
    let (tx, rx) = mpsc::channel::<String>();
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let tx2 = tx.clone();
    let h_out = thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let h_err = thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if tx2.send(line).is_err() {
                break;
            }
        }
    });

    let start = Instant::now();
    let mut window = TailWindow::new(ui::accent(job.tag));
    window.open(&job.running);
    let mut full: Vec<String> = Vec::new();
    for line in rx {
        full.push(line.clone());
        window.push(line);
    }
    let _ = h_out.join();
    let _ = h_err.join();
    let status = child.wait().with_context(|| format!("wait {prog}"))?;
    window.close();
    let elapsed = start.elapsed().as_secs_f64();

    if status.success() {
        log::success(format!(
            "{} {}",
            job.done,
            style(format!("({elapsed:.1}s)")).dim()
        ))?;
        Ok(())
    } else {
        // Replay the captured output so the failure is debuggable; the wizard
        // prints the single error summary line from the returned error.
        replay_on_failure(&full);
        bail!("{shown} exited with {status}");
    }
}

/// Print the tail of a failed job's output so the error stays debuggable, even
/// though the live window has been folded away.
fn replay_on_failure(full: &[String]) {
    let term = Term::stderr();
    let bar = style("│").dim();
    let start = full.len().saturating_sub(FAIL_TAIL);
    for line in &full[start..] {
        let _ = term.write_line(&format!("{bar}  {}", style(strip_ansi_codes(line)).dim()));
    }
}

/// A self-redrawing window showing a header plus the last few output lines. On a
/// TTY it folds in place when closed; off a TTY it just passes lines through.
struct TailWindow {
    term: Term,
    interactive: bool,
    accent: Style,
    header: String,
    lines: VecDeque<String>,
    drawn: usize,
}

impl TailWindow {
    fn new(accent: Style) -> Self {
        let term = Term::stderr();
        let interactive = term.is_term();
        TailWindow {
            term,
            interactive,
            accent,
            header: String::new(),
            lines: VecDeque::with_capacity(TAIL_LINES),
            drawn: 0,
        }
    }

    fn open(&mut self, header: &str) {
        self.header = header.to_string();
        if self.interactive {
            self.redraw();
        } else {
            let _ = self.term.write_line(&format!(
                "{}  {}",
                self.accent.apply_to("◇"),
                style(header).bold()
            ));
        }
    }

    fn push(&mut self, line: String) {
        if !self.interactive {
            let _ = self
                .term
                .write_line(&format!("   {}", strip_ansi_codes(&line)));
            return;
        }
        if self.lines.len() == TAIL_LINES {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
        self.redraw();
    }

    fn redraw(&mut self) {
        if self.drawn > 0 {
            let _ = self.term.clear_last_lines(self.drawn);
        }
        let inner = (self.term.size().1 as usize).saturating_sub(3).max(8);
        let _ = self.term.write_line(&format!(
            "{}  {}",
            self.accent.apply_to("◇"),
            style(&self.header).bold()
        ));
        let bar = self.accent.apply_to("│");
        for line in &self.lines {
            let clean = strip_ansi_codes(line);
            let shown = truncate_str(clean.trim_end(), inner, "…");
            let _ = self
                .term
                .write_line(&format!("{bar}  {}", style(shown).dim()));
        }
        self.drawn = 1 + self.lines.len();
    }

    /// Fold the live region away, leaving the caller to print the summary line.
    fn close(&mut self) {
        if self.interactive && self.drawn > 0 {
            let _ = self.term.clear_last_lines(self.drawn);
        }
        self.drawn = 0;
        self.lines.clear();
    }
}
