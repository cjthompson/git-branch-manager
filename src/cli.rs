use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "git-branch-manager", about = "TUI git branch manager with squash-merge detection")]
pub struct Cli {
    /// Override the base branch (default: auto-detect from remote HEAD)
    #[arg(long, short)]
    pub base: Option<String>,

    /// Print branch list to stdout and exit (no TUI)
    #[arg(long)]
    pub list: bool,

    /// Symbol set: ascii, unicode, powerline (default: auto-detect)
    #[arg(long)]
    pub symbols: Option<String>,
}
