use super::*;

#[test]
fn action_navigation_skips_disabled_rows_in_info_modal_and_menu() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    let items = || {
        vec![
            MenuItem {
                label: "Disabled first".into(),
                shortcut: Some('a'),
                action: BranchAction::DeleteLocal,
                target: "a".into(),
                remote: None,
                enabled: false,
                reason: Some("disabled".into()),
            },
            MenuItem {
                label: "Enabled middle".into(),
                shortcut: Some('b'),
                action: BranchAction::DeleteLocal,
                target: "b".into(),
                remote: None,
                enabled: true,
                reason: None,
            },
            MenuItem {
                label: "Disabled middle".into(),
                shortcut: Some('c'),
                action: BranchAction::DeleteLocal,
                target: "c".into(),
                remote: None,
                enabled: false,
                reason: Some("disabled".into()),
            },
            MenuItem {
                label: "Enabled last".into(),
                shortcut: Some('d'),
                action: BranchAction::DeleteLocal,
                target: "d".into(),
                remote: None,
                enabled: true,
                reason: None,
            },
        ]
    };
    let row = InfoModalRow::Tag(tag("v1.0.0"));
    app.overlay = Some(Overlay::InfoModal {
        items: items(),
        cursor: 0,
        info_cursor: 0,
        focus: InfoModalFocus::Actions,
        row: row.clone(),
    });
    app.handle_overlay_key(KeyEvent::new(
        KeyCode::Down,
        crossterm::event::KeyModifiers::NONE,
    ));
    assert!(matches!(
        app.overlay,
        Some(Overlay::InfoModal {
            cursor: 3,
            focus: InfoModalFocus::Actions,
            ..
        })
    ));
    app.handle_overlay_key(KeyEvent::new(
        KeyCode::Down,
        crossterm::event::KeyModifiers::NONE,
    ));
    assert!(matches!(
        app.overlay,
        Some(Overlay::InfoModal {
            cursor: 3,
            focus: InfoModalFocus::Actions,
            ..
        })
    ));
    app.handle_overlay_key(KeyEvent::new(
        KeyCode::Up,
        crossterm::event::KeyModifiers::NONE,
    ));
    assert!(matches!(
        app.overlay,
        Some(Overlay::InfoModal {
            cursor: 1,
            focus: InfoModalFocus::Actions,
            ..
        })
    ));
    app.handle_overlay_key(KeyEvent::new(
        KeyCode::Up,
        crossterm::event::KeyModifiers::NONE,
    ));
    assert!(matches!(
        app.overlay,
        Some(Overlay::InfoModal {
            cursor: 1,
            focus: InfoModalFocus::Actions,
            ..
        })
    ));

    app.overlay = Some(Overlay::Menu {
        items: items(),
        cursor: 0,
    });
    app.handle_overlay_key(KeyEvent::new(
        KeyCode::Down,
        crossterm::event::KeyModifiers::NONE,
    ));
    assert!(matches!(app.overlay, Some(Overlay::Menu { cursor: 3, .. })));
    app.handle_overlay_key(KeyEvent::new(
        KeyCode::Down,
        crossterm::event::KeyModifiers::NONE,
    ));
    assert!(matches!(app.overlay, Some(Overlay::Menu { cursor: 3, .. })));
    app.handle_overlay_key(KeyEvent::new(
        KeyCode::Up,
        crossterm::event::KeyModifiers::NONE,
    ));
    assert!(matches!(app.overlay, Some(Overlay::Menu { cursor: 1, .. })));
    app.handle_overlay_key(KeyEvent::new(
        KeyCode::Up,
        crossterm::event::KeyModifiers::NONE,
    ));
    assert!(matches!(app.overlay, Some(Overlay::Menu { cursor: 1, .. })));
}

#[test]
fn info_modal_focus_and_cursors_are_independent() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    app.active_view = ViewId::Branches;
    app.branches.set_items(vec![BranchInfo {
        name: "feature/test".into(),
        is_current: false,
        is_base: false,
        tracking: TrackingStatus::Local,
        ahead: None,
        behind: None,
        last_commit_date: Utc::now(),
        merge_status: MergeStatus::Unmerged,
        base_branch: "main".into(),
        merge_base_commit: None,
        pr: None,
        squash_confidence: None,
    }]);
    app.open_context_menu();

    assert!(matches!(
        app.overlay,
        Some(Overlay::InfoModal {
            focus: InfoModalFocus::Actions,
            cursor: 0,
            info_cursor: 0,
            ..
        })
    ));
    app.handle_overlay_key(KeyEvent::new(
        KeyCode::Tab,
        crossterm::event::KeyModifiers::NONE,
    ));
    app.handle_overlay_key(KeyEvent::new(
        KeyCode::Down,
        crossterm::event::KeyModifiers::NONE,
    ));
    assert!(matches!(
        app.overlay,
        Some(Overlay::InfoModal {
            focus: InfoModalFocus::Info,
            cursor: 0,
            info_cursor: 1,
            ..
        })
    ));
    app.handle_overlay_key(KeyEvent::new(
        KeyCode::Tab,
        crossterm::event::KeyModifiers::NONE,
    ));
    app.handle_overlay_key(KeyEvent::new(
        KeyCode::Down,
        crossterm::event::KeyModifiers::NONE,
    ));
    assert!(matches!(
        app.overlay,
        Some(Overlay::InfoModal {
            focus: InfoModalFocus::Actions,
            cursor: 1,
            info_cursor: 1,
            ..
        })
    ));
}

#[test]
fn worktree_menu_delete_branch_enabled_when_clean() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    app.worktrees.set_items(vec![worktree("feature/clean")]);
    app.branches.set_items(vec![BranchInfo {
        name: "feature/clean".into(),
        is_current: false,
        is_base: false,
        tracking: TrackingStatus::Tracked {
            remote_ref: "origin/feature/clean".into(),
            gone: false,
        },
        ahead: None,
        behind: None,
        last_commit_date: Utc::now(),
        merge_status: MergeStatus::Unmerged,
        base_branch: "main".into(),
        merge_base_commit: None,
        pr: None,
        squash_confidence: None,
    }]);

    let items = app.build_worktree_menu();
    let solo = items
        .iter()
        .find(|mi| mi.label == "Remove worktree + branch")
        .unwrap();
    let combo = items
        .iter()
        .find(|mi| mi.label == "Remove worktree + branch (local + remote)")
        .unwrap();
    assert!(solo.enabled && solo.reason.is_none());
    assert!(combo.enabled && combo.reason.is_none());
}

#[test]
fn worktree_menu_delete_branch_remote_disabled_when_no_remote() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    app.worktrees.set_items(vec![worktree("feature/no-remote")]);
    app.branches.set_items(vec![BranchInfo {
        name: "feature/no-remote".into(),
        is_current: false,
        is_base: false,
        tracking: TrackingStatus::Local,
        ahead: None,
        behind: None,
        last_commit_date: Utc::now(),
        merge_status: MergeStatus::Unmerged,
        base_branch: "main".into(),
        merge_base_commit: None,
        pr: None,
        squash_confidence: None,
    }]);

    let items = app.build_worktree_menu();
    let solo = items
        .iter()
        .find(|mi| mi.label == "Remove worktree + branch")
        .unwrap();
    let combo = items
        .iter()
        .find(|mi| mi.label == "Remove worktree + branch (local + remote)")
        .unwrap();
    assert!(solo.enabled && solo.reason.is_none());
    assert!(!combo.enabled);
    assert_eq!(combo.reason.as_deref(), Some("no remote"));
}

#[test]
fn worktree_menu_delete_branch_disabled_when_dirty() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    let mut wt = worktree("feature/dirty");
    wt.wt_status = WorkingTreeStatus {
        has_staged: false,
        has_modified: true,
        has_untracked: false,
        changed_files: Vec::new(),
    };
    app.worktrees.set_items(vec![wt]);

    let solo = app
        .build_worktree_menu()
        .into_iter()
        .find(|mi| mi.label == "Remove worktree + branch")
        .unwrap();
    assert!(!solo.enabled);
    assert_eq!(solo.reason.as_deref(), Some("dirty"));
}

#[test]
fn worktree_menu_delete_branch_disabled_when_detached() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    let mut wt = worktree("feature/detached");
    wt.branch = None;
    app.worktrees.set_items(vec![wt]);

    let solo = app
        .build_worktree_menu()
        .into_iter()
        .find(|mi| mi.label == "Remove worktree + branch")
        .unwrap();
    assert!(!solo.enabled);
    assert_eq!(solo.reason.as_deref(), Some("detached HEAD"));
}

#[test]
fn worktree_menu_delete_branch_disabled_when_base() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    let mut wt = worktree("main");
    wt.is_base = true;
    app.worktrees.set_items(vec![wt]);

    let combo = app
        .build_worktree_menu()
        .into_iter()
        .find(|mi| mi.label == "Remove worktree + branch (local + remote)")
        .unwrap();
    assert!(!combo.enabled);
    assert_eq!(combo.reason.as_deref(), Some("base branch"));
}

// `execute_action`'s worktree-remove-and-delete-branch coverage moved to
// `job_queue.rs`'s own test module, since that's where the (now private)
// function lives.
