use super::*;

#[test]
fn pending_graph_details_success_is_abandoned_when_help_opens() {
    let mut app = graph_app(vec![]);
    let commit = app.graph.selected_commit().unwrap().clone();
    let (tx, rx) = mpsc::channel();
    app.commit_details_target = Some(commit.oid.clone());
    app.commit_details_rx = Some(rx);
    tx.send((commit.oid.clone(), Ok(details_for(&commit, Vec::new()))))
        .unwrap();

    app.handle_key(KeyEvent::new(
        KeyCode::Char('?'),
        crossterm::event::KeyModifiers::NONE,
    ));
    app.drain_channels();

    assert!(matches!(app.overlay, Some(Overlay::Help { .. })));
    assert!(app.commit_details_target.is_none());
    assert!(app.commit_details_rx.is_none());
    assert!(app.toast.is_none());
}

#[test]
fn pending_graph_details_error_is_abandoned_when_view_changes() {
    let mut app = graph_app(vec![]);
    let commit = app.graph.selected_commit().unwrap();
    let (tx, rx) = mpsc::channel();
    app.commit_details_target = Some(commit.oid.clone());
    app.commit_details_rx = Some(rx);
    tx.send((commit.oid.clone(), Err("late details error".into())))
        .unwrap();

    app.handle_key(KeyEvent::new(
        KeyCode::Tab,
        crossterm::event::KeyModifiers::NONE,
    ));
    app.toast = None;
    app.drain_channels();

    assert_ne!(app.active_view, ViewId::Graph);
    assert!(app.overlay.is_none());
    assert!(app.commit_details_target.is_none());
    assert!(app.commit_details_rx.is_none());
    assert!(app.toast.is_none());
}

#[test]
fn tag_metadata_refreshes_commit_diff_and_parent_details_on_escape() {
    let mut app = graph_app(vec![
        graph_ref("feature/local", graph::GraphRefKind::LocalBranch),
        graph_ref("v1.2.3", graph::GraphRefKind::Tag),
    ]);
    app.branches
        .set_items(vec![branch("feature/local", TrackingStatus::Local)]);
    let commit = app.graph.selected_commit().unwrap().clone();
    let initial_items = app.build_graph_menu_for(&commit);
    let initial_cursor = first_enabled_index(&initial_items).expect("local action");
    let selected_action = &initial_items[initial_cursor];
    let selected_identity = (
        selected_action.action,
        selected_action.target.clone(),
        selected_action.remote.clone(),
    );
    let files = ["first.txt", "second.txt"]
        .into_iter()
        .map(|path| commit_details::ChangedCommitFile {
            path: path.into(),
            old_path: None,
            kind: commit_details::CommitFileKind::Modified,
        })
        .collect();
    let details = details_for(&commit, files);
    let mut parent_scroll = ModalScroll::default();
    parent_scroll.offset = 7;
    app.overlay = Some(Overlay::CommitDetails {
        commit: commit.clone(),
        details,
        items: initial_items,
        cursor: initial_cursor,
        file_cursor: 1,
        focus: CommitDetailsFocus::Files,
        scroll: parent_scroll,
    });

    app.handle_overlay_key(KeyEvent::new(
        KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
    ));
    let target = app.commit_file_diff_target.clone().unwrap();
    let (diff_tx, diff_rx) = mpsc::channel();
    app.commit_file_diff_rx = Some(diff_rx);
    diff_tx
        .send((
            target,
            Ok(commit_details::CommitFileDiff {
                path: "second.txt".into(),
                patch: "@@ -1 +1 @@\n-old\n+new".into(),
            }),
        ))
        .unwrap();
    app.drain_channels();
    assert!(matches!(app.overlay, Some(Overlay::CommitDiff { .. })));

    if let Some(Overlay::CommitDiff { scroll, .. }) = app.overlay.as_mut() {
        scroll.offset = 4;
    }
    let (tag_tx, tag_rx) = mpsc::channel();
    tag_tx.send(vec![tag("v1.2.3")]).unwrap();
    app.tag_load_rx = Some(tag_rx);
    app.drain_channels();

    assert!(matches!(
        app.overlay,
        Some(Overlay::CommitDiff {
            cursor,
            file_cursor: 1,
            focus: CommitDetailsFocus::Files,
            details_scroll: ModalScroll { offset: 7, .. },
            scroll: ModalScroll { offset: 4, .. },
            ref items,
            ..
        }) if items.get(cursor).is_some_and(|item| (item.action, item.target.clone(), item.remote.clone()) == selected_identity)
            && items.iter().any(|item| item.action == BranchAction::PushTag)
    ));
    app.handle_overlay_key(KeyEvent::new(
        KeyCode::Esc,
        crossterm::event::KeyModifiers::NONE,
    ));
    assert!(matches!(
        app.overlay,
        Some(Overlay::CommitDetails {
            file_cursor: 1,
            focus: CommitDetailsFocus::Files,
            scroll: ModalScroll { offset: 7, .. },
            ref items,
            ..
        }) if items.iter().any(|item| item.action == BranchAction::PushTag)
    ));
}

#[test]
fn commit_details_nested_navigation_preserves_file_cursor() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let dir = tmpdir.path();
    run_git(dir, &["init", "-b", "main"]);
    run_git(dir, &["config", "user.email", "test@example.com"]);
    run_git(dir, &["config", "user.name", "Test User"]);
    std::fs::write(dir.join("one.txt"), "one\n").unwrap();
    std::fs::write(dir.join("two.txt"), "two\n").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "initial"]);
    let oid = String::from_utf8(
        Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(dir)
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_string();
    let mut app = App::new(dir.to_path_buf(), "main".into(), Config::default());
    let details = commit_details::CommitDetails {
        oid: oid.clone(),
        summary: "initial".into(),
        author_name: String::new(),
        author_email: String::new(),
        authored_at: None,
        message_lines: vec!["initial".into()],
        mode: commit_details::CommitDetailMode::Commit { oid: oid.clone() },
        files: vec![
            commit_details::ChangedCommitFile {
                path: "one.txt".into(),
                old_path: None,
                kind: commit_details::CommitFileKind::Added,
            },
            commit_details::ChangedCommitFile {
                path: "two.txt".into(),
                old_path: None,
                kind: commit_details::CommitFileKind::Added,
            },
        ],
        branch_log: Vec::new(),
    };
    let file_menu = Vec::new();
    let mut details_scroll = ModalScroll::default();
    details_scroll.offset = 3;
    app.overlay = Some(Overlay::CommitDetails {
        commit: graph::GraphCommit {
            oid: oid.clone(),
            ..Default::default()
        },
        details,
        items: file_menu,
        cursor: 0,
        file_cursor: 0,
        focus: CommitDetailsFocus::Files,
        scroll: details_scroll,
    });
    app.handle_overlay_key(KeyEvent::new(
        KeyCode::Char('j'),
        crossterm::event::KeyModifiers::NONE,
    ));
    app.handle_overlay_key(KeyEvent::new(
        KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
    ));
    assert!(app.commit_file_diff_rx.is_some());
    let (tx, rx) = mpsc::channel();
    app.commit_file_diff_rx = Some(rx);
    tx.send((
        (oid.clone(), "two.txt".into()),
        Ok(commit_details::CommitFileDiff {
            path: "two.txt".into(),
            patch: "@@ -0,0 +1 @@\n+two".into(),
        }),
    ))
    .unwrap();
    app.drain_channels();
    assert!(matches!(
        app.overlay,
        Some(Overlay::CommitDiff { file_cursor: 1, .. })
    ));
    app.handle_overlay_key(KeyEvent::new(
        KeyCode::Esc,
        crossterm::event::KeyModifiers::NONE,
    ));
    assert!(matches!(
        app.overlay,
        Some(Overlay::CommitDetails {
            file_cursor: 1,
            scroll: ModalScroll { offset: 3, .. },
            ..
        })
    ));
    app.handle_overlay_key(KeyEvent::new(
        KeyCode::Esc,
        crossterm::event::KeyModifiers::NONE,
    ));
    assert!(app.overlay.is_none());
}
