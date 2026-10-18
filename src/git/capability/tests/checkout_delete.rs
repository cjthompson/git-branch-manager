use super::*;

// -- can_checkout_local --

#[test]
fn checkout_happy_path_allows_other_branch() {
    let branch = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
    assert!(can_checkout_local(&branch, None).available);
}

#[test]
fn checkout_blocks_current_branch() {
    let branch = test_branch("feature/x", true, false, TrackingStatus::Local, None, None);
    let cap = can_checkout_local(&branch, None);
    assert!(!cap.available);
    assert_eq!(cap.reason, Some("current"));
}

#[test]
fn checkout_blocks_base_branch() {
    let branch = test_branch("main", false, true, TrackingStatus::Local, None, None);
    let cap = can_checkout_local(&branch, None);
    assert!(!cap.available);
    assert_eq!(cap.reason, Some("base"));
}

#[test]
fn checkout_blocks_branch_in_linked_worktree() {
    let branch = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
    let cap = can_checkout_local(&branch, Some(linked_worktree(true)));
    assert!(!cap.available);
    assert_eq!(cap.reason, Some("checked out in worktree"));
}

// -- can_delete_local --

#[test]
fn delete_local_happy_path_allows_other_branch() {
    let branch = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
    assert!(can_delete_local(&branch, None).available);
}

#[test]
fn delete_local_blocks_current_branch() {
    let branch = test_branch("feature/x", true, false, TrackingStatus::Local, None, None);
    let cap = can_delete_local(&branch, None);
    assert!(!cap.available);
    assert_eq!(cap.reason, Some("current"));
}

#[test]
fn delete_local_blocks_base_branch() {
    let branch = test_branch("main", false, true, TrackingStatus::Local, None, None);
    let cap = can_delete_local(&branch, None);
    assert!(!cap.available);
    assert_eq!(cap.reason, Some("base"));
}

#[test]
fn delete_local_blocks_branch_in_linked_worktree() {
    let branch = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
    let cap = can_delete_local(&branch, Some(linked_worktree(true)));
    assert!(!cap.available);
    assert_eq!(cap.reason, Some("checked out in worktree"));
}

// -- can_delete_remote --

#[test]
fn delete_remote_happy_path_allows_when_remote_configured() {
    let branch = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
    assert!(can_delete_remote(&branch, None, true).available);
}

#[test]
fn delete_remote_blocks_when_no_remote() {
    let branch = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
    let cap = can_delete_remote(&branch, None, false);
    assert!(!cap.available);
    assert_eq!(cap.reason, Some("no remote"));
}

#[test]
fn delete_remote_blocks_branch_in_linked_worktree() {
    let branch = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
    let cap = can_delete_remote(&branch, Some(linked_worktree(true)), true);
    assert!(!cap.available);
    assert_eq!(cap.reason, Some("checked out in worktree"));
}

// -- can_force_delete_local --

#[test]
fn force_delete_local_happy_path_allows_other_branch() {
    let branch = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
    assert!(can_force_delete_local(&branch, None).available);
}

#[test]
fn force_delete_local_blocks_base_branch() {
    let branch = test_branch("main", false, true, TrackingStatus::Local, None, None);
    let cap = can_force_delete_local(&branch, None);
    assert!(!cap.available);
    assert_eq!(cap.reason, Some("base"));
}

#[test]
fn force_delete_local_blocks_branch_in_linked_worktree() {
    let branch = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
    let cap = can_force_delete_local(&branch, Some(linked_worktree(true)));
    assert!(!cap.available);
    assert_eq!(cap.reason, Some("checked out in worktree"));
}

// -- can_delete_local_and_remote --

#[test]
fn delete_local_and_remote_happy_path_allows_when_remote_configured() {
    let branch = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
    assert!(can_delete_local_and_remote(&branch, None, true).available);
}

#[test]
fn delete_local_and_remote_blocks_when_no_remote() {
    let branch = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
    let cap = can_delete_local_and_remote(&branch, None, false);
    assert!(!cap.available);
    assert_eq!(cap.reason, Some("no remote"));
}

#[test]
fn delete_local_and_remote_blocks_branch_in_linked_worktree() {
    let branch = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
    let cap = can_delete_local_and_remote(&branch, Some(linked_worktree(true)), true);
    assert!(!cap.available);
    assert_eq!(cap.reason, Some("checked out in worktree"));
}
