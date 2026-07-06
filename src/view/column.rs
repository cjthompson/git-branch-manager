use super::ViewItem;
use crate::types::{MergeStatus, WorkingTreeStatus, WorktreeInfo};
use std::cmp::Ordering;

/// Defines a single column in a view's table layout.
/// The compare function is used for sorting; None means not sortable.
pub struct ColumnDef<T: ViewItem> {
    pub key: &'static str,
    pub name: &'static str,
    pub min_width: u16,
    /// When Some(w), use width w instead of min_width when terminal width >= 70.
    pub wide_width: Option<u16>,
    /// Hide this column when terminal width is below this threshold
    pub hide_below_width: Option<u16>,
    /// Comparison function for sorting. None = column is not sortable.
    pub compare: Option<fn(&T, &T) -> Ordering>,
}

/// Comparator: sort by last commit date (ascending = oldest first).
pub fn age_cmp<T: ViewItem>(a: &T, b: &T) -> Ordering {
    a.last_commit_date().cmp(b.last_commit_date())
}

/// Build a standard "Age" column definition for any view.
pub fn age_column<T: ViewItem>() -> ColumnDef<T> {
    ColumnDef {
        key: "age",
        name: "Age",
        min_width: 5,
        wide_width: Some(14),
        hide_below_width: Some(60),
        compare: Some(age_cmp),
    }
}

/// Comparator: sort by ahead count, then behind count (ascending).
pub fn ahead_behind_cmp<T: ViewItem>(a: &T, b: &T) -> Ordering {
    a.ahead()
        .unwrap_or(0)
        .cmp(&b.ahead().unwrap_or(0))
        .then(a.behind().unwrap_or(0).cmp(&b.behind().unwrap_or(0)))
}

/// Build a standard "A/B" column definition.
pub fn ahead_behind_column<T: ViewItem>() -> ColumnDef<T> {
    ColumnDef {
        key: "ahead_behind",
        name: "A/B",
        min_width: 8,
        wide_width: None,
        hide_below_width: Some(80),
        compare: Some(ahead_behind_cmp),
    }
}

/// Comparator: sort by PR number. Items with a PR sort before items without.
pub fn pr_cmp<T: ViewItem>(a: &T, b: &T) -> Ordering {
    match (a.pr_info().map(|p| p.number), b.pr_info().map(|p| p.number)) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

/// Build a standard "PR" column definition.
pub fn pr_column<T: ViewItem>() -> ColumnDef<T> {
    ColumnDef {
        key: "pr",
        name: "PR",
        min_width: 5,
        wide_width: None,
        hide_below_width: Some(80),
        compare: Some(pr_cmp),
    }
}

/// Rank a MergeStatus for sorting: lower = sorted first.
pub fn merge_status_rank(status: &MergeStatus) -> u8 {
    match status {
        MergeStatus::Merged => 0,
        MergeStatus::InSync => 1,
        MergeStatus::SquashMerged => 2,
        MergeStatus::RemoteMerged => 3,
        MergeStatus::LocalMerged => 4,
        MergeStatus::RemoteSquashMerged => 5,
        MergeStatus::LocalSquashMerged => 6,
        MergeStatus::Unmerged => 7,
        MergeStatus::Pending => 8,
    }
}

/// Comparator: sort by merge status rank. Items without a merge status sort last.
pub fn merge_status_cmp<T: ViewItem>(a: &T, b: &T) -> Ordering {
    let rank_a = a.merge_status().map_or(u8::MAX, merge_status_rank);
    let rank_b = b.merge_status().map_or(u8::MAX, merge_status_rank);
    rank_a.cmp(&rank_b)
}

/// Build the shared merge-status column ("Merge" in every view). The name is a
/// parameter only so callers read explicitly; pass `"Merge"`.
pub fn merge_status_column<T: ViewItem>(name: &'static str) -> ColumnDef<T> {
    ColumnDef {
        key: "merge",
        name,
        min_width: 5,
        wide_width: Some(16),
        hide_below_width: None,
        compare: Some(merge_status_cmp),
    }
}

/// Rank a working-tree status for sorting: dirtier states sort later.
pub fn wt_status_rank(s: &WorkingTreeStatus) -> u8 {
    (s.has_staged as u8) * 4 + (s.has_modified as u8) * 2 + (s.has_untracked as u8)
}

/// Comparator: sort worktrees by working-tree dirtiness.
pub fn wt_status_cmp(a: &WorktreeInfo, b: &WorktreeInfo) -> Ordering {
    wt_status_rank(&a.wt_status).cmp(&wt_status_rank(&b.wt_status))
}

/// Build the shared worktree "Status" (working-tree dirtiness) column. This is
/// the single definition of how the Status column sizes and sorts; views use it
/// rather than declaring their own. Renders full words when wide and single
/// letters when narrow (see `ui::cells::worktree_status_line`).
pub fn worktree_status_column() -> ColumnDef<WorktreeInfo> {
    ColumnDef {
        key: "status",
        name: "Status",
        min_width: 3,
        wide_width: Some(9),
        hide_below_width: Some(80),
        compare: Some(wt_status_cmp),
    }
}

#[cfg(test)]
mod tests {
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
        }
    }

    #[test]
    fn sort_by_name() {
        let a = sample_branch("alpha", 1);
        let b = sample_branch("beta", 1);
        let col = ColumnDef::<BranchInfo> {
            key: "name",
            name: "Name",
            min_width: 10,
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
            min_width: 5,
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
            min_width: 8,
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
        assert_eq!(merge_status_rank(&MergeStatus::Unmerged), 7);
        assert_eq!(merge_status_rank(&MergeStatus::Pending), 8);
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
}
