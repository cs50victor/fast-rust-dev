//! Format-preserving edits to the three config files, plus already-applied detection,
//! timestamped backups, and unified diffs. All edits go through toml_edit so existing
//! comments, ordering, and whitespace survive.

use crate::suggestion::{Scope, TomlChange, TomlOp, TomlValue};
use crate::system::SystemReport;
use anyhow::{Context, Result};
use similar::TextDiff;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use toml_edit::{Array, DocumentMut, Item, Table, value};

pub struct TomlPlan {
    pub path: PathBuf,
    pub before: String,
    pub after: String,
}

pub fn plan(report: &SystemReport, change: &TomlChange) -> Result<TomlPlan> {
    let path = target_path(report, change.scope);
    let before = fs::read_to_string(&path).unwrap_or_default();
    let mut doc: DocumentMut = before
        .parse()
        .with_context(|| format!("parse {}", path.display()))?;
    for op in &change.ops {
        set_op(&mut doc, op);
    }
    let after = doc.to_string();
    Ok(TomlPlan {
        path,
        before,
        after,
    })
}

pub fn apply(plan: &TomlPlan, dry_run: bool) -> Result<Option<PathBuf>> {
    if dry_run {
        return Ok(None);
    }
    if let Some(parent) = plan.path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).with_context(|| format!("mkdir {}", parent.display()))?;
    }
    let backup = if plan.path.exists() {
        let b = backup_path(&plan.path);
        fs::copy(&plan.path, &b).with_context(|| format!("backup {}", plan.path.display()))?;
        Some(b)
    } else {
        None
    };
    fs::write(&plan.path, &plan.after).with_context(|| format!("write {}", plan.path.display()))?;
    Ok(backup)
}

pub fn is_applied(report: &SystemReport, change: &TomlChange) -> bool {
    let path = target_path(report, change.scope);
    let Ok(text) = fs::read_to_string(&path) else {
        return false;
    };
    let Ok(doc) = text.parse::<DocumentMut>() else {
        return false;
    };
    change.ops.iter().all(|op| is_satisfied(&doc, op))
}

pub fn unified(before: &str, after: &str, label: &str) -> String {
    let diff = TextDiff::from_lines(before, after);
    let mut ud = diff.unified_diff();
    ud.context_radius(3);
    ud.header(&format!("a/{label}"), &format!("b/{label}"));
    ud.to_string()
}

fn target_path(report: &SystemReport, scope: Scope) -> PathBuf {
    match scope {
        Scope::Global => report.project.global_cargo_config.clone(),
        Scope::ProjectCargo => report.project.cargo_toml.clone(),
        Scope::ProjectConfig => report.project.cargo_config.clone(),
    }
}

fn set_op(doc: &mut DocumentMut, op: &TomlOp) {
    let Some((key, tables)) = op.path.split_last() else {
        return;
    };
    // Build real (header) tables, not inline ones. Walk the path with the entry API,
    // creating each missing segment as Item::Table. Pure intermediate parents are marked
    // implicit so an empty `[profile]` header is never emitted; the leaf table that holds
    // the key stays explicit so its `[profile.dev]` header is rendered.
    let mut tbl: &mut Table = doc.as_table_mut();
    for (i, name) in tables.iter().enumerate() {
        let is_leaf = i == tables.len() - 1;
        let existed = tbl.get(name).is_some();
        let segment = tbl
            .entry(name)
            .or_insert_with(|| Item::Table(Table::new()))
            .as_table_mut()
            .expect("config path segment is not a table");
        if is_leaf {
            segment.set_implicit(false);
        } else if !existed {
            segment.set_implicit(true);
        }
        tbl = segment;
    }

    let new_item = match &op.value {
        TomlValue::AppendFlags(flags) => {
            let existing: Vec<String> = tbl
                .get(key)
                .and_then(|i| i.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let mut arr = Array::new();
            for f in &existing {
                arr.push(f.as_str());
            }
            for f in flags {
                if !existing.iter().any(|e| e == f) {
                    arr.push(f.as_str());
                }
            }
            value(arr)
        }
        TomlValue::Str(s) => value(s.clone()),
        TomlValue::Int(n) => value(*n),
        TomlValue::Bool(b) => value(*b),
    };
    tbl[key.as_str()] = new_item;
}

fn is_satisfied(doc: &DocumentMut, op: &TomlOp) -> bool {
    let Some(item) = get_path(doc, &op.path) else {
        return false;
    };
    match &op.value {
        TomlValue::Str(s) => item.as_str() == Some(s.as_str()),
        TomlValue::Int(n) => item.as_integer() == Some(*n),
        TomlValue::Bool(b) => item.as_bool() == Some(*b),
        TomlValue::AppendFlags(flags) => item
            .as_array()
            .map(|a| {
                flags
                    .iter()
                    .all(|f| a.iter().any(|v| v.as_str() == Some(f.as_str())))
            })
            .unwrap_or(false),
    }
}

fn get_path<'a>(doc: &'a DocumentMut, path: &[String]) -> Option<&'a Item> {
    let mut item = doc.as_table().get(&path[0])?;
    for name in &path[1..] {
        item = item.as_table()?.get(name)?;
    }
    Some(item)
}

fn backup_path(p: &Path) -> PathBuf {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut name = p.as_os_str().to_os_string();
    name.push(format!(".frd-bak-{ts}"));
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::suggestion::{Scope, TomlChange, TomlOp, TomlValue};
    use crate::system::{Project, SystemReport};
    use std::path::Path;

    fn report_for(dir: &Path) -> SystemReport {
        SystemReport {
            os: "macos",
            arch: "aarch64",
            cores: 8,
            ram_bytes: None,
            disk_total_bytes: None,
            disk_free_bytes: None,
            rustc_version: None,
            nightly: true,
            project: Project {
                root: dir.to_path_buf(),
                cargo_toml: dir.join("Cargo.toml"),
                has_cargo_toml: true,
                cargo_config: dir.join(".cargo").join("config.toml"),
                target_bytes: None,
                global_cargo_config: dir.join("global-config.toml"),
            },
        }
    }

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("frd-test-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(d.join(".cargo")).unwrap();
        d
    }

    #[test]
    fn nested_tables_and_quoted_star_render() {
        let dir = temp("dev");
        let r = report_for(&dir);
        let change = TomlChange {
            scope: Scope::ProjectCargo,
            ops: vec![
                TomlOp::new(
                    &["profile", "dev", "debug"],
                    TomlValue::Str("line-tables-only".into()),
                ),
                TomlOp::new(
                    &["profile", "dev", "package", "*", "opt-level"],
                    TomlValue::Int(2),
                ),
            ],
        };
        let p = plan(&r, &change).unwrap();
        assert!(p.after.contains("[profile.dev]"), "got:\n{}", p.after);
        assert!(
            p.after.contains("[profile.dev.package.\"*\"]"),
            "expected header table for quoted star key:\n{}",
            p.after
        );
        assert!(
            p.after.contains("debug = \"line-tables-only\""),
            "got:\n{}",
            p.after
        );
        assert!(p.after.contains("opt-level = 2"), "got:\n{}", p.after);
        assert!(!p.after.contains("= {"), "must not be inline:\n{}", p.after);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rustflags_append_is_idempotent_and_detected() {
        let dir = temp("flags");
        let r = report_for(&dir);
        let change = TomlChange {
            scope: Scope::ProjectConfig,
            ops: vec![TomlOp::new(
                &["build", "rustflags"],
                TomlValue::AppendFlags(vec!["-Zthreads=0".into(), "-Zshare-generics=y".into()]),
            )],
        };
        let p = plan(&r, &change).unwrap();
        assert!(p.after.contains("[build]"), "got:\n{}", p.after);
        assert!(p.after.contains("-Zthreads=0"), "got:\n{}", p.after);
        assert!(p.after.contains("-Zshare-generics=y"), "got:\n{}", p.after);

        std::fs::write(&r.project.cargo_config, &p.after).unwrap();
        assert!(
            is_applied(&r, &change),
            "is_applied should be true after write"
        );

        let p2 = plan(&r, &change).unwrap();
        assert_eq!(p.after, p2.after, "re-applying must not duplicate flags");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn preserves_existing_content_and_comments() {
        let dir = temp("preserve");
        let r = report_for(&dir);
        std::fs::write(
            &r.project.cargo_toml,
            "# keep me\n[package]\nname = \"x\"\n",
        )
        .unwrap();
        let change = TomlChange {
            scope: Scope::ProjectCargo,
            ops: vec![TomlOp::new(
                &["profile", "release", "strip"],
                TomlValue::Bool(true),
            )],
        };
        let p = plan(&r, &change).unwrap();
        assert!(
            p.after.contains("# keep me"),
            "comment dropped:\n{}",
            p.after
        );
        assert!(
            p.after.contains("name = \"x\""),
            "package lost:\n{}",
            p.after
        );
        assert!(p.after.contains("[profile.release]"), "got:\n{}", p.after);
        assert!(p.after.contains("strip = true"), "got:\n{}", p.after);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
