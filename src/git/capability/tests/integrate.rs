use super::*;

// -- can_create_worktree --

#[test]
fn create_worktree_happy_path_allows_other_branch() {
    let branch = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
    assert!(can_create_worktree(&branch, None).available);
}

#[test]
fn create_worktree_blocks_base_branch() {
    let branch = test_branch("main", false, true, TrackingStatus::Local, None, None);
    let cap = can_create_worktree(&branch, None);
    assert!(!cap.available);
    assert_eq!(cap.reason, Some("base"));
}

#[test]
fn create_worktree_blocks_branch_in_any_worktree() {
    let current = test_branch("main-work", true, false, TrackingStatus::Local, None, None);
    let current_cap = can_create_worktree(&current, None);
    assert!(!current_cap.available);
    assert_eq!(current_cap.reason, Some("current"));

    let linked = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
    let linked_cap = can_create_worktree(&linked, Some(linked_worktree(true)));
    assert!(!linked_cap.available);
    assert_eq!(linked_cap.reason, Some("checked out in worktree"));
}

// -- can_rebase --

#[test]
fn rebase_allows_current_branch_regardless_of_cleanliness() {
    let branch = test_branch("feature/x", true, false, TrackingStatus::Local, None, None);
    assert!(can_rebase(&branch, Some(linked_worktree(false)), false).available);
    assert!(can_rebase(&branch, None, false).available);
}

#[test]
fn rebase_allows_branch_not_checked_out_anywhere() {
    let branch = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
    assert!(can_rebase(&branch, None, true).available);
}

#[test]
fn rebase_blocks_base_branch() {
    let branch = test_branch("main", false, true, TrackingStatus::Local, None, None);
    let cap = can_rebase(&branch, None, true);
    assert!(!cap.available);
    assert_eq!(cap.reason, Some("base"));
}

#[test]
fn rebase_blocks_branch_in_other_worktree() {
    let branch = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
    let cap = can_rebase(&branch, Some(linked_worktree(true)), true);
    assert!(!cap.available);
    assert_eq!(cap.reason, Some("checked out in other worktree"));
}

// -- can_merge_into_base --

#[test]
fn merge_into_base_allows_when_base_not_in_worktree() {
    let branch = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
    assert!(can_merge_into_base(&branch, None).available);
}

#[test]
fn merge_into_base_allows_when_base_worktree_clean() {
    let branch = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
    assert!(can_merge_into_base(&branch, Some(linked_worktree(true))).available);
}

#[test]
fn merge_into_base_blocks_when_base_worktree_dirty() {
    let branch = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
    let cap = can_merge_into_base(&branch, Some(linked_worktree(false)));
    assert!(!cap.available);
    assert_eq!(cap.reason, Some("base dirty"));
}

// -- can_squash_into_base --

#[test]
fn squash_into_base_allows_when_base_not_in_worktree() {
    let branch = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
    assert!(can_squash_into_base(&branch, None).available);
}

#[test]
fn squash_into_base_blocks_when_base_worktree_dirty() {
    let branch = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
    let cap = can_squash_into_base(&branch, Some(linked_worktree(false)));
    assert!(!cap.available);
    assert_eq!(cap.reason, Some("base dirty"));
}
