<div align="center">
  <h1>frd — fast rust dev</h1>
  <p><b>Interactive optimizer for local Rust builds.</b></p>
</div>

`frd` reads your machine and project, then walks you through build-speed and
`target/`-shrinking changes one at a time. Accept or skip each; it changes nothing
without your approval.

Runs on macOS, Linux, and Windows. macOS gets the full catalog; elsewhere `frd` shows
only the suggestions that fit (the `split-debuginfo` tweak, for one, is macOS-only).

## Install

Prebuilt binary (no compile), via [cargo-binstall](https://github.com/cargo-bins/cargo-binstall):

```sh
cargo binstall frd
```

From source:

```sh
cargo install frd
```

Or download a binary directly: each [GitHub release](https://github.com/cs50victor/fast-rust-dev/releases)
ships macOS (Apple Silicon + Intel), Linux (x86_64 + arm64), and Windows (x86_64) builds.

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

Each suggestion is a card: why it helps, the exact diff, and a color for what it
optimizes (disk, speed, or both). Choose Accept, Skip, or Quit. Installs and sweeps
stream their output, then fold to one line when they finish.

## What it can change

Every edit preserves your comments and ordering, and copies the file to a timestamped
`.frd-bak-*` backup first.

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
