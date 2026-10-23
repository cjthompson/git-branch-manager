use super::*;
use chrono::{Duration, TimeZone, Utc};
use ratatui::backend::TestBackend;
use std::process::Command;

mod cancel_job;
mod cherry_pick_local_prompt;
mod commit_details_actions;
mod commit_details_nav;
mod delete_preflight;
mod enrichment_sort;
mod force_confirm;
mod graph_action_refresh;
mod graph_enrichment;
mod graph_enrichment_publish;
mod graph_updates;
mod info_modal_worktree_menu;
mod jump_to_graph;
mod merge_confidence;
mod modal_keys;
mod navigation;
mod operation_results;
mod primary_code;
mod recovery;
mod render_rows;
mod squash_stash_prompt;
mod startup_settings;

fn render_app(app: &mut App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| {
            let mut ctx = app.build_render_context();
            git_branch_manager::ui::render::draw(frame, &mut ctx);
        })
        .unwrap();

    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

fn worktree(branch: &str) -> WorktreeInfo {
    WorktreeInfo {
        path: PathBuf::from(format!("/repo/.worktrees/{}", branch.replace('/', "-"))),
        branch: Some(branch.to_string()),
        is_main: false,
        is_base: false,
        commit_hash: "abc1234".into(),
        wt_status: WorkingTreeStatus::clean(),
        age_date: Utc::now(),
        merge_status: MergeStatus::Unmerged,
        ahead: None,
        behind: None,
        pr: None,
    }
}

fn remote_branch() -> RemoteBranchInfo {
    RemoteBranchInfo {
        full_ref: "origin/feature/remote-age".into(),
        remote: "origin".into(),
        short_name: "feature/remote-age".into(),
        has_local: false,
        is_base: false,
        last_commit_date: Utc::now() - Duration::minutes(5),
        merge_status: MergeStatus::Unmerged,
        ahead: None,
        behind: None,
        disjoint: false,
        pr: None,
        squash_confidence: None,
    }
}

fn branch(name: &str, tracking: TrackingStatus) -> BranchInfo {
    BranchInfo {
        name: name.into(),
        is_current: false,
        is_base: false,
        tracking,
        ahead: None,
        behind: None,
        last_commit_date: Utc::now(),
        merge_status: MergeStatus::Unmerged,
        base_branch: "main".into(),
        merge_base_commit: None,
        pr: None,
        squash_confidence: None,
    }
}

fn remote(full_ref: &str, short_name: &str) -> RemoteBranchInfo {
    RemoteBranchInfo {
        full_ref: full_ref.into(),
        remote: full_ref.split_once('/').unwrap().0.into(),
        short_name: short_name.into(),
        has_local: false,
        is_base: false,
        last_commit_date: Utc::now(),
        merge_status: MergeStatus::Unmerged,
        ahead: None,
        behind: None,
        disjoint: false,
        pr: None,
        squash_confidence: None,
    }
}

fn tag(name: &str) -> TagInfo {
    TagInfo {
        name: name.into(),
        commit_hash: "1111111".into(),
        date: Utc::now(),
        message: None,
        is_annotated: false,
    }
}

fn graph_snapshot(refs: Vec<graph::GraphRef>) -> graph::GraphSnapshot {
    graph::GraphSnapshot {
        source: graph::GraphSource::Gleisbau,
        commits: vec![graph::GraphCommit {
            oid: "1111111111111111111111111111111111111111".into(),
            summary: "selected commit".into(),
            parents: vec!["0000000000000000000000000000000000000000".into()],
            lane: Some(0),
            branch: None,
            refs,
            relationships: vec![],
            ..graph::GraphCommit::default()
        }],
        lines: vec![graph::GraphLine {
            graph: "*".into(),
            commit_index: Some(0),
        }],
        ref_counts: Default::default(),
        max_count: 500,
        includes_remotes: true,
        generation: None,
    }
}

fn graph_ref(name: &str, kind: graph::GraphRefKind) -> graph::GraphRef {
    graph::GraphRef {
        name: name.into(),
        kind,
        has_linked_worktree: false,
        is_current: false,
        tracking: None,
        ahead_of_base: None,
    }
}

fn graph_app(refs: Vec<graph::GraphRef>) -> App {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    app.active_view = ViewId::Graph;
    app.graph.apply_result(Ok(graph_snapshot(refs)));
    app
}

fn action_refresh_app(repo_path: &std::path::Path) -> App {
    run_git(repo_path, &["init", "-b", "main"]);
    run_git(repo_path, &["config", "user.email", "test@example.com"]);
    run_git(repo_path, &["config", "user.name", "Test User"]);
    std::fs::write(repo_path.join("README.md"), "base\n").unwrap();
    run_git(repo_path, &["add", "README.md"]);
    run_git(repo_path, &["commit", "-m", "initial"]);
    run_git(repo_path, &["branch", "feature/refresh"]);

    App::new(repo_path.to_path_buf(), "main".into(), Config::default())
}

fn complete_action_job_for_test(app: &mut App, action: BranchAction, origin: ViewId) {
    let (op_tx, op_rx) = mpsc::channel();
    let (_progress_tx, progress_rx) = mpsc::channel();
    app.job_queue.inject_running_for_test(
        git_branch_manager::job_queue::ActionJob {
            action,
            targets: vec!["feature/refresh".into()],
            remote: None,
            return_view: origin,
            dispatch_path: None,
        },
        op_rx,
        progress_rx,
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    );
    op_tx
        .send(vec![OperationResult::success(
            "feature/refresh",
            action,
            "Completed feature/refresh",
        )])
        .unwrap();

    app.drain_channels();
}

/// Polls until `done` holds or 60s pass; background git work has taken
/// ~18s under parallel test load behind a slow `git` wrapper.
fn wait_until(mut done: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while !done() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

fn unchanged_graph_observation(path: &std::path::Path) -> Receiver<Result<graph::GraphRepositoryDelta, String>> {
    let state = graph::GraphRepositoryState::capture(path).unwrap();
    let (tx, rx) = mpsc::channel();
    tx.send(Ok(graph::GraphRepositoryDelta::between(state.clone(), state))).unwrap();
    rx
}

fn info_modal_items(app: &App) -> &[MenuItem] {
    match app.overlay.as_ref() {
        Some(Overlay::InfoModal { items, .. }) => items,
        other => panic!("expected info modal, got {other:?}"),
    }
}

fn commit_details_items(app: &App) -> &[MenuItem] {
    match app.overlay.as_ref() {
        Some(Overlay::CommitDetails { items, .. }) => items,
        other => panic!("expected commit details, got {other:?}"),
    }
}

fn complete_graph_commit_details(app: &mut App) {
    let commit = app
        .graph
        .selected_commit()
        .cloned()
        .expect("selected graph commit");
    let details = commit_details::CommitDetails {
        oid: commit.oid.clone(),
        summary: commit.summary.clone(),
        author_name: String::new(),
        author_email: String::new(),
        authored_at: None,
        message_lines: vec![commit.summary.clone()],
        mode: commit_details::CommitDetailMode::Commit {
            oid: commit.oid.clone(),
        },
        files: Vec::new(),
        branch_log: Vec::new(),
    };
    let (tx, rx) = mpsc::channel();
    app.commit_details_rx = Some(rx);
    tx.send((commit.oid, Ok(details))).unwrap();
    app.drain_channels();
}

fn details_for(
    commit: &graph::GraphCommit,
    files: Vec<commit_details::ChangedCommitFile>,
) -> commit_details::CommitDetails {
    commit_details::CommitDetails {
        oid: commit.oid.clone(),
        summary: commit.summary.clone(),
        author_name: String::new(),
        author_email: String::new(),
        authored_at: None,
        message_lines: vec![commit.summary.clone()],
        mode: commit_details::CommitDetailMode::Commit {
            oid: commit.oid.clone(),
        },
        files,
        branch_log: Vec::new(),
    }
}

fn cell_text(line: &Line<'static>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect()
}

fn run_git(dir: &std::path::Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap_or_else(|e| panic!("failed to run git {:?}: {}", args, e));
    if !output.status.success() {
        panic!(
            "git {:?} failed in {}: {}",
            args,
            dir.display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

fn run_git_with_env(dir: &std::path::Path, args: &[&str], env: &[(&str, &str)]) {
    let output = Command::new("git")
        .args(args)
        .envs(env.iter().copied())
        .current_dir(dir)
        .output()
        .unwrap_or_else(|e| panic!("failed to run git {:?}: {}", args, e));
    if !output.status.success() {
        panic!(
            "git {:?} failed in {}: {}",
            args,
            dir.display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

fn results_fixture() -> Vec<OperationResult> {
    vec![
        OperationResult::success(
            "feature/deleted",
            BranchAction::DeleteLocal,
            "deleted feature/deleted",
        ),
        OperationResult::failure(
            "feature/unmerged",
            BranchAction::DeleteLocal,
            FailureCause::NotMerged,
            "error: the branch 'feature/unmerged' is not fully merged",
        ),
        OperationResult::failure(
            "feature/worktree",
            BranchAction::DeleteLocal,
            FailureCause::CheckedOutInWorktree {
                worktree_path: PathBuf::from("/repo/.worktrees/feature-worktree"),
                is_main: false,
            },
            "fatal: cannot delete branch used by worktree",
        ),
        OperationResult::failure(
            "feature/other",
            BranchAction::DeleteLocal,
            FailureCause::Other {
                raw_message: "fatal: unexpected ref transaction failure".into(),
            },
            "operation failed",
        ),
    ]
}

fn open_results_fixture(app: &mut App) {
    app.return_view = ViewId::Branches;
    app.overlay = Some(Overlay::results(results_fixture()));
}

fn press_overlay_key(app: &mut App, key: KeyCode) {
    app.handle_overlay_key(KeyEvent::new(key, crossterm::event::KeyModifiers::NONE));
}
