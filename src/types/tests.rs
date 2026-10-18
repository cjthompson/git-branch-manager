use super::*;
use chrono::Utc;
use std::path::PathBuf;

#[test]
fn format_age_just_now() {
    let now = Utc::now();
    assert_eq!(format_age(&now), "just now");
}

#[test]
fn format_age_minutes() {
    let date = Utc::now() - chrono::Duration::minutes(5);
    assert_eq!(format_age(&date), "5 minutes ago");
}

#[test]
fn format_age_singular_day() {
    let date = Utc::now() - chrono::Duration::days(1);
    assert_eq!(format_age(&date), "1 day ago");
}

#[test]
fn format_age_plural_days() {
    let date = Utc::now() - chrono::Duration::days(3);
    assert_eq!(format_age(&date), "3 days ago");
}

#[test]
fn format_age_short_days() {
    let date = Utc::now() - chrono::Duration::days(3);
    assert_eq!(format_age_short(&date), "3d");
}

#[test]
fn format_age_short_weeks() {
    let date = Utc::now() - chrono::Duration::weeks(2);
    assert_eq!(format_age_short(&date), "2w");
}

#[test]
fn format_age_short_months() {
    let date = Utc::now() - chrono::Duration::days(60);
    assert_eq!(format_age_short(&date), "2mo");
}

#[test]
fn format_age_short_years() {
    let date = Utc::now() - chrono::Duration::days(400);
    assert_eq!(format_age_short(&date), "1y");
}

#[test]
fn format_local_absolute_renders_in_local_timezone_with_offset() {
    use chrono::{Local, TimeZone};
    let date = Utc.with_ymd_and_hms(2026, 9, 2, 21, 23, 45).unwrap();
    let rendered = format_local_absolute(&date);

    // Compute the expected rendering using the same Local timezone the
    // implementation uses — works in any offset including half/quarter-hour
    // zones (India +05:30, Nepal +05:45, Newfoundland -03:30, Iran +03:30, etc.).
    let expected_local = date.with_timezone(&Local);
    let expected_prefix = expected_local.format("%Y-%m-%d %H:%M:%S ").to_string();
    assert!(
        rendered.starts_with(&expected_prefix),
        "expected local prefix {expected_prefix:?}, got {rendered:?}"
    );
    assert!(
        rendered.len() == expected_prefix.len() + 6,
        "expected trailing 6-char offset, got {rendered:?}"
    );

    // Final 6 chars must be a UTC offset like -07:00, +05:30, +00:00
    let suffix = &rendered[rendered.len() - 6..];
    assert!(
        (suffix.starts_with('+') || suffix.starts_with('-')) && suffix.contains(':'),
        "expected ±HH:MM suffix, got: {suffix}"
    );
}

#[test]
fn working_tree_status_clean() {
    let s = WorkingTreeStatus::clean();
    assert!(s.is_clean());
    assert_eq!(s.summary(), "");
}

#[test]
fn working_tree_status_staged_only() {
    let s = WorkingTreeStatus {
        has_staged: true,
        has_modified: false,
        has_untracked: false,
        changed_files: Vec::new(),
    };
    assert!(!s.is_clean());
    assert_eq!(s.summary(), "staged");
}

#[test]
fn working_tree_status_all_three() {
    let s = WorkingTreeStatus {
        has_staged: true,
        has_modified: true,
        has_untracked: true,
        changed_files: Vec::new(),
    };
    assert_eq!(s.summary(), "staged+modified+untracked");
}

#[test]
fn branch_info_is_pinned() {
    let b = BranchInfo {
        name: "main".into(),
        is_current: false,
        is_base: true,
        tracking: TrackingStatus::Local,
        ahead: None,
        behind: None,
        last_commit_date: Utc::now(),
        merge_status: MergeStatus::Unmerged,
        base_branch: "main".into(),
        merge_base_commit: None,
        pr: None,
        squash_confidence: None,
    };
    assert!(b.is_pinned());
}

#[test]
fn branch_info_not_pinned() {
    let b = BranchInfo {
        name: "feature/x".into(),
        is_current: false,
        is_base: false,
        tracking: TrackingStatus::Local,
        ahead: None,
        behind: None,
        last_commit_date: Utc::now(),
        merge_status: MergeStatus::Unmerged,
        base_branch: "main".into(),
        merge_base_commit: None,
        pr: None,
        squash_confidence: None,
    };
    assert!(!b.is_pinned());
}

#[test]
fn tag_info_age_short() {
    let t = TagInfo {
        name: "v1.0".into(),
        commit_hash: "abc1234".into(),
        date: Utc::now() - chrono::Duration::days(5),
        message: Some("Release 1.0".into()),
        is_annotated: true,
    };
    assert_eq!(t.age_short(), "5d");
}

#[test]
fn worktree_info_main_is_pinned() {
    let w = WorktreeInfo {
        path: PathBuf::from("/repo"),
        branch: Some("main".into()),
        is_main: true,
        is_base: true,
        commit_hash: "abc1234".into(),
        wt_status: WorkingTreeStatus::clean(),
        age_date: Utc::now(),
        merge_status: MergeStatus::Unmerged,
        ahead: None,
        behind: None,
        pr: None,
    };
    assert!(w.is_pinned());
}

#[test]
fn branch_action_labels() {
    assert_eq!(BranchAction::DeleteLocal.label(), "Delete local");
    assert_eq!(BranchAction::Checkout.label(), "Checkout");
    assert_eq!(BranchAction::PushTag.label(), "Push tag");
    assert_eq!(BranchAction::WorktreeRemove.label(), "Remove worktree");
}

#[test]
fn operation_result_success_sets_success_true_and_no_failure_cause() {
    let result = OperationResult::success(
        "feature/success",
        BranchAction::DeleteLocal,
        "Deleted feature/success",
    );
    assert!(result.success);
    assert!(result.failure.is_none());
    assert_eq!(result.branch_name, "feature/success");
    assert_eq!(result.message, "Deleted feature/success");
    assert_eq!(result.action, BranchAction::DeleteLocal);
}

#[test]
fn operation_result_failure_sets_success_false_and_typed_cause() {
    let result = OperationResult::failure(
        "feature/failure",
        BranchAction::DeleteLocal,
        FailureCause::NotMerged,
        "Failed to delete feature/failure: not merged",
    );
    assert!(!result.success);
    assert_eq!(result.branch_name, "feature/failure");
    assert_eq!(
        result.message,
        "Failed to delete feature/failure: not merged"
    );
    assert_eq!(result.failure, Some(FailureCause::NotMerged));
    assert_eq!(result.action, BranchAction::DeleteLocal);
}

#[test]
fn operation_result_success_and_failure_invariant_matches_failure_option() {
    // `success` and `failure.is_some()` must never agree: success carries
    // no cause, failure always carries one.
    let success = OperationResult::success("b", BranchAction::DeleteLocal, "ok");
    assert_eq!(success.success, success.failure.is_none());

    let failure = OperationResult::failure(
        "b",
        BranchAction::DeleteLocal,
        FailureCause::BranchNotFound,
        "fallback",
    );
    assert_eq!(failure.success, failure.failure.is_none());
}

#[test]
fn operation_result_failure_preserves_checked_out_in_worktree_payload() {
    let worktree_path = PathBuf::from("/repo/.worktrees/feature-linked");
    let cause = FailureCause::CheckedOutInWorktree {
        worktree_path: worktree_path.clone(),
        is_main: false,
    };
    let result = OperationResult::failure(
        "feature/linked",
        BranchAction::DeleteLocal,
        cause,
        "checked out elsewhere",
    );
    match result.failure {
        Some(FailureCause::CheckedOutInWorktree {
            worktree_path: got_path,
            is_main,
        }) => {
            assert_eq!(got_path, worktree_path);
            assert!(!is_main);
        }
        other => panic!("expected CheckedOutInWorktree payload, got {other:?}"),
    }
}
