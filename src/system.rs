//! Best-effort system and project facts. Every probe degrades to None instead of
//! failing, so a missing tool never aborts the report. Memory and disk facts come
//! from sysinfo (native on macOS, Linux, and Windows) rather than shelling out to
//! Unix-only tools, so the same probes work on every supported platform.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use sysinfo::{Disks, System};

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
            ram_bytes: total_ram_bytes(),
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
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| {
        if dir.join(tool).is_file() {
            return true;
        }
        // On Windows the binaries cargo installs carry an extension (e.g. sccache.exe),
        // so a bare-name lookup misses them. Probe the usual executable extensions.
        #[cfg(windows)]
        {
            ["exe", "cmd", "bat"]
                .iter()
                .any(|ext| dir.join(format!("{tool}.{ext}")).is_file())
        }
        #[cfg(not(windows))]
        false
    })
}

fn cores() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

/// Total physical RAM in bytes, via sysinfo (works on macOS, Linux, and Windows).
/// sysinfo reports 0 when it cannot read memory, which we treat as unknown.
fn total_ram_bytes() -> Option<u64> {
    let mut sys = System::new();
    sys.refresh_memory();
    let total = sys.total_memory();
    (total > 0).then_some(total)
}

fn rustc_version() -> Option<String> {
    let out = Command::new("rustc").arg("--version").output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Total and free bytes of the filesystem holding `path`, via sysinfo. Picks the
/// mounted disk whose mount point is the longest prefix of `path` (the same disk
/// `df <path>` would report). Returns (None, None) if no mount point matches.
fn disk_total_free(path: &Path) -> (Option<u64>, Option<u64>) {
    // Make the path absolute without resolving symlinks: canonicalize() on Windows
    // returns a `\\?\` verbatim prefix that breaks mount-point prefix matching.
    let abs = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let disks = Disks::new_with_refreshed_list();
    let best = disks
        .list()
        .iter()
        .filter(|d| abs.starts_with(d.mount_point()))
        .max_by_key(|d| d.mount_point().as_os_str().len());
    match best {
        Some(d) => (Some(d.total_space()), Some(d.available_space())),
        None => (None, None),
    }
}

/// Total size in bytes of every regular file under `path`, summed iteratively so a
/// deep tree cannot overflow the stack. Symlinks are skipped to avoid double-counting
/// and cycles. None when the path is absent. Cross-platform replacement for `du -sk`.
pub fn dir_size_bytes(path: &Path) -> Option<u64> {
    if !path.exists() {
        return None;
    }
    let mut total: u64 = 0;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                stack.push(entry.path());
            } else if let Ok(meta) = entry.metadata() {
                total += meta.len();
            }
        }
    }
    Some(total)
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
