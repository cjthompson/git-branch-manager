mod ahead_behind;
mod app;
mod dump;
mod logging;
mod phase1;
mod terminal;

use crate::dump::DumpView;
use crate::logging::init_debug_log;
use anyhow::Result;
use clap::Parser;

use git_branch_manager::cli::Cli;
use git_branch_manager::config::Config;
use git_branch_manager::git::{branch, cache, diagnostics, worktree};
use git_branch_manager::symbols::SymbolSet;

fn main() -> Result<()> {
    let cli = Cli::parse();
    let cache_root = cache::CacheRoot::from_env();
    cache::prune_stale_caches(&cache_root);

    // Non-interactive dumps (also covers the deprecated `--list`) are what
    // `cargo test` uses to spawn this binary many times per run; skip the
    // optional debug log for them so test runs don't flood an interactive
    // user's debug.log with process-start noise.
    let is_dump_mode = cli.branches || cli.list || cli.remotes || cli.tags || cli.worktrees;

    // Optional debug log, opt-in via GBM_DEBUG so the same instrumentation
    // can be captured in debug and release builds. Initialized before
    // Config::load() below so the load path's own tracing is captured too.
    let _log_guard = if is_dump_mode { None } else { init_debug_log() };

    let mut config = Config::load();
    git_branch_manager::view::sort_keys::migrate_legacy_config(&mut config);

    // Open repo
    let search_path = cli.repo.as_deref().unwrap_or(std::path::Path::new("."));
    let repo = git2::Repository::discover(search_path)?;
    let repo_path = repo
        .workdir()
        .ok_or_else(|| anyhow::anyhow!("Not a git working directory"))?
        .to_path_buf();

    // Detect base branch
    let base_branch = branch::detect_base_branch(&repo, cli.base.as_deref())?;

    // Non-interactive view dumps (also covers the deprecated `--list`).
    let dump_view = if cli.branches || cli.list {
        Some(DumpView::Branches)
    } else if cli.remotes {
        Some(DumpView::Remotes)
    } else if cli.tags {
        Some(DumpView::Tags)
    } else if cli.worktrees {
        Some(DumpView::Worktrees)
    } else {
        None
    };
    if let Some(view) = dump_view {
        if cli.list {
            eprintln!("note: --list is deprecated; use --branches");
        }
        let out = dump::run(
            &repo,
            &repo_path,
            &base_branch,
            &config,
            cli.symbols.as_deref(),
            view,
            cli.color,
            &cache_root,
        )?;
        print!("{out}");
        return Ok(());
    }

    // Spawn background phase-1 load so the TUI starts immediately.
    // Sends Phase1Msg::Fast first (branch list, no merge detection),
    // then Phase1Msg::MergeStatuses once detect_merged_branches completes.
    let phase1_rx = phase1::spawn_phase1_loader(&repo_path, &base_branch, &cache_root);

    // Create app (TUI launches immediately; branches arrive via phase1_rx)
    let mut app = app::App::with_cache_root(
        repo_path.clone(),
        base_branch.clone(),
        config,
        cache_root.clone(),
    );
    app.phase1_rx = Some(phase1_rx);

    // Silently verify (and correct) the cache in the background unless disabled.
    // No dependency on phase-1: audit_cache enumerates branches from its own
    // Repository handle, so it doesn't need to wait for anything above.
    if app.config.verify_cache_on_launch != Some(false) {
        app.cache_verify_rx = Some(diagnostics::spawn_cache_verifier(
            repo_path.clone(),
            base_branch.clone(),
            cache_root.clone(),
        ));
    }

    // Apply CLI symbol override
    if let Some(ref sym) = cli.symbols {
        app.symbols = SymbolSet::from_name(sym);
    }

    let graph_max_count = app.graph.max_count();
    let include_remotes = app.graph.includes_remotes();
    app.spawn_graph_load(graph_max_count, include_remotes);
    app.preload_graph_action_metadata();

    // Auto-fetch if configured
    if app.config.auto_fetch == Some(true) {
        app.start_auto_fetch();
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

    // Setup terminal, run the app, then restore the terminal
    terminal::run_app(&mut app)
}
