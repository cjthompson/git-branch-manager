use super::*;

// --- Navigation tests ---

#[test]
fn nav_down_moves_cursor() {
    let mut state = ListState::new(sample_branches());
    nav_down(&mut state);
    assert_eq!(state.table_state.selected(), Some(1));
}

#[test]
fn nav_down_stops_at_end() {
    let mut state = ListState::new(sample_branches());
    nav_down(&mut state);
    nav_down(&mut state);
    nav_down(&mut state); // past end
    assert_eq!(state.table_state.selected(), Some(2)); // stays at last
}

#[test]
fn nav_up_moves_cursor() {
    let mut state = ListState::new(sample_branches());
    nav_down(&mut state);
    nav_down(&mut state);
    nav_up(&mut state);
    assert_eq!(state.table_state.selected(), Some(1));
}

#[test]
fn nav_up_stops_at_top() {
    let mut state = ListState::new(sample_branches());
    nav_up(&mut state);
    assert_eq!(state.table_state.selected(), Some(0));
}

#[test]
fn nav_to_end() {
    let mut state = ListState::new(sample_branches());
    nav_end(&mut state);
    assert_eq!(state.table_state.selected(), Some(2));
}

#[test]
fn nav_to_home() {
    let mut state = ListState::new(sample_branches());
    nav_end(&mut state);
    nav_home(&mut state);
    assert_eq!(state.table_state.selected(), Some(0));
}

#[test]
fn nav_page_down_moves() {
    let mut state = ListState::new(sample_branches());
    nav_page_down(&mut state, 2);
    assert_eq!(state.table_state.selected(), Some(2));
}

#[test]
fn nav_page_up_moves() {
    let mut state = ListState::new(sample_branches());
    nav_end(&mut state);
    nav_page_up(&mut state, 2);
    assert_eq!(state.table_state.selected(), Some(0));
}

#[test]
fn nav_on_empty() {
    let mut state = ListState::<BranchInfo>::empty();
    nav_down(&mut state);
    nav_up(&mut state);
    nav_home(&mut state);
    nav_end(&mut state);
    nav_page_down(&mut state, 10);
    nav_page_up(&mut state, 10);
    // Should not panic
    assert!(state.table_state.selected().is_none() || state.table_state.selected() == Some(0));
}

// --- Selection tests ---

#[test]
fn select_toggle_on_non_pinned() {
    let mut state = ListState::new(sample_branches());
    nav_down(&mut state); // move to feature/a
    select_toggle(&mut state);
    assert!(state.selected()[1]); // feature/a selected
}

#[test]
fn select_toggle_on_pinned_does_nothing() {
    let mut state = ListState::new(sample_branches());
    // cursor on main (pinned)
    select_toggle(&mut state);
    assert!(!state.selected()[0]); // main is pinned, not selectable
}

#[test]
fn select_all_skips_pinned() {
    let mut state = ListState::new(sample_branches());
    select_all(&mut state);
    assert!(!state.selected()[0]); // main is pinned
    assert!(state.selected()[1]);
    assert!(state.selected()[2]);
}

#[test]
fn deselect_all_clears() {
    let mut state = ListState::new(sample_branches());
    select_all(&mut state);
    deselect_all(&mut state);
    assert!(state.selected().iter().all(|&s| !s));
}

#[test]
fn invert_selection_works() {
    let mut state = ListState::new(sample_branches());
    state.selected_mut()[1] = true;
    invert_selection(&mut state);
    assert!(!state.selected()[0]); // pinned stays unselected
    assert!(!state.selected()[1]); // was selected, now not
    assert!(state.selected()[2]); // was not selected, now selected
}

#[test]
fn select_merged_works() {
    let mut state = ListState::new(sample_branches());
    select_merged(&mut state);
    assert!(!state.selected()[0]); // main (pinned)
    assert!(!state.selected()[1]); // unmerged
    assert!(state.selected()[2]); // merged
}

#[test]
fn select_merged_excludes_likely_squash_merged() {
    let mut branches = sample_branches();
    branches[1].merge_status = MergeStatus::LikelySquashMerged;
    let mut state = ListState::new(branches);
    select_merged(&mut state);
    assert!(!state.selected()[1]);
}
