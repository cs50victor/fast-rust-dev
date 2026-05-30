//! The `doctor` command: a read-only audit of which catalog optimizations are already
//! applied on this machine and project. Writes and runs nothing, and returns false when
//! any state-based optimization is still pending so the caller can exit non-zero, which
//! makes it usable as a CI or pre-commit check.

use crate::catalog;
use crate::status_of;
use crate::suggestion::{Action, Status, Suggestion};
use crate::system::SystemReport;
use console::{StyledObject, style};

/// Print the checkup. Returns true when every state-based optimization is applied.
pub fn run(report: &SystemReport) -> bool {
    // Run actions (e.g. sweeping stale artifacts) are recurring maintenance, not a state we
    // can detect as "done", so they never count toward the verdict. Check only the config-
    // and tool-based items, which status_of can classify as applied or pending.
    let (maintenance, checkable): (Vec<Suggestion>, Vec<Suggestion>) = catalog::build(report)
        .into_iter()
        .partition(|s| matches!(s.action, Action::Run(_)));

    println!("{}", style("Doctor: optimization checkup").bold().cyan());
    let mut pending = 0usize;
    for s in &checkable {
        let applied = status_of(report, s) == Status::AlreadyApplied;
        if !applied {
            pending += 1;
        }
        println!("  {} {}  {}", mark(applied), tag(s), s.title);
    }

    if !maintenance.is_empty() {
        println!();
        println!("{}", style("Maintenance (re-run anytime):").dim());
        for s in &maintenance {
            if let Action::Run(spec) = &s.action {
                println!(
                    "  {} {}",
                    style(format!("{}:", s.title)).dim(),
                    style(spec.display()).dim()
                );
            }
        }
    }

    println!();
    let total = checkable.len();
    if pending == 0 {
        println!(
            "{}",
            style(format!("All {total} optimizations applied; nothing to do.")).green()
        );
        true
    } else {
        println!(
            "{}",
            style(format!(
                "{}/{total} applied, {pending} pending. Run `frd` to apply them.",
                total - pending
            ))
            .yellow()
        );
        false
    }
}

fn mark(applied: bool) -> StyledObject<&'static str> {
    if applied {
        style("[ok]").green().bold()
    } else {
        style("[--]").red().bold()
    }
}

fn tag(s: &Suggestion) -> StyledObject<String> {
    style(format!("{:<5}", s.tag.label())).dim()
}
