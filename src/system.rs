//! Best-effort system and project facts. Every probe degrades to None instead of
//! failing, so a missing tool never aborts the report.

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone)]
pub struct SystemReport {
    pub os: &'static str,
    pub arch: &'static str,
    pub cores: usize,
    pub ram_bytes: Option<u64>,
    pub disk_total_bytes: Option<u64>,
    pub disk_free_bytes: Option<u64>,
    pub rustc_version: Option<String>,
    pub nightly: bool,
    pub project: Project,
}

#[derive(Debug, Clone)]
pub struct Project {
    pub root: PathBuf,
    pub cargo_toml: PathBuf,
    pub has_cargo_toml: bool,
    pub cargo_config: PathBuf,
    pub target_bytes: Option<u64>,
    pub global_cargo_config: PathBuf,
}

impl SystemReport {
    pub fn gather(root: &Path) -> Self {
        let cargo_toml = root.join("Cargo.toml");
        let project = Project {
            has_cargo_toml: cargo_toml.is_file(),
            cargo_toml,
            cargo_config: root.join(".cargo").join("config.toml"),
            target_bytes: dir_size_bytes(&root.join("target")),
            global_cargo_config: global_cargo_config(),
            root: root.to_path_buf(),
        };
        let rustc_version = rustc_version();
        let nightly = rustc_version
            .as_deref()
            .is_some_and(|v| v.contains("nightly"));
        let (disk_total_bytes, disk_free_bytes) = disk_total_free(root);
        SystemReport {
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            cores: cores(),
            ram_bytes: sysctl_u64("hw.memsize"),
            disk_total_bytes,
            disk_free_bytes,
            rustc_version,
            nightly,
            project,
        }
    }

    pub fn is_macos(&self) -> bool {
        self.os == "macos"
    }

    pub fn disk_used_pct(&self) -> Option<u64> {
        let total = self.disk_total_bytes?;
        let free = self.disk_free_bytes?;
        if total == 0 {
            return None;
        }
        Some(((total - free) * 100) / total)
    }
}

/// True if `tool` resolves to a file on PATH. Cheap enough to call ad hoc.
pub fn have(tool: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|d| d.join(tool).is_file()))
        .unwrap_or(false)
}

fn cores() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

fn sysctl_u64(key: &str) -> Option<u64> {
    let out = Command::new("sysctl").arg("-n").arg(key).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

fn rustc_version() -> Option<String> {
    let out = Command::new("rustc").arg("--version").output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// `df -Pk <path>` data row: Filesystem 1024-blocks Used Available Capacity Mount.
fn disk_total_free(path: &Path) -> (Option<u64>, Option<u64>) {
    let Ok(out) = Command::new("df").arg("-Pk").arg(path).output() else {
        return (None, None);
    };
    if !out.status.success() {
        return (None, None);
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let Some(row) = text.lines().nth(1) else {
        return (None, None);
    };
    let f: Vec<&str> = row.split_whitespace().collect();
    let kib = |i: usize| {
        f.get(i)
            .and_then(|s| s.parse::<u64>().ok())
            .map(|k| k * 1024)
    };
    (kib(1), kib(3))
}

/// `du -sk <path>` first column (KiB). None when the path is absent.
fn dir_size_bytes(path: &Path) -> Option<u64> {
    if !path.exists() {
        return None;
    }
    let out = Command::new("du").arg("-sk").arg(path).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    text.split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()
        .map(|k| k * 1024)
}

fn global_cargo_config() -> PathBuf {
    if let Some(home) = std::env::var_os("CARGO_HOME") {
        return PathBuf::from(home).join("config.toml");
    }
    dirs::home_dir()
        .unwrap_or_default()
        .join(".cargo")
        .join("config.toml")
}

pub fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}
