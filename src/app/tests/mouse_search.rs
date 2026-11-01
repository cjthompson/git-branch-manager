use super::*;
use crossterm::event::{KeyModifiers, MouseEvent, MouseEventKind};

const LIST_VIEWS: [ViewId; 4] = [
    ViewId::Branches,
    ViewId::Remotes,
    ViewId::Tags,
    ViewId::Worktrees,
];

fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn empty_app() -> (tempfile::TempDir, App) {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    (tmpdir, app)
}

fn fill_list(app: &mut App, view: ViewId, count: usize) {
    app.active_view = view;
    match view {
        ViewId::Branches => app.branches.set_items(
            (0..count)
                .map(|i| branch(&format!("feature/{i}"), TrackingStatus::Local))
                .collect(),
        ),
        ViewId::Remotes => app.remotes.set_items(
            (0..count)
                .map(|i| remote(&format!("origin/feature/{i}"), &format!("feature/{i}")))
                .collect(),
        ),
        ViewId::Tags => app
            .tags
            .set_items((0..count).map(|i| tag(&format!("v{i}.0.0"))).collect()),
        ViewId::Worktrees => app.worktrees.set_items(
            (0..count)
                .map(|i| worktree(&format!("feature/{i}")))
                .collect(),
        ),
        ViewId::Graph => panic!("graph is not a list view"),
    }
}

fn app_with_items(view: ViewId, count: usize) -> (tempfile::TempDir, App) {
    let (dir, mut app) = empty_app();
    fill_list(&mut app, view, count);
    (dir, app)
}

fn two_commit_graph_app() -> App {
    let mut app = graph_app(Vec::new());
    let mut snapshot = graph_snapshot(Vec::new());
    let mut second = snapshot.commits[0].clone();
    second.oid = "2222222222222222222222222222222222222222".into();
    second.summary = "second commit".into();
    snapshot.commits.push(second);
    snapshot.lines.push(graph::GraphLine {
        graph: "*".into(),
        commit_index: Some(1),
    });
    app.graph.apply_result(Ok(snapshot));
    app
}

fn status_bar_app(view: ViewId) -> (tempfile::TempDir, App) {
    let (dir, mut app) = empty_app();
    app.active_view = view;
    app.terminal_rows = 24;
    let items = vec![(0, 10, KeyCode::Char('?'))];
    match view {
        ViewId::Branches => app.branches.status_bar_items = items,
        ViewId::Remotes => app.remotes.status_bar_items = items,
        ViewId::Tags => app.tags.status_bar_items = items,
        ViewId::Worktrees => app.worktrees.status_bar_items = items,
        ViewId::Graph => panic!("graph has no status bar items"),
    }
    (dir, app)
}

fn set_header_columns(app: &mut App, view: ViewId, columns: Vec<(u16, u16, usize)>) {
    match view {
        ViewId::Branches => app.branches.header_columns = columns,
        ViewId::Remotes => app.remotes.header_columns = columns,
        ViewId::Tags => app.tags.header_columns = columns,
        ViewId::Worktrees => app.worktrees.header_columns = columns,
        ViewId::Graph => panic!("graph has no header columns"),
    }
}

fn cursor_of(app: &App, view: ViewId) -> usize {
    match view {
        ViewId::Branches => app.branches.cursor(),
        ViewId::Remotes => app.remotes.cursor(),
        ViewId::Tags => app.tags.cursor(),
        ViewId::Worktrees => app.worktrees.cursor(),
        ViewId::Graph => app.graph.commit_cursor(),
    }
}

fn table_selected_of(app: &App, view: ViewId) -> Option<usize> {
    match view {
        ViewId::Branches => app.branches.table_state().selected(),
        ViewId::Remotes => app.remotes.table_state().selected(),
        ViewId::Tags => app.tags.table_state().selected(),
        ViewId::Worktrees => app.worktrees.table_state().selected(),
        ViewId::Graph => panic!("graph has no list table state"),
    }
}

fn sort_column_of(app: &App, view: ViewId) -> Option<usize> {
    match view {
        ViewId::Branches => app.branches.sort_column(),
        ViewId::Remotes => app.remotes.sort_column(),
        ViewId::Tags => app.tags.sort_column(),
        ViewId::Worktrees => app.worktrees.sort_column(),
        ViewId::Graph => panic!("graph has no list sort column"),
    }
}

fn search_query_of(app: &App, view: ViewId) -> &str {
    match view {
        ViewId::Branches => app.branches.search_query(),
        ViewId::Remotes => app.remotes.search_query(),
        ViewId::Tags => app.tags.search_query(),
        ViewId::Worktrees => app.worktrees.search_query(),
        ViewId::Graph => panic!("graph has no list search query"),
    }
}

fn search_active_of(app: &App, view: ViewId) -> bool {
    match view {
        ViewId::Branches => app.branches.search_active(),
        ViewId::Remotes => app.remotes.search_active(),
        ViewId::Tags => app.tags.search_active(),
        ViewId::Worktrees => app.worktrees.search_active(),
        ViewId::Graph => panic!("graph has no list search state"),
    }
}

#[test]
fn toggle_search_activates_search_on_each_list_view() {
    let (_dir, mut app) = empty_app();
    for view in LIST_VIEWS {
        app.active_view = view;
        assert!(!app.is_search_active(), "{view:?} starts inactive");
        app.toggle_search();
        assert!(app.is_search_active(), "{view:?} active after toggle");
    }

    app.active_view = ViewId::Graph;
    app.toggle_search();
    assert!(!app.is_search_active());
}

#[test]
fn handle_search_key_edits_query_per_view() {
    let (_dir, mut app) = empty_app();
    app.active_view = ViewId::Branches;
    app.toggle_search();
    app.handle_search_key(key(KeyCode::Char('a')));
    app.handle_search_key(key(KeyCode::Char('b')));
    assert_eq!(app.branches.search_query(), "ab");

    app.handle_search_key(key(KeyCode::Left));
    assert_eq!(app.branches.search_query(), "ab");

    app.handle_search_key(key(KeyCode::Backspace));
    assert_eq!(app.branches.search_query(), "a");

    app.handle_search_key(key(KeyCode::Enter));
    assert!(!app.branches.search_active());
    assert_eq!(app.branches.search_query(), "a");

    app.toggle_search();
    assert!(app.is_search_active());
    app.handle_search_key(key(KeyCode::Esc));
    assert!(!app.is_search_active());
    assert_eq!(app.branches.search_query(), "");

    for view in [ViewId::Remotes, ViewId::Tags, ViewId::Worktrees] {
        app.active_view = view;
        app.handle_search_key(key(KeyCode::Char('x')));
    }
    assert_eq!(app.remotes.search_query(), "x");
    assert_eq!(app.tags.search_query(), "x");
    assert_eq!(app.worktrees.search_query(), "x");

    app.active_view = ViewId::Graph;
    app.handle_search_key(key(KeyCode::Char('y')));
    assert_eq!(app.branches.search_query(), "");
    assert_eq!(app.remotes.search_query(), "x");
    assert_eq!(app.tags.search_query(), "x");
    assert_eq!(app.worktrees.search_query(), "x");
}

#[test]
fn handle_search_edit_and_close_keys_on_each_list_view() {
    let (_dir, mut app) = empty_app();

    for view in LIST_VIEWS {
        app.active_view = view;
        app.toggle_search();
        app.handle_search_key(key(KeyCode::Char('a')));
        app.handle_search_key(key(KeyCode::Char('b')));
        assert_eq!(search_query_of(&app, view), "ab", "{view:?} appends chars");

        app.handle_search_key(key(KeyCode::Backspace));
        assert_eq!(
            search_query_of(&app, view),
            "a",
            "{view:?} handles Backspace"
        );
        assert!(
            search_active_of(&app, view),
            "{view:?} stays active after Backspace"
        );

        app.handle_search_key(key(KeyCode::Enter));
        assert!(!search_active_of(&app, view), "{view:?} closes on Enter");
        assert_eq!(
            search_query_of(&app, view),
            "a",
            "{view:?} keeps query on Enter"
        );

        app.toggle_search();
        app.handle_search_key(key(KeyCode::Char('z')));
        app.handle_search_key(key(KeyCode::Esc));
        assert!(!search_active_of(&app, view), "{view:?} closes on Esc");
        assert_eq!(
            search_query_of(&app, view),
            "",
            "{view:?} clears query on Esc"
        );
    }
}

#[test]
fn info_modal_wheel_scrolls_and_ignores_other_events() {
    let (_dir, mut app) = empty_app();
    app.active_view = ViewId::Branches;
    app.branches
        .set_items(vec![branch("feature/one", TrackingStatus::Local)]);
    app.open_context_menu();
    assert!(matches!(app.overlay, Some(Overlay::InfoModal { .. })));
    assert_eq!(app.info_modal_scroll_offset, 0);

    app.handle_mouse(mouse(MouseEventKind::ScrollDown, 5, 5));
    assert_eq!(app.info_modal_scroll_offset, 3);
    app.handle_mouse(mouse(MouseEventKind::ScrollDown, 5, 5));
    assert_eq!(app.info_modal_scroll_offset, 6);

    app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Right), 5, 5));
    app.handle_mouse(mouse(MouseEventKind::Moved, 5, 5));
    assert_eq!(app.info_modal_scroll_offset, 6);
    assert!(matches!(app.overlay, Some(Overlay::InfoModal { .. })));

    app.handle_mouse(mouse(MouseEventKind::ScrollUp, 5, 5));
    assert_eq!(app.info_modal_scroll_offset, 3);
    app.handle_mouse(mouse(MouseEventKind::ScrollUp, 5, 5));
    assert_eq!(app.info_modal_scroll_offset, 0);
    app.handle_mouse(mouse(MouseEventKind::ScrollUp, 5, 5));
    assert_eq!(app.info_modal_scroll_offset, 0);

    // Half-open bounds: x 10..20 and y 5..6, so (15, 6) misses. A hit would copy to the real clipboard.
    app.info_hit_regions = vec![InfoHitRegion {
        rect: Rect::new(10, 5, 10, 1),
        label: "Branch".into(),
        value: "feature/one".into(),
    }];
    app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 15, 6));
    assert!(app.info_copied_msg.is_none());
    assert!(matches!(app.overlay, Some(Overlay::InfoModal { .. })));
}

#[test]
fn other_overlays_ignore_mouse_input() {
    let (_dir, mut app) = app_with_items(ViewId::Branches, 2);
    app.overlay = Some(Overlay::Help { scroll: 0 });
    assert_eq!(app.branches.cursor(), 0);

    app.handle_mouse(mouse(MouseEventKind::ScrollDown, 5, 5));
    app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 5, 3));
    app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Right), 5, 3));

    assert_eq!(app.branches.cursor(), 0);
    assert!(matches!(app.overlay, Some(Overlay::Help { scroll: 0 })));
}

#[test]
fn scroll_moves_cursor_on_each_list_view() {
    for view in LIST_VIEWS {
        let (_dir, mut app) = app_with_items(view, 2);
        assert_eq!(cursor_of(&app, view), 0, "{view:?} starts at 0");

        app.handle_mouse(mouse(MouseEventKind::Moved, 5, 5));
        assert_eq!(cursor_of(&app, view), 0, "{view:?} ignores mouse movement");

        app.handle_mouse(mouse(MouseEventKind::ScrollDown, 5, 5));
        assert_eq!(cursor_of(&app, view), 1, "{view:?} after ScrollDown");

        app.handle_mouse(mouse(MouseEventKind::ScrollUp, 5, 5));
        assert_eq!(cursor_of(&app, view), 0, "{view:?} after ScrollUp");
    }
}

#[test]
fn scroll_moves_graph_commit_cursor() {
    let mut app = two_commit_graph_app();
    app.handle_mouse(mouse(MouseEventKind::ScrollDown, 5, 5));
    assert_eq!(app.graph.commit_cursor(), 1);

    app.handle_mouse(mouse(MouseEventKind::ScrollUp, 5, 5));
    assert_eq!(app.graph.commit_cursor(), 0);
}

#[test]
fn left_click_header_sorts_list_views() {
    for view in [ViewId::Branches, ViewId::Tags, ViewId::Worktrees] {
        let (_dir, mut app) = app_with_items(view, 2);
        set_header_columns(&mut app, view, vec![(10, 14, 0), (30, 34, 2)]);
        assert_eq!(sort_column_of(&app, view), None, "{view:?} starts unsorted");

        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 11, 1));
        assert_eq!(sort_column_of(&app, view), Some(0), "{view:?} col 0");

        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 31, 1));
        assert_eq!(sort_column_of(&app, view), Some(2), "{view:?} col 2");

        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 20, 1));
        assert_eq!(sort_column_of(&app, view), Some(2), "{view:?} miss");
    }
}

#[test]
fn left_click_on_graph_header_and_row_leaves_commit_cursor_unchanged() {
    let mut app = two_commit_graph_app();
    app.handle_mouse(mouse(MouseEventKind::ScrollDown, 5, 5));
    assert_eq!(app.graph.commit_cursor(), 1);

    app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 11, 1));
    app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 5, 3));
    assert_eq!(app.graph.commit_cursor(), 1);
    assert!(app.overlay.is_none());
}

#[test]
fn status_bar_click_dispatches_key_on_each_list_view() {
    for view in LIST_VIEWS {
        let (_dir, mut app) = status_bar_app(view);
        assert!(app.overlay.is_none(), "{view:?} starts without overlay");

        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 5, 23));
        assert!(
            matches!(app.overlay, Some(Overlay::Help { .. })),
            "{view:?} status bar click opens help"
        );
    }
}

#[test]
fn status_bar_click_outside_item_or_on_graph_does_nothing() {
    let (_dir, mut app) = status_bar_app(ViewId::Branches);
    app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 15, 23));
    assert!(app.overlay.is_none());

    // Graph ignores the list status-bar items even when they are set.
    app.active_view = ViewId::Graph;
    app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 5, 23));
    assert!(app.overlay.is_none());
}

#[test]
fn left_click_data_row_selects_display_row_on_each_list_view() {
    for view in LIST_VIEWS {
        let (_dir, mut app) = app_with_items(view, 3);

        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 5, 3));
        assert_eq!(cursor_of(&app, view), 1, "{view:?} row click cursor");
        assert_eq!(
            table_selected_of(&app, view),
            Some(1),
            "{view:?} row click selection"
        );

        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 5, 10));
        assert_eq!(cursor_of(&app, view), 1, "{view:?} out-of-range cursor");
        assert_eq!(
            table_selected_of(&app, view),
            Some(1),
            "{view:?} out-of-range selection"
        );
    }
}

fn filtered_branch_app_with_scroll_offset() -> (tempfile::TempDir, App) {
    let (dir, mut app) = empty_app();
    app.active_view = ViewId::Branches;
    app.branches.set_items(vec![
        branch("bugfix/skip", TrackingStatus::Local),
        branch("feature/a", TrackingStatus::Local),
        branch("feature/b", TrackingStatus::Local),
        branch("feature/c", TrackingStatus::Local),
    ]);
    app.branches.set_search_query("feature".into());
    assert_eq!(app.branches.display_indices(), &[1, 2, 3]);
    *app.branches.table_state_mut().offset_mut() = 1;
    (dir, app)
}

#[test]
fn left_click_uses_filtered_display_indices_and_scroll_offset() {
    let (_dir, mut app) = filtered_branch_app_with_scroll_offset();

    app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 5, 2));

    assert_eq!(app.branches.cursor(), 2, "raw item index for feature/b");
    assert_eq!(app.branches.items()[2].name, "feature/b");
    assert_eq!(app.branches.table_state().selected(), Some(1));
}

#[test]
fn right_click_uses_filtered_display_indices_and_scroll_offset() {
    let (_dir, mut app) = filtered_branch_app_with_scroll_offset();

    app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Right), 5, 2));

    assert_eq!(app.branches.cursor(), 2, "raw item index for feature/b");
    assert_eq!(app.branches.items()[2].name, "feature/b");
    assert_eq!(app.branches.table_state().selected(), Some(1));
    assert!(matches!(app.overlay, Some(Overlay::InfoModal { .. })));
}

#[test]
fn find_header_click_resolves_per_view() {
    let (_dir, mut app) = empty_app();

    app.active_view = ViewId::Graph;
    assert_eq!(app.find_header_click(5), None);

    app.active_view = ViewId::Branches;
    assert_eq!(app.find_header_click(11), None);

    for view in [ViewId::Tags, ViewId::Worktrees] {
        app.active_view = view;
        set_header_columns(&mut app, view, vec![(10, 14, 0), (30, 34, 2)]);
        assert_eq!(app.find_header_click(11), Some(0), "{view:?} hit");
        assert_eq!(app.find_header_click(20), None, "{view:?} miss");
    }
}

#[test]
fn right_click_row_opens_info_modal_on_each_list_view() {
    for view in LIST_VIEWS {
        let (_dir, mut app) = app_with_items(view, 2);
        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Right), 5, 3));
        assert_eq!(
            cursor_of(&app, view),
            1,
            "{view:?} cursor moved to clicked row"
        );
        assert!(
            matches!(app.overlay, Some(Overlay::InfoModal { .. })),
            "{view:?} opens info modal"
        );
    }
}

#[test]
fn right_click_without_row_or_on_header_does_nothing() {
    for view in LIST_VIEWS {
        let (_dir, mut app) = app_with_items(view, 2);

        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Right), 5, 10));
        assert!(app.overlay.is_none(), "{view:?} past last row");
        assert_eq!(cursor_of(&app, view), 0, "{view:?} past last row cursor");

        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Right), 5, 1));
        assert!(app.overlay.is_none(), "{view:?} header row");
        assert_eq!(cursor_of(&app, view), 0, "{view:?} header row cursor");
    }

    let mut app = two_commit_graph_app();
    app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Right), 5, 3));
    assert!(app.overlay.is_none());
    assert_eq!(app.graph.commit_cursor(), 0);
}
