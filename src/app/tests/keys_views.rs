use super::*;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, crossterm::event::KeyModifiers::NONE)
}

fn fresh_app() -> App {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    )
}

fn fresh_app_with_repo() -> (tempfile::TempDir, App) {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    run_git(tmpdir.path(), &["init", "-b", "main"]);
    run_git(tmpdir.path(), &["config", "user.name", "Test"]);
    run_git(tmpdir.path(), &["config", "user.email", "test@example.com"]);
    run_git(tmpdir.path(), &["commit", "--allow-empty", "-m", "root"]);

    let app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    (tmpdir, app)
}

fn confirm_parts(app: &App) -> (&[ConfirmChoice], &ConfirmStage) {
    match app.overlay.as_ref() {
        Some(Overlay::Confirm { choices, stage, .. }) => (choices, stage),
        other => panic!("expected confirm overlay, got {other:?}"),
    }
}

fn app_with_commits(n: usize) -> App {
    let mut app = fresh_app();
    app.active_view = ViewId::Graph;
    let commits = (0..n)
        .map(|i| graph::GraphCommit {
            oid: format!("{i:040}"),
            summary: format!("commit {i}"),
            ..graph::GraphCommit::default()
        })
        .collect();
    let lines = (0..n)
        .map(|i| graph::GraphLine {
            graph: "*".into(),
            commit_index: Some(i),
        })
        .collect();
    app.graph.apply_result(Ok(graph::GraphSnapshot {
        source: graph::GraphSource::Gleisbau,
        commits,
        lines,
        ref_counts: Default::default(),
        max_count: 500,
        includes_remotes: false,
        generation: None,
    }));
    app
}

fn toast_message(app: &App) -> Option<&str> {
    app.toast.as_ref().map(|toast| toast.message.as_str())
}

#[test]
fn branches_d_opens_local_delete_confirm() {
    let mut app = fresh_app();
    app.branches
        .set_items(vec![branch("feature/a", TrackingStatus::Local)]);

    app.handle_branches_key(key(KeyCode::Char('d')));

    let (choices, stage) = confirm_parts(&app);
    assert!(choices
        .iter()
        .any(|choice| choice.action == BranchAction::DeleteLocal
            && choice.targets == ["feature/a"]));
    assert_eq!(stage, &ConfirmStage::Initial);
    assert_eq!(app.return_view, ViewId::Branches);
}

#[test]
fn branches_upper_d_opens_local_and_remote_delete_confirm() {
    let mut app = fresh_app();
    app.branches.set_items(vec![branch(
        "feature/a",
        TrackingStatus::Tracked {
            remote_ref: "origin/feature/a".into(),
            gone: false,
        },
    )]);

    app.handle_branches_key(key(KeyCode::Char('D')));

    let (choices, stage) = confirm_parts(&app);
    assert_eq!(choices[0].action, BranchAction::DeleteLocalAndRemote);
    assert_eq!(choices[0].targets, ["feature/a"]);
    assert_eq!(stage, &ConfirmStage::Initial);
}

#[test]
fn branches_upper_d_without_remote_tracking_opens_no_confirm() {
    let mut app = fresh_app();
    app.branches
        .set_items(vec![branch("feature/a", TrackingStatus::Local)]);

    app.handle_branches_key(key(KeyCode::Char('D')));

    assert!(app.overlay.is_none());
}

#[test]
fn branches_p_opens_push_confirm_only_with_remote() {
    let mut app = fresh_app();
    app.has_configured_remote = true;
    app.branches
        .set_items(vec![branch("feature/a", TrackingStatus::Local)]);

    app.handle_branches_key(key(KeyCode::Char('p')));

    let (choices, _) = confirm_parts(&app);
    assert_eq!(choices[0].action, BranchAction::Push);
    assert_eq!(choices[0].targets, ["feature/a"]);

    let mut no_remote = fresh_app();
    no_remote.has_configured_remote = false;
    no_remote
        .branches
        .set_items(vec![branch("feature/a", TrackingStatus::Local)]);

    no_remote.handle_branches_key(key(KeyCode::Char('p')));

    assert!(no_remote.overlay.is_none());
}

#[test]
fn branches_upper_r_clears_cache_and_sets_toast() {
    let mut app = fresh_app();

    app.handle_branches_key(key(KeyCode::Char('R')));

    assert_eq!(toast_message(&app), Some("Cache cleared"));
}

#[test]
fn branches_f_and_upper_f_start_fetch() {
    let mut app = fresh_app();
    app.active_view = ViewId::Branches;
    app.handle_branches_key(key(KeyCode::Char('f')));
    assert!(
        matches!(&app.overlay, Some(Overlay::Executing { label, .. }) if label == "Fetching..."),
        "got {:?}",
        app.overlay
    );
    assert!(app.op_rx.is_some());
    assert_eq!(app.return_view, ViewId::Branches);

    let mut app = fresh_app();
    app.active_view = ViewId::Branches;
    app.handle_branches_key(key(KeyCode::Char('F')));
    assert!(
        matches!(&app.overlay, Some(Overlay::Executing { label, .. }) if label == "Fetching with prune..."),
        "got {:?}",
        app.overlay
    );
    assert!(app.op_rx.is_some());
    assert_eq!(app.return_view, ViewId::Branches);
}

#[test]
fn branches_r_t_w_switch_views_and_start_loads() {
    let mut app = fresh_app();
    app.handle_branches_key(key(KeyCode::Char('r')));
    assert_eq!(app.active_view, ViewId::Remotes);
    assert!(app.remotes.loading);
    assert!(app.remote_load_rx.is_some());

    let mut app = fresh_app();
    app.handle_branches_key(key(KeyCode::Char('t')));
    assert_eq!(app.active_view, ViewId::Tags);
    assert!(app.tags.loading);
    assert!(app.tag_load_rx.is_some());

    let mut app = fresh_app();
    app.handle_branches_key(key(KeyCode::Char('w')));
    assert_eq!(app.active_view, ViewId::Worktrees);
    assert!(app.worktrees.loading);
    assert!(app.worktree_load_rx.is_some());
}

#[test]
fn graph_k_and_up_move_cursor_up() {
    let mut app = app_with_commits(5);
    for _ in 0..3 {
        app.graph.move_down();
    }
    assert_eq!(app.graph.commit_cursor(), 3);

    app.handle_graph_key(key(KeyCode::Char('k')));
    assert_eq!(app.graph.commit_cursor(), 2);

    app.handle_graph_key(key(KeyCode::Up));
    assert_eq!(app.graph.commit_cursor(), 1);
}

#[test]
fn graph_page_down_and_page_up_move_by_20() {
    let mut app = app_with_commits(45);

    app.handle_graph_key(key(KeyCode::PageDown));
    assert_eq!(app.graph.commit_cursor(), 20);

    app.handle_graph_key(key(KeyCode::PageUp));
    assert_eq!(app.graph.commit_cursor(), 0);
}

#[test]
fn graph_r_reloads_graph() {
    let mut app = app_with_commits(3);

    app.handle_graph_key(key(KeyCode::Char('r')));

    assert!(app.graph.is_loading());
    assert!(app.graph_rx.is_some());
    assert_eq!(toast_message(&app), Some("Loading graph..."));
}

#[test]
fn graph_unknown_key_is_ignored() {
    let mut app = app_with_commits(3);

    app.handle_graph_key(key(KeyCode::Char('z')));

    assert_eq!(app.graph.commit_cursor(), 0);
    assert!(!app.graph.is_loading());
    assert!(app.graph_rx.is_none());
    assert!(app.overlay.is_none());
}

#[test]
fn jump_from_graph_view_is_noop() {
    let mut app = fresh_app();
    app.active_view = ViewId::Graph;

    app.jump_to_graph_from_active_list();

    assert_eq!(app.active_view, ViewId::Graph);
    assert!(!app.graph.is_loading());
    assert!(app.graph_rx.is_none());
}

#[test]
fn g_on_empty_list_is_noop() {
    for view in [
        ViewId::Branches,
        ViewId::Remotes,
        ViewId::Tags,
        ViewId::Worktrees,
    ] {
        let mut app = fresh_app();
        app.active_view = view;

        app.handle_key(key(KeyCode::Char('g')));

        assert_eq!(app.active_view, view);
        assert!(app.graph_rx.is_none());
    }
}

#[test]
fn g_on_branch_without_snapshot_spawns_load() {
    let mut app = fresh_app();
    app.active_view = ViewId::Branches;
    app.branches
        .set_items(vec![branch("feature/a", TrackingStatus::Local)]);

    app.handle_key(key(KeyCode::Char('g')));

    assert_eq!(app.active_view, ViewId::Graph);
    assert!(app.graph.is_loading());
    assert!(app.graph_rx.is_some());
    assert!(!app.graph.includes_remotes());
}

#[test]
fn g_on_remote_requests_remote_graph() {
    let mut app = fresh_app();
    app.active_view = ViewId::Remotes;
    app.remotes
        .set_items(vec![remote("origin/feature/a", "feature/a")]);

    app.handle_key(key(KeyCode::Char('g')));

    assert_eq!(app.active_view, ViewId::Graph);
    assert!(app.graph.includes_remotes());
    assert!(app.graph.is_loading());
}

#[test]
fn g_on_remote_reloads_snapshot_without_remotes() {
    let (_repo_dir, mut app) = fresh_app_with_repo();
    let mut snapshot = graph_snapshot(vec![]);
    snapshot.includes_remotes = false;
    app.graph.apply_result(Ok(snapshot));
    app.active_view = ViewId::Remotes;
    app.remotes
        .set_items(vec![remote("origin/feature/a", "feature/a")]);

    app.handle_key(key(KeyCode::Char('g')));

    assert_eq!(app.active_view, ViewId::Graph);
    assert!(app.graph.includes_remotes());
    assert!(app.graph.is_loading());

    let result = app
        .graph_rx
        .take()
        .expect("remote jump should spawn a graph reload")
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("graph reload should finish");
    app.graph.apply_result(result);

    assert!(!app.graph.is_loading());
    assert!(
        app.graph
            .snapshot()
            .expect("graph snapshot should load")
            .includes_remotes
    );
}

#[test]
fn g_on_detached_worktree_focuses_its_commit() {
    let mut app = app_with_commits(3);
    let commit_hash = format!("{:040}", 2);
    let mut detached = worktree("detached");
    detached.branch = None;
    detached.commit_hash = commit_hash;
    app.worktrees.set_items(vec![detached]);
    app.active_view = ViewId::Worktrees;

    app.handle_key(key(KeyCode::Char('g')));

    assert_eq!(app.active_view, ViewId::Graph);
    assert_eq!(app.graph.commit_cursor(), 2);
    assert!(!app.graph.is_loading());
}

#[test]
fn remotes_d_confirms_remote_delete_skipping_base() {
    let mut app = fresh_app();
    app.remotes
        .set_items(vec![remote("origin/feature/a", "feature/a")]);

    app.handle_remotes_key(key(KeyCode::Char('d')));

    let (choices, _) = confirm_parts(&app);
    assert_eq!(choices[0].action, BranchAction::DeleteRemoteBranch);
    assert_eq!(choices[0].targets, ["feature/a"]);
    assert_eq!(app.return_view, ViewId::Remotes);

    let mut base_app = fresh_app();
    base_app.remotes.set_items(vec![RemoteBranchInfo {
        is_base: true,
        ..remote("origin/main", "main")
    }]);

    base_app.handle_remotes_key(key(KeyCode::Char('d')));

    assert!(base_app.overlay.is_none());
}

#[test]
fn remotes_f_and_upper_f_start_fetch() {
    let mut app = fresh_app();
    app.active_view = ViewId::Remotes;
    app.handle_remotes_key(key(KeyCode::Char('f')));
    assert!(
        matches!(&app.overlay, Some(Overlay::Executing { label, .. }) if label == "Fetching..."),
        "got {:?}",
        app.overlay
    );
    assert!(app.op_rx.is_some());
    assert_eq!(app.return_view, ViewId::Remotes);

    let mut app = fresh_app();
    app.active_view = ViewId::Remotes;
    app.handle_remotes_key(key(KeyCode::Char('F')));
    assert!(
        matches!(&app.overlay, Some(Overlay::Executing { label, .. }) if label == "Fetching with prune..."),
        "got {:?}",
        app.overlay
    );
    assert!(app.op_rx.is_some());
    assert_eq!(app.return_view, ViewId::Remotes);
}

#[test]
fn remotes_b_r_esc_return_to_branches_without_loading() {
    for code in [KeyCode::Char('b'), KeyCode::Char('r'), KeyCode::Esc] {
        let mut app = fresh_app();
        app.active_view = ViewId::Remotes;

        app.handle_remotes_key(key(code));

        assert_eq!(app.active_view, ViewId::Branches);
        assert!(!app.remotes.loading);
        assert!(app.remote_load_rx.is_none());
    }
}

#[test]
fn remotes_t_and_w_switch_and_load() {
    let mut app = fresh_app();
    app.active_view = ViewId::Remotes;
    app.handle_remotes_key(key(KeyCode::Char('t')));
    assert_eq!(app.active_view, ViewId::Tags);
    assert!(app.tags.loading);

    let mut app = fresh_app();
    app.active_view = ViewId::Remotes;
    app.handle_remotes_key(key(KeyCode::Char('w')));
    assert_eq!(app.active_view, ViewId::Worktrees);
    assert!(app.worktrees.loading);
}

#[test]
fn remotes_unknown_key_is_ignored() {
    let mut app = fresh_app();
    app.active_view = ViewId::Remotes;

    app.handle_remotes_key(key(KeyCode::Char('z')));

    assert_eq!(app.active_view, ViewId::Remotes);
    assert!(app.overlay.is_none());
}

#[test]
fn tags_d_upper_d_p_confirm() {
    let cases = [
        (KeyCode::Char('d'), BranchAction::DeleteTag),
        (KeyCode::Char('D'), BranchAction::DeleteTagAndRemote),
        (KeyCode::Char('p'), BranchAction::PushTag),
    ];
    for (code, action) in cases {
        let mut app = fresh_app();
        app.tags.set_items(vec![tag("v1.0.0")]);

        app.handle_tags_key(key(code));

        let (choices, stage) = confirm_parts(&app);
        assert_eq!(choices.len(), 1, "key {code:?}");
        assert_eq!(choices[0].action, action, "key {code:?}");
        assert_eq!(choices[0].targets, ["v1.0.0"], "key {code:?}");
        assert_eq!(stage, &ConfirmStage::Initial, "key {code:?}");
        assert_eq!(app.return_view, ViewId::Tags, "key {code:?}");
    }
}

#[test]
fn tags_f_and_upper_f_start_fetch() {
    let mut app = fresh_app();
    app.active_view = ViewId::Tags;
    app.handle_tags_key(key(KeyCode::Char('f')));
    assert!(
        matches!(&app.overlay, Some(Overlay::Executing { label, .. }) if label == "Fetching..."),
        "got {:?}",
        app.overlay
    );
    assert!(app.op_rx.is_some());
    assert_eq!(app.return_view, ViewId::Tags);

    let mut app = fresh_app();
    app.active_view = ViewId::Tags;
    app.handle_tags_key(key(KeyCode::Char('F')));
    assert!(
        matches!(&app.overlay, Some(Overlay::Executing { label, .. }) if label == "Fetching with prune..."),
        "got {:?}",
        app.overlay
    );
    assert!(app.op_rx.is_some());
    assert_eq!(app.return_view, ViewId::Tags);
}

#[test]
fn tags_b_t_esc_return_to_branches() {
    for code in [KeyCode::Char('b'), KeyCode::Char('t'), KeyCode::Esc] {
        let mut app = fresh_app();
        app.active_view = ViewId::Tags;

        app.handle_tags_key(key(code));

        assert_eq!(app.active_view, ViewId::Branches);
    }
}

#[test]
fn tags_r_and_w_switch_and_load() {
    let mut app = fresh_app();
    app.active_view = ViewId::Tags;
    app.handle_tags_key(key(KeyCode::Char('r')));
    assert_eq!(app.active_view, ViewId::Remotes);
    assert!(app.remotes.loading);

    let mut app = fresh_app();
    app.active_view = ViewId::Tags;
    app.handle_tags_key(key(KeyCode::Char('w')));
    assert_eq!(app.active_view, ViewId::Worktrees);
    assert!(app.worktrees.loading);
}

#[test]
fn tags_unknown_key_is_ignored() {
    let mut app = fresh_app();
    app.active_view = ViewId::Tags;

    app.handle_tags_key(key(KeyCode::Char('z')));

    assert_eq!(app.active_view, ViewId::Tags);
    assert!(app.overlay.is_none());
}

#[test]
fn worktrees_d_confirms_remove_skipping_main() {
    let mut app = fresh_app();
    app.worktrees.set_items(vec![worktree("feature/a")]);

    app.handle_worktrees_key(key(KeyCode::Char('d')));

    let (choices, stage) = confirm_parts(&app);
    assert_eq!(choices.len(), 1);
    assert_eq!(choices[0].action, BranchAction::WorktreeRemove);
    assert_eq!(choices[0].targets, ["/repo/.worktrees/feature-a"]);
    assert_eq!(stage, &ConfirmStage::Initial);

    let mut main_app = fresh_app();
    main_app.worktrees.set_items(vec![WorktreeInfo {
        is_main: true,
        ..worktree("feature/a")
    }]);

    main_app.handle_worktrees_key(key(KeyCode::Char('d')));

    assert!(main_app.overlay.is_none());
}

#[test]
fn worktrees_upper_d_opens_final_destructive_confirm() {
    let mut app = fresh_app();
    app.worktrees.set_items(vec![worktree("feature/a")]);

    app.handle_worktrees_key(key(KeyCode::Char('D')));

    let (choices, stage) = confirm_parts(&app);
    assert_eq!(choices.len(), 1);
    assert_eq!(choices[0].action, BranchAction::WorktreeForceRemove);
    assert!(matches!(
        stage,
        ConfirmStage::FinalDestructive { choice, target: ConfirmTarget::Worktree(path) }
            if choice.action == BranchAction::WorktreeForceRemove
                && path.as_path() == std::path::Path::new("/repo/.worktrees/feature-a")
    ));
}

#[test]
fn worktrees_f_and_upper_f_start_fetch() {
    let mut app = fresh_app();
    app.active_view = ViewId::Worktrees;
    app.handle_worktrees_key(key(KeyCode::Char('f')));
    assert!(
        matches!(&app.overlay, Some(Overlay::Executing { label, .. }) if label == "Fetching..."),
        "got {:?}",
        app.overlay
    );
    assert!(app.op_rx.is_some());
    assert_eq!(app.return_view, ViewId::Worktrees);

    let mut app = fresh_app();
    app.active_view = ViewId::Worktrees;
    app.handle_worktrees_key(key(KeyCode::Char('F')));
    assert!(
        matches!(&app.overlay, Some(Overlay::Executing { label, .. }) if label == "Fetching with prune..."),
        "got {:?}",
        app.overlay
    );
    assert!(app.op_rx.is_some());
    assert_eq!(app.return_view, ViewId::Worktrees);
}

#[test]
fn worktrees_b_w_esc_return_to_branches() {
    for code in [KeyCode::Char('b'), KeyCode::Char('w'), KeyCode::Esc] {
        let mut app = fresh_app();
        app.active_view = ViewId::Worktrees;

        app.handle_worktrees_key(key(code));

        assert_eq!(app.active_view, ViewId::Branches);
    }
}

#[test]
fn worktrees_r_and_t_switch_and_load() {
    let mut app = fresh_app();
    app.active_view = ViewId::Worktrees;
    app.handle_worktrees_key(key(KeyCode::Char('r')));
    assert_eq!(app.active_view, ViewId::Remotes);
    assert!(app.remotes.loading);

    let mut app = fresh_app();
    app.active_view = ViewId::Worktrees;
    app.handle_worktrees_key(key(KeyCode::Char('t')));
    assert_eq!(app.active_view, ViewId::Tags);
    assert!(app.tags.loading);
}

#[test]
fn worktrees_unknown_key_is_ignored() {
    let mut app = fresh_app();
    app.active_view = ViewId::Worktrees;

    app.handle_worktrees_key(key(KeyCode::Char('z')));

    assert_eq!(app.active_view, ViewId::Worktrees);
    assert!(app.overlay.is_none());
}
