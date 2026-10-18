use super::*;
use crate::types::*;
use chrono::Utc;
use std::cmp::Ordering;

fn sample_branch(name: &str, days_ago: i64) -> BranchInfo {
    BranchInfo {
        name: name.to_string(),
        is_current: false,
        is_base: false,
        tracking: TrackingStatus::Local,
        ahead: None,
        behind: None,
        last_commit_date: Utc::now() - chrono::Duration::days(days_ago),
        merge_status: MergeStatus::Unmerged,
        base_branch: "main".into(),
        merge_base_commit: None,
        pr: None,
        squash_confidence: None,
    }
}

fn sample_remote(name: &str, days_ago: i64) -> RemoteBranchInfo {
    RemoteBranchInfo {
        full_ref: format!("origin/{}", name),
        remote: "origin".to_string(),
        short_name: name.to_string(),
        has_local: false,
        is_base: false,
        last_commit_date: Utc::now() - chrono::Duration::days(days_ago),
        merge_status: MergeStatus::Unmerged,
        ahead: None,
        behind: None,
        disjoint: false,
        pr: None,
        squash_confidence: None,
    }
}

#[test]
fn sort_by_name() {
    let a = sample_branch("alpha", 1);
    let b = sample_branch("beta", 1);
    let col = ColumnDef::<BranchInfo> {
        key: "name",
        name: "Name",
        show_header: true,
        min_width: 10,
        content_min_width: None,
        wide_width: None,
        hide_below_width: None,
        compare: Some(|a, b| a.name.cmp(&b.name)),
    };
    let cmp_fn = col.compare.unwrap();
    assert_eq!(cmp_fn(&a, &b), Ordering::Less);
}

#[test]
fn sort_by_age() {
    let older = sample_branch("old", 10);
    let newer = sample_branch("new", 1);
    let col = ColumnDef::<BranchInfo> {
        key: "age",
        name: "Age",
        show_header: true,
        min_width: 5,
        content_min_width: None,
        wide_width: None,
        hide_below_width: Some(60),
        compare: Some(|a, b| a.last_commit_date.cmp(&b.last_commit_date)),
    };
    let cmp_fn = col.compare.unwrap();
    assert_eq!(cmp_fn(&older, &newer), Ordering::Less);
}

#[test]
fn non_sortable_column() {
    let col = ColumnDef::<BranchInfo> {
        key: "remote",
        name: "Remote",
        show_header: true,
        min_width: 8,
        content_min_width: None,
        wide_width: None,
        hide_below_width: None,
        compare: None,
    };
    assert!(col.compare.is_none());
}

#[test]
fn age_cmp_sorts_older_first() {
    let older = sample_branch("old", 10);
    let newer = sample_branch("new", 1);
    assert_eq!(age_cmp(&older, &newer), Ordering::Less);
    assert_eq!(age_cmp(&newer, &older), Ordering::Greater);
}

#[test]
fn age_cmp_remote_sorts_older_first() {
    let older = sample_remote("old", 10);
    let newer = sample_remote("new", 1);
    assert_eq!(age_cmp(&older, &newer), Ordering::Less);
    assert_eq!(age_cmp(&newer, &older), Ordering::Greater);
}

#[test]
fn age_column_has_correct_properties() {
    let col = age_column::<BranchInfo>();
    assert_eq!(col.name, "Age");
    assert_eq!(col.min_width, 5);
    assert_eq!(col.wide_width, Some(14));
    assert_eq!(col.hide_below_width, Some(60));
    assert!(col.compare.is_some());
}

#[test]
fn ahead_behind_cmp_sorts_by_ahead_then_behind() {
    let mut a = sample_branch("a", 1);
    let mut b = sample_branch("b", 1);

    a.ahead = Some(5);
    a.behind = Some(0);
    b.ahead = Some(3);
    b.behind = Some(0);
    assert_eq!(ahead_behind_cmp(&a, &b), Ordering::Greater);

    a.ahead = Some(5);
    a.behind = Some(2);
    b.ahead = Some(5);
    b.behind = Some(3);
    assert_eq!(ahead_behind_cmp(&a, &b), Ordering::Less);

    a.ahead = None;
    a.behind = None;
    b.ahead = None;
    b.behind = None;
    assert_eq!(ahead_behind_cmp(&a, &b), Ordering::Equal);
}

#[test]
fn ahead_behind_cmp_treats_none_as_zero() {
    let mut a = sample_branch("a", 1);
    let mut b = sample_branch("b", 1);

    a.ahead = Some(5);
    a.behind = None;
    b.ahead = None;
    b.behind = None;
    assert_eq!(ahead_behind_cmp(&a, &b), Ordering::Greater);
}

#[test]
fn pr_cmp_both_some_compares_numbers() {
    let mut a = sample_branch("a", 1);
    let mut b = sample_branch("b", 1);

    a.pr = Some(PrInfo {
        number: 100,
        status: PrStatus::Open,
    });
    b.pr = Some(PrInfo {
        number: 200,
        status: PrStatus::Open,
    });
    assert_eq!(pr_cmp(&a, &b), Ordering::Less);
    assert_eq!(pr_cmp(&b, &a), Ordering::Greater);
}

#[test]
fn pr_cmp_some_sorts_before_none() {
    let mut a = sample_branch("a", 1);
    let mut b = sample_branch("b", 1);

    a.pr = Some(PrInfo {
        number: 100,
        status: PrStatus::Open,
    });
    b.pr = None;
    assert_eq!(pr_cmp(&a, &b), Ordering::Less);
    assert_eq!(pr_cmp(&b, &a), Ordering::Greater);
}

#[test]
fn pr_cmp_both_none_equal() {
    let a = sample_branch("a", 1);
    let b = sample_branch("b", 1);
    assert_eq!(pr_cmp(&a, &b), Ordering::Equal);
}

#[test]
fn merge_status_rank_correct_values() {
    assert_eq!(merge_status_rank(&MergeStatus::Merged), 0);
    assert_eq!(merge_status_rank(&MergeStatus::InSync), 1);
    assert_eq!(merge_status_rank(&MergeStatus::SquashMerged), 2);
    assert_eq!(merge_status_rank(&MergeStatus::RemoteMerged), 3);
    assert_eq!(merge_status_rank(&MergeStatus::LocalMerged), 4);
    assert_eq!(merge_status_rank(&MergeStatus::RemoteSquashMerged), 5);
    assert_eq!(merge_status_rank(&MergeStatus::LocalSquashMerged), 6);
    assert_eq!(merge_status_rank(&MergeStatus::CherryPicked), 7);
    assert_eq!(merge_status_rank(&MergeStatus::RemoteCherryPicked), 8);
    assert_eq!(merge_status_rank(&MergeStatus::LocalCherryPicked), 9);
    assert_eq!(merge_status_rank(&MergeStatus::LikelySquashMerged), 10);
    assert_eq!(merge_status_rank(&MergeStatus::Unmerged), 11);
    assert_eq!(merge_status_rank(&MergeStatus::Pending), 12);
}

#[test]
fn merge_status_cmp_sorts_by_rank() {
    let mut a = sample_branch("a", 1);
    let mut b = sample_branch("b", 1);

    a.merge_status = MergeStatus::Merged;
    b.merge_status = MergeStatus::Unmerged;
    assert_eq!(merge_status_cmp(&a, &b), Ordering::Less);

    a.merge_status = MergeStatus::SquashMerged;
    b.merge_status = MergeStatus::Unmerged;
    assert_eq!(merge_status_cmp(&a, &b), Ordering::Less);

    a.merge_status = MergeStatus::Unmerged;
    b.merge_status = MergeStatus::Pending;
    assert_eq!(merge_status_cmp(&a, &b), Ordering::Less);
}

#[test]
fn apply_merge_status_if_confident_allows_more_confident_overwrite() {
    let mut current = MergeStatus::Unmerged;
    assert!(apply_merge_status_if_confident(
        &mut current,
        MergeStatus::SquashMerged
    ));
    assert_eq!(current, MergeStatus::SquashMerged);
}

#[test]
fn apply_merge_status_if_confident_allows_equal_rank_overwrite() {
    let mut current = MergeStatus::LocalSquashMerged;
    assert!(apply_merge_status_if_confident(
        &mut current,
        MergeStatus::LocalSquashMerged
    ));
    assert_eq!(current, MergeStatus::LocalSquashMerged);
}

#[test]
fn apply_merge_status_if_confident_rejects_less_confident_overwrite() {
    let mut current = MergeStatus::SquashMerged;
    assert!(!apply_merge_status_if_confident(
        &mut current,
        MergeStatus::Unmerged
    ));
    assert_eq!(
        current,
        MergeStatus::SquashMerged,
        "a stale, less-confident result must not regress an already-confirmed status"
    );
}

#[test]
fn ahead_behind_column_has_correct_properties() {
    let col = ahead_behind_column::<BranchInfo>();
    assert_eq!(col.name, "A/B");
    assert_eq!(col.min_width, 3);
    assert_eq!(col.wide_width, Some(8));
    assert_eq!(col.hide_below_width, None);
    assert!(col.compare.is_some());
}

#[test]
fn pr_column_has_correct_properties() {
    let col = pr_column::<BranchInfo>();
    assert_eq!(col.name, "PR");
    assert_eq!(col.min_width, 2);
    assert_eq!(col.wide_width, Some(9));
    assert_eq!(col.hide_below_width, None);
    assert!(col.compare.is_some());
}

#[test]
fn merge_status_column_has_correct_properties() {
    let col = merge_status_column::<BranchInfo>("Merge");
    assert_eq!(col.name, "Merge");
    assert_eq!(col.min_width, 5);
    assert_eq!(col.wide_width, Some(16));
    assert_eq!(col.hide_below_width, None);
    assert!(col.compare.is_some());
}

#[test]
fn merge_status_column_with_different_name() {
    let col = merge_status_column::<BranchInfo>("Merge");
    assert_eq!(col.name, "Merge");
    assert_eq!(col.min_width, 5);
    assert_eq!(col.wide_width, Some(16));
}
