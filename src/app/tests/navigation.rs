use super::*;

#[test]
fn graph_navigation_and_horizontal_scroll_keys_bypass_generic_table_handler() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    app.active_view = ViewId::Graph;
    app.graph.apply_result(Ok(graph::GraphSnapshot {
        source: graph::GraphSource::Gleisbau,
        commits: vec![
            graph::GraphCommit {
                oid: "1111111111111111111111111111111111111111".into(),
                summary: "first".into(),
                parents: vec![],
                lane: Some(0),
                branch: None,
                refs: vec![],
                relationships: vec![],
                ..graph::GraphCommit::default()
            },
            graph::GraphCommit {
                oid: "2222222222222222222222222222222222222222".into(),
                summary: "second".into(),
                parents: vec![],
                lane: Some(0),
                branch: None,
                refs: vec![],
                relationships: vec![],
                ..graph::GraphCommit::default()
            },
        ],
        lines: vec![
            graph::GraphLine {
                graph: "*".into(),
                commit_index: Some(0),
            },
            graph::GraphLine {
                graph: "*".into(),
                commit_index: Some(1),
            },
        ],
        ref_counts: Default::default(),
        max_count: 500,
        includes_remotes: false,
        generation: None,
    }));

    app.handle_key(KeyEvent::new(
        KeyCode::Char('j'),
        crossterm::event::KeyModifiers::NONE,
    ));
    assert_eq!(app.graph.commit_cursor(), 1);
    app.handle_key(KeyEvent::new(
        KeyCode::Char('g'),
        crossterm::event::KeyModifiers::NONE,
    ));
    assert_eq!(app.graph.commit_cursor(), 0);
    app.handle_key(KeyEvent::new(
        KeyCode::Char('G'),
        crossterm::event::KeyModifiers::NONE,
    ));
    assert_eq!(app.graph.commit_cursor(), 1);
    app.handle_key(KeyEvent::new(
        KeyCode::Char('l'),
        crossterm::event::KeyModifiers::NONE,
    ));
    assert_eq!(app.graph.commit_cursor(), 1);
    assert_eq!(app.graph.horizontal_offset(), 1);
    app.handle_key(KeyEvent::new(
        KeyCode::Left,
        crossterm::event::KeyModifiers::NONE,
    ));
    assert_eq!(app.graph.horizontal_offset(), 0);
    app.handle_key(KeyEvent::new(
        KeyCode::Right,
        crossterm::event::KeyModifiers::NONE,
    ));
    assert_eq!(app.graph.horizontal_offset(), 1);
    assert_eq!(app.graph.commit_cursor(), 1);
    app.handle_key(KeyEvent::new(
        KeyCode::Tab,
        crossterm::event::KeyModifiers::NONE,
    ));
    assert_eq!(app.active_view, ViewId::Branches);
}

#[test]
fn horizontal_table_scroll_is_configured_per_list_view() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    app.branches.set_items(vec![
        branch("feature/a", TrackingStatus::Local),
        branch("feature/b", TrackingStatus::Local),
    ]);
    app.branches.set_cursor(1);
    app.branches.selected_mut()[0] = true;
    app.branches.set_sort(Some(0), false);
    app.branches.set_horizontal_scroll_bounds(4);

    app.active_view = ViewId::Branches;
    app.handle_key(KeyEvent::new(
        KeyCode::Char('l'),
        crossterm::event::KeyModifiers::NONE,
    ));
    assert_eq!(app.branches.horizontal_offset(), 0);

    app.config.horizontal_scrolling = Some(true);
    app.handle_key(KeyEvent::new(
        KeyCode::Char('l'),
        crossterm::event::KeyModifiers::NONE,
    ));
    app.handle_key(KeyEvent::new(
        KeyCode::Right,
        crossterm::event::KeyModifiers::NONE,
    ));
    assert_eq!(app.branches.horizontal_offset(), 2);
    assert_eq!(app.branches.cursor(), 1);
    assert_eq!(app.branches.selected(), &[true, false]);
    assert_eq!(app.branches.sort_column(), Some(0));
    assert!(!app.branches.sort_ascending());

    app.remotes.set_horizontal_scroll_bounds(2);
    app.active_view = ViewId::Remotes;
    app.handle_key(KeyEvent::new(
        KeyCode::Right,
        crossterm::event::KeyModifiers::NONE,
    ));
    assert_eq!(app.remotes.horizontal_offset(), 1);
    app.active_view = ViewId::Branches;
    assert_eq!(app.branches.horizontal_offset(), 2);

    app.branches.header_columns = vec![(10, 14, 5), (30, 34, 2)];
    assert_eq!(app.find_header_click(11), Some(5));
    assert_eq!(app.find_header_click(20), None);
    assert_eq!(app.find_header_click(32), Some(2));
}

#[test]
fn tab_cycles_through_all_five_views_forward_and_reverse() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    assert_eq!(app.active_view, ViewId::Graph);

    // Forward: Graph -> Branches -> Remotes -> Tags -> Worktrees -> Graph
    let forward = [
        ViewId::Branches,
        ViewId::Remotes,
        ViewId::Tags,
        ViewId::Worktrees,
        ViewId::Graph,
    ];
    for expected in forward {
        app.handle_key(KeyEvent::new(
            KeyCode::Tab,
            crossterm::event::KeyModifiers::NONE,
        ));
        assert_eq!(app.active_view, expected);
    }

    // Reverse: Shift-Tab (Worktrees -> Tags -> Remotes -> Branches -> Graph)
    app.active_view = ViewId::Graph;
    let reverse_shift = [
        ViewId::Worktrees,
        ViewId::Tags,
        ViewId::Remotes,
        ViewId::Branches,
        ViewId::Graph,
    ];
    for expected in reverse_shift {
        app.handle_key(KeyEvent::new(
            KeyCode::Tab,
            crossterm::event::KeyModifiers::SHIFT,
        ));
        assert_eq!(app.active_view, expected);
    }

    // Reverse: BackTab (same direction as Shift-Tab)
    app.active_view = ViewId::Graph;
    let reverse_backtab = [
        ViewId::Worktrees,
        ViewId::Tags,
        ViewId::Remotes,
        ViewId::Branches,
        ViewId::Graph,
    ];
    for expected in reverse_backtab {
        app.handle_key(KeyEvent::new(
            KeyCode::BackTab,
            crossterm::event::KeyModifiers::NONE,
        ));
        assert_eq!(app.active_view, expected);
    }
}

#[test]
fn existing_four_tab_navigation_still_works_after_graph_tab_added() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );

    // Branches: cursor moves with j/k; Enter opens a context menu
    app.active_view = ViewId::Branches;
    app.branches
        .set_items(vec![branch("feature/a", TrackingStatus::Local)]);
    app.handle_key(KeyEvent::new(
        KeyCode::Char('j'),
        crossterm::event::KeyModifiers::NONE,
    ));
    app.handle_key(KeyEvent::new(
        KeyCode::Char('k'),
        crossterm::event::KeyModifiers::NONE,
    ));
    app.handle_key(KeyEvent::new(
        KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
    ));
    assert!(matches!(app.overlay, Some(Overlay::InfoModal { .. })));
    app.overlay = None;

    // Remotes: cursor moves with j/k; Enter opens a context menu
    app.active_view = ViewId::Remotes;
    app.remotes
        .set_items(vec![remote("origin/feature/b", "feature/b")]);
    app.handle_key(KeyEvent::new(
        KeyCode::Char('j'),
        crossterm::event::KeyModifiers::NONE,
    ));
    app.handle_key(KeyEvent::new(
        KeyCode::Char('k'),
        crossterm::event::KeyModifiers::NONE,
    ));
    app.handle_key(KeyEvent::new(
        KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
    ));
    assert!(matches!(app.overlay, Some(Overlay::InfoModal { .. })));
    app.overlay = None;

    // Tags: cursor moves with j/k; Enter opens a context menu
    app.active_view = ViewId::Tags;
    app.tags.set_items(vec![tag("v1.0.0")]);
    app.handle_key(KeyEvent::new(
        KeyCode::Char('j'),
        crossterm::event::KeyModifiers::NONE,
    ));
    app.handle_key(KeyEvent::new(
        KeyCode::Char('k'),
        crossterm::event::KeyModifiers::NONE,
    ));
    app.handle_key(KeyEvent::new(
        KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
    ));
    assert!(matches!(app.overlay, Some(Overlay::InfoModal { .. })));
    app.overlay = None;

    // Worktrees: cursor moves with j/k; Enter opens a context menu
    app.active_view = ViewId::Worktrees;
    app.worktrees.set_items(vec![worktree("feature/c")]);
    app.handle_key(KeyEvent::new(
        KeyCode::Char('j'),
        crossterm::event::KeyModifiers::NONE,
    ));
    app.handle_key(KeyEvent::new(
        KeyCode::Char('k'),
        crossterm::event::KeyModifiers::NONE,
    ));
    app.handle_key(KeyEvent::new(
        KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
    ));
    assert!(matches!(app.overlay, Some(Overlay::InfoModal { .. })));
}
