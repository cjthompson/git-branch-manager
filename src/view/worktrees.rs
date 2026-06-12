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
                min_width: 20,
                wide_width: Some(32),
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
            super::column::age_column(),
            super::column::merge_status_column("Merge"),
        ]
    }

    pub fn filter_tokens(&self) -> Vec<FilterTokenDef> {
        let mut t = super::filter::status_tokens();
        t.extend(super::filter::age_tokens());
        t
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
