use super::column::ColumnDef;
use super::filter::FilterTokenDef;
use crate::types::RemoteBranchInfo;

pub struct RemotesViewDef;

impl RemotesViewDef {
    pub fn columns(&self) -> Vec<ColumnDef<RemoteBranchInfo>> {
        vec![
            ColumnDef {
                name: "Name",
                min_width: 15,
                wide_width: None,
                hide_below_width: None,
                compare: Some(|a, b| a.short_name.cmp(&b.short_name)),
            },
            ColumnDef {
                name: "Local",
                min_width: 6,
                wide_width: None,
                hide_below_width: Some(80),
                compare: Some(|a, b| a.has_local.cmp(&b.has_local)),
            },
            super::column::ahead_behind_column(),
            super::column::pr_column(),
            super::column::age_column(),
            super::column::merge_status_column("Merge"),
        ]
    }

    pub fn filter_tokens(&self) -> Vec<FilterTokenDef> {
        let mut t = super::filter::merge_tokens();
        t.extend(super::filter::pr_tokens());
        t.extend(super::filter::sync_tokens());
        t.extend(super::filter::age_tokens());
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_correct_column_count() {
        let view = RemotesViewDef;
        assert_eq!(view.columns().len(), 6);
    }

    #[test]
    fn name_column_is_sortable() {
        let view = RemotesViewDef;
        let name_col = &view.columns()[0];
        assert!(name_col.compare.is_some());
    }

    #[test]
    fn local_column_is_sortable() {
        let view = RemotesViewDef;
        let local_col = &view.columns()[1];
        assert!(local_col.compare.is_some());
    }

    #[test]
    fn filter_tokens_include_status() {
        let view = RemotesViewDef;
        let tokens = view.filter_tokens();
        assert!(tokens.iter().any(|t| t.token == "merge:merged"));
    }

    #[test]
    fn filter_tokens_include_sync() {
        let view = RemotesViewDef;
        let tokens = view.filter_tokens();
        assert!(tokens.iter().any(|t| t.token == "sync:ahead"));
        assert!(tokens.iter().any(|t| t.token == "sync:behind"));
    }
}
