//! The catalog of optimizations, gated to what applies to the live system. Each entry
//! is declarative; toml_ops and runner carry it out. Numbers in the rationale come from
//! the live report so the user sees why an item matters on their machine.

use crate::suggestion::{
    Action, InstallSpec, RunSpec, Scope, Suggestion, Tag, TomlChange, TomlOp, TomlValue,
};
use crate::system::{SystemReport, have, human_bytes};
use std::path::PathBuf;

pub fn build(r: &SystemReport) -> Vec<Suggestion> {
    let mut out = Vec::new();
    let p = &r.project;

    let disk_ctx = match (r.disk_free_bytes, r.disk_used_pct()) {
        (Some(free), Some(pct)) => format!("Disk is {pct}% full, {} free.", human_bytes(free)),
        _ => String::new(),
    };

    // --- global: ~/.cargo/config.toml ---
    out.push(toml_sug(
        "Shared target dir for every project and worktree",
        Tag::Disk,
        format!(
            "{disk_ctx} Point all builds at one dir so repos and git worktrees stop \
             duplicating target/. The single biggest lever on disk sprawl."
        ),
        Scope::Global,
        vec![TomlOp::new(
            &["build", "target-dir"],
            TomlValue::Str(shared_target_dir()),
        )],
    ));

    if r.nightly {
        out.push(toml_sug(
            "Stop duplicating crate metadata into rlibs",
            Tag::Disk,
            "Nightly Cargo flag: roughly 5-35% smaller target/ by not embedding metadata \
             in every .rlib."
                .into(),
            Scope::Global,
            vec![TomlOp::new(
                &["unstable", "no-embed-metadata"],
                TomlValue::Bool(true),
            )],
        ));
    }

    // Offer the wrapper only once sccache exists: setting rustc-wrapper without the binary
    // breaks every cargo build globally. On a fresh machine you install sccache first, then
    // re-run frd and the wrapper appears.
    if have("sccache") {
        out.push(toml_sug(
            "Route rustc through sccache",
            Tag::Both,
            "Caveat: this disables incremental compilation, so it helps cold and cross-project \
             builds more than tight single-crate edit loops."
                .into(),
            Scope::Global,
            vec![TomlOp::new(
                &["build", "rustc-wrapper"],
                TomlValue::Str("sccache".into()),
            )],
        ));
    } else {
        out.push(install_sug(
            "Install sccache (cross-project compile cache)",
            Tag::Both,
            "Caches compiled crates across every project and survives cargo clean. Best for \
             cold and cross-project builds. Re-run frd afterward to route rustc through it."
                .into(),
            "sccache",
            "sccache",
        ));
    }

    // --- project: ./Cargo.toml profiles ---
    if p.has_cargo_toml {
        out.push(toml_sug(
            "dev profile: debug = line-tables-only",
            Tag::Disk,
            "Debug info is the biggest single contributor to target/ size. line-tables-only \
             keeps readable backtraces while dropping most of the bytes. Oxide ships this."
                .into(),
            Scope::ProjectCargo,
            vec![TomlOp::new(
                &["profile", "dev", "debug"],
                TomlValue::Str("line-tables-only".into()),
            )],
        ));

        if r.is_macos() {
            out.push(toml_sug(
                "dev profile: split-debuginfo = unpacked (macOS)",
                Tag::Speed,
                "On macOS this speeds relinking in the edit loop by keeping debug info out \
                 of the main object file."
                    .into(),
                Scope::ProjectCargo,
                vec![TomlOp::new(
                    &["profile", "dev", "split-debuginfo"],
                    TomlValue::Str("unpacked".into()),
                )],
            ));
        }

        out.push(toml_sug(
            "dev profile: opt-level = 2 for dependencies",
            Tag::Speed,
            "Compile dependencies optimized while your own crate stays at 0: snappier dev \
             binaries without slowing your edit-compile loop."
                .into(),
            Scope::ProjectCargo,
            vec![TomlOp::new(
                &["profile", "dev", "package", "*", "opt-level"],
                TomlValue::Int(2),
            )],
        ));

        out.push(toml_sug(
            "Add a disk-light fast-build profile",
            Tag::Both,
            "A profile with no debug info for when you do not need a debugger: smallest \
             output and fastest link. Build with: cargo build --profile fast-build."
                .into(),
            Scope::ProjectCargo,
            vec![
                TomlOp::new(
                    &["profile", "fast-build", "inherits"],
                    TomlValue::Str("dev".into()),
                ),
                TomlOp::new(&["profile", "fast-build", "debug"], TomlValue::Int(0)),
                TomlOp::new(
                    &["profile", "fast-build", "strip"],
                    TomlValue::Str("debuginfo".into()),
                ),
            ],
        ));

        out.push(toml_sug(
            "release profile: strip = true",
            Tag::Disk,
            "Strip symbols and debug info from shipped binaries, shrinking target/release.".into(),
            Scope::ProjectCargo,
            vec![TomlOp::new(
                &["profile", "release", "strip"],
                TomlValue::Bool(true),
            )],
        ));
    }

    // --- project: ./.cargo/config.toml nightly rustflags ---
    // Kept project-local on purpose: a global rustflags entry OVERRIDES (not merges with)
    // a repo's own flags, which is the trap Oxide documents.
    if r.nightly && p.has_cargo_toml {
        out.push(toml_sug(
            "Nightly: parallel frontend + share-generics",
            Tag::Speed,
            "-Zthreads=0 parallelizes the compiler frontend; -Zshare-generics=y cuts \
             duplicate monomorphization across crates."
                .into(),
            Scope::ProjectConfig,
            vec![TomlOp::new(
                &["build", "rustflags"],
                TomlValue::AppendFlags(vec!["-Zthreads=0".into(), "-Zshare-generics=y".into()]),
            )],
        ));
    }

    // --- tools ---
    if !have("cargo-sweep") {
        out.push(install_sug(
            "Install cargo-sweep (disk reclaim)",
            Tag::Disk,
            "Garbage-collects stale build artifacts that Cargo never removes, by age, instead \
             of the all-or-nothing cargo clean."
                .into(),
            "cargo-sweep",
            "cargo-sweep",
        ));
    }
    out.push(run_sug(
        "Sweep stale artifacts in this project",
        Tag::Disk,
        "Removes artifacts untouched for more than 15 days while keeping warm ones. Re-run \
         with --recursive ~/dev to sweep every repo. Needs cargo-sweep."
            .into(),
        "cargo",
        vec!["sweep".into(), "--time".into(), "15".into()],
        Some(p.root.clone()),
    ));

    if !have("cargo-machete") {
        out.push(install_sug(
            "Install cargo-machete (find unused deps)",
            Tag::Both,
            "Finds dependencies you no longer use. Fewer deps means less to compile and store."
                .into(),
            "cargo-machete",
            "cargo-machete",
        ));
    }

    out
}

fn shared_target_dir() -> String {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".cache")
        .join("cargo-target")
        .display()
        .to_string()
}

fn toml_sug(title: &str, tag: Tag, why: String, scope: Scope, ops: Vec<TomlOp>) -> Suggestion {
    Suggestion {
        title: title.into(),
        tag,
        why,
        action: Action::Toml(TomlChange { scope, ops }),
    }
}

fn install_sug(title: &str, tag: Tag, why: String, crate_name: &str, bin: &str) -> Suggestion {
    Suggestion {
        title: title.into(),
        tag,
        why,
        action: Action::Install(InstallSpec {
            crate_name: crate_name.into(),
            bin_name: bin.into(),
        }),
    }
}

fn run_sug(
    title: &str,
    tag: Tag,
    why: String,
    program: &str,
    args: Vec<String>,
    cwd: Option<PathBuf>,
) -> Suggestion {
    Suggestion {
        title: title.into(),
        tag,
        why,
        action: Action::Run(RunSpec {
            program: program.into(),
            args,
            cwd,
        }),
    }
}
