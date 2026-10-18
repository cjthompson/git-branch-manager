use super::*;

#[test]
fn local_remote_delete_keeps_remote_when_remote_deletion_fails() {
    let mut app = graph_app(vec![graph_ref(
        "feature/local",
        graph::GraphRefKind::LocalBranch,
    )]);
    app.remotes
        .set_items(vec![remote("origin/feature/local", "feature/local")]);

    let (op_tx, op_rx) = mpsc::channel();
    let (_prog_tx, prog_rx) = mpsc::channel();
    app.job_queue.inject_running_for_test(
        git_branch_manager::job_queue::ActionJob {
            action: BranchAction::DeleteLocalAndRemote,
            targets: vec!["feature/local".into()],
            remote: None,
            return_view: ViewId::Graph,
            dispatch_path: None,
        },
        op_rx,
        prog_rx,
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    );
    op_tx
        .send(vec![
            OperationResult::success(
                "feature/local",
                BranchAction::DeleteLocal,
                "Deleted local feature/local",
            ),
            OperationResult {
                branch_name: "feature/local".into(),
                action: BranchAction::DeleteRemoteBranch,
                success: false,
                message: "Remote deletion failed".into(),
                failure: None,
            },
        ])
        .unwrap();

    app.drain_channels();

    assert!(app.remotes.loading);
    assert!(app
        .remotes
        .items()
        .iter()
        .any(|branch| branch.short_name == "feature/local"));
}

#[test]
fn local_remote_delete_filters_only_the_selected_remote() {
    let mut app = graph_app(vec![graph_ref(
        "feature/local",
        graph::GraphRefKind::LocalBranch,
    )]);
    app.remotes.set_items(vec![
        remote("origin/feature/local", "feature/local"),
        remote("upstream/feature/local", "feature/local"),
    ]);

    let (op_tx, op_rx) = mpsc::channel();
    let (_prog_tx, prog_rx) = mpsc::channel();
    app.job_queue.inject_running_for_test(
        git_branch_manager::job_queue::ActionJob {
            action: BranchAction::DeleteLocalAndRemote,
            targets: vec!["feature/local".into()],
            remote: Some("upstream".into()),
            return_view: ViewId::Graph,
            dispatch_path: None,
        },
        op_rx,
        prog_rx,
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    );
    op_tx
        .send(vec![
            OperationResult::success(
                "feature/local",
                BranchAction::DeleteLocal,
                "Deleted local feature/local",
            ),
            OperationResult::success(
                "feature/local",
                BranchAction::DeleteRemoteBranch,
                "Deleted upstream/feature/local",
            ),
        ])
        .unwrap();

    app.drain_channels();

    assert!(app
        .remotes
        .items()
        .iter()
        .any(|branch| branch.full_ref == "origin/feature/local"));
    assert!(!app
        .remotes
        .items()
        .iter()
        .any(|branch| branch.full_ref == "upstream/feature/local"));
}

#[test]
fn branch_operation_result_refreshes_branch_metadata_immediately() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let dir = tmpdir.path();
    run_git(dir, &["init"]);
    run_git(dir, &["config", "user.email", "test@example.com"]);
    run_git(dir, &["config", "user.name", "Test User"]);

    std::fs::write(dir.join("README.md"), "base\n").unwrap();
    run_git(dir, &["add", "README.md"]);
    run_git_with_env(
        dir,
        &["commit", "-m", "initial"],
        &[
            ("GIT_AUTHOR_DATE", "2001-01-01T00:00:00Z"),
            ("GIT_COMMITTER_DATE", "2001-01-01T00:00:00Z"),
        ],
    );
    run_git(dir, &["branch", "-M", "main"]);

    let branch_name = "feature/rebase-refresh";
    run_git(dir, &["checkout", "-b", branch_name]);
    std::fs::write(dir.join("feature.txt"), "old\n").unwrap();
    run_git(dir, &["add", "feature.txt"]);
    run_git_with_env(
        dir,
        &["commit", "-m", "feature old"],
        &[
            ("GIT_AUTHOR_DATE", "2001-01-02T00:00:00Z"),
            ("GIT_COMMITTER_DATE", "2001-01-02T00:00:00Z"),
        ],
    );

    let mut app = App::new(dir.to_path_buf(), "main".into(), Config::default());
    app.return_view = ViewId::Branches;
    let stale_date = Utc.with_ymd_and_hms(1999, 1, 1, 0, 0, 0).unwrap();
    app.branches.set_items(vec![BranchInfo {
        name: branch_name.into(),
        is_current: false,
        is_base: false,
        tracking: TrackingStatus::Local,
        ahead: Some(99),
        behind: Some(99),
        last_commit_date: stale_date,
        merge_status: MergeStatus::Merged,
        base_branch: "main".into(),
        merge_base_commit: Some("stale".into()),
        pr: Some(PrInfo {
            number: 123,
            status: PrStatus::Open,
        }),
        squash_confidence: None,
    }]);

    std::fs::write(dir.join("feature.txt"), "new\n").unwrap();
    run_git(dir, &["add", "feature.txt"]);
    run_git_with_env(
        dir,
        &["commit", "-m", "feature new"],
        &[
            ("GIT_AUTHOR_DATE", "2001-01-03T00:00:00Z"),
            ("GIT_COMMITTER_DATE", "2001-01-03T00:00:00Z"),
        ],
    );

    // Simulate having navigated to a different view while the job ran in
    // the background -- completing it must not snap the user back.
    app.active_view = ViewId::Worktrees;

    let (op_tx, op_rx) = mpsc::channel();
    let (_prog_tx, prog_rx) = mpsc::channel();
    app.job_queue.inject_running_for_test(
        git_branch_manager::job_queue::ActionJob {
            action: BranchAction::Rebase,
            targets: vec![branch_name.to_string()],
            remote: None,
            return_view: ViewId::Branches,
            dispatch_path: None,
        },
        op_rx,
        prog_rx,
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    );
    op_tx
        .send(vec![OperationResult::success(
            branch_name,
            BranchAction::Rebase,
            "Rebased feature/rebase-refresh onto main",
        )])
        .unwrap();

    app.drain_channels();

    let refreshed = app
        .branches
        .items()
        .iter()
        .find(|branch| branch.name == branch_name)
        .expect("refreshed feature branch");
    assert_eq!(
        refreshed.last_commit_date,
        Utc.with_ymd_and_hms(2001, 1, 3, 0, 0, 0).unwrap(),
        "branch metadata should refresh as soon as operation results arrive"
    );
    assert_eq!(
        app.active_view,
        ViewId::Worktrees,
        "completing a background job must not snap the user back to its view"
    );
    assert!(
        app.overlay.is_none(),
        "confirmed-action completion is non-modal"
    );
}

/// Companion to `confirmed_action_completion_is_non_modal`: when the
/// background job returns a typed failure, the Results overlay must
/// auto-open so the user sees the cause and the recovery keys
/// (`!`/`r` once those are wired in #072). Plan P005 §5 + §9.
#[test]
fn failed_confirmed_action_opens_results_overlay() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let dir = tmpdir.path();
    run_git(dir, &["init"]);
    run_git(dir, &["config", "user.email", "test@example.com"]);
    run_git(dir, &["config", "user.name", "Test User"]);
    std::fs::write(dir.join("README.md"), "base\n").unwrap();
    run_git(dir, &["add", "README.md"]);
    run_git(dir, &["commit", "-m", "initial"]);
    run_git(dir, &["branch", "-M", "main"]);

    let mut app = App::new(dir.to_path_buf(), "main".into(), Config::default());

    let branch_name = "feature/delete-fail";
    let (op_tx, op_rx) = mpsc::channel();
    let (_prog_tx, prog_rx) = mpsc::channel();
    app.job_queue.inject_running_for_test(
        git_branch_manager::job_queue::ActionJob {
            action: BranchAction::DeleteLocal,
            targets: vec![branch_name.to_string()],
            remote: None,
            return_view: ViewId::Branches,
            dispatch_path: None,
        },
        op_rx,
        prog_rx,
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    );
    op_tx
        .send(vec![OperationResult::failure(
            branch_name,
            BranchAction::DeleteLocal,
            FailureCause::Other {
                raw_message: "could not delete because reasons".into(),
            },
            "Failed to delete feature/delete-fail: could not delete because reasons",
        )])
        .unwrap();

    app.drain_channels();

    match &app.overlay {
        Some(Overlay::Results { results, .. }) => {
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].branch_name, branch_name);
            assert!(matches!(
                results[0].failure,
                Some(FailureCause::Other { .. })
            ));
        }
        other => panic!("expected Overlay::Results after a failed job, got {other:?}"),
    }
}

/// `BranchNotFound` is treated as success for overlay purposes (plan
/// P005 §9): the row will vanish on the next refresh, so we suppress
/// the modal so the user isn't interrupted by something already
/// resolved on disk.
#[test]
fn branch_not_found_failure_does_not_open_overlay() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let dir = tmpdir.path();
    run_git(dir, &["init"]);
    run_git(dir, &["config", "user.email", "test@example.com"]);
    run_git(dir, &["config", "user.name", "Test User"]);
    std::fs::write(dir.join("README.md"), "base\n").unwrap();
    run_git(dir, &["add", "README.md"]);
    run_git(dir, &["commit", "-m", "initial"]);
    run_git(dir, &["branch", "-M", "main"]);

    let mut app = App::new(dir.to_path_buf(), "main".into(), Config::default());

    let branch_name = "feature/already-gone";
    let (op_tx, op_rx) = mpsc::channel();
    let (_prog_tx, prog_rx) = mpsc::channel();
    app.job_queue.inject_running_for_test(
        git_branch_manager::job_queue::ActionJob {
            action: BranchAction::DeleteLocal,
            targets: vec![branch_name.to_string()],
            remote: None,
            return_view: ViewId::Branches,
            dispatch_path: None,
        },
        op_rx,
        prog_rx,
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    );
    op_tx
        .send(vec![OperationResult::failure(
            branch_name,
            BranchAction::DeleteLocal,
            FailureCause::BranchNotFound,
            format!("Branch not found: {branch_name}"),
        )])
        .unwrap();

    app.drain_channels();

    assert!(
        app.overlay.is_none(),
        "BranchNotFound must not auto-open the Results overlay"
    );
}
