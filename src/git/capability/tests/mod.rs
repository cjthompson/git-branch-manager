mod checkout_delete;
mod cherry_pick_local;
mod integrate;
mod remote_sync;

use super::*;
use crate::types::{MergeStatus, TrackingStatus};
use chrono::Utc;

/// `BranchInfo` does not derive `Default` (confirmed in `src/types.rs`,
/// only `#[derive(Debug, Clone)]`), so build a minimal fixture inline
/// rather than adding `Default` to the production type.
fn test_branch(
    name: &str,
    is_current: bool,
    is_base: bool,
    tracking: TrackingStatus,
    ahead: Option<u32>,
    behind: Option<u32>,
) -> BranchInfo {
    BranchInfo {
        name: name.to_string(),
        is_current,
        is_base,
        tracking,
        ahead,
        behind,
        last_commit_date: Utc::now(),
        merge_status: MergeStatus::Unmerged,
        base_branch: "main".to_string(),
        merge_base_commit: None,
        pr: None,
        squash_confidence: None,
    }
}

fn linked_worktree(is_clean: bool) -> WorktreePresence<'static> {
    WorktreePresence {
        path: Path::new("/tmp/wt"),
        is_main: false,
        is_clean,
    }
}
