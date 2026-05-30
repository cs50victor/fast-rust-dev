mod catalog;
mod cli;
mod doctor;
mod runner;
mod suggestion;
mod system;
mod toml_ops;
mod wizard;

use anyhow::Result;
use clap::Parser;
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
    print_report(&report);

    if matches!(args.command, Some(cli::Commands::Report)) {
        return Ok(());
    }

    if matches!(args.command, Some(cli::Commands::Doctor)) {
        if !doctor::run(&report) {
            std::process::exit(1);
        }
        return Ok(());
    }

    if !report.project.has_cargo_toml {
        println!(
            "{}",
            style(format!(
                "No Cargo.toml in {}; only global suggestions apply.",
                root.display()
            ))
            .yellow()
        );
        println!();
    }

    let all = catalog::build(&report);
    let (pending, already): (Vec<Suggestion>, Vec<Suggestion>) = all
        .into_iter()
        .partition(|s| status_of(&report, s) == Status::Pending);

    if !already.is_empty() {
        println!(
            "{}",
            style(format!("Already applied ({}):", already.len())).dim()
        );
        for s in &already {
            println!("{}", style(format!("  + {}", s.title)).dim());
        }
        println!();
    }

    if pending.is_empty() {
        println!(
            "{}",
            style("Nothing to do: your setup already covers the catalog.").green()
        );
        return Ok(());
    }

    if args.dry_run {
        println!(
            "{}",
            style("(dry-run: nothing will be written or run)").yellow()
        );
        println!();
    }

    let mut runner = runner::Runner::new();
    let summary = wizard::run(&report, pending, &mut runner, args.dry_run, args.yes)?;

    println!(
        "{}",
        style(format!(
            "Done: {} applied, {} skipped, {} failed{}.",
            summary.applied,
            summary.skipped,
            summary.failed,
            if summary.quit { " (quit early)" } else { "" }
        ))
        .bold()
    );
    if report.project.target_bytes.is_some() {
        println!(
            "{}",
            style(
                "Note: existing target/ dirs are not moved automatically. Accept the sweep, \
                 or run cargo clean per project, to reclaim space now."
            )
            .dim()
        );
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
        Action::Run(_) => Status::Pending,
    }
}

fn print_report(r: &SystemReport) {
    println!("{}", style("frd  fast rust dev").bold().cyan());

    let ram = r.ram_bytes.map(human_bytes).unwrap_or_else(|| "?".into());
    let disk = match (r.disk_free_bytes, r.disk_used_pct()) {
        (Some(free), Some(pct)) => format!("disk {pct}% used, {} free", human_bytes(free)),
        _ => "disk ?".into(),
    };
    println!(
        "  {} {}, {} cores, {} RAM, {}",
        r.os, r.arch, r.cores, ram, disk
    );

    match &r.rustc_version {
        Some(v) => {
            let note = if r.nightly {
                style(" (nightly: -Z flags available)").green().to_string()
            } else {
                String::new()
            };
            println!("  {v}{note}");
        }
        None => println!("  rustc: not found"),
    }

    let tgt = r
        .project
        .target_bytes
        .map(human_bytes)
        .unwrap_or_else(|| "none".into());
    println!(
        "  project: {}  (target/: {})",
        r.project.root.display(),
        tgt
    );

    let tools = [
        "sccache",
        "cargo-sweep",
        "cargo-machete",
        "cargo-nextest",
        "cargo-binstall",
    ];
    let present: Vec<&str> = tools.iter().copied().filter(|t| system::have(t)).collect();
    let missing: Vec<&str> = tools.iter().copied().filter(|t| !system::have(t)).collect();
    println!(
        "  tools: have [{}]  missing [{}]",
        present.join(", "),
        missing.join(", ")
    );
    println!();
}
