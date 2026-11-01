use super::*;

fn plain_app() -> (tempfile::TempDir, App) {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    (tmpdir, app)
}

/// Sets the view's sort to its second cycle state, runs `$save`, and checks the
/// column key and direction that reached disk.
macro_rules! persist_sort {
    ($app:ident, $list:ident, $cols:ident, $key:ident, $asc:ident, $save:expr) => {{
        let cols = sort_keys::sort_state_cycle(&$app.$cols);
        let (col, asc) = cols[1];
        $app.$list.set_sort(col, asc);
        $save;
        let saved = Config::load_from(&$app.config_root);
        assert_eq!(
            saved.$key.as_deref(),
            sort_keys::key_for_index(&$app.$cols, col.unwrap())
        );
        assert_eq!(saved.$asc, Some(asc));
    }};
}

macro_rules! assert_cycle_sort {
    ($app:ident, $view:expr, $list:ident, $cols:ident) => {{
        let pairs = sort_keys::sort_state_cycle(&$app.$cols);
        let len = pairs.len();
        $app.$list.set_sort(pairs[0].0, pairs[0].1);

        $app.cycle_view_sort($view, true);
        assert_eq!(
            ($app.$list.sort_column(), $app.$list.sort_ascending()),
            pairs[1]
        );
        $app.cycle_view_sort($view, true);
        assert_eq!(
            ($app.$list.sort_column(), $app.$list.sort_ascending()),
            pairs[2]
        );
        $app.cycle_view_sort($view, false);
        assert_eq!(
            ($app.$list.sort_column(), $app.$list.sort_ascending()),
            pairs[1]
        );
        $app.cycle_view_sort($view, false);
        assert_eq!(
            ($app.$list.sort_column(), $app.$list.sort_ascending()),
            pairs[0]
        );
        $app.cycle_view_sort($view, false);
        assert_eq!(
            ($app.$list.sort_column(), $app.$list.sort_ascending()),
            pairs[len - 1]
        );
        $app.cycle_view_sort($view, true);
        assert_eq!(
            ($app.$list.sort_column(), $app.$list.sort_ascending()),
            pairs[0]
        );
    }};
}

#[test]
fn save_symbols_persists_cycled_symbol_set() {
    let (_tmpdir, mut app) = plain_app();
    assert_ne!(app.config_root, Config::default_root());

    app.symbols = app.symbols.next();
    app.save_symbols();

    let saved = Config::load_from(&app.config_root);
    assert_eq!(saved.symbols, Some(app.symbols.name.to_string()));
}

#[test]
fn save_include_remotes_persists_graph_toggle() {
    let mut app = graph_app(vec![]);
    assert_ne!(app.config_root, Config::default_root());

    app.save_include_remotes();

    let saved = Config::load_from(&app.config_root);
    assert_eq!(saved.include_remotes, Some(true));
}

#[test]
fn save_auto_fetch_and_load_worktrees_persist_flags() {
    let (_tmpdir, mut app) = plain_app();
    assert_ne!(app.config_root, Config::default_root());

    app.config.auto_fetch = Some(true);
    app.save_auto_fetch();
    assert_eq!(Config::load_from(&app.config_root).auto_fetch, Some(true));
    app.config.auto_fetch = Some(false);
    app.save_auto_fetch();
    assert_eq!(Config::load_from(&app.config_root).auto_fetch, Some(false));

    app.config.load_worktrees_on_launch = Some(true);
    app.save_load_worktrees_on_launch();
    assert_eq!(
        Config::load_from(&app.config_root).load_worktrees_on_launch,
        Some(true)
    );
    app.config.load_worktrees_on_launch = Some(false);
    app.save_load_worktrees_on_launch();
    assert_eq!(
        Config::load_from(&app.config_root).load_worktrees_on_launch,
        Some(false)
    );
}

#[test]
fn save_writes_only_its_field_on_disk() {
    let (_tmpdir, mut app) = plain_app();
    assert_ne!(app.config_root, Config::default_root());

    Config::update_at(&app.config_root, |c| c.include_remotes = Some(true));
    app.config.auto_fetch = Some(true);
    app.save_auto_fetch();

    let saved = Config::load_from(&app.config_root);
    assert_eq!(saved.include_remotes, Some(true));
    assert_eq!(saved.auto_fetch, Some(true));
}

#[test]
fn save_sort_field_persists_each_view_sort() {
    let (_tmpdir, mut app) = plain_app();
    assert_ne!(app.config_root, Config::default_root());

    persist_sort!(
        app,
        branches,
        branch_columns,
        sort_column_branches,
        sort_asc_branches,
        app.save_sort_field(ViewId::Branches)
    );
    persist_sort!(
        app,
        remotes,
        remote_columns,
        sort_column_remotes,
        sort_asc_remotes,
        app.save_sort_field(ViewId::Remotes)
    );
    persist_sort!(
        app,
        tags,
        tag_columns,
        sort_column_tags,
        sort_asc_tags,
        app.save_sort_field(ViewId::Tags)
    );
    persist_sort!(
        app,
        worktrees,
        worktree_columns,
        sort_column_worktrees,
        sort_asc_worktrees,
        app.save_sort_field(ViewId::Worktrees)
    );

    let (_graph_tmpdir, mut fresh) = plain_app();
    assert_ne!(fresh.config_root, Config::default_root());
    fresh.save_sort_field(ViewId::Graph);
    let saved = Config::load_from(&fresh.config_root);
    assert!(saved.sort_column_branches.is_none());
    assert!(saved.sort_column_remotes.is_none());
    assert!(saved.sort_column_tags.is_none());
    assert!(saved.sort_column_worktrees.is_none());
}

#[test]
fn cycle_view_sort_steps_through_each_view() {
    let (_tmpdir, mut app) = plain_app();

    assert_cycle_sort!(app, ViewId::Branches, branches, branch_columns);
    assert_cycle_sort!(app, ViewId::Remotes, remotes, remote_columns);
    assert_cycle_sort!(app, ViewId::Tags, tags, tag_columns);
    assert_cycle_sort!(app, ViewId::Worktrees, worktrees, worktree_columns);

    app.branches.set_sort(None, true);
    app.cycle_view_sort(ViewId::Graph, true);
    assert_eq!(
        (app.branches.sort_column(), app.branches.sort_ascending()),
        (None, true)
    );
}

#[test]
fn toggle_horizontal_scrolling_resets_all_list_offsets_and_bounds() {
    let (_tmpdir, mut app) = plain_app();

    app.branches.set_horizontal_scroll_bounds(4);
    app.branches.scroll_right();
    app.branches.scroll_right();
    app.remotes.set_horizontal_scroll_bounds(4);
    app.remotes.scroll_right();
    app.remotes.scroll_right();
    app.tags.set_horizontal_scroll_bounds(4);
    app.tags.scroll_right();
    app.tags.scroll_right();
    app.worktrees.set_horizontal_scroll_bounds(4);
    app.worktrees.scroll_right();
    app.worktrees.scroll_right();

    assert_eq!(app.branches.horizontal_offset(), 2);
    assert_eq!(app.remotes.horizontal_offset(), 2);
    assert_eq!(app.tags.horizontal_offset(), 2);
    assert_eq!(app.worktrees.horizontal_offset(), 2);

    app.toggle_horizontal_scrolling();

    assert_eq!(app.config.horizontal_scrolling, Some(true));
    assert_eq!(app.branches.horizontal_offset(), 0);
    assert_eq!(app.remotes.horizontal_offset(), 0);
    assert_eq!(app.tags.horizontal_offset(), 0);
    assert_eq!(app.worktrees.horizontal_offset(), 0);

    // If the previous bounds survived the toggle, scrolling would move again.
    app.branches.scroll_right();
    app.remotes.scroll_right();
    app.tags.scroll_right();
    app.worktrees.scroll_right();
    assert_eq!(app.branches.horizontal_offset(), 0);
    assert_eq!(app.remotes.horizontal_offset(), 0);
    assert_eq!(app.tags.horizontal_offset(), 0);
    assert_eq!(app.worktrees.horizontal_offset(), 0);
}

#[test]
fn save_settings_field_routes_rows_to_their_setting() {
    let (_tmpdir, mut app) = plain_app();
    assert_ne!(app.config_root, Config::default_root());

    app.symbols = app.symbols.next();
    app.save_settings_field(0);
    assert_eq!(
        Config::load_from(&app.config_root).symbols,
        Some(app.symbols.name.to_string())
    );

    app.theme = app.theme.next();
    app.save_settings_field(1);
    assert_eq!(
        Config::load_from(&app.config_root).theme,
        Some(app.theme.name.to_string())
    );

    persist_sort!(
        app,
        branches,
        branch_columns,
        sort_column_branches,
        sort_asc_branches,
        app.save_settings_field(2)
    );
    persist_sort!(
        app,
        remotes,
        remote_columns,
        sort_column_remotes,
        sort_asc_remotes,
        app.save_settings_field(3)
    );
    persist_sort!(
        app,
        tags,
        tag_columns,
        sort_column_tags,
        sort_asc_tags,
        app.save_settings_field(4)
    );
    persist_sort!(
        app,
        worktrees,
        worktree_columns,
        sort_column_worktrees,
        sort_asc_worktrees,
        app.save_settings_field(5)
    );

    app.config.auto_fetch = Some(true);
    app.save_settings_field(6);
    assert_eq!(Config::load_from(&app.config_root).auto_fetch, Some(true));

    app.config.load_worktrees_on_launch = Some(true);
    app.save_settings_field(7);
    assert_eq!(
        Config::load_from(&app.config_root).load_worktrees_on_launch,
        Some(true)
    );
}

#[test]
fn save_settings_field_ignores_out_of_range_row() {
    let (_tmpdir, mut app) = plain_app();
    assert_ne!(app.config_root, Config::default_root());

    app.config.auto_fetch = Some(true);
    app.save_settings_field(9);

    assert_eq!(Config::load_from(&app.config_root).auto_fetch, None);
}

#[test]
fn filter_helpers_touch_only_active_view() {
    let views = [
        ViewId::Branches,
        ViewId::Remotes,
        ViewId::Tags,
        ViewId::Worktrees,
    ];
    for (i, view) in views.into_iter().enumerate() {
        let (_tmpdir, mut app) = plain_app();
        app.active_view = view;
        app.set_active_filter("feature".into());

        assert_eq!(app.active_filter_query(), "feature");
        let filters = [
            app.branches.filter_query(),
            app.remotes.filter_query(),
            app.tags.filter_query(),
            app.worktrees.filter_query(),
        ];
        for (j, filter) in filters.iter().enumerate() {
            let expected = if i == j { "feature" } else { "" };
            assert_eq!(*filter, expected, "filter for view index {j}");
        }
    }

    let (_tmpdir, mut app) = plain_app();
    app.active_view = ViewId::Graph;
    app.set_active_filter("x".into());
    assert_eq!(app.active_filter_query(), "");
    assert_eq!(app.branches.filter_query(), "");
    assert_eq!(app.remotes.filter_query(), "");
    assert_eq!(app.tags.filter_query(), "");
    assert_eq!(app.worktrees.filter_query(), "");
}

#[test]
fn run_cache_audit_streams_progress_then_reports() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = action_refresh_app(tmpdir.path());
    assert_ne!(app.config_root, Config::default_root());

    app.run_cache_audit();

    assert!(matches!(
        &app.overlay,
        Some(Overlay::Executing { label, .. }) if label == "Verifying cache accuracy..."
    ));
    assert!(app.diag_rx.is_some());
    assert!(app.progress_rx.is_some());
    assert!(app.cancel_flag.is_some());

    wait_until(|| {
        app.drain_channels();
        matches!(app.overlay, Some(Overlay::DiagnosticsReport { .. }))
    });

    assert!(matches!(
        app.overlay,
        Some(Overlay::DiagnosticsReport { .. })
    ));
    assert!(app.progress_rx.is_none());
}

#[test]
fn run_cache_audit_non_git_root_drops_channel() {
    let (_tmpdir, mut app) = plain_app();

    app.run_cache_audit();
    wait_until(|| {
        app.drain_channels();
        app.diag_rx.is_none()
    });

    assert!(app.diag_rx.is_none());
}

#[test]
fn apply_cache_fix_toasts_and_refreshes_return_view() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = action_refresh_app(tmpdir.path());
    app.active_view = ViewId::Tags;
    app.return_view = ViewId::Branches;

    app.apply_cache_fix(CacheAudit::default());

    assert_eq!(
        app.toast.as_ref().map(|toast| toast.message.as_str()),
        Some("Cache corrected")
    );
    assert_eq!(app.active_view, ViewId::Branches);
}
