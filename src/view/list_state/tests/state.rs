use super::*;

#[test]
fn collect_targets_all_selected_no_filter() {
    let mut state = ListState::new(sample_branches());
    state.selected_mut().iter_mut().for_each(|s| *s = true);
    let targets = collect_targets(&state, |b| Some(b.name.clone()));
    assert_eq!(targets, vec!["main", "feature/a", "feature/b"]);
}

#[test]
fn collect_targets_filters_pinned() {
    let mut state = ListState::new(sample_branches());
    state.selected_mut().iter_mut().for_each(|s| *s = true);
    let targets = collect_targets(&state, |b| (!b.is_pinned()).then(|| b.name.clone()));
    assert_eq!(targets, vec!["feature/a", "feature/b"]);
}

#[test]
fn collect_targets_cursor_fallback_when_none_selected() {
    let state = ListState::new(sample_branches());
    // Nothing checked: selected_indices falls back to the cursor item (index 0).
    let targets = collect_targets(&state, |b| Some(b.name.clone()));
    assert_eq!(targets, vec!["main"]);
}

#[test]
fn collect_targets_empty_when_all_selected_are_filtered() {
    let mut state = ListState::new(sample_branches());
    // Select only the pinned base branch; the mapper filters pinned items.
    state.selected_mut()[0] = true;
    let targets = collect_targets(&state, |b| (!b.is_pinned()).then(|| b.name.clone()));
    assert!(targets.is_empty());
}

#[test]
fn horizontal_scroll_is_bounded_and_does_not_change_list_state() {
    let mut state = ListState::new(sample_branches());
    state.set_cursor(1);
    state.selected_mut()[2] = true;
    state.set_sort(Some(0), false);
    state.set_horizontal_scroll_bounds(3);

    state.scroll_right();
    state.scroll_right();
    state.scroll_right();
    state.scroll_right();
    assert_eq!(state.horizontal_offset(), 3);
    assert_eq!(state.cursor(), 1);
    assert_eq!(state.selected(), &[false, false, true]);
    assert_eq!(state.sort_column(), Some(0));
    assert!(!state.sort_ascending());

    state.set_horizontal_scroll_bounds(1);
    assert_eq!(state.horizontal_offset(), 1);
    state.scroll_left();
    state.scroll_left();
    assert_eq!(state.horizontal_offset(), 0);
}

#[test]
fn replacing_list_items_resets_horizontal_scroll() {
    let mut state = ListState::new(sample_branches());
    state.set_horizontal_scroll_bounds(8);
    state.scroll_right();
    state.set_items(sample_branches());

    assert_eq!(state.horizontal_offset(), 0);
    assert_eq!(state.max_horizontal_offset(), 0);
}

// --- ListState basic tests ---

#[test]
fn new_state_has_correct_defaults() {
    let state = ListState::new(sample_branches());
    assert_eq!(state.cursor(), 0);
    assert_eq!(state.items().len(), 3);
    assert!(state.selected().iter().all(|&s| !s));
}

#[test]
fn set_items_preserves_existing_selection() {
    let mut state = ListState::new(sample_branches());
    state.selected_mut()[1] = true;
    state.set_items(sample_branches());
    assert!(state.selected()[1]);
}

#[test]
fn set_items_preserves_cursor_and_selection_by_row_identity() {
    let mut state = ListState::new(sample_branches());
    nav_down(&mut state);
    select_toggle(&mut state);
    assert_eq!(state.cursor_item().unwrap().display_name(), "feature/a");

    let mut replacement = sample_branches();
    replacement.swap(1, 2);
    state.set_items(replacement);

    assert_eq!(state.cursor_item().unwrap().display_name(), "feature/a");
    let selected_names: Vec<&str> = state
        .items()
        .iter()
        .zip(state.selected())
        .filter_map(|(item, selected)| selected.then_some(item.display_name()))
        .collect();
    assert_eq!(selected_names, vec!["feature/a"]);
}

#[test]
fn display_indices_returns_all_when_no_filter() {
    let state = ListState::new(sample_branches());
    let indices = state.display_indices();
    assert_eq!(indices.len(), 3);
}

#[test]
fn display_indices_pinned_first() {
    let state = ListState::new(sample_branches());
    let indices = state.display_indices();
    // main (index 0) is pinned, should be first
    assert_eq!(indices[0], 0);
}

#[test]
fn empty_state() {
    let state = ListState::<BranchInfo>::empty();
    assert_eq!(state.items().len(), 0);
    assert!(state.display_indices().is_empty());
    assert!(state.cursor_item().is_none());
    assert!(state.selected_indices().is_empty());
}

#[test]
fn cursor_item_returns_correct_item() {
    let state = ListState::new(sample_branches());
    let item = state.cursor_item().unwrap();
    assert_eq!(item.name, "main");
}

#[test]
fn selected_indices_returns_cursor_when_none_selected() {
    let state = ListState::new(sample_branches());
    let indices = state.selected_indices();
    assert_eq!(indices.len(), 1);
    assert_eq!(indices[0], 0); // cursor is at 0
}

#[test]
fn selected_indices_returns_selected() {
    let mut state = ListState::new(sample_branches());
    state.selected_mut()[1] = true;
    state.selected_mut()[2] = true;
    let indices = state.selected_indices();
    assert_eq!(indices, vec![1, 2]);
}

// --- Filter integration tests ---

#[test]
fn search_filters_display() {
    let mut state = ListState::new(sample_branches());
    state.set_search_query("feature".to_string());
    assert_eq!(state.display_indices().len(), 2); // feature/a, feature/b
}

#[test]
fn filter_query_status_merged() {
    let mut state = ListState::new(sample_branches());
    state.set_filter_query("merge:merged".to_string());
    // After rebuilding, only merged items + pinned should show
    let visible: Vec<&str> = state
        .display_indices()
        .iter()
        .map(|&i| state.items()[i].name.as_str())
        .collect();
    assert!(visible.contains(&"main")); // pinned
    assert!(visible.contains(&"feature/b")); // merged
    assert!(!visible.contains(&"feature/a")); // unmerged
}

#[test]
fn search_and_filter_combine() {
    let mut state = ListState::new(sample_branches());
    state.set_search_query("feature".to_string());
    state.set_filter_query("merge:merged".to_string());
    // Only feature/b matches both
    let visible: Vec<&str> = state
        .display_indices()
        .iter()
        .map(|&i| state.items()[i].name.as_str())
        .collect();
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0], "feature/b");
}

#[test]
fn set_cursor_updates_table_state() {
    let mut state = ListState::new(sample_branches());
    state.set_cursor(2);
    assert_eq!(state.cursor(), 2);
    // feature/b is at display index 2
    assert_eq!(state.table_state.selected(), Some(2));
}
