//! Best-effort system and project facts. Every probe degrades to None instead of
//! failing, so a missing tool never aborts the report. Memory and disk facts come
//! from sysinfo (native on macOS, Linux, and Windows) rather than shelling out to
//! Unix-only tools, so the same probes work on every supported platform.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use jwalk::WalkDir;
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

/// Total size in bytes of every regular file under `path`, walked in parallel across
/// cores via jwalk. Symlinks are skipped to avoid double-counting and cycles; hidden
/// files (e.g. target/.rustc_info.json) are counted. None when the path is absent.
/// Cross-platform and, on a large tree, ~2.5x faster than single-threaded `du`.
pub fn dir_size_bytes(path: &Path) -> Option<u64> {
    if !path.exists() {
        return None;
    }
    let total = WalkDir::new(path)
        .skip_hidden(false)
        .into_iter()
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_type.is_file())
        .filter_map(|entry| entry.metadata().ok())
        .map(|meta| meta.len())
        .sum();
    Some(total)
}

/// The cargo build directories under `root`, identified by the `CACHEDIR.TAG` marker
/// cargo writes at the root of every `target/`. Descent stops at each match, so a
/// target's contents are not scanned during discovery and a target nested inside
/// another is never counted twice. The walk runs in parallel across cores.
pub fn cargo_target_dirs(root: &Path) -> Vec<PathBuf> {
    let found = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&found);
    WalkDir::new(root)
        .process_read_dir(move |_, dir, _, children| {
            let is_target = children.iter().any(|child| {
                child.as_ref().is_ok_and(|entry| {
                    entry.file_type.is_file() && entry.file_name == *OsStr::new("CACHEDIR.TAG")
                })
            });
            if is_target {
                sink.lock().unwrap().push(dir.to_path_buf());
                for child in children.iter_mut().flatten() {
                    child.read_children_path = None;
                }
            }
        })
        .into_iter()
        .for_each(drop);
    std::mem::take(&mut *found.lock().unwrap())
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

#[cfg(test)]
mod tests {
    use super::{cargo_target_dirs, dir_size_bytes};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A fresh empty directory under the system temp dir, unique per call.
    fn scratch() -> PathBuf {
        static SEQ: AtomicU32 = AtomicU32::new(0);
        let id = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("frd-{}-{id}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, bytes: usize) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, vec![0u8; bytes]).unwrap();
    }

    /// Marks `dir` as a cargo target by writing the CACHEDIR.TAG cargo leaves there.
    fn mark_target(dir: &Path) {
        write(&dir.join("CACHEDIR.TAG"), 177);
    }

    #[test]
    fn dir_size_sums_files_including_hidden() {
        let root = scratch();
        write(&root.join("a.bin"), 1000);
        write(&root.join("sub/b.bin"), 500);
        write(&root.join(".rustc_info.json"), 24); // hidden, still counted
        assert_eq!(dir_size_bytes(&root), Some(1524));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn dir_size_is_none_when_absent() {
        assert_eq!(dir_size_bytes(Path::new("/no/such/frd/path")), None);
    }

    #[test]
    fn discovery_finds_targets_and_prunes_nested() {
        let root = scratch();
        // Two sibling project targets, plus a target nested inside one of them.
        mark_target(&root.join("proj_a/target"));
        write(&root.join("proj_a/target/debug/big.bin"), 1000);
        write(&root.join("proj_a/src/lib.rs"), 50); // source, not a target
        mark_target(&root.join("proj_a/target/nested/target"));
        write(&root.join("proj_a/target/nested/target/x.bin"), 9);
        mark_target(&root.join("proj_b/target"));
        write(&root.join("proj_b/target/y.bin"), 500);

        let mut found = cargo_target_dirs(&root);
        found.sort();
        assert_eq!(
            found,
            vec![root.join("proj_a/target"), root.join("proj_b/target")],
            "nested target must be pruned, source dirs ignored"
        );

        // proj_a/target's size includes the pruned nested target's bytes (counted once).
        let total: u64 = found.iter().filter_map(|t| dir_size_bytes(t)).sum();
        assert_eq!(total, (177 + 1000) + (177 + 9) + (177 + 500));
        fs::remove_dir_all(&root).unwrap();
    }
}
