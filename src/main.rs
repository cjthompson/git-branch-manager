mod app;

use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io;

use git_branch_manager::cli::Cli;
use git_branch_manager::config::Config;
use git_branch_manager::git::{self, branch, cache, merge_detection, operations, worktree};
use git_branch_manager::symbols::SymbolSet;
use git_branch_manager::types::MergeStatus;
use tracing::instrument;

fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = Config::load();

    #[cfg(debug_assertions)]
    {
        use std::fs::OpenOptions;
        use tracing_subscriber::fmt::format::FmtSpan;
        let log_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/gbm-timing.log")
            .expect("failed to open timing log");
        tracing_subscriber::fmt()
            .with_writer(move || log_file.try_clone().expect("clone log file"))
            .with_ansi(false)
            .with_span_events(FmtSpan::CLOSE)
            .init();
    }

    // Open repo
    let repo = git2::Repository::discover(".")?;
    let repo_path = repo
        .workdir()
        .ok_or_else(|| anyhow::anyhow!("Not a git working directory"))?
        .to_path_buf();

    // Detect base branch
    let base_branch = branch::detect_base_branch(&repo, cli.base.as_deref())?;

    // Non-interactive list mode
    if cli.list {
        use git_branch_manager::types::TrackingStatus;
        let branches = branch::list_branches_phase1(&repo, &base_branch)?;
        if branches.is_empty() {
            eprintln!("No branches found.");
            return Ok(());
        }
        println!("base: {}\n", base_branch);
        for b in &branches {
            let status = match b.merge_status {
                MergeStatus::Merged => "merged",
                MergeStatus::SquashMerged => "squash-merged",
                MergeStatus::Unmerged | MergeStatus::Pending => "unmerged",
            };
            let tracking = match &b.tracking {
                TrackingStatus::Tracked { remote_ref, gone } => {
                    if *gone {
                        "gone".into()
                    } else {
                        remote_ref.clone()
                    }
                }
                TrackingStatus::Local => "(local)".into(),
            };
            let current = if b.is_current { "* " } else { "  " };
            println!(
                "{}{:<25} {:<20} {:<15} {}",
                current,
                b.name,
                tracking,
                b.age_display(),
                status
            );
        }
        return Ok(());
    }

    // Spawn background phase-1 load so the TUI starts immediately.
    // Sends Phase1Msg::Fast first (branch list, no merge detection),
    // then Phase1Msg::MergeStatuses once detect_merged_branches completes.
    let (phase1_tx, phase1_rx) = std::sync::mpsc::channel::<app::Phase1Msg>();
    {
        let repo_path_bg = repo_path.clone();
        let base_branch_bg = base_branch.clone();
        std::thread::spawn(move || {
            let Ok(repo) = git2::Repository::open(&repo_path_bg) else {
                return;
            };

            // Fast: metadata only, no merge detection
            let Ok(mut branches) = branch::list_branches_fast(&repo, &base_branch_bg) else {
                return;
            };
            let working_tree_status = git::status::detect_working_tree_status(&repo);
            let cache_for_app = cache::BranchCache::load(&repo_path_bg);
            let cache_for_squash = cache::BranchCache::load(&repo_path_bg);
            if phase1_tx
                .send(app::Phase1Msg::Fast(
                    branches.clone(),
                    working_tree_status,
                    cache_for_app,
                    cache_for_squash,
                ))
                .is_err()
            {
                return;
            }

            // Ahead/behind: compute for tracked non-gone branches and send update
            let ahead_behind_updates = compute_ahead_behind(&repo, &branches);
            let _ = phase1_tx.send(app::Phase1Msg::AheadBehind(ahead_behind_updates));

            // Slow: merge detection — update statuses in-place then send deltas
            if merge_detection::detect_merged_branches(&repo, &base_branch_bg, &mut branches)
                .is_ok()
            {
                let updates = branches
                    .into_iter()
                    .map(|b| (b.name, b.merge_status))
                    .collect();
                let _ = phase1_tx.send(app::Phase1Msg::MergeStatuses(updates));
            }
        });
    }

    // Create app (TUI launches immediately; branches arrive via phase1_rx)
    let mut app = app::App::new(repo_path.clone(), base_branch.clone(), config);
    app.phase1_rx = Some(phase1_rx);

    // Apply CLI symbol override
    if let Some(ref sym) = cli.symbols {
        app.symbols = SymbolSet::from_name(sym);
    }

    // Auto-fetch if configured
    if app.config.auto_fetch == Some(true) {
        let path = repo_path.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        app.remote_fetch_rx = Some(rx);
        std::thread::spawn(move || {
            let success = operations::fetch_sync(&path);
            let _ = tx.send(success);
        });
    }

    // Preload worktrees if configured
    if app.config.load_worktrees_on_launch == Some(true) {
        app.worktrees.loading = true;
        let path = repo_path.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        app.worktree_load_rx = Some(rx);
        std::thread::spawn(move || {
            let wts = worktree::list_worktrees(&path);
            let _ = tx.send(wts);
        });
    }

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Run app
    let result = app.run(&mut terminal);

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result.map_err(Into::into)
}

#[instrument(skip(repo, branches), fields(branch_count = branches.len()))]
fn compute_ahead_behind(
    repo: &git2::Repository,
    branches: &[git_branch_manager::types::BranchInfo],
) -> Vec<(String, Option<u32>, Option<u32>)> {
    use git_branch_manager::types::TrackingStatus;
    let mut updates: Vec<(String, Option<u32>, Option<u32>)> = Vec::new();
    for b in branches {
        if let TrackingStatus::Tracked { gone: false, .. } = &b.tracking {
            if let Ok(local_branch) = repo.find_branch(&b.name, git2::BranchType::Local) {
                if let Ok(upstream) = local_branch.upstream() {
                    let branch_oid = match local_branch.get().peel_to_commit() {
                        Ok(c) => c.id(),
                        Err(_) => continue,
                    };
                    let upstream_oid = match upstream.get().peel_to_commit() {
                        Ok(c) => c.id(),
                        Err(_) => continue,
                    };
                    let (ahead, behind) = match repo.graph_ahead_behind(branch_oid, upstream_oid) {
                        Ok((a, bh)) => (Some(a as u32), Some(bh as u32)),
                        Err(_) => (None, None),
                    };
                    updates.push((b.name.clone(), ahead, behind));
                }
            }
        }
    }
    updates
}
