use super::column::ColumnDef;
use super::filter::FilterTokenDef;
use crate::types::WorktreeInfo;

pub struct WorktreesViewDef;

impl WorktreesViewDef {
    pub fn columns(&self) -> Vec<ColumnDef<WorktreeInfo>> {
        vec![
            ColumnDef {
                name: "Path",
                min_width: 15,
                wide_width: None,
                hide_below_width: None,
                compare: Some(|a, b| a.path.cmp(&b.path)),
            },
            ColumnDef {
                name: "Branch",
                min_width: 10,
                wide_width: None,
                hide_below_width: None,
                compare: Some(|a, b| a.branch.cmp(&b.branch)),
            },
            ColumnDef {
                name: "Status",
                min_width: 8,
                wide_width: None,
                hide_below_width: Some(80),
                compare: Some(|a, b| {
                    let rank = |w: &WorktreeInfo| {
                        let s = &w.wt_status;
                        (s.has_staged as u8) * 4
                            + (s.has_unstaged as u8) * 2
                            + (s.has_untracked as u8)
                    };
                    rank(a).cmp(&rank(b))
                }),
            },
            ColumnDef {
                name: "Age",
                min_width: 5,
                wide_width: Some(12),
                hide_below_width: Some(60),
                compare: Some(|a, b| a.age_date.cmp(&b.age_date)),
            },
            ColumnDef {
                name: "Merge",
                min_width: 4,
                wide_width: Some(15),
                hide_below_width: None,
                compare: Some(|a, b| {
                    let rank = |s: &crate::types::MergeStatus| match s {
                        crate::types::MergeStatus::Merged => 0,
                        crate::types::MergeStatus::SquashMerged => 1,
                        crate::types::MergeStatus::Unmerged => 2,
                        crate::types::MergeStatus::Pending => 3,
                    };
                    rank(&a.merge_status).cmp(&rank(&b.merge_status))
                }),
            },
        ]
    }

    pub fn filter_tokens(&self) -> Vec<FilterTokenDef> {
        vec![
            FilterTokenDef {
                key: 'm',
                label: "Merged",
                token: "status:merged",
            },
            FilterTokenDef {
                key: 's',
                label: "Squash-merged",
                token: "status:squash",
            },
            FilterTokenDef {
                key: 'u',
                label: "Unmerged",
                token: "status:unmerged",
            },
            FilterTokenDef {
                key: '1',
                label: "<7 days",
                token: "age:<7d",
            },
            FilterTokenDef {
                key: '2',
                label: "<30 days",
                token: "age:<30d",
            },
            FilterTokenDef {
                key: '3',
                label: ">30 days",
                token: "age:>30d",
            },
            FilterTokenDef {
                key: '4',
                label: ">90 days",
                token: "age:>90d",
            },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_correct_column_count() {
        let view = WorktreesViewDef;
        assert_eq!(view.columns().len(), 5);
    }

    #[test]
    fn path_column_is_sortable() {
        let view = WorktreesViewDef;
        let path_col = &view.columns()[0];
        assert!(path_col.compare.is_some());
    }

    #[test]
    fn status_column_is_sortable() {
        let view = WorktreesViewDef;
        let status_col = &view.columns()[2];
        assert!(status_col.compare.is_some());
    }

    #[test]
    fn filter_tokens_include_status_and_age() {
        let view = WorktreesViewDef;
        let tokens = view.filter_tokens();
        assert!(tokens.iter().any(|t| t.token == "status:merged"));
        assert!(tokens.iter().any(|t| t.token == "age:<7d"));
    }

    #[test]
    fn filter_tokens_no_pr_or_sync() {
        let view = WorktreesViewDef;
        let tokens = view.filter_tokens();
        assert!(!tokens.iter().any(|t| t.token.starts_with("pr:")));
        assert!(!tokens.iter().any(|t| t.token.starts_with("sync:")));
    }
}
