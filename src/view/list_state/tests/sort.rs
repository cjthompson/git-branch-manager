use super::*;

#[test]
fn sort_by_column_click_sets_column_and_applies() {
    let columns = vec![name_column()];
    let mut state = ListState::new(sample_branches());
    assert_eq!(state.sort_column(), None);

    sort_by_column_click(&mut state, &columns, 0);

    assert_eq!(state.sort_column(), Some(0));
    assert!(state.sort_ascending());
    // main is the base branch (pinned first), then ascending by name.
    assert_eq!(state.items()[0].name, "main");
    assert_eq!(state.items()[1].name, "feature/a");
    assert_eq!(state.items()[2].name, "feature/b");
}

#[test]
fn sort_by_column_click_same_column_toggles_direction() {
    let columns = vec![name_column()];
    let mut state = ListState::new(sample_branches());

    sort_by_column_click(&mut state, &columns, 0);
    assert!(state.sort_ascending());

    sort_by_column_click(&mut state, &columns, 0);
    assert!(!state.sort_ascending());
    assert_eq!(state.sort_column(), Some(0));
    // Descending: base still pinned first, then feature/b, feature/a.
    assert_eq!(state.items()[0].name, "main");
    assert_eq!(state.items()[1].name, "feature/b");
    assert_eq!(state.items()[2].name, "feature/a");

    sort_by_column_click(&mut state, &columns, 0);
    assert!(state.sort_ascending());
}

#[test]
fn sort_by_column_click_different_column_resets_to_ascending() {
    let age_col = ColumnDef::<BranchInfo> {
        key: "age",
        name: "Age",
        show_header: true,
        min_width: 5,
        content_min_width: None,
        wide_width: None,
        hide_below_width: None,
        compare: Some(|a, b| a.last_commit_date.cmp(&b.last_commit_date)),
    };
    let columns = vec![name_column(), age_col];
    let mut state = ListState::new(sample_branches());

    sort_by_column_click(&mut state, &columns, 0);
    sort_by_column_click(&mut state, &columns, 0); // now descending
    assert!(!state.sort_ascending());

    sort_by_column_click(&mut state, &columns, 1);
    assert_eq!(state.sort_column(), Some(1));
    assert!(state.sort_ascending());
}

// --- Sorting tests ---

#[test]
fn sort_by_name_ascending() {
    let columns = vec![ColumnDef::<BranchInfo> {
        key: "name",
        name: "Name",
        show_header: true,
        min_width: 10,
        content_min_width: None,
        wide_width: None,
        hide_below_width: None,
        compare: Some(|a, b| a.name.cmp(&b.name)),
    }];
    let mut state = ListState::new(sample_branches());
    state.set_sort(Some(0), true);
    apply_sort(&mut state, &columns);
    // main is pinned (first), then feature/a, feature/b
    assert_eq!(state.items()[0].name, "main");
    assert_eq!(state.items()[1].name, "feature/a");
    assert_eq!(state.items()[2].name, "feature/b");
}

#[test]
fn sort_by_name_descending() {
    let columns = vec![ColumnDef::<BranchInfo> {
        key: "name",
        name: "Name",
        show_header: true,
        min_width: 10,
        content_min_width: None,
        wide_width: None,
        hide_below_width: None,
        compare: Some(|a, b| a.name.cmp(&b.name)),
    }];
    let mut state = ListState::new(sample_branches());
    state.set_sort(Some(0), false);
    apply_sort(&mut state, &columns);
    assert_eq!(state.items()[0].name, "main"); // pinned first
    assert_eq!(state.items()[1].name, "feature/b");
    assert_eq!(state.items()[2].name, "feature/a");
}

#[test]
fn sort_keeps_base_first_when_items_not_pre_sorted() {
    let mut branches = sample_branches();
    branches.rotate_left(1); // move main to the end

    let columns = vec![ColumnDef::<BranchInfo> {
        key: "name",
        name: "Name",
        show_header: true,
        min_width: 10,
        content_min_width: None,
        wide_width: None,
        hide_below_width: None,
        compare: Some(|a, b| a.name.cmp(&b.name)),
    }];
    let mut state = ListState::new(branches);
    state.set_sort(Some(0), true);
    apply_sort(&mut state, &columns);
    assert_eq!(state.items()[0].name, "main");
}

#[test]
fn sort_descending_keeps_base_first() {
    let columns = vec![ColumnDef::<BranchInfo> {
        key: "name",
        name: "Name",
        show_header: true,
        min_width: 10,
        content_min_width: None,
        wide_width: None,
        hide_below_width: None,
        compare: Some(|a, b| a.name.cmp(&b.name)),
    }];
    let mut state = ListState::new(sample_branches());
    state.set_sort(Some(0), false);
    apply_sort(&mut state, &columns);
    assert_eq!(state.items()[0].name, "main");
    assert_eq!(state.items()[1].name, "feature/b");
    assert_eq!(state.items()[2].name, "feature/a");
}

#[test]
fn cycle_sort_column_works() {
    let columns = vec![
        ColumnDef::<BranchInfo> {
            key: "name",
            name: "Name",
            show_header: true,
            min_width: 10,
            content_min_width: None,
            wide_width: None,
            hide_below_width: None,
            compare: Some(|a, b| a.name.cmp(&b.name)),
        },
        ColumnDef::<BranchInfo> {
            key: "unsortable",
            name: "Unsortable",
            show_header: true,
            min_width: 5,
            content_min_width: None,
            wide_width: None,
            hide_below_width: None,
            compare: None,
        },
        ColumnDef::<BranchInfo> {
            key: "age",
            name: "Age",
            show_header: true,
            min_width: 5,
            content_min_width: None,
            wide_width: None,
            hide_below_width: None,
            compare: Some(|a, b| a.last_commit_date.cmp(&b.last_commit_date)),
        },
    ];
    let mut state = ListState::new(sample_branches());

    // None → first sortable (0)
    cycle_sort_column(&mut state, &columns);
    assert_eq!(state.sort_column(), Some(0));

    // 0 → skip 1 (unsortable) → 2
    cycle_sort_column(&mut state, &columns);
    assert_eq!(state.sort_column(), Some(2));

    // 2 → None (wrap)
    cycle_sort_column(&mut state, &columns);
    assert_eq!(state.sort_column(), None);
}

#[test]
fn toggle_sort_direction_works() {
    let mut state = ListState::new(sample_branches());
    assert!(state.sort_ascending());

    // No-op when sort_column is None
    toggle_sort_direction(&mut state);
    assert!(state.sort_ascending());

    // Toggles when a sort column is active
    state.set_sort(Some(0), true);
    toggle_sort_direction(&mut state);
    assert!(!state.sort_ascending());
    toggle_sort_direction(&mut state);
    assert!(state.sort_ascending());
}
