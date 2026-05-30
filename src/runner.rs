//! Runs installs and commands with live output. Install path follows the user's rule:
//! use cargo-binstall if present; if absent, ask once whether to install it, otherwise
//! fall back to `cargo install`.

use crate::suggestion::{InstallSpec, RunSpec};
use crate::system;
use anyhow::{Context, Result, bail};
use console::{Term, style};
use std::io::Write;
use std::path::Path;
use std::process::Command;

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

    pub fn install(&mut self, spec: &InstallSpec, dry_run: bool) -> Result<()> {
        if system::have(&spec.bin_name) {
            println!(
                "{}",
                style(format!("  {} already installed", spec.bin_name)).green()
            );
            return Ok(());
        }
        let use_binstall = self.ensure_binstall(dry_run)?;
        let args: Vec<String> = if use_binstall {
            vec!["binstall".into(), "-y".into(), spec.crate_name.clone()]
        } else {
            vec!["install".into(), spec.crate_name.clone()]
        };
        run_streaming("cargo", &args, None, dry_run)
    }

    pub fn run(&self, spec: &RunSpec, dry_run: bool) -> Result<()> {
        run_streaming(&spec.program, &spec.args, spec.cwd.as_deref(), dry_run)
    }

    fn ensure_binstall(&mut self, dry_run: bool) -> Result<bool> {
        match self.binstall {
            Binstall::Available => Ok(true),
            Binstall::Fallback => Ok(false),
            Binstall::Unknown => {
                if dry_run {
                    return Ok(false);
                }
                let install_it = confirm(
                    "cargo-binstall not found. Install it for fast prebuilt binaries? \
                     (otherwise fall back to cargo install)",
                )?;
                if install_it {
                    run_streaming(
                        "cargo",
                        &["install".into(), "cargo-binstall".into()],
                        None,
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

fn confirm(msg: &str) -> Result<bool> {
    print!(
        "  {} {} {} ",
        style("?").yellow(),
        msg,
        style("[y/N]").dim()
    );
    std::io::stdout().flush().ok();
    let term = Term::stdout();
    loop {
        match term.read_char() {
            Ok('y' | 'Y') => {
                println!("y");
                return Ok(true);
            }
            Ok('n' | 'N' | '\n' | '\r') => {
                println!("n");
                return Ok(false);
            }
            Ok(_) => continue,
            Err(e) => bail!("input error (need a TTY): {e}"),
        }
    }
}

fn run_streaming(prog: &str, args: &[String], cwd: Option<&Path>, dry_run: bool) -> Result<()> {
    let shown = format!("{prog} {}", args.join(" "));
    if dry_run {
        println!(
            "{}",
            style(format!("  [dry-run] would run: {shown}")).yellow()
        );
        return Ok(());
    }
    println!("{}", style(format!("  $ {shown}")).dim());
    let mut cmd = Command::new(prog);
    cmd.args(args);
    if let Some(d) = cwd {
        cmd.current_dir(d);
    }
    let status = cmd.status().with_context(|| format!("spawn {prog}"))?;
    if !status.success() {
        bail!("{shown} exited with {status}");
    }
    Ok(())
}
