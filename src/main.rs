mod app;
mod dump;

use crate::dump::DumpView;
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
use tracing::{field, info_span, instrument, Span};

fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = Config::load();

    // Optional timing log, opt-in via GBM_TIMING_LOG so the same instrumentation
    // can be captured in debug and release builds.
    let _log_guard = init_timing_log();

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
        )?;
        print!("{out}");
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
            let branch_load_span = info_span!(
                "branch_load",
                trigger = "startup",
                path = "phase1_async",
                inputs_changed = true,
            );
            let _branch_load_enter = branch_load_span.enter();
            let phase1_span = info_span!(
                "phase1_load_worker",
                trigger = "initial_load",
                repo_path = ?repo_path_bg,
                base_branch = %base_branch_bg,
                fast_branch_count = field::Empty,
                ahead_behind_update_count = field::Empty,
                merge_base_update_count = field::Empty,
                merge_status_update_count = field::Empty,
                result_state = field::Empty,
            );
            let _phase1_enter = phase1_span.enter();
            let Ok(repo) = git2::Repository::open(&repo_path_bg) else {
                phase1_span.record("result_state", "open_repo_error");
                return;
            };

            // Fast: metadata only, no merge detection
            let Ok(mut branches) = branch::list_branches_fast(&repo, &base_branch_bg) else {
                phase1_span.record("result_state", "list_branches_fast_error");
                return;
            };
            phase1_span.record("fast_branch_count", branches.len() as u64);
            let cache_for_app = cache::BranchCache::load(&repo_path_bg);
            let cache_for_squash = cache::BranchCache::load(&repo_path_bg);
            if phase1_tx
                .send(app::Phase1Msg::Fast(
                    branches.clone(),
                    cache_for_app,
                    cache_for_squash,
                ))
                .is_err()
            {
                phase1_span.record("result_state", "fast_send_error");
                return;
            }

            // Ahead/behind + merge-base: both are graph traversals deferred from fast path
            let ahead_behind_updates = compute_ahead_behind(&repo, &branches);
            phase1_span.record(
                "ahead_behind_update_count",
                ahead_behind_updates.len() as u64,
            );
            let _ = phase1_tx.send(app::Phase1Msg::AheadBehind(ahead_behind_updates));

            let merge_base_updates = compute_merge_bases(&repo, &base_branch_bg, &branches);
            phase1_span.record("merge_base_update_count", merge_base_updates.len() as u64);
            let _ = phase1_tx.send(app::Phase1Msg::MergeBaseCommits(merge_base_updates));

            // Slow: merge detection — update statuses in-place then send deltas
            match merge_detection::detect_merged_branches(&repo, &base_branch_bg, &mut branches) {
                Ok(()) => {
                    let updates = branches
                        .into_iter()
                        .map(|b| (b.name, b.merge_status))
                        .collect();
                    let updates: Vec<(String, MergeStatus)> = updates;
                    phase1_span.record("merge_status_update_count", updates.len() as u64);
                    let _ = phase1_tx.send(app::Phase1Msg::MergeStatuses(updates));
                    phase1_span.record("result_state", "success");
                }
                Err(_) => {
                    phase1_span.record("result_state", "detect_merged_branches_error");
                }
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

/// Initialise the optional timing-log subscriber.
///
/// Disabled unless `GBM_TIMING_LOG` is set: `1`/`true` writes the default
/// `/tmp/gbm-timing.log`, any other non-empty value is used as the log path.
fn init_timing_log() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    use std::fs::OpenOptions;
    use tracing_subscriber::fmt::format::FmtSpan;
    use tracing_subscriber::EnvFilter;

    let path = match std::env::var("GBM_TIMING_LOG") {
        Ok(v) if v == "1" || v.eq_ignore_ascii_case("true") => "/tmp/gbm-timing.log".to_string(),
        Ok(v) if !v.is_empty() && v != "0" => v,
        _ => return None,
    };

    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .expect("failed to open timing log");
    let (non_blocking, guard) = tracing_appender::non_blocking(log_file);
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("git_branch_manager=debug"));

    tracing_subscriber::fmt()
        .pretty()
        .with_env_filter(filter)
        .with_writer(non_blocking)
        .with_ansi(false)
        .with_span_events(FmtSpan::CLOSE)
        .init();

    Some(guard)
}

#[instrument(
    skip(repo, branches),
    fields(
        branch_count = branches.len(),
        base_oid = field::Empty,
        base_lookup_result = field::Empty,
        result_count = field::Empty,
        skipped_base_count = field::Empty,
        find_branch_error_count = field::Empty,
        peel_error_count = field::Empty,
        merge_base_success_count = field::Empty,
        merge_base_error_count = field::Empty,
    )
)]
fn compute_merge_bases(
    repo: &git2::Repository,
    base_branch: &str,
    branches: &[git_branch_manager::types::BranchInfo],
) -> Vec<(String, String)> {
    let span = Span::current();
    let base_lookup_span = info_span!("compute_merge_bases_base_lookup", base_branch);
    let base_oid = match base_lookup_span.in_scope(|| {
        repo.find_branch(base_branch, git2::BranchType::Local)
            .ok()
            .and_then(|b| b.get().target())
    }) {
        Some(oid) => {
            let base_oid = oid.to_string();
            span.record("base_oid", base_oid.as_str());
            span.record("base_lookup_result", "success");
            oid
        }
        None => {
            span.record("base_lookup_result", "missing_target");
            span.record("result_count", 0);
            span.record("skipped_base_count", 0);
            span.record("find_branch_error_count", 0);
            span.record("peel_error_count", 0);
            span.record("merge_base_success_count", 0);
            span.record("merge_base_error_count", 0);
            return Vec::new();
        }
    };

    let mut updates = Vec::new();
    let mut skipped_base_count = 0usize;
    let mut find_branch_error_count = 0usize;
    let mut peel_error_count = 0usize;
    let mut merge_base_success_count = 0usize;
    let mut merge_base_error_count = 0usize;
    for b in branches {
        let branch_span = info_span!(
            "compute_merge_base_branch",
            branch_name = %b.name,
            is_base = b.is_base,
            is_current = b.is_current,
            branch_tip = field::Empty,
            merge_base_oid = field::Empty,
            result_state = field::Empty,
        );
        let _branch_enter = branch_span.enter();

        if b.is_base {
            skipped_base_count += 1;
            branch_span.record("result_state", "skipped_base");
            continue;
        }

        let branch = match info_span!("compute_merge_base_find_branch", branch_name = %b.name)
            .in_scope(|| repo.find_branch(&b.name, git2::BranchType::Local))
        {
            Ok(branch) => branch,
            Err(_) => {
                find_branch_error_count += 1;
                branch_span.record("result_state", "find_branch_error");
                continue;
            }
        };

        let commit = match info_span!("compute_merge_base_peel_commit", branch_name = %b.name)
            .in_scope(|| branch.get().peel_to_commit())
        {
            Ok(commit) => commit,
            Err(_) => {
                peel_error_count += 1;
                branch_span.record("result_state", "peel_error");
                continue;
            }
        };

        let branch_tip = commit.id().to_string();
        branch_span.record("branch_tip", branch_tip.as_str());

        match info_span!(
            "compute_merge_base_graph",
            branch_name = %b.name,
            base_oid = %base_oid,
            branch_tip = %commit.id(),
        )
        .in_scope(|| repo.merge_base(base_oid, commit.id()))
        {
            Ok(mb_oid) => {
                merge_base_success_count += 1;
                let merge_base_oid = mb_oid.to_string();
                branch_span.record("merge_base_oid", merge_base_oid.as_str());
                branch_span.record("result_state", "success");
                let hash = merge_base_oid[..8].to_string();
                updates.push((b.name.clone(), hash));
            }
            Err(_) => {
                merge_base_error_count += 1;
                branch_span.record("result_state", "merge_base_error");
            }
        }
    }
    span.record("result_count", updates.len());
    span.record("skipped_base_count", skipped_base_count);
    span.record("find_branch_error_count", find_branch_error_count);
    span.record("peel_error_count", peel_error_count);
    span.record("merge_base_success_count", merge_base_success_count);
    span.record("merge_base_error_count", merge_base_error_count);
    updates
}

#[instrument(
    skip(repo, branches),
    fields(
        branch_count = branches.len(),
        tracked_branch_count = field::Empty,
        untracked_branch_count = field::Empty,
        gone_branch_count = field::Empty,
        result_count = field::Empty,
        skipped_count = field::Empty,
        find_branch_error_count = field::Empty,
        upstream_error_count = field::Empty,
        peel_error_count = field::Empty,
        graph_success_count = field::Empty,
        graph_error_count = field::Empty,
    )
)]
fn compute_ahead_behind(
    repo: &git2::Repository,
    branches: &[git_branch_manager::types::BranchInfo],
) -> Vec<(String, Option<u32>, Option<u32>)> {
    use git_branch_manager::types::TrackingStatus;
    let span = Span::current();
    let mut updates: Vec<(String, Option<u32>, Option<u32>)> = Vec::new();
    let mut tracked_branch_count = 0usize;
    let mut untracked_branch_count = 0usize;
    let mut gone_branch_count = 0usize;
    let mut skipped_count = 0usize;
    let mut find_branch_error_count = 0usize;
    let mut upstream_error_count = 0usize;
    let mut peel_error_count = 0usize;
    let mut graph_success_count = 0usize;
    let mut graph_error_count = 0usize;

    for b in branches {
        let branch_span = info_span!(
            "compute_ahead_behind_branch",
            branch_name = %b.name,
            tracking_status = field::Empty,
            remote_ref = field::Empty,
            branch_tip = field::Empty,
            upstream_oid = field::Empty,
            ahead = field::Empty,
            behind = field::Empty,
            result_state = field::Empty,
        );
        let _branch_enter = branch_span.enter();

        let remote_ref = match &b.tracking {
            TrackingStatus::Tracked {
                remote_ref,
                gone: false,
            } => {
                tracked_branch_count += 1;
                branch_span.record("tracking_status", "tracked");
                branch_span.record("remote_ref", remote_ref.as_str());
                remote_ref
            }
            TrackingStatus::Tracked {
                remote_ref,
                gone: true,
            } => {
                gone_branch_count += 1;
                skipped_count += 1;
                branch_span.record("tracking_status", "gone");
                branch_span.record("remote_ref", remote_ref.as_str());
                branch_span.record("result_state", "skipped_gone");
                continue;
            }
            TrackingStatus::Local => {
                untracked_branch_count += 1;
                skipped_count += 1;
                branch_span.record("tracking_status", "local");
                branch_span.record("result_state", "skipped_untracked");
                continue;
            }
        };

        let local_branch =
            match info_span!("compute_ahead_behind_find_branch", branch_name = %b.name)
                .in_scope(|| repo.find_branch(&b.name, git2::BranchType::Local))
            {
                Ok(local_branch) => local_branch,
                Err(_) => {
                    find_branch_error_count += 1;
                    branch_span.record("result_state", "find_branch_error");
                    continue;
                }
            };

        let upstream = match info_span!(
            "compute_ahead_behind_find_upstream",
            branch_name = %b.name,
            remote_ref = %remote_ref,
        )
        .in_scope(|| local_branch.upstream())
        {
            Ok(upstream) => upstream,
            Err(_) => {
                upstream_error_count += 1;
                branch_span.record("result_state", "upstream_error");
                continue;
            }
        };

        let branch_oid = match info_span!("compute_ahead_behind_peel_branch", branch_name = %b.name)
            .in_scope(|| local_branch.get().peel_to_commit())
        {
            Ok(c) => c.id(),
            Err(_) => {
                peel_error_count += 1;
                branch_span.record("result_state", "branch_peel_error");
                continue;
            }
        };
        let branch_oid_string = branch_oid.to_string();
        branch_span.record("branch_tip", branch_oid_string.as_str());

        let upstream_oid = match info_span!(
            "compute_ahead_behind_peel_upstream",
            branch_name = %b.name,
            remote_ref = %remote_ref,
        )
        .in_scope(|| upstream.get().peel_to_commit())
        {
            Ok(c) => c.id(),
            Err(_) => {
                peel_error_count += 1;
                branch_span.record("result_state", "upstream_peel_error");
                continue;
            }
        };
        let upstream_oid_string = upstream_oid.to_string();
        branch_span.record("upstream_oid", upstream_oid_string.as_str());

        let (ahead, behind) = match info_span!(
            "compute_ahead_behind_graph",
            branch_name = %b.name,
            branch_tip = %branch_oid,
            upstream_oid = %upstream_oid,
        )
        .in_scope(|| repo.graph_ahead_behind(branch_oid, upstream_oid))
        {
            Ok((a, bh)) => {
                graph_success_count += 1;
                branch_span.record("ahead", a);
                branch_span.record("behind", bh);
                branch_span.record("result_state", "success");
                (Some(a as u32), Some(bh as u32))
            }
            Err(_) => {
                graph_error_count += 1;
                branch_span.record("result_state", "graph_error");
                (None, None)
            }
        };
        updates.push((b.name.clone(), ahead, behind));
    }

    span.record("tracked_branch_count", tracked_branch_count);
    span.record("untracked_branch_count", untracked_branch_count);
    span.record("gone_branch_count", gone_branch_count);
    span.record("result_count", updates.len());
    span.record("skipped_count", skipped_count);
    span.record("find_branch_error_count", find_branch_error_count);
    span.record("upstream_error_count", upstream_error_count);
    span.record("peel_error_count", peel_error_count);
    span.record("graph_success_count", graph_success_count);
    span.record("graph_error_count", graph_error_count);
    updates
}
