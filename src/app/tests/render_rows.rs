use super::*;

#[test]
fn branch_row_renders_up_indicator_before_branch_name() {
    let theme = Theme::dark();
    let symbols = SymbolSet::ascii();
    let ctx = CellContext {
        theme: &theme,
        symbols: &symbols,
        area_width: 120,
        compact: false,
        data_col_widths: vec![2, 40],
        first_col_width: 2,
    };
    let item = BranchInfo {
        name: "feature/test".into(),
        is_current: false,
        is_base: false,
        tracking: TrackingStatus::Tracked {
            remote_ref: "origin/feature/test".into(),
            gone: false,
        },
        ahead: None,
        behind: None,
        last_commit_date: Utc::now(),
        merge_status: MergeStatus::Unmerged,
        base_branch: "main".into(),
        merge_base_commit: Some("ac13ef04".into()),
        pr: None,
        squash_confidence: None,
    };

    let rows = render_branch_row(&item, 0, false, false, &[0, 1], &ctx);

    assert_eq!(cell_text(&rows[0]), symbols.graph_remote_ref);
    assert_eq!(rows[0].spans[0].style, theme.remote_title);
    assert_eq!(cell_text(&rows[1]), "feature/test (main - ac13ef04)");
}

#[test]
fn branch_row_distinguishes_gone_and_local_upstreams() {
    let theme = Theme::dark();
    let symbols = SymbolSet::ascii();
    let ctx = CellContext {
        theme: &theme,
        symbols: &symbols,
        area_width: 120,
        compact: false,
        data_col_widths: vec![4, 40],
        first_col_width: 4,
    };
    let gone = branch(
        "feature/gone",
        TrackingStatus::Tracked {
            remote_ref: "origin/feature/gone".into(),
            gone: true,
        },
    );
    let local = branch("feature/local", TrackingStatus::Local);

    let gone_row = render_branch_row(&gone, 0, false, false, &[0, 1], &ctx);
    let local_row = render_branch_row(&local, 0, false, false, &[0, 1], &ctx);

    assert_eq!(cell_text(&gone_row[0]), "gone");
    assert_eq!(cell_text(&local_row[0]), "-");
    assert_eq!(cell_text(&gone_row[1]), "feature/gone");
    assert_eq!(cell_text(&local_row[1]), "feature/local");
}

#[test]
fn worktree_branch_cell_left_truncates_to_column_width() {
    let theme = Theme::dark();
    let symbols = SymbolSet::ascii();
    let ctx = CellContext {
        theme: &theme,
        symbols: &symbols,
        area_width: 100,
        compact: false,
        data_col_widths: vec![20, 12],
        first_col_width: 20,
    };
    let rows = render_worktree_row(
        &worktree("feature/very-long-branch-name"),
        0,
        false,
        false,
        &[0, 1],
        &ctx,
    );

    assert_eq!(cell_text(&rows[1]), "\u{2026}branch-name");
    assert_eq!(rows[1].alignment, Some(Alignment::Right));
}

#[test]
fn worktree_base_merge_cell_is_blank() {
    let theme = Theme::dark();
    let symbols = SymbolSet::ascii();
    let ctx = CellContext {
        theme: &theme,
        symbols: &symbols,
        area_width: 120,
        compact: false,
        data_col_widths: vec![30, 20, 12, 8, 12],
        first_col_width: 2,
    };

    // Base-branch worktree: a branch can't be merged into itself, so the
    // Merge cell must be blank (mirrors the Branches view's base row).
    let mut base_wt = worktree("main");
    base_wt.is_base = true;
    base_wt.merge_status = MergeStatus::Unmerged;
    let rows = render_worktree_row(&base_wt, 0, false, false, &[0, 1, 2, 3, 4], &ctx);
    assert_eq!(
        cell_text(&rows[4]),
        "",
        "base worktree Merge cell should be blank, not 'unmerged'"
    );

    // Non-base worktree: the Merge cell shows the real status.
    let mut feat_wt = worktree("feature/x");
    feat_wt.merge_status = MergeStatus::Merged;
    let rows = render_worktree_row(&feat_wt, 0, false, false, &[0, 1, 2, 3, 4], &ctx);
    assert_ne!(
        cell_text(&rows[4]),
        "",
        "non-base worktree should display its merge status"
    );
}

#[test]
fn remote_age_cell_uses_short_text_when_column_is_too_narrow() {
    let theme = Theme::dark();
    let symbols = SymbolSet::ascii();
    let ctx = CellContext {
        theme: &theme,
        symbols: &symbols,
        area_width: 120,
        compact: false,
        data_col_widths: vec![2, 30, 8, 5, 12],
        first_col_width: 2,
    };
    let rows = render_remote_row(&remote_branch(), 0, false, false, &[0, 1, 2, 3, 4], &ctx);

    assert_eq!(cell_text(&rows[4]), "5m");
    assert_eq!(rows[4].alignment, Some(Alignment::Right));
}

#[test]
fn remote_age_cell_uses_long_text_when_column_fits() {
    let theme = Theme::dark();
    let symbols = SymbolSet::ascii();
    let ctx = CellContext {
        theme: &theme,
        symbols: &symbols,
        area_width: 120,
        compact: false,
        data_col_widths: vec![2, 30, 8, 5, 14],
        first_col_width: 2,
    };
    let rows = render_remote_row(&remote_branch(), 0, false, false, &[0, 1, 2, 3, 4], &ctx);

    assert_eq!(cell_text(&rows[4]), "5 minutes ago");
    assert_eq!(rows[4].alignment, Some(Alignment::Right));
}

#[test]
fn remote_local_cell_renders_local_branch_marker_or_dash() {
    let theme = Theme::dark();
    let symbols = SymbolSet::ascii();
    let ctx = CellContext {
        theme: &theme,
        symbols: &symbols,
        area_width: 120,
        compact: false,
        data_col_widths: vec![2, 30, 8, 5, 12],
        first_col_width: 2,
    };

    let without_local =
        render_remote_row(&remote_branch(), 0, false, false, &[0, 1, 2, 3, 4], &ctx);
    assert_eq!(cell_text(&without_local[0]), "-");

    let mut with_local = remote_branch();
    with_local.has_local = true;
    let with_local = render_remote_row(&with_local, 0, false, false, &[0, 1], &ctx);
    assert_eq!(cell_text(&with_local[0]), symbols.graph_local_ref);
    assert_eq!(with_local[0].spans[0].style, theme.merged);
    assert_eq!(cell_text(&with_local[1]), "origin/feature/remote-age");
}

#[test]
fn rendered_remotes_header_clicks_sort_local_then_name() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    app.active_view = ViewId::Remotes;
    let mut with_local = remote_branch();
    with_local.has_local = true;
    app.remotes.set_items(vec![remote_branch(), with_local]);
    let _ = render_app(&mut app, 120, 20);

    assert_eq!(app.remotes.header_columns[0].2, 0);
    assert_eq!(app.remotes.header_columns[1].2, 1);
    let first_x = app.remotes.header_columns[0].0;
    app.handle_left_click(first_x, 1);
    assert_eq!(app.remotes.sort_column(), Some(0));
    let _ = render_app(&mut app, 120, 20);
    let second_x = app.remotes.header_columns[1].0;
    app.handle_left_click(second_x, 1);
    assert_eq!(app.remotes.sort_column(), Some(1));
}

#[test]
fn worktree_branch_cell_does_not_truncate_when_width_unknown() {
    let theme = Theme::dark();
    let symbols = SymbolSet::ascii();
    let ctx = CellContext {
        theme: &theme,
        symbols: &symbols,
        area_width: 100,
        compact: false,
        data_col_widths: Vec::new(),
        first_col_width: 20,
    };
    let rows = render_worktree_row(
        &worktree("feature/very-long-branch-name"),
        0,
        false,
        false,
        &[1],
        &ctx,
    );

    assert_eq!(cell_text(&rows[0]), "feature/very-long-branch-name");
    assert_eq!(rows[0].alignment, None);
}
