mod catalog;
mod cli;
mod doctor;
mod runner;
mod suggestion;
mod system;
mod toml_ops;
mod ui;
mod wizard;

use anyhow::Result;
use clap::Parser;
use cliclack::{intro, log, outro, outro_cancel};
use console::style;
use suggestion::{Action, Status, Suggestion};
use system::{SystemReport, human_bytes};

fn main() -> Result<()> {
    let args = cli::Cli::parse();
    let root = match &args.root {
        Some(r) => r.clone(),
        None => std::env::current_dir()?,
    };
    let report = SystemReport::gather(&root);

    ui::reset();
    let _ = intro(style(" frd · fast rust dev ").black().on_cyan().bold());
    print_report(&report);

    if matches!(args.command, Some(cli::Commands::Report)) {
        let _ = outro(style("read-only report").dim());
        return Ok(());
    }

    if matches!(args.command, Some(cli::Commands::Doctor)) {
        if !doctor::run(&report) {
            std::process::exit(1);
        }
        return Ok(());
    }

    if !report.project.has_cargo_toml {
        let _ = log::warning(format!(
            "No Cargo.toml in {} — only global suggestions apply.",
            ui::tildify(&root)
        ));
    }

    let all = catalog::build(&report);
    let (pending, already): (Vec<Suggestion>, Vec<Suggestion>) = all
        .into_iter()
        .partition(|s| status_of(&report, s) == Status::Pending);

    if !already.is_empty() {
        let titles = already
            .iter()
            .map(|s| s.title.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let _ = log::info(format!(
            "Already tuned ({}): {}",
            already.len(),
            style(titles).dim()
        ));
    }

    if pending.is_empty() {
        let _ = outro(style("Nothing to do — your setup already covers the catalog.").green());
        return Ok(());
    }

    if args.dry_run {
        let _ = log::warning("dry-run — nothing will be written or run");
    }

    let mut runner = runner::Runner::new();
    let summary = wizard::run(&report, pending, &mut runner, args.dry_run, args.yes)?;

    if report.project.target_bytes.is_some() {
        let _ = log::remark(
            "Existing target/ dirs are not moved automatically. Accept the sweep, or run \
             cargo clean per project, to reclaim space now.",
        );
    }

    let counts = format!(
        "{} applied   {} skipped   {} failed",
        style(summary.applied).green().bold(),
        style(summary.skipped).dim(),
        if summary.failed > 0 {
            style(summary.failed).red().bold().to_string()
        } else {
            style(summary.failed).dim().to_string()
        }
    );
    if summary.quit {
        let _ = outro_cancel(format!("Stopped early   {counts}"));
    } else {
        let _ = outro(counts);
    }
    Ok(())
}

pub(crate) fn status_of(r: &SystemReport, s: &Suggestion) -> Status {
    match &s.action {
        Action::Toml(c) => {
            if toml_ops::is_applied(r, c) {
                Status::AlreadyApplied
            } else {
                Status::Pending
            }
        }
        Action::Install(spec) => {
            if system::have(&spec.bin_name) {
                Status::AlreadyApplied
            } else {
                Status::Pending
            }
        }
        Action::Sweep(_) => Status::Pending,
    }
}

/// One report fact as a cliclack info line with a fixed-width bold label, so the
/// values line up into a scannable left edge.
fn fact(label: &str, value: String) {
    let _ = log::info(format!("{}{}", style(format!("{label:<8}")).bold(), value));
}

fn print_report(r: &SystemReport) {
    let ram = r.ram_bytes.map(human_bytes).unwrap_or_else(|| "?".into());
    fact(
        "System",
        format!("{} {} · {} cores · {} RAM", r.os, r.arch, r.cores, ram),
    );

    match (r.disk_free_bytes, r.disk_used_pct()) {
        (Some(free), Some(pct)) => fact(
            "Disk",
            format!(
                "{} used · {} free",
                ui::disk_style(pct).apply_to(format!("{pct}%")),
                human_bytes(free)
            ),
        ),
        _ => fact("Disk", style("unknown").dim().to_string()),
    }

    match &r.rustc_version {
        // The "Rust" label already says what this is, so drop rustc's own prefix.
        Some(v) => {
            let v = v.strip_prefix("rustc ").unwrap_or(v);
            if r.nightly {
                fact(
                    "Rust",
                    format!("{v}  {}", style("· -Z flags ready").green()),
                );
            } else {
                fact("Rust", v.to_string());
            }
        }
        None => fact("Rust", style("not found").red().to_string()),
    }

    let tgt = r
        .project
        .target_bytes
        .map(human_bytes)
        .unwrap_or_else(|| "none".into());
    fact(
        "Project",
        format!("{} · target/ {tgt}", ui::tildify(&r.project.root)),
    );

    let tools = [
        "sccache",
        "cargo-sweep",
        "cargo-machete",
        "cargo-nextest",
        "cargo-binstall",
    ];
    let have: Vec<String> = tools
        .iter()
        .filter(|t| system::have(t))
        .map(|t| format!("{} {t}", ui::check()))
        .collect();
    let miss: Vec<String> = tools
        .iter()
        .filter(|t| !system::have(t))
        .map(|t| format!("{} {}", ui::cross(), style(t).dim()))
        .collect();
    let tools_line = match (have.is_empty(), miss.is_empty()) {
        (true, _) => miss.join("  "),
        (_, true) => have.join("  "),
        _ => format!(
            "{}   {}   {}",
            have.join("  "),
            style("·").dim(),
            miss.join("  ")
        ),
    };
    fact("Tools", tools_line);
}
