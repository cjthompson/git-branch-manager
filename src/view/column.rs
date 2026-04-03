use super::ViewItem;
use std::cmp::Ordering;

/// Defines a single column in a view's table layout.
/// The compare function is used for sorting; None means not sortable.
pub struct ColumnDef<T: ViewItem> {
    pub name: &'static str,
    pub min_width: u16,
    /// Hide this column when terminal width is below this threshold
    pub hide_below_width: Option<u16>,
    /// Comparison function for sorting. None = column is not sortable.
    pub compare: Option<fn(&T, &T) -> Ordering>,
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
        }
    }

    #[test]
    fn sort_by_name() {
        let a = sample_branch("alpha", 1);
        let b = sample_branch("beta", 1);
        let col = ColumnDef::<BranchInfo> {
            name: "Name",
            min_width: 10,
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
            name: "Age",
            min_width: 5,
            hide_below_width: Some(60),
            compare: Some(|a, b| a.last_commit_date.cmp(&b.last_commit_date)),
        };
        let cmp_fn = col.compare.unwrap();
        assert_eq!(cmp_fn(&older, &newer), Ordering::Less);
    }

    #[test]
    fn non_sortable_column() {
        let col = ColumnDef::<BranchInfo> {
            name: "Remote",
            min_width: 8,
            hide_below_width: None,
            compare: None,
        };
        assert!(col.compare.is_none());
    }
}
