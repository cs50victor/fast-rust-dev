use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "frd",
    version,
    about = "fast rust dev: interactive build-speed and disk optimizer for cargo projects (best on macOS)"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Project directory to operate on (defaults to the current directory).
    #[arg(long, global = true)]
    pub root: Option<PathBuf>,

    /// Accept every applicable suggestion without prompting.
    #[arg(long, short = 'y', global = true)]
    pub yes: bool,

    /// Show what would change but write nothing and run nothing.
    #[arg(long, global = true)]
    pub dry_run: bool,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Print the system and project report, then exit.
    Report,

    /// Audit which optimizations are applied, change nothing, and exit non-zero if any are pending.
    Doctor,
}
