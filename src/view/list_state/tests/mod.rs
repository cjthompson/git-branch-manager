mod nav_select;
mod sort;
mod state;

use super::*;
use crate::types::*;
use crate::view::column::ColumnDef;
use chrono::Utc;

fn sample_branches() -> Vec<BranchInfo> {
    vec![
        BranchInfo {
            name: "main".into(),
            is_current: false,
            is_base: true,
            tracking: TrackingStatus::Local,
            ahead: None,
            behind: None,
            last_commit_date: Utc::now(),
            merge_status: MergeStatus::Unmerged,
            base_branch: "main".into(),
            merge_base_commit: None,
            pr: None,
            squash_confidence: None,
        },
        BranchInfo {
            name: "feature/a".into(),
            is_current: false,
            is_base: false,
            tracking: TrackingStatus::Local,
            ahead: None,
            behind: None,
            last_commit_date: Utc::now() - chrono::Duration::days(1),
            merge_status: MergeStatus::Unmerged,
            base_branch: "main".into(),
            merge_base_commit: None,
            pr: None,
            squash_confidence: None,
        },
        BranchInfo {
            name: "feature/b".into(),
            is_current: false,
            is_base: false,
            tracking: TrackingStatus::Local,
            ahead: None,
            behind: None,
            last_commit_date: Utc::now() - chrono::Duration::days(2),
            merge_status: MergeStatus::Merged,
            base_branch: "main".into(),
            merge_base_commit: None,
            pr: None,
            squash_confidence: None,
        },
    ]
}

fn name_column() -> ColumnDef<BranchInfo> {
    ColumnDef::<BranchInfo> {
        key: "name",
        name: "Name",
        show_header: true,
        min_width: 10,
        content_min_width: None,
        wide_width: None,
        hide_below_width: None,
        compare: Some(|a, b| a.name.cmp(&b.name)),
    }
}
