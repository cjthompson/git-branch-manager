use super::column::ColumnDef;
use super::filter::FilterTokenDef;
use crate::types::RemoteBranchInfo;

pub struct RemotesViewDef;

impl RemotesViewDef {
    pub fn columns(&self) -> Vec<ColumnDef<RemoteBranchInfo>> {
        vec![
            ColumnDef {
                key: "local",
                name: "",
                show_header: false,
                min_width: 2,
                content_min_width: None,
                wide_width: None,
                hide_below_width: None,
                compare: Some(|a, b| a.has_local.cmp(&b.has_local)),
            },
            ColumnDef {
                key: "name",
                name: "Name",
                show_header: true,
                min_width: 15,
                content_min_width: None,
                wide_width: None,
                hide_below_width: None,
                compare: Some(|a, b| a.short_name.cmp(&b.short_name)),
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
    fn local_column_is_first_and_sortable() {
        let view = RemotesViewDef;
        let name_col = &view.columns()[0];
        assert_eq!(name_col.key, "local");
        assert_eq!(name_col.name, "");
        assert_eq!(name_col.min_width, 2);
        assert!(name_col.hide_below_width.is_none());
        assert!(name_col.compare.is_some());
    }

    #[test]
    fn name_column_is_second_and_sortable() {
        let view = RemotesViewDef;
        let name_col = &view.columns()[1];
        assert_eq!(name_col.key, "name");
        assert!(name_col.compare.is_some());
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
