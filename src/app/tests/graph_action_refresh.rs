use super::*;

#[test]
fn graph_multiple_refs_confirm_the_selected_authoritative_target() {
    let mut app = graph_app(vec![
        graph_ref("feature/local", graph::GraphRefKind::LocalBranch),
        graph_ref("origin/feature/remote", graph::GraphRefKind::RemoteBranch),
        graph_ref("v1.2.3", graph::GraphRefKind::Tag),
    ]);
    let mut local = branch("feature/local", TrackingStatus::Local);
    local.is_current = true;
    app.branches.set_items(vec![local]);
    app.remotes
        .set_items(vec![remote("origin/feature/remote", "feature/remote")]);
    app.tags.set_items(vec![tag("v1.2.3")]);

    app.handle_key(KeyEvent::new(
        KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
    ));
    complete_graph_commit_details(&mut app);

    let items = commit_details_items(&app);
    assert!(items
        .iter()
        .all(|item| item.shortcut.is_none() && item.label.contains(":")));
    let disabled_local_delete = items
        .iter()
        .find(|item| item.action == BranchAction::DeleteLocal)
        .expect("local Delete action");
    assert!(!disabled_local_delete.enabled);
    assert_eq!(disabled_local_delete.reason.as_deref(), Some("current"));

    let remote_delete_cursor = items
        .iter()
        .position(|item| item.action == BranchAction::DeleteRemoteBranch)
        .expect("remote Delete action");
    let overlay = app.overlay.take().unwrap();
    let Overlay::CommitDetails {
        commit,
        details,
        items,
        file_cursor,
        focus,
        scroll,
        ..
    } = overlay
    else {
        unreachable!();
    };
    app.overlay = Some(Overlay::CommitDetails {
        commit,
        details,
        items,
        cursor: remote_delete_cursor,
        file_cursor,
        focus,
        scroll,
    });

    app.handle_overlay_key(KeyEvent::new(
        KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
    ));

    assert!(matches!(
        app.overlay,
        Some(Overlay::Confirm {
            ref choices,
            ..
        }) if matches!(
            choices.as_slice(),
            [ConfirmChoice {
                action: BranchAction::DeleteRemoteBranch,
                targets,
                ..
            }] if targets == &["feature/remote"]
        )
    ));
    assert_eq!(app.return_view, ViewId::Graph);
}

#[test]
fn graph_origin_job_refreshes_authoritative_list_and_graph() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let dir = tmpdir.path();
    run_git(dir, &["init"]);
    run_git(dir, &["config", "user.email", "test@example.com"]);
    run_git(dir, &["config", "user.name", "Test User"]);
    std::fs::write(dir.join("README.md"), "base\n").unwrap();
    run_git(dir, &["add", "README.md"]);
    run_git(dir, &["commit", "-m", "initial"]);
    run_git(dir, &["branch", "-M", "main"]);
    run_git(dir, &["branch", "feature/refresh"]);

    let mut app = App::new(dir.to_path_buf(), "main".into(), Config::default());
    app.active_view = ViewId::Tags;
    app.branches.set_items(vec![]);
    app.graph.apply_result(Ok(graph_snapshot(vec![graph_ref(
        "feature/refresh",
        graph::GraphRefKind::LocalBranch,
    )])));

    let (op_tx, op_rx) = mpsc::channel();
    let (_prog_tx, prog_rx) = mpsc::channel();
    app.job_queue.inject_running_with_graph_delta_for_test(
        git_branch_manager::job_queue::ActionJob {
            action: BranchAction::Push,
            targets: vec!["feature/refresh".into()],
            remote: None,
            return_view: ViewId::Graph,
            dispatch_path: None,
        },
        op_rx,
        unchanged_graph_observation(dir),
        prog_rx,
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    );
    op_tx
        .send(vec![OperationResult::success(
            "feature/refresh",
            BranchAction::Push,
            "Pushed feature/refresh",
        )])
        .unwrap();

    app.drain_channels();

    assert!(app
        .branches
        .items()
        .iter()
        .any(|branch| branch.name == "feature/refresh"));
    assert!(app.remotes.loading);
    assert!(app.graph.snapshot().is_some());
    assert_eq!(app.active_view, ViewId::Tags);
}

#[test]
fn graph_origin_remote_checkout_refreshes_branches_remotes_and_graph() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let dir = tmpdir.path();
    run_git(dir, &["init"]);
    run_git(dir, &["config", "user.email", "test@example.com"]);
    run_git(dir, &["config", "user.name", "Test User"]);
    std::fs::write(dir.join("README.md"), "base\n").unwrap();
    run_git(dir, &["add", "README.md"]);
    run_git(dir, &["commit", "-m", "initial"]);
    run_git(dir, &["branch", "-M", "main"]);
    run_git(dir, &["branch", "feature/remote"]);

    let mut app = App::new(dir.to_path_buf(), "main".into(), Config::default());
    app.active_view = ViewId::Tags;
    app.branches.set_items(vec![]);
    app.graph.apply_result(Ok(graph_snapshot(vec![graph_ref(
        "origin/feature/remote",
        graph::GraphRefKind::RemoteBranch,
    )])));

    let (op_tx, op_rx) = mpsc::channel();
    let (_prog_tx, prog_rx) = mpsc::channel();
    app.job_queue.inject_running_with_graph_delta_for_test(
        git_branch_manager::job_queue::ActionJob {
            action: BranchAction::CheckoutRemote,
            targets: vec!["feature/remote".into()],
            remote: None,
            return_view: ViewId::Graph,
            dispatch_path: None,
        },
        op_rx,
        unchanged_graph_observation(dir),
        prog_rx,
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    );
    op_tx
        .send(vec![OperationResult::success(
            "feature/remote",
            BranchAction::CheckoutRemote,
            "Checked out feature/remote",
        )])
        .unwrap();

    app.drain_channels();

    assert!(app
        .branches
        .items()
        .iter()
        .any(|branch| branch.name == "feature/remote"));
    assert!(app.remotes.loading);
    assert!(app.worktrees.loading);
    assert!(app.graph.snapshot().is_some());
    assert_eq!(app.active_view, ViewId::Tags);
}

#[test]
fn graph_origin_remote_delete_and_local_refreshes_branches_remotes_and_graph() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let dir = tmpdir.path();
    run_git(dir, &["init"]);
    run_git(dir, &["config", "user.email", "test@example.com"]);
    run_git(dir, &["config", "user.name", "Test User"]);
    std::fs::write(dir.join("README.md"), "base\n").unwrap();
    run_git(dir, &["add", "README.md"]);
    run_git(dir, &["commit", "-m", "initial"]);
    run_git(dir, &["branch", "-M", "main"]);
    run_git(dir, &["branch", "feature/remote"]);

    let mut app = App::new(dir.to_path_buf(), "main".into(), Config::default());
    app.active_view = ViewId::Tags;
    app.branches.set_items(vec![]);
    app.graph.apply_result(Ok(graph_snapshot(vec![graph_ref(
        "origin/feature/remote",
        graph::GraphRefKind::RemoteBranch,
    )])));

    let (op_tx, op_rx) = mpsc::channel();
    let (_prog_tx, prog_rx) = mpsc::channel();
    app.job_queue.inject_running_with_graph_delta_for_test(
        git_branch_manager::job_queue::ActionJob {
            action: BranchAction::DeleteRemoteAndLocal,
            targets: vec!["feature/remote".into()],
            remote: None,
            return_view: ViewId::Graph,
            dispatch_path: None,
        },
        op_rx,
        unchanged_graph_observation(dir),
        prog_rx,
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    );
    op_tx
        .send(vec![OperationResult::success(
            "feature/remote",
            BranchAction::DeleteRemoteAndLocal,
            "Deleted feature/remote",
        )])
        .unwrap();

    app.drain_channels();

    assert!(app
        .branches
        .items()
        .iter()
        .any(|branch| branch.name == "feature/remote"));
    assert!(app.remotes.loading);
    assert!(app.graph.snapshot().is_some());
    assert_eq!(app.active_view, ViewId::Tags);
}

#[test]
fn graph_action_refreshes_dependent_ref_views() {
    // P110: every state-mutating action now also refreshes the Graph view.
    assert_eq!(
        graph_affected_views(BranchAction::DeleteLocal),
        &[ViewId::Branches, ViewId::Remotes, ViewId::Graph]
    );
    assert_eq!(
        graph_affected_views(BranchAction::DeleteRemoteBranch),
        &[ViewId::Branches, ViewId::Remotes, ViewId::Graph]
    );
    assert_eq!(
        graph_affected_views(BranchAction::FetchRemote),
        &[ViewId::Branches, ViewId::Remotes, ViewId::Tags, ViewId::Graph]
    );
    // UI-only actions stay empty.
    assert_eq!(graph_affected_views(BranchAction::JumpToGraph), &[]);
    assert_eq!(graph_affected_views(BranchAction::ViewRemotePR), &[]);
}

#[test]
fn relevant_action_from_another_view_refreshes_loaded_graph_without_navigating() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = action_refresh_app(tmpdir.path());
    app.active_view = ViewId::Branches;
    app.graph.apply_result(Ok(graph_snapshot(vec![graph_ref(
        "feature/refresh",
        graph::GraphRefKind::LocalBranch,
    )])));
    let generation = app.graph_generation;

    complete_action_job_for_test(&mut app, BranchAction::Checkout, ViewId::Branches);

    assert!(app.graph.is_loading());
    assert_eq!(app.graph_generation, generation + 1);
    assert_eq!(
        app.graph_generation_signal.load(Ordering::Acquire),
        app.graph_generation
    );
    assert_eq!(app.active_view, ViewId::Branches);
}

#[test]
fn relevant_action_does_not_load_graph_before_a_snapshot_exists() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = action_refresh_app(tmpdir.path());
    app.active_view = ViewId::Branches;

    complete_action_job_for_test(&mut app, BranchAction::Checkout, ViewId::Branches);

    assert!(!app.graph.is_loading());
    assert!(app.graph.snapshot().is_none());
    assert_eq!(app.graph_generation, 0);
    assert_eq!(app.active_view, ViewId::Branches);
}

#[test]
fn branch_origin_delete_refreshes_remote_and_worktree_views() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let dir = tmpdir.path();
    run_git(dir, &["init", "-b", "main"]);
    run_git(dir, &["config", "user.email", "test@example.com"]);
    run_git(dir, &["config", "user.name", "Test User"]);
    std::fs::write(dir.join("README.md"), "base\n").unwrap();
    run_git(dir, &["add", "README.md"]);
    run_git(dir, &["commit", "-m", "initial"]);

    let mut app = App::new(dir.to_path_buf(), "main".into(), Config::default());
    app.remotes.loading = false;
    app.worktrees.loading = false;

    let (op_tx, op_rx) = mpsc::channel();
    let (_prog_tx, prog_rx) = mpsc::channel();
    app.job_queue.inject_running_for_test(
        git_branch_manager::job_queue::ActionJob {
            action: BranchAction::DeleteBranchAndRemoveWorktreeForce,
            targets: vec!["feature/delete".into()],
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
            "feature/delete",
            BranchAction::DeleteBranchAndRemoveWorktreeForce,
            "Deleted feature/delete",
        )])
        .unwrap();

    app.drain_channels();

    assert!(app.remotes.loading);
    assert!(app.worktrees.loading);
}
