use super::column::ColumnDef;
use super::filter::FilterTokenDef;
use crate::types::BranchInfo;

pub struct BranchesViewDef;

impl BranchesViewDef {
    pub fn columns(&self) -> Vec<ColumnDef<BranchInfo>> {
        vec![
            ColumnDef {
                key: "remote",
                name: "Up",
                show_header: false,
                min_width: 2,
                content_min_width: Some(|item| match &item.tracking {
                    crate::types::TrackingStatus::Tracked { gone: true, .. } => 4,
                    _ => 2,
                }),
                wide_width: None,
                hide_below_width: None,
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
            ColumnDef {
                key: "name",
                name: "Branch",
                show_header: true,
                min_width: 15,
                content_min_width: None,
                wide_width: None,
                hide_below_width: None,
                compare: Some(|a, b| a.name.cmp(&b.name)),
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
    use crate::types::{MergeStatus, TrackingStatus};
    use chrono::Utc;
    use std::cmp::Ordering;

    fn branch(tracking: TrackingStatus) -> BranchInfo {
        BranchInfo {
            name: "feature/test".into(),
            is_current: false,
            is_base: false,
            tracking,
            ahead: None,
            behind: None,
            last_commit_date: Utc::now(),
            merge_status: MergeStatus::Unmerged,
            base_branch: "main".into(),
            merge_base_commit: None,
            pr: None,
            squash_confidence: None,
        }
    }

    #[test]
    fn has_correct_column_count() {
        let view = BranchesViewDef;
        assert_eq!(view.columns().len(), 6);
    }

    #[test]
    fn name_column_is_sortable() {
        let view = BranchesViewDef;
        let name_col = &view.columns()[1];
        assert!(name_col.compare.is_some());
        assert_eq!(name_col.key, "name");
    }

    #[test]
    fn upstream_column_precedes_branch_without_a_header() {
        let view = BranchesViewDef;
        let columns = view.columns();
        let upstream_col = &columns[0];
        assert_eq!(upstream_col.key, "remote");
        assert_eq!(upstream_col.name, "Up");
        assert!(!upstream_col.show_header);
        assert!(upstream_col.compare.is_some());
        assert_eq!(upstream_col.min_width, 2);
        assert_eq!(
            upstream_col.content_min_width.unwrap()(&branch(TrackingStatus::Local)),
            2
        );
        assert_eq!(columns[1].key, "name");
        assert_eq!(columns[1].name, "Branch");
    }

    #[test]
    fn upstream_column_widens_only_for_gone_rows() {
        let columns = BranchesViewDef.columns();
        let upstream_col = &columns[0];
        let local = branch(TrackingStatus::Local);
        let present = branch(TrackingStatus::Tracked {
            remote_ref: "origin/feature/test".into(),
            gone: false,
        });
        let gone = branch(TrackingStatus::Tracked {
            remote_ref: "origin/feature/test".into(),
            gone: true,
        });
        let content_min_width = upstream_col.content_min_width.unwrap();
        assert_eq!(content_min_width(&local), 2);
        assert_eq!(content_min_width(&present), 2);
        assert_eq!(content_min_width(&gone), 4);
        assert_eq!(upstream_col.min_width_for_items(&[local, present]), 2);
        assert_eq!(upstream_col.min_width_for_items(&[gone]), 4);
    }

    #[test]
    fn upstream_sorting_keeps_the_remote_ref_ordering() {
        let columns = BranchesViewDef.columns();
        let upstream_col = &columns[0];
        let compare = upstream_col.compare.unwrap();
        let tracked = branch(TrackingStatus::Tracked {
            remote_ref: "origin/feature/test".into(),
            gone: false,
        });
        let local = branch(TrackingStatus::Local);
        assert_eq!(compare(&local, &tracked), Ordering::Less);
    }

    #[test]
    fn filter_tokens_include_status() {
        let view = BranchesViewDef;
        let tokens = view.filter_tokens();
        assert!(tokens.iter().any(|t| t.token == "merge:merged"));
    }

    #[test]
    fn filter_tokens_include_age() {
        let view = BranchesViewDef;
        let tokens = view.filter_tokens();
        assert!(tokens.iter().any(|t| t.token == "age:<7d"));
        assert!(tokens.iter().any(|t| t.token == "age:>90d"));
    }
}
