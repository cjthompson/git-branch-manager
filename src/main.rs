mod app;
mod cli;
mod ui;

use anyhow::Result;
use clap::Parser;
use cli::Cli;
use git_branch_manager::git;

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Open the git repository in the current directory
    let repo = git2::Repository::discover(".")?;

    // Detect base branch
    let base_branch = git::branch::detect_base_branch(&repo, cli.base.as_deref())?;

    // Gather branch information
    let branches = git::branch::list_branches(&repo, &base_branch)?;

    if branches.is_empty() {
        eprintln!("No branches found.");
        return Ok(());
    }

    // Run the TUI
    let mut app = app::App::new(base_branch, branches);
    let mut terminal = ratatui::init();
    let result = app.run(&mut terminal);
    ratatui::restore();
    result
}
