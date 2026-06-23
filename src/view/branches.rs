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
                min_width: 6,
                wide_width: None,
                hide_below_width: Some(80),
                compare: Some(|a, b| {
                    // Sort by presence: local-only (0) < gone (1) < tracked (2).
                    let key = |item: &BranchInfo| -> u8 {
                        match &item.tracking {
                            crate::types::TrackingStatus::Tracked { gone, .. } => {
                                if *gone {
                                    1
                                } else {
                                    2
                                }
                            }
                            crate::types::TrackingStatus::Local => 0,
                        }
                    };
                    key(a).cmp(&key(b))
                }),
            },
            super::column::ahead_behind_column(),
            super::column::pr_column(),
            super::column::age_column(),
            super::column::merge_status_column("Merge"),
        ]
    }

    pub fn filter_tokens(&self) -> Vec<FilterTokenDef> {
        let mut t = super::filter::status_tokens();
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
