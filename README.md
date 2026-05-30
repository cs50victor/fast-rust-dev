<div align="center">
  <h1>frd — fast rust dev</h1>
  <p><b>Interactive optimizer for local Rust builds.</b></p>
</div>

`frd` reads your machine and project, then walks you through a catalog of build-speed
and `target/`-shrinking changes one at a time. You accept or skip each one; nothing is
written without your say-so.

Works on macOS, Linux, and Windows. It works best on macOS, where every probe and
suggestion applies; on other platforms the catalog narrows to what fits (for example,
the macOS-only `split-debuginfo` tweak is hidden).

## Install

Prebuilt binary (no compile), via [cargo-binstall](https://github.com/cargo-bins/cargo-binstall):

```sh
cargo binstall frd
```

From source:

```sh
cargo install frd
```

Prebuilt binaries for macOS (Apple Silicon + Intel), Linux (x86_64 + arm64), and
Windows (x86_64) are attached to each [GitHub release](https://github.com/cs50victor/fast-rust-dev/releases).

## Usage

Run it inside a Cargo project (or anywhere, for the global suggestions):

```sh
frd              # report, then the interactive wizard
frd report       # print the system and project report, then exit
frd doctor       # audit which optimizations are applied; exit non-zero if any are pending
frd --dry-run    # show every change as a diff, write and run nothing
frd --yes        # accept every applicable suggestion without prompting
frd --root DIR   # operate on DIR instead of the current directory
```

In the wizard, each suggestion is a card: `[a]ccept`, `[s]kip`, or `[q]uit`.

## What it can change

Every edit is format-preserving (comments and ordering survive) and backed up with a
timestamped `.frd-bak-*` copy before writing.

- **`~/.cargo/config.toml`** — a shared `target-dir` so repos and git worktrees stop
  duplicating `target/`; on nightly, `no-embed-metadata`; route `rustc` through
  `sccache` once it is installed.
- **`./Cargo.toml` profiles** — `dev` debug as `line-tables-only`, optimized
  dependencies, a disk-light `fast-build` profile, `release` `strip = true`, and on
  macOS `split-debuginfo = "unpacked"`.
- **`./.cargo/config.toml`** — on nightly, parallel-frontend and share-generics
  rustflags, kept project-local so they do not override a repo's own flags.
- **Tools** — install `sccache`, `cargo-sweep`, and `cargo-machete` (preferring
  `cargo-binstall` when present), and sweep stale build artifacts.

## License

MIT. See [LICENSE](LICENSE).
