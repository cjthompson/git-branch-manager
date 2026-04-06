use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "git-branch-manager", about = "TUI for managing git branches")]
pub struct Cli {
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
