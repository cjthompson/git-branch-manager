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
            ColumnDef {
                name: "A/B",
                min_width: 8,
                wide_width: None,
                hide_below_width: Some(80),
                compare: Some(|a, b| {
                    a.ahead
                        .unwrap_or(0)
                        .cmp(&b.ahead.unwrap_or(0))
                        .then(a.behind.unwrap_or(0).cmp(&b.behind.unwrap_or(0)))
                }),
            },
            ColumnDef {
                name: "PR",
                min_width: 5,
                wide_width: None,
                hide_below_width: Some(80),
                compare: Some(|a, b| {
                    let pr_key = |item: &RemoteBranchInfo| item.pr.as_ref().map(|p| p.number);
                    match (pr_key(a), pr_key(b)) {
                        (Some(x), Some(y)) => x.cmp(&y),
                        (Some(_), None) => std::cmp::Ordering::Less,
                        (None, Some(_)) => std::cmp::Ordering::Greater,
                        (None, None) => std::cmp::Ordering::Equal,
                    }
                }),
            },
            ColumnDef {
                name: "Age",
                min_width: 5,
                wide_width: Some(12),
                hide_below_width: Some(60),
                compare: Some(|a, b| a.last_commit_date.cmp(&b.last_commit_date)),
            },
            ColumnDef {
                name: "Status",
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
                key: 'p',
                label: "Has PR",
                token: "pr:yes",
            },
            FilterTokenDef {
                key: 'P',
                label: "No PR",
                token: "pr:no",
            },
            FilterTokenDef {
                key: 'a',
                label: "Ahead",
                token: "sync:ahead",
            },
            FilterTokenDef {
                key: 'b',
                label: "Behind",
                token: "sync:behind",
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
        assert!(tokens.iter().any(|t| t.token == "status:merged"));
    }

    #[test]
    fn filter_tokens_include_sync() {
        let view = RemotesViewDef;
        let tokens = view.filter_tokens();
        assert!(tokens.iter().any(|t| t.token == "sync:ahead"));
        assert!(tokens.iter().any(|t| t.token == "sync:behind"));
    }
}
