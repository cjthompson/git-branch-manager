use super::*;

// -- can_push --

#[test]
fn push_happy_path_when_remote_configured() {
    let branch = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
    assert!(can_push(&branch, true, true).available);
}

#[test]
fn push_blocks_when_no_remote() {
    let branch = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
    let cap = can_push(&branch, false, true);
    assert!(!cap.available);
    assert_eq!(cap.reason, Some("no remote"));
}

#[test]
fn push_ignores_worktree_state() {
    // can_push has no worktree parameter at all; a dirty, current branch
    // with a configured remote must still be pushable.
    let branch = test_branch("feature/x", true, false, TrackingStatus::Local, None, None);
    assert!(can_push(&branch, true, false).available);
}

#[test]
fn push_blocks_when_tracked_and_not_ahead() {
    let tracked = TrackingStatus::Tracked {
        remote_ref: "origin/feature/x".to_string(),
        gone: false,
    };
    let branch = test_branch("feature/x", false, false, tracked, Some(0), None);
    let cap = can_push(&branch, true, true);
    assert!(!cap.available);
    assert_eq!(cap.reason, Some("not ahead"));
}

#[test]
fn push_allows_when_tracked_and_ahead() {
    let tracked = TrackingStatus::Tracked {
        remote_ref: "origin/feature/x".to_string(),
        gone: false,
    };
    let branch = test_branch("feature/x", false, false, tracked, Some(3), None);
    assert!(can_push(&branch, true, true).available);
}

// -- can_pull --

#[test]
fn pull_allows_behind_current_branch() {
    let branch = test_branch(
        "feature/x",
        true,
        false,
        TrackingStatus::Local,
        None,
        Some(3),
    );
    assert!(can_pull(&branch, None, true).available);
}

#[test]
fn pull_allows_non_current_branch_behind_and_not_checked_out() {
    let branch = test_branch(
        "feature/x",
        false,
        false,
        TrackingStatus::Local,
        None,
        Some(3),
    );
    assert!(can_pull(&branch, None, true).available);
}

#[test]
fn pull_blocks_non_current_branch_when_not_behind() {
    let branch = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
    let cap = can_pull(&branch, None, true);
    assert!(!cap.available);
    assert_eq!(cap.reason, Some("not behind"));
}

#[test]
fn pull_blocks_behind_branch_in_other_worktree() {
    let branch = test_branch(
        "feature/x",
        false,
        false,
        TrackingStatus::Local,
        None,
        Some(3),
    );
    let cap = can_pull(&branch, Some(linked_worktree(true)), true);
    assert!(!cap.available);
    assert_eq!(cap.reason, Some("checked out in worktree"));
}

#[test]
fn pull_blocks_when_no_remote() {
    let branch = test_branch(
        "feature/x",
        true,
        false,
        TrackingStatus::Local,
        None,
        Some(3),
    );
    let cap = can_pull(&branch, None, false);
    assert!(!cap.available);
    assert_eq!(cap.reason, Some("no remote"));
}

// -- can_fast_forward --

#[test]
fn fast_forward_happy_path_when_behind_and_not_ahead() {
    let branch = test_branch(
        "feature/x",
        false,
        false,
        TrackingStatus::Local,
        Some(0),
        Some(2),
    );
    assert!(can_fast_forward(&branch, None, true).available);
}

#[test]
fn fast_forward_blocks_when_in_sync() {
    let branch = test_branch(
        "feature/x",
        false,
        false,
        TrackingStatus::Local,
        Some(0),
        Some(0),
    );
    let cap = can_fast_forward(&branch, None, true);
    assert!(!cap.available);
    assert_eq!(cap.reason, Some("in sync"));
}

#[test]
fn fast_forward_blocks_branch_in_other_worktree() {
    let branch = test_branch(
        "feature/x",
        false,
        false,
        TrackingStatus::Local,
        Some(0),
        Some(2),
    );
    let cap = can_fast_forward(&branch, Some(linked_worktree(true)), true);
    assert!(!cap.available);
    assert_eq!(cap.reason, Some("checked out in worktree"));
}

#[test]
fn fast_forward_blocks_when_no_remote() {
    let branch = test_branch(
        "feature/x",
        false,
        false,
        TrackingStatus::Local,
        Some(0),
        Some(2),
    );
    let cap = can_fast_forward(&branch, None, false);
    assert!(!cap.available);
    assert_eq!(cap.reason, Some("no remote"));
}

// -- can_force_push --

#[test]
fn force_push_happy_path_when_ahead() {
    let branch = test_branch(
        "feature/x",
        false,
        false,
        TrackingStatus::Local,
        Some(2),
        Some(0),
    );
    assert!(can_force_push(&branch, true).available);
}

#[test]
fn force_push_blocks_when_behind() {
    let branch = test_branch(
        "feature/x",
        false,
        false,
        TrackingStatus::Local,
        Some(0),
        Some(2),
    );
    let cap = can_force_push(&branch, true);
    assert!(!cap.available);
    assert_eq!(cap.reason, Some("behind or in sync"));
}

#[test]
fn force_push_blocks_when_no_remote() {
    let branch = test_branch(
        "feature/x",
        false,
        false,
        TrackingStatus::Local,
        Some(2),
        Some(0),
    );
    let cap = can_force_push(&branch, false);
    assert!(!cap.available);
    assert_eq!(cap.reason, Some("no remote"));
}
