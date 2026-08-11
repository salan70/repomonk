//! CLI argument parsing.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "repomonk", version, about = "Type through real repositories")]
pub struct Cli {
    /// GitHub URL, owner/repo, or local path. Omit to open the home screen.
    pub target: Option<String>,

    /// Re-clone / re-scan GitHub cache.
    #[arg(long)]
    pub refresh: bool,

    /// Delete all repomonk-managed cache and progress data after confirmation.
    #[arg(long)]
    pub purge: bool,

    /// Skip purge confirmation (still prints the target paths).
    #[arg(long)]
    pub yes: bool,

    /// Disable visual effects (splash animation, glow, trails).
    #[arg(long = "no-fx")]
    pub no_fx: bool,

    /// Override cache directory (tests / portable installs).
    #[arg(long, env = "REPOMONK_CACHE_DIR", hide = true)]
    pub cache_dir: Option<PathBuf>,

    /// Override data directory containing the SQLite DB.
    #[arg(long, env = "REPOMONK_DATA_DIR", hide = true)]
    pub data_dir: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Show a short version string (also available as --version).
    Version,
}
