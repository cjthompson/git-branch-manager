use super::column::ColumnDef;
use super::filter::FilterTokenDef;
use crate::types::BranchInfo;

pub struct BranchesViewDef;

impl BranchesViewDef {
    pub fn columns(&self) -> Vec<ColumnDef<BranchInfo>> {
        vec![
            ColumnDef {
                name: "Branch",
                min_width: 15,
                wide_width: None,
                hide_below_width: None,
                compare: Some(|a, b| a.name.cmp(&b.name)),
            },
            ColumnDef {
                name: "Remote",
                min_width: 18,
                wide_width: Some(28),
                hide_below_width: Some(80),
                compare: Some(|a, b| {
                    let key = |item: &BranchInfo| -> String {
                        match &item.tracking {
                            crate::types::TrackingStatus::Tracked { remote_ref, gone } => {
                                if *gone {
                                    "gone".to_string()
                                } else {
                                    remote_ref.clone()
                                }
                            }
                            crate::types::TrackingStatus::Local => "local".to_string(),
                        }
                    };
                    key(a).cmp(&key(b))
                }),
            },
            super::column::ahead_behind_column(),
            super::column::pr_column(),
            super::column::age_column(),
            super::column::merge_status_column("Status"),
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
        let view = BranchesViewDef;
        assert_eq!(view.columns().len(), 6);
    }

    #[test]
    fn name_column_is_sortable() {
        let view = BranchesViewDef;
        let name_col = &view.columns()[0];
        assert!(name_col.compare.is_some());
    }

    #[test]
    fn remote_column_is_sortable() {
        let view = BranchesViewDef;
        let remote_col = &view.columns()[1];
        assert!(remote_col.compare.is_some());
    }

    #[test]
    fn filter_tokens_include_status() {
        let view = BranchesViewDef;
        let tokens = view.filter_tokens();
        assert!(tokens.iter().any(|t| t.token == "status:merged"));
    }

    #[test]
    fn filter_tokens_include_age() {
        let view = BranchesViewDef;
        let tokens = view.filter_tokens();
        assert!(tokens.iter().any(|t| t.token == "age:<7d"));
        assert!(tokens.iter().any(|t| t.token == "age:>90d"));
    }
}
