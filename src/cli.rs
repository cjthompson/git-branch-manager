use clap::{ArgGroup, Parser, ValueEnum};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Default)]
pub enum ColorChoice {
    #[default]
    Auto,
    Always,
    Never,
}

#[derive(Parser, Debug)]
#[command(
    name = "git-branch-manager",
    about = "TUI for managing git branches",
    group(ArgGroup::new("dump").args(["branches", "remotes", "tags", "worktrees", "list"]).multiple(false)),
)]
pub struct Cli {
    /// Path to the git repository (defaults to current directory)
    #[arg(long)]
    pub repo: Option<PathBuf>,

    /// Override the auto-detected base branch
    #[arg(long)]
    pub base: Option<String>,

    /// Non-interactive: print branch list to stdout
    #[arg(long)]
    pub list: bool,

    /// Override symbol set (ascii, unicode, powerline)
    #[arg(long)]
    pub symbols: Option<String>,

    /// Non-interactive: print the Branches view to stdout (fully enriched)
    #[arg(long)]
    pub branches: bool,

    /// Non-interactive: print the Remotes view to stdout (fully enriched)
    #[arg(long)]
    pub remotes: bool,

    /// Non-interactive: print the Tags view to stdout (fully enriched)
    #[arg(long)]
    pub tags: bool,

    /// Non-interactive: print the Worktrees view to stdout (fully enriched)
    #[arg(long)]
    pub worktrees: bool,

    /// When to colorize dump output
    #[arg(long, value_enum, default_value_t = ColorChoice::Auto)]
    pub color: ColorChoice,
}
