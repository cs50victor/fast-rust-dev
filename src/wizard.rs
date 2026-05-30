//! The accept/skip wizard: one cliclack card per suggestion (themed by its
//! optimization category), a single Select decision, and apply on accept.

use crate::runner::Runner;
use crate::suggestion::{Action, Suggestion};
use crate::system::SystemReport;
use crate::toml_ops::{self, TomlPlan};
use crate::ui;
use anyhow::{Result, anyhow};
use cliclack::{log, note, select};
use console::style;
use std::path::Path;

pub struct Summary {
    pub applied: usize,
    pub skipped: usize,
    pub failed: usize,
    pub quit: bool,
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
            Decision::Accept => match execute(sug, plan.as_ref(), runner, dry_run) {
                Ok(()) => applied += 1,
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
        Action::Run(s) => {
            body.push_str(&format!("\n\n{} {}", style("run").bold(), s.display()));
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
        Action::Run(s) => runner.run(s, sug.tag, dry_run)?,
    }
    Ok(())
}

fn filename(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}
