use super::*;

#[test]
fn ref_view_context_menus_include_jump_to_graph_for_all_ref_types() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );

    app.active_view = ViewId::Branches;
    app.branches
        .set_items(vec![branch("feature/nav", TrackingStatus::Local)]);
    let branch_jump = app
        .build_menu_items()
        .into_iter()
        .find(|item| item.action == BranchAction::JumpToGraph)
        .expect("Branches menu should expose Graph navigation");
    assert_eq!(branch_jump.shortcut, Some('g'));
    assert_eq!(branch_jump.target, "feature/nav");
    let branch_check = app
        .build_menu_items()
        .into_iter()
        .find(|item| item.action == BranchAction::CheckPrimaryBranchCode)
        .expect("Branches menu should expose primary code check");
    assert!(branch_check.enabled);
    assert_eq!(branch_check.target, "refs/heads/feature/nav");

    app.active_view = ViewId::Remotes;
    app.remotes
        .set_items(vec![remote("origin/feature/nav", "feature/nav")]);
    let remote_jump = app
        .build_menu_items()
        .into_iter()
        .find(|item| item.action == BranchAction::JumpToGraph)
        .expect("Remotes menu should expose Graph navigation");
    assert_eq!(remote_jump.target, "origin/feature/nav");
    let remote_check = app
        .build_menu_items()
        .into_iter()
        .find(|item| item.action == BranchAction::CheckPrimaryBranchCode)
        .expect("Remotes menu should expose primary code check");
    assert_eq!(remote_check.target, "refs/remotes/origin/feature/nav");

    app.active_view = ViewId::Tags;
    app.tags.set_items(vec![tag("v1.2.3")]);
    let tag_jump = app
        .build_menu_items()
        .into_iter()
        .find(|item| item.action == BranchAction::JumpToGraph)
        .expect("Tags menu should expose Graph navigation");
    assert_eq!(tag_jump.target, "v1.2.3");
    let tag_check = app
        .build_menu_items()
        .into_iter()
        .find(|item| item.action == BranchAction::CheckPrimaryBranchCode)
        .expect("Tags menu should expose primary code check");
    assert_eq!(tag_check.target, "refs/tags/v1.2.3");

    app.active_view = ViewId::Worktrees;
    app.worktrees.set_items(vec![worktree("feature/nav")]);
    let worktree_jump = app
        .build_menu_items()
        .into_iter()
        .find(|item| item.action == BranchAction::JumpToGraph)
        .expect("Worktrees menu should expose Graph navigation");
    assert_eq!(worktree_jump.target, "feature/nav");
    let worktree_check = app
        .build_menu_items()
        .into_iter()
        .find(|item| item.action == BranchAction::CheckPrimaryBranchCode)
        .expect("Worktrees menu should expose primary code check");
    assert_eq!(worktree_check.target, "refs/heads/feature/nav");

    let mut detached = worktree("detached");
    detached.branch = None;
    detached.commit_hash = "1234567890abcdef".into();
    app.worktrees.set_items(vec![detached]);
    let detached_check = app
        .build_menu_items()
        .into_iter()
        .find(|item| item.action == BranchAction::CheckPrimaryBranchCode)
        .expect("detached worktree menu should expose primary code check");
    assert_eq!(detached_check.target, "1234567890abcdef");
}

#[test]
fn g_jumps_from_each_ref_view_to_the_selected_graph_commit() {
    let mut app = graph_app(vec![
        graph_ref("feature/nav", graph::GraphRefKind::LocalBranch),
        graph_ref("origin/feature/nav", graph::GraphRefKind::RemoteBranch),
        graph_ref("v1.2.3", graph::GraphRefKind::Tag),
    ]);
    app.branches
        .set_items(vec![branch("feature/nav", TrackingStatus::Local)]);
    app.remotes
        .set_items(vec![remote("origin/feature/nav", "feature/nav")]);
    app.tags.set_items(vec![tag("v1.2.3")]);
    app.worktrees.set_items(vec![worktree("feature/nav")]);

    for view in [
        ViewId::Branches,
        ViewId::Remotes,
        ViewId::Tags,
        ViewId::Worktrees,
    ] {
        app.active_view = view;
        app.handle_key(KeyEvent::new(
            KeyCode::Char('g'),
            crossterm::event::KeyModifiers::NONE,
        ));
        assert_eq!(app.active_view, ViewId::Graph);
        assert_eq!(
            app.graph.selected_commit().unwrap().oid,
            "1111111111111111111111111111111111111111"
        );
    }
}

#[test]
fn context_menu_jump_to_graph_closes_modal_and_selects_commit() {
    let mut app = graph_app(vec![graph_ref(
        "feature/nav",
        graph::GraphRefKind::LocalBranch,
    )]);
    app.active_view = ViewId::Branches;
    app.branches
        .set_items(vec![branch("feature/nav", TrackingStatus::Local)]);

    app.handle_key(KeyEvent::new(
        KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
    ));
    assert!(info_modal_items(&app)
        .iter()
        .any(|item| item.action == BranchAction::JumpToGraph));

    app.handle_key(KeyEvent::new(
        KeyCode::Char('g'),
        crossterm::event::KeyModifiers::NONE,
    ));

    assert_eq!(app.active_view, ViewId::Graph);
    assert!(app.overlay.is_none());
    assert_eq!(app.graph.commit_cursor(), 0);
}
