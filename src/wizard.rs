//! The accept/skip wizard: one cliclack card per suggestion (themed by its
//! optimization category), a single Select decision, and apply on accept.

use crate::runner::Runner;
use crate::suggestion::{Action, PurgeSpec, RunSpec, Suggestion, SweepSpec, Tag};
use crate::system::{self, SystemReport, human_bytes};
use crate::toml_ops::{self, TomlPlan};
use crate::ui;
use anyhow::{Result, anyhow};
use cliclack::{log, note, select, spinner};
use console::style;
use std::path::{Path, PathBuf};

pub struct Summary {
    pub applied: usize,
    pub skipped: usize,
    pub failed: usize,
    pub quit: bool,
    /// Whether a sweep was accepted this run. Lets the caller drop the
    /// "accept the sweep to reclaim space" epilogue once it is moot.
    pub swept: bool,
}

#[derive(Clone, PartialEq, Eq)]
enum Decision {
    Accept,
    Skip,
    Quit,
}

pub fn run(
    report: &SystemReport,
    suggestions: Vec<Suggestion>,
    runner: &mut Runner,
    dry_run: bool,
    yes: bool,
) -> Result<Summary> {
    let total = suggestions.len();
    let mut applied = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;
    let mut swept = false;

    for (i, sug) in suggestions.iter().enumerate() {
        // Build the TOML plan up front so the card can show the exact diff.
        let plan = match &sug.action {
            Action::Toml(c) => Some(toml_ops::plan(report, c)?),
            _ => None,
        };

        ui::set_category(sug.tag);
        show_card(i + 1, total, sug, plan.as_ref(), runner)?;

        let decision = if yes { Decision::Accept } else { ask()? };
        match decision {
            Decision::Accept => match execute(sug, plan.as_ref(), runner, dry_run, yes) {
                Ok(()) => {
                    applied += 1;
                    swept |= matches!(sug.action, Action::Sweep(_));
                }
                Err(e) => {
                    let _ = log::error(format!("{e:#}"));
                    failed += 1;
                }
            },
            Decision::Skip => {
                let _ = log::info("skipped");
                skipped += 1;
            }
            Decision::Quit => {
                ui::reset();
                return Ok(Summary {
                    applied,
                    skipped,
                    failed,
                    quit: true,
                    swept,
                });
            }
        }
    }

    ui::reset();
    Ok(Summary {
        applied,
        skipped,
        failed,
        quit: false,
        swept,
    })
}

/// Ask the accept/skip/quit decision. Errors with a clear hint when there is no
/// TTY, pointing at `--yes` for non-interactive runs.
fn ask() -> Result<Decision> {
    select("Apply this change?")
        .item(Decision::Accept, "Accept", "write it")
        .item(Decision::Skip, "Skip", "leave as is")
        .item(Decision::Quit, "Quit", "stop here")
        .initial_value(Decision::Accept)
        .interact()
        .map_err(|e| {
            anyhow!("interactive prompt needs a TTY; use --yes for non-interactive runs: {e}")
        })
}

/// Render one suggestion as a titled note box: why on top, then the file and a
/// clean additive diff (or the install/run command).
fn show_card(
    n: usize,
    total: usize,
    sug: &Suggestion,
    plan: Option<&TomlPlan>,
    runner: &Runner,
) -> Result<()> {
    // The title is plain text: cliclack wraps it, so styling here would corrupt
    // the box border. The category color comes from the themed gutter instead.
    let title = format!("{}   [{}] {n}/{total}", sug.title, sug.tag.label());

    let mut body = sug.why.clone();
    match &sug.action {
        Action::Toml(c) => {
            body.push_str("\n\n");
            body.push_str(&style(c.scope.label()).underlined().to_string());
            if let Some(p) = plan {
                let diff = render_diff(p);
                if !diff.is_empty() {
                    body.push('\n');
                    body.push_str(&diff);
                }
            }
        }
        Action::Install(s) => {
            body.push_str(&format!(
                "\n\n{} {} {}",
                style("install").bold(),
                s.crate_name,
                style(format!("· via {}", runner.install_method_label())).dim()
            ));
        }
        Action::Sweep(s) => {
            let scope = if s.candidates.len() == 1 {
                ui::tildify(&s.candidates[0])
            } else {
                format!("a dir you pick · {} options up to ~", s.candidates.len())
            };
            let cmd = if s.sweep_all {
                format!("cargo sweep --all --recursive {}", scope)
            } else {
                format!("cargo sweep --time {} {}", s.time_days, scope)
            };
            body.push_str(&format!("\n\n{} {}", style("run").bold(), style(cmd).dim(),));
        }
        Action::Purge(s) => {
            let scope = if s.candidates.len() == 1 {
                ui::tildify(&s.candidates[0])
            } else {
                format!("a dir you pick · {} options up to ~", s.candidates.len())
            };
            body.push_str(&format!(
                "\n\n{} delete leftover target/ dirs in {}",
                style("run").bold(),
                style(scope).dim(),
            ));
        }
    }

    note(title, body)?;
    Ok(())
}

/// The added/removed lines of the change, color-coded. Git-diff scaffolding
/// (---, +++, @@) and unchanged context are dropped: the file is named just
/// above and the TOML keys are fully qualified, so context only adds noise.
fn render_diff(plan: &TomlPlan) -> String {
    let diff = toml_ops::unified(&plan.before, &plan.after, &filename(&plan.path));
    let mut out: Vec<String> = Vec::new();
    for line in diff.lines() {
        if line.starts_with("---") || line.starts_with("+++") || line.starts_with("@@") {
            continue;
        }
        if let Some(rest) = line.strip_prefix('+') {
            if rest.trim().is_empty() {
                continue;
            }
            out.push(style(format!("+ {rest}")).green().to_string());
        } else if let Some(rest) = line.strip_prefix('-') {
            if rest.trim().is_empty() {
                continue;
            }
            out.push(style(format!("- {rest}")).red().to_string());
        }
    }
    out.join("\n")
}

fn execute(
    sug: &Suggestion,
    plan: Option<&TomlPlan>,
    runner: &mut Runner,
    dry_run: bool,
    yes: bool,
) -> Result<()> {
    match &sug.action {
        Action::Toml(_) => {
            let p = plan.ok_or_else(|| anyhow!("missing toml plan"))?;
            let backup = toml_ops::apply(p, dry_run)?;
            if dry_run {
                log::info(format!("would write {}", ui::tildify(&p.path)))?;
            } else {
                match backup {
                    Some(b) => log::success(format!(
                        "wrote {} {}",
                        ui::tildify(&p.path),
                        style(format!("(backup {})", ui::tildify(&b))).dim()
                    ))?,
                    None => log::success(format!("wrote {}", ui::tildify(&p.path)))?,
                }
            }
        }
        Action::Install(s) => runner.install(s, sug.tag, dry_run)?,
        Action::Sweep(s) => run_sweep(s, sug.tag, runner, dry_run, yes)?,
        Action::Purge(s) => run_purge(s, dry_run, yes)?,
    }
    Ok(())
}

fn filename(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Resolve the sweep's directory, run cargo-sweep there, and report reclaimed space
/// by sizing the target dirs within it before and after. Dry-run only echoes the
/// command; the size pass is skipped because nothing is removed.
fn run_sweep(spec: &SweepSpec, tag: Tag, runner: &Runner, dry_run: bool, yes: bool) -> Result<()> {
    let dir = pick_dir(&spec.candidates, yes, "Which directory to sweep?")?;
    let recursive = dir != spec.candidates[0];
    let run = sweep_runspec(&dir, spec.time_days, recursive, spec.sweep_all);

    if dry_run {
        return runner.run(&run, tag, true);
    }

    let before = measure_targets(&dir, "Scanning target dirs");
    runner.run(&run, tag, false)?;
    let after = measure_targets(&dir, "Re-scanning after sweep");

    let freed = before.saturating_sub(after);
    log::success(format!(
        "{}  {} → {}  {}",
        ui::tildify(&dir),
        human_bytes(before),
        human_bytes(after),
        style(format!("(freed {})", human_bytes(freed)))
            .green()
            .bold(),
    ))?;
    Ok(())
}

/// Delete the leftover per-project target dirs under a chosen directory, now that
/// builds are centralized. Reports how many cargo build targets it finds and their
/// size, then removes them behind an explicit confirm; the configured central dir is
/// always spared. Destructive, so `--yes` reports nothing and deletes nothing.
pub(crate) fn run_purge(spec: &PurgeSpec, dry_run: bool, yes: bool) -> Result<()> {
    let dir = pick_dir(&spec.candidates, yes, "Which directory to clean?")?;

    // Never delete unattended. Under --yes there is no confirm to show, so skip rather
    // than walk a potentially large tree we would not act on.
    if yes && !dry_run {
        log::warning(
            "Skipped target purge: it needs an interactive confirm; re-run without --yes.",
        )?;
        return Ok(());
    }

    let sp = spinner();
    sp.start("Finding cargo target dirs");
    let protected = spec
        .protected
        .as_deref()
        .and_then(|p| std::fs::canonicalize(p).ok());
    let targets: Vec<PathBuf> = system::cargo_build_target_dirs(&dir)
        .into_iter()
        .filter(|t| !is_protected(t, protected.as_deref()))
        .collect();
    let total: u64 = targets
        .iter()
        .filter_map(|t| system::dir_size_bytes(t))
        .sum();
    sp.stop(format!(
        "{} target {} found · {}",
        targets.len(),
        if targets.len() == 1 {
            "directory"
        } else {
            "directories"
        },
        human_bytes(total),
    ));

    if targets.is_empty() {
        log::info("Nothing to reclaim here.")?;
        return Ok(());
    }
    if dry_run {
        log::warning(format!(
            "dry-run: would delete {} dir(s), reclaiming {}",
            targets.len(),
            human_bytes(total),
        ))?;
        return Ok(());
    }

    let go = cliclack::confirm(format!(
        "Delete {} target dir(s) and reclaim {}? Each project rebuilds from scratch next time.",
        targets.len(),
        human_bytes(total),
    ))
    .initial_value(false)
    .interact()?;
    if !go {
        log::info("Left them in place.")?;
        return Ok(());
    }

    let mut freed = 0u64;
    let mut removed = 0usize;
    for t in &targets {
        let size = system::dir_size_bytes(t).unwrap_or(0);
        match std::fs::remove_dir_all(t) {
            Ok(()) => {
                freed += size;
                removed += 1;
            }
            Err(e) => {
                let _ = log::warning(format!("skip {}: {e}", ui::tildify(t)));
            }
        }
    }
    log::success(format!(
        "Removed {removed} target dir(s), reclaimed {}",
        style(human_bytes(freed)).green().bold(),
    ))?;
    Ok(())
}

/// Whether `dir` is the configured central target dir (or sits inside it). The purge
/// must spare it even when it falls within the chosen scope, or it would wipe the very
/// dir builds were just pointed at.
fn is_protected(dir: &Path, protected: Option<&Path>) -> bool {
    match (protected, std::fs::canonicalize(dir).ok()) {
        (Some(p), Some(d)) => d == p || d.starts_with(p),
        _ => false,
    }
}

/// Ask which candidate directory to act on. A single option or a `--yes` run takes
/// the narrowest scope (the project dir) without prompting.
fn pick_dir(candidates: &[PathBuf], yes: bool, prompt: &str) -> Result<PathBuf> {
    if yes || candidates.len() == 1 {
        return Ok(candidates[0].clone());
    }
    let mut menu = select(prompt);
    for (i, dir) in candidates.iter().enumerate() {
        let hint = if i == 0 { "this project" } else { "recursive" };
        menu = menu.item(dir.clone(), ui::tildify(dir), hint);
    }
    menu.interact()
        .map_err(|e| anyhow!("interactive prompt needs a TTY; use --yes: {e}"))
}

fn sweep_runspec(dir: &Path, time_days: u32, recursive: bool, sweep_all: bool) -> RunSpec {
    let mut args = vec!["sweep".into()];
    if sweep_all {
        args.push("--all".into());
        args.push("--recursive".into());
    } else {
        args.push("--time".into());
        args.push(time_days.to_string());
    }
    if recursive && !sweep_all {
        args.push("--recursive".into());
    }
    RunSpec {
        program: "cargo".into(),
        args,
        cwd: Some(dir.to_path_buf()),
    }
}

/// Total bytes of the cargo target dirs under `dir`, the only thing cargo-sweep can
/// reclaim. Sizing just the targets (not the whole selected tree) keeps a wide,
/// recursive sweep root from walking unrelated source and VCS files. Shown via a
/// spinner since a wide root can hold many targets.
fn measure_targets(dir: &Path, label: &str) -> u64 {
    let sp = spinner();
    sp.start(label);
    let targets = system::cargo_target_dirs(dir);
    let bytes: u64 = targets
        .iter()
        .filter_map(|t| system::dir_size_bytes(t))
        .sum();
    sp.stop(format!(
        "{label}: {} across {} target dir(s)",
        human_bytes(bytes),
        targets.len()
    ));
    bytes
}
