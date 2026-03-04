mod app;
mod cli;
mod ui;

use anyhow::Result;
use clap::Parser;
use cli::Cli;
use git_branch_manager::config::Config;
use git_branch_manager::git;
use git_branch_manager::types::MergeStatus;

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Open the git repository in the current directory
    let repo = git2::Repository::discover(".")?;

    // Detect base branch
    let base_branch = git::branch::detect_base_branch(&repo, cli.base.as_deref())?;

    let repo_path = repo
        .workdir()
        .unwrap_or_else(|| repo.path())
        .to_path_buf();

    // Non-interactive list mode (synchronous, full pipeline with cache)
    if cli.list {
        use git_branch_manager::types::TrackingStatus;
        let mut cache = git::cache::BranchCache::load(&repo_path);
        let branches = git::branch::list_branches_cached(&repo, &base_branch, &mut cache)?;
        cache.save();
        if branches.is_empty() {
            eprintln!("No branches found.");
            return Ok(());
        }
        println!("base: {}\n", base_branch);
        for b in &branches {
            let status = match b.merge_status {
                MergeStatus::Merged => "merged",
                MergeStatus::SquashMerged => "squash-merged",
                MergeStatus::Unmerged => "unmerged",
            };
            let tracking = match &b.tracking {
                TrackingStatus::Tracked { remote_ref, gone } => {
                    if *gone { "gone".into() } else { remote_ref.clone() }
                }
                TrackingStatus::Local => "(local)".into(),
            };
            let current = if b.is_current { "* " } else { "  " };
            println!("{}{:<25} {:<20} {:<15} {}", current, b.name, tracking, b.age_display(), status);
        }
        return Ok(());
    }

    // TUI mode: phase 1 (fast git2), then spawn background squash checker with cache
    let branches = git::branch::list_branches_phase1(&repo, &base_branch)?;

    if branches.is_empty() {
        eprintln!("No branches found.");
        return Ok(());
    }

    let cache = git::cache::BranchCache::load(&repo_path);

    let candidates: Vec<(String, String)> = branches
        .iter()
        .filter(|b| b.merge_status == MergeStatus::Unmerged && !b.is_base && !b.is_current)
        .filter_map(|b| {
            git::branch::get_commit_hash(&repo, &b.name)
                .map(|hash| (b.name.clone(), hash))
        })
        .collect();

    let squash_total = candidates.len();

    let squash_rx = if candidates.is_empty() {
        None
    } else {
        Some(git::squash_loader::spawn_squash_checker(
            repo_path.clone(),
            base_branch.clone(),
            candidates,
            cache,
        ))
    };

    // Detect working tree status
    let working_tree_status = git::status::detect_working_tree_status(&repo);

    // Resolve symbol set: CLI flag > config file > auto-detect
    let config = Config::load();
    let symbols = match cli.symbols.as_deref().or(config.symbols.as_deref()) {
        Some(name) => ui::symbols::from_name(name),
        None => ui::symbols::detect(),
    };

    let trim_strategy = config.trim_strategy.unwrap_or_else(|| "end".to_string());
    let mut app = app::App::new(base_branch, repo_path, branches, squash_rx, squash_total, working_tree_status, symbols, trim_strategy);
    let mut terminal = ratatui::init();
    let result = app.run(&mut terminal);
    ratatui::restore();
    result
}
