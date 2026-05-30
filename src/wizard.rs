//! The accept/skip wizard: one card per suggestion, single-key decision, apply on accept.

use crate::runner::Runner;
use crate::suggestion::{Action, Suggestion, Tag};
use crate::system::SystemReport;
use crate::toml_ops::{self, TomlPlan};
use anyhow::{Result, anyhow};
use console::{StyledObject, Term, style};
use std::io::Write;
use std::path::Path;

pub struct Summary {
    pub applied: usize,
    pub skipped: usize,
    pub failed: usize,
    pub quit: bool,
}

pub fn run(
    report: &SystemReport,
    suggestions: Vec<Suggestion>,
    runner: &mut Runner,
    dry_run: bool,
    yes: bool,
) -> Result<Summary> {
    let term = Term::stdout();
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
        print_card(i + 1, total, sug, plan.as_ref(), runner);

        let decision = if yes { 'a' } else { read_decision(&term)? };
        match decision {
            'a' => {
                println!();
                match execute(sug, plan.as_ref(), runner, dry_run) {
                    Ok(()) => applied += 1,
                    Err(e) => {
                        println!("{}", style(format!("  failed: {e:#}")).red());
                        failed += 1;
                    }
                }
            }
            'q' => {
                println!();
                return Ok(Summary {
                    applied,
                    skipped,
                    failed,
                    quit: true,
                });
            }
            _ => {
                println!("{}", style("  skipped").dim());
                skipped += 1;
            }
        }
        println!();
    }

    Ok(Summary {
        applied,
        skipped,
        failed,
        quit: false,
    })
}

fn read_decision(term: &Term) -> Result<char> {
    loop {
        match term.read_char() {
            Ok('a' | 'A') => return Ok('a'),
            Ok('s' | 'S') => return Ok('s'),
            Ok('q' | 'Q') => return Ok('q'),
            Ok(_) => continue,
            Err(e) => {
                return Err(anyhow!(
                    "input error (need a TTY; use --yes for non-interactive): {e}"
                ));
            }
        }
    }
}

fn print_card(n: usize, total: usize, sug: &Suggestion, plan: Option<&TomlPlan>, runner: &Runner) {
    println!(
        "{} {}  {}  {}",
        style(format!("[{n}/{total}]")).dim(),
        style(&sug.title).bold(),
        tag_style(sug.tag),
        style(sug.id).dim(),
    );
    for line in wrap(&sug.why, 76) {
        println!("  {line}");
    }
    match &sug.action {
        Action::Toml(c) => {
            println!("  {} {}", style("target:").dim(), c.scope.label());
            if let Some(p) = plan {
                print_diff(&toml_ops::unified(&p.before, &p.after, &filename(&p.path)));
            }
        }
        Action::Install(s) => {
            println!(
                "  {} {} (via {})",
                style("install:").dim(),
                s.crate_name,
                runner.install_method_label()
            );
        }
        Action::Run(s) => {
            println!("  {} {}", style("run:").dim(), s.display());
        }
    }
    print!("  {}  ", style("[a]ccept  [s]kip  [q]uit").dim());
    let _ = std::io::stdout().flush();
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
                println!(
                    "{}",
                    style(format!("  [dry-run] would write {}", p.path.display())).yellow()
                );
            } else {
                let msg = match backup {
                    Some(b) => format!("  wrote {} (backup {})", p.path.display(), b.display()),
                    None => format!("  wrote {}", p.path.display()),
                };
                println!("{}", style(msg).green());
            }
        }
        Action::Install(s) => runner.install(s, dry_run)?,
        Action::Run(s) => runner.run(s, dry_run)?,
    }
    Ok(())
}

fn tag_style(tag: Tag) -> StyledObject<&'static str> {
    let l = tag.label();
    match tag {
        Tag::Disk => style(l).yellow(),
        Tag::Speed => style(l).cyan(),
        Tag::Both => style(l).green(),
    }
}

fn print_diff(diff: &str) {
    for line in diff.lines() {
        let styled = if line.starts_with('+') {
            style(line).green()
        } else if line.starts_with('-') {
            style(line).red()
        } else if line.starts_with('@') {
            style(line).cyan()
        } else {
            style(line).dim()
        };
        println!("  {styled}");
    }
}

fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        if !cur.is_empty() && cur.len() + 1 + word.len() > width {
            lines.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

fn filename(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}
