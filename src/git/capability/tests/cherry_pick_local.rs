use super::*;

#[test]
fn can_cherry_pick_local_blocks_base() {
    let branch = test_branch("main", false, true, TrackingStatus::Local, None, None);
    let cap = can_cherry_pick_local(&branch);
    assert!(!cap.available);
    assert_eq!(cap.reason, Some("base"));
}

#[test]
fn can_cherry_pick_local_blocks_current() {
    let branch = test_branch("feature/x", true, false, TrackingStatus::Local, None, None);
    let cap = can_cherry_pick_local(&branch);
    assert!(!cap.available);
    assert_eq!(cap.reason, Some("current"));
}

#[test]
fn can_cherry_pick_local_allows_other_local_branch() {
    let branch = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
    assert!(can_cherry_pick_local(&branch).available);
}
