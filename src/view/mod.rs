pub mod branches;
pub mod column;
pub mod filter;
pub mod list_state;
pub mod remotes;
pub mod tags;
pub mod worktrees;

use crate::types::*;
use chrono::{DateTime, Utc};

/// Identifies which primary view is active
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewId {
    Branches,
    Remotes,
    Tags,
    Worktrees,
}

impl ViewId {
    /// Fixed tab cycle order: Branches -> Remotes -> Tags -> Worktrees
    pub fn next(self) -> Self {
        match self {
            Self::Branches => Self::Remotes,
            Self::Remotes => Self::Tags,
            Self::Tags => Self::Worktrees,
            Self::Worktrees => Self::Branches,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Branches => Self::Worktrees,
            Self::Remotes => Self::Branches,
            Self::Tags => Self::Remotes,
            Self::Worktrees => Self::Tags,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Branches => "Branches",
            Self::Remotes => "Remote",
            Self::Tags => "Tags",
            Self::Worktrees => "Worktrees",
        }
    }

    /// All 4 views in tab order
    pub const ALL: [ViewId; 4] = [Self::Branches, Self::Remotes, Self::Tags, Self::Worktrees];
}

/// Trait implemented by every list item type (BranchInfo, RemoteBranchInfo, etc.)
/// Provides the common interface the generic framework needs.
pub trait ViewItem: Clone {
    fn display_name(&self) -> &str;
    fn is_pinned(&self) -> bool;
    /// Whether this is the base/default branch (always sorts first among pinned items)
    fn is_base(&self) -> bool {
        false
    }
    fn merge_status(&self) -> Option<&MergeStatus> {
        None
    }
    fn last_commit_date(&self) -> &DateTime<Utc>;
    fn ahead(&self) -> Option<u32> {
        None
    }
    fn behind(&self) -> Option<u32> {
        None
    }
    fn pr_info(&self) -> Option<&PrInfo> {
        None
    }
    fn is_current(&self) -> bool {
        false
    }
}

// --- ViewItem implementations for all 4 data types ---

impl ViewItem for BranchInfo {
    fn display_name(&self) -> &str {
        &self.name
    }
    fn is_pinned(&self) -> bool {
        self.is_base || self.is_current
    }
    fn is_base(&self) -> bool {
        self.is_base
    }
    fn merge_status(&self) -> Option<&MergeStatus> {
        Some(&self.merge_status)
    }
    fn last_commit_date(&self) -> &DateTime<Utc> {
        &self.last_commit_date
    }
    fn ahead(&self) -> Option<u32> {
        self.ahead
    }
    fn behind(&self) -> Option<u32> {
        self.behind
    }
    fn is_current(&self) -> bool {
        self.is_current
    }
    fn pr_info(&self) -> Option<&PrInfo> {
        self.pr.as_ref()
    }
}

impl ViewItem for RemoteBranchInfo {
    fn display_name(&self) -> &str {
        &self.full_ref
    }
    fn is_pinned(&self) -> bool {
        self.is_base
    }
    fn is_base(&self) -> bool {
        self.is_base
    }
    fn merge_status(&self) -> Option<&MergeStatus> {
        Some(&self.merge_status)
    }
    fn last_commit_date(&self) -> &DateTime<Utc> {
        &self.last_commit_date
    }
    fn ahead(&self) -> Option<u32> {
        self.ahead
    }
    fn behind(&self) -> Option<u32> {
        self.behind
    }
    fn pr_info(&self) -> Option<&PrInfo> {
        self.pr.as_ref()
    }
}

impl ViewItem for TagInfo {
    fn display_name(&self) -> &str {
        &self.name
    }
    fn is_pinned(&self) -> bool {
        false
    }
    fn last_commit_date(&self) -> &DateTime<Utc> {
        &self.date
    }
}

impl ViewItem for WorktreeInfo {
    fn display_name(&self) -> &str {
        self.branch.as_deref().unwrap_or("[detached]")
    }
    fn is_pinned(&self) -> bool {
        self.is_main
    }
    fn merge_status(&self) -> Option<&MergeStatus> {
        Some(&self.merge_status)
    }
    fn last_commit_date(&self) -> &DateTime<Utc> {
        &self.age_date
    }
    fn ahead(&self) -> Option<u32> {
        self.ahead
    }
    fn behind(&self) -> Option<u32> {
        self.behind
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::path::PathBuf;

    #[test]
    fn view_id_next_cycle() {
        assert_eq!(ViewId::Branches.next(), ViewId::Remotes);
        assert_eq!(ViewId::Remotes.next(), ViewId::Tags);
        assert_eq!(ViewId::Tags.next(), ViewId::Worktrees);
        assert_eq!(ViewId::Worktrees.next(), ViewId::Branches);
    }

    #[test]
    fn view_id_prev_cycle() {
        assert_eq!(ViewId::Branches.prev(), ViewId::Worktrees);
        assert_eq!(ViewId::Worktrees.prev(), ViewId::Tags);
        assert_eq!(ViewId::Tags.prev(), ViewId::Remotes);
        assert_eq!(ViewId::Remotes.prev(), ViewId::Branches);
    }

    #[test]
    fn view_id_labels() {
        assert_eq!(ViewId::Branches.label(), "Branches");
        assert_eq!(ViewId::Remotes.label(), "Remote");
        assert_eq!(ViewId::Tags.label(), "Tags");
        assert_eq!(ViewId::Worktrees.label(), "Worktrees");
    }

    #[test]
    fn view_id_all_order() {
        assert_eq!(
            ViewId::ALL,
            [ViewId::Branches, ViewId::Remotes, ViewId::Tags, ViewId::Worktrees]
        );
    }

    #[test]
    fn branch_info_view_item() {
        let b = BranchInfo {
            name: "feature/x".into(),
            is_current: true,
            is_base: false,
            tracking: TrackingStatus::Local,
            ahead: Some(3),
            behind: Some(1),
            last_commit_date: Utc::now(),
            merge_status: MergeStatus::Unmerged,
            base_branch: "main".into(),
            merge_base_commit: None,
            pr: None,
        };
        assert_eq!(b.display_name(), "feature/x");
        assert!(b.is_pinned()); // is_current = true
        assert_eq!(*b.merge_status().unwrap(), MergeStatus::Unmerged);
        assert_eq!(b.ahead(), Some(3));
        assert_eq!(b.behind(), Some(1));
        assert!(b.is_current());
    }

    #[test]
    fn remote_branch_info_view_item() {
        let r = RemoteBranchInfo {
            full_ref: "origin/main".into(),
            remote: "origin".into(),
            short_name: "main".into(),
            has_local: true,
            is_base: true,
            last_commit_date: Utc::now(),
            merge_status: MergeStatus::Merged,
            ahead: None,
            behind: Some(2),
            pr: None,
        };
        assert_eq!(r.display_name(), "origin/main");
        assert!(r.is_pinned());
        assert_eq!(r.behind(), Some(2));
        assert!(!r.is_current()); // default
    }

    #[test]
    fn tag_info_view_item() {
        let t = TagInfo {
            name: "v1.0".into(),
            commit_hash: "abc".into(),
            date: Utc::now(),
            message: None,
            is_annotated: false,
        };
        assert_eq!(t.display_name(), "v1.0");
        assert!(!t.is_pinned());
        assert!(t.merge_status().is_none());
        assert!(t.ahead().is_none());
    }

    #[test]
    fn worktree_info_view_item() {
        let w = WorktreeInfo {
            path: PathBuf::from("/repo"),
            branch: Some("feature/y".into()),
            is_main: false,
            commit_hash: "def".into(),
            wt_status: WorkingTreeStatus::clean(),
            age_date: Utc::now(),
            merge_status: MergeStatus::SquashMerged,
            ahead: Some(1),
            behind: None,
            pr: None,
        };
        assert_eq!(w.display_name(), "feature/y");
        assert!(!w.is_pinned());
        assert_eq!(*w.merge_status().unwrap(), MergeStatus::SquashMerged);
    }

    #[test]
    fn worktree_info_detached_display_name() {
        let w = WorktreeInfo {
            path: PathBuf::from("/repo"),
            branch: None,
            is_main: true,
            commit_hash: "def".into(),
            wt_status: WorkingTreeStatus::clean(),
            age_date: Utc::now(),
            merge_status: MergeStatus::Unmerged,
            ahead: None,
            behind: None,
            pr: None,
        };
        assert_eq!(w.display_name(), "[detached]");
        assert!(w.is_pinned());
    }
}
