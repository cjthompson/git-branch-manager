mod app;
mod cli;
mod ui;

use anyhow::Result;
use clap::Parser;
use cli::Cli;
use git_branch_manager::config::Config;
use git_branch_manager::git::{self, operations};
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

    // Load config early so auto_fetch and other settings are available throughout
    let config = Config::load();

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

    // Resolve symbol set: CLI flag > config file > auto-detect
    let symbols = match cli.symbols.as_deref().or(config.symbols.as_deref()) {
        Some(name) => ui::symbols::from_name(name),
        None => ui::symbols::detect(),
    };

    let theme = ui::theme::Theme::from_name(config.theme.as_deref().unwrap_or("dark"));

    // Spawn background thread for auto-fetch + phase 1 branch loading
    let auto_fetch = config.auto_fetch == Some(true);
    let load_repo_path = repo_path.clone();
    let load_base = base_branch.clone();
    let (load_tx, load_rx) = std::sync::mpsc::channel();
    let (prog_tx, prog_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        // Auto-fetch first if configured (network I/O)
        if auto_fetch {
            let _ = prog_tx.send(app::LoadProgress {
                message: "Fetching from remote...".into(),
            });
            let _ = operations::fetch(&load_repo_path);
        }

        let _ = prog_tx.send(app::LoadProgress {
            message: "Reading branches...".into(),
        });

        // Phase 1: branch listing + regular merge detection
        let Ok(repo) = git2::Repository::open(&load_repo_path) else { return };
        let Ok(branches) = git::branch::list_branches_phase1(&repo, &load_base) else { return };

        let working_tree_status = git::status::detect_working_tree_status(&repo);
        let cache = git::cache::BranchCache::load(&load_repo_path);

        let candidates: Vec<(String, String)> = branches
            .iter()
            .filter(|b| b.merge_status == MergeStatus::Unmerged && !b.is_base && !b.is_current)
            .filter_map(|b| {
                git::branch::get_commit_hash(&repo, &b.name)
                    .map(|hash| (b.name.clone(), hash))
            })
            .collect();

        let _ = load_tx.send(app::InitialLoad {
            branches,
            working_tree_status,
            candidates,
            cache,
        });
    });

    // Start TUI immediately — branches arrive via load_rx
    let mut app = app::App::new(base_branch, repo_path, symbols, theme, config, load_rx, prog_rx);
    let mut terminal = ratatui::init();
    crossterm::execute!(std::io::stdout(), crossterm::event::EnableMouseCapture)?;
    let result = app.run(&mut terminal);
    crossterm::execute!(std::io::stdout(), crossterm::event::DisableMouseCapture).ok();
    ratatui::restore();
    result
}
