use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "git-branch-manager", about = "TUI for managing git branches")]
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
}
