//! Declarative description of every optimization the wizard can offer. The catalog
//! builds these; toml_ops and runner interpret the `Action`.

use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tag {
    Disk,
    Speed,
    Both,
}

impl Tag {
    pub fn label(self) -> &'static str {
        match self {
            Tag::Disk => "DISK",
            Tag::Speed => "SPEED",
            Tag::Both => "BOTH",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Global,
    ProjectCargo,
    ProjectConfig,
}

impl Scope {
    pub fn label(self) -> &'static str {
        match self {
            Scope::Global => "~/.cargo/config.toml",
            Scope::ProjectCargo => "./Cargo.toml",
            Scope::ProjectConfig => "./.cargo/config.toml",
        }
    }
}

#[derive(Clone)]
pub enum TomlValue {
    Str(String),
    Int(i64),
    Bool(bool),
    /// Append flags to a string array (e.g. build.rustflags), skipping duplicates.
    AppendFlags(Vec<String>),
}

#[derive(Clone)]
pub struct TomlOp {
    pub path: Vec<String>,
    pub value: TomlValue,
}

impl TomlOp {
    pub fn new(path: &[&str], value: TomlValue) -> Self {
        TomlOp {
            path: path.iter().map(|s| s.to_string()).collect(),
            value,
        }
    }
}

#[derive(Clone)]
pub struct TomlChange {
    pub scope: Scope,
    pub ops: Vec<TomlOp>,
}

#[derive(Clone)]
pub struct InstallSpec {
    pub crate_name: String,
    pub bin_name: String,
}

#[derive(Clone)]
pub struct RunSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
}

impl RunSpec {
    pub fn display(&self) -> String {
        let mut parts = vec![self.program.clone()];
        parts.extend(self.args.iter().cloned());
        parts.join(" ")
    }
}

/// A cargo-sweep run whose target directory the wizard resolves interactively: the
/// user picks one of `candidates` (the project dir up to the home dir) at accept
/// time, so one suggestion can sweep a single repo or a whole tree.
#[derive(Clone)]
pub struct SweepSpec {
    pub candidates: Vec<PathBuf>,
    pub time_days: u32,
}

impl SweepSpec {
    /// The base command, without the directory (resolved interactively at accept
    /// time). Used by the doctor's read-only maintenance listing.
    pub fn display(&self) -> String {
        format!("cargo sweep --time {}", self.time_days)
    }
}

#[derive(Clone)]
pub enum Action {
    Toml(TomlChange),
    Install(InstallSpec),
    Sweep(SweepSpec),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Pending,
    AlreadyApplied,
}

#[derive(Clone)]
pub struct Suggestion {
    pub title: String,
    pub tag: Tag,
    pub why: String,
    pub action: Action,
}
