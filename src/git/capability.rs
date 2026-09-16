//! Pure, unit-testable capability evaluators for branch-menu availability.
//!
//! Each `can_*` function answers "is this `BranchAction` safe to offer for
//! this branch right now?" using only already-loaded facts (`BranchInfo`, an
//! optional `WorktreePresence` for the relevant worktree, and small bool/
//! tracking flags the caller already has in hand). No I/O, no `Command`, no
//! `git2::Repository` — evaluators must stay pure so they're unit-testable
//! without a repo fixture. `src/app.rs`'s `build_branch_menu_for` (and
//! friends) call these instead of inlining `is_current`/`is_base` checks.
//!
//! Step 2 fills in real logic per the design in
//! `docs/superpowers/plans/2026-09-15-graph-actions-menu-availability.md`.

use std::path::Path;

use crate::types::{BranchInfo, TrackingStatus};

/// Snapshot of the facts an evaluator needs about a branch's own worktree
/// state. Callers pass `None` for the `Option<WorktreePresence>` parameter
/// when the branch is "not checked out in any worktree the app has loaded".
pub struct WorktreePresence<'a> {
    pub path: &'a Path,
    pub is_main: bool,
    pub is_clean: bool,
}

/// Availability plus, when unavailable, a short stable reason matching the
/// existing `MenuItem.reason` convention ("current", "base", "no remote", …).
pub struct Capability {
    pub available: bool,
    pub reason: Option<&'static str>,
}

impl Capability {
    pub fn yes() -> Self {
        Self {
            available: true,
            reason: None,
        }
    }

    pub fn no(reason: &'static str) -> Self {
        Self {
            available: false,
            reason: Some(reason),
        }
    }
}

pub fn can_checkout_local(branch: &BranchInfo, worktree: Option<WorktreePresence>) -> Capability {
    if branch.is_base {
        Capability::no("base")
    } else if branch.is_current {
        Capability::no("current")
    } else if worktree.is_some() {
        Capability::no("checked out in worktree")
    } else {
        Capability::yes()
    }
}

pub fn can_create_worktree(branch: &BranchInfo, worktree: Option<WorktreePresence>) -> Capability {
    if branch.is_base {
        Capability::no("base")
    } else if branch.is_current {
        Capability::no("current")
    } else if worktree.is_some() {
        Capability::no("checked out in worktree")
    } else {
        Capability::yes()
    }
}

pub fn can_delete_local(branch: &BranchInfo, worktree: Option<WorktreePresence>) -> Capability {
    if branch.is_base {
        Capability::no("base")
    } else if branch.is_current {
        Capability::no("current")
    } else if worktree.is_some() {
        Capability::no("checked out in worktree")
    } else {
        Capability::yes()
    }
}

pub fn can_delete_remote(
    branch: &BranchInfo,
    worktree: Option<WorktreePresence>,
    has_remote: bool,
) -> Capability {
    if branch.is_base {
        Capability::no("base")
    } else if branch.is_current {
        Capability::no("current")
    } else if worktree.is_some() {
        Capability::no("checked out in worktree")
    } else if !has_remote {
        Capability::no("no remote")
    } else {
        Capability::yes()
    }
}

pub fn can_force_delete_local(
    branch: &BranchInfo,
    worktree: Option<WorktreePresence>,
) -> Capability {
    if branch.is_base {
        Capability::no("base")
    } else if branch.is_current {
        Capability::no("current")
    } else if worktree.is_some() {
        Capability::no("checked out in worktree")
    } else {
        Capability::yes()
    }
}

pub fn can_delete_local_and_remote(
    branch: &BranchInfo,
    worktree: Option<WorktreePresence>,
    has_remote: bool,
) -> Capability {
    if branch.is_base {
        Capability::no("base")
    } else if branch.is_current {
        Capability::no("current")
    } else if worktree.is_some() {
        Capability::no("checked out in worktree")
    } else if !has_remote {
        Capability::no("no remote")
    } else {
        Capability::yes()
    }
}

pub fn can_push(branch: &BranchInfo, has_configured_remote: bool, _is_clean: bool) -> Capability {
    if !has_configured_remote {
        return Capability::no("no remote");
    }
    // For a tracked branch, only enable Push when there are new commits to push.
    // Untracked local branches with a configured remote are always pushable
    // (the dispatch uses `git push --set-upstream` to create the tracking ref).
    if matches!(branch.tracking, TrackingStatus::Tracked { .. })
        && branch.ahead.is_none_or(|n| n == 0)
    {
        return Capability::no("not ahead");
    }
    Capability::yes()
}

pub fn can_pull(
    branch: &BranchInfo,
    worktree: Option<WorktreePresence>,
    has_remote: bool,
) -> Capability {
    if !has_remote {
        return Capability::no("no remote");
    }
    if branch.is_current {
        if branch.behind.is_some_and(|n| n > 0) {
            Capability::yes()
        } else {
            Capability::no("not behind")
        }
    } else {
        // Non-current branch: must be behind to pull, and must not be checked out
        // in any worktree (the underlying `git fetch <remote> <src>:<dst>` ref
        // git refuses to update a ref that's checked out anywhere).
        if branch.behind.is_none_or(|n| n == 0) {
            Capability::no("not behind")
        } else if worktree.is_some() {
            Capability::no("checked out in worktree")
        } else {
            Capability::yes()
        }
    }
}

pub fn can_fast_forward(
    branch: &BranchInfo,
    worktree: Option<WorktreePresence>,
    has_remote: bool,
) -> Capability {
    let ahead_zero = branch.ahead.is_some_and(|n| n == 0);
    let behind_positive = branch.behind.is_some_and(|n| n > 0);

    if !has_remote {
        Capability::no("no remote")
    } else if !behind_positive {
        Capability::no("in sync")
    } else if worktree.is_some() {
        Capability::no("checked out in worktree")
    } else if !ahead_zero {
        Capability::no("ahead")
    } else {
        Capability::yes()
    }
}

pub fn can_force_push(branch: &BranchInfo, has_remote: bool) -> Capability {
    if !has_remote {
        Capability::no("no remote")
    } else if branch.ahead.is_some_and(|n| n > 0) {
        Capability::yes()
    } else {
        Capability::no("behind or in sync")
    }
}

pub fn can_rebase(
    branch: &BranchInfo,
    worktree: Option<WorktreePresence>,
    _is_clean: bool,
) -> Capability {
    if branch.is_base {
        Capability::no("base")
    } else if worktree.is_some() && !branch.is_current {
        Capability::no("checked out in other worktree")
    } else {
        Capability::yes()
    }
}

pub fn can_merge_into_base(
    _branch: &BranchInfo,
    base_worktree: Option<WorktreePresence>,
) -> Capability {
    match base_worktree {
        None => Capability::yes(),
        Some(wt) if wt.is_clean => Capability::yes(),
        Some(_) => Capability::no("base dirty"),
    }
}

pub fn can_squash_into_base(
    _branch: &BranchInfo,
    base_worktree: Option<WorktreePresence>,
) -> Capability {
    match base_worktree {
        None => Capability::yes(),
        Some(wt) if wt.is_clean => Capability::yes(),
        Some(_) => Capability::no("base dirty"),
    }
}

#[cfg(test)]
mod tests {
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

    // -- can_checkout_local --

    #[test]
    fn checkout_happy_path_allows_other_branch() {
        let branch = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
        assert!(can_checkout_local(&branch, None).available);
    }

    #[test]
    fn checkout_blocks_current_branch() {
        let branch = test_branch("feature/x", true, false, TrackingStatus::Local, None, None);
        let cap = can_checkout_local(&branch, None);
        assert!(!cap.available);
        assert_eq!(cap.reason, Some("current"));
    }

    #[test]
    fn checkout_blocks_base_branch() {
        let branch = test_branch("main", false, true, TrackingStatus::Local, None, None);
        let cap = can_checkout_local(&branch, None);
        assert!(!cap.available);
        assert_eq!(cap.reason, Some("base"));
    }

    #[test]
    fn checkout_blocks_branch_in_linked_worktree() {
        let branch = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
        let cap = can_checkout_local(&branch, Some(linked_worktree(true)));
        assert!(!cap.available);
        assert_eq!(cap.reason, Some("checked out in worktree"));
    }

    // -- can_create_worktree --

    #[test]
    fn create_worktree_happy_path_allows_other_branch() {
        let branch = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
        assert!(can_create_worktree(&branch, None).available);
    }

    #[test]
    fn create_worktree_blocks_base_branch() {
        let branch = test_branch("main", false, true, TrackingStatus::Local, None, None);
        let cap = can_create_worktree(&branch, None);
        assert!(!cap.available);
        assert_eq!(cap.reason, Some("base"));
    }

    #[test]
    fn create_worktree_blocks_branch_in_any_worktree() {
        let current = test_branch("main-work", true, false, TrackingStatus::Local, None, None);
        let current_cap = can_create_worktree(&current, None);
        assert!(!current_cap.available);
        assert_eq!(current_cap.reason, Some("current"));

        let linked = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
        let linked_cap = can_create_worktree(&linked, Some(linked_worktree(true)));
        assert!(!linked_cap.available);
        assert_eq!(linked_cap.reason, Some("checked out in worktree"));
    }

    // -- can_delete_local --

    #[test]
    fn delete_local_happy_path_allows_other_branch() {
        let branch = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
        assert!(can_delete_local(&branch, None).available);
    }

    #[test]
    fn delete_local_blocks_current_branch() {
        let branch = test_branch("feature/x", true, false, TrackingStatus::Local, None, None);
        let cap = can_delete_local(&branch, None);
        assert!(!cap.available);
        assert_eq!(cap.reason, Some("current"));
    }

    #[test]
    fn delete_local_blocks_base_branch() {
        let branch = test_branch("main", false, true, TrackingStatus::Local, None, None);
        let cap = can_delete_local(&branch, None);
        assert!(!cap.available);
        assert_eq!(cap.reason, Some("base"));
    }

    #[test]
    fn delete_local_blocks_branch_in_linked_worktree() {
        let branch = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
        let cap = can_delete_local(&branch, Some(linked_worktree(true)));
        assert!(!cap.available);
        assert_eq!(cap.reason, Some("checked out in worktree"));
    }

    // -- can_delete_remote --

    #[test]
    fn delete_remote_happy_path_allows_when_remote_configured() {
        let branch = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
        assert!(can_delete_remote(&branch, None, true).available);
    }

    #[test]
    fn delete_remote_blocks_when_no_remote() {
        let branch = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
        let cap = can_delete_remote(&branch, None, false);
        assert!(!cap.available);
        assert_eq!(cap.reason, Some("no remote"));
    }

    #[test]
    fn delete_remote_blocks_branch_in_linked_worktree() {
        let branch = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
        let cap = can_delete_remote(&branch, Some(linked_worktree(true)), true);
        assert!(!cap.available);
        assert_eq!(cap.reason, Some("checked out in worktree"));
    }

    // -- can_force_delete_local --

    #[test]
    fn force_delete_local_happy_path_allows_other_branch() {
        let branch = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
        assert!(can_force_delete_local(&branch, None).available);
    }

    #[test]
    fn force_delete_local_blocks_base_branch() {
        let branch = test_branch("main", false, true, TrackingStatus::Local, None, None);
        let cap = can_force_delete_local(&branch, None);
        assert!(!cap.available);
        assert_eq!(cap.reason, Some("base"));
    }

    #[test]
    fn force_delete_local_blocks_branch_in_linked_worktree() {
        let branch = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
        let cap = can_force_delete_local(&branch, Some(linked_worktree(true)));
        assert!(!cap.available);
        assert_eq!(cap.reason, Some("checked out in worktree"));
    }

    // -- can_delete_local_and_remote --

    #[test]
    fn delete_local_and_remote_happy_path_allows_when_remote_configured() {
        let branch = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
        assert!(can_delete_local_and_remote(&branch, None, true).available);
    }

    #[test]
    fn delete_local_and_remote_blocks_when_no_remote() {
        let branch = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
        let cap = can_delete_local_and_remote(&branch, None, false);
        assert!(!cap.available);
        assert_eq!(cap.reason, Some("no remote"));
    }

    #[test]
    fn delete_local_and_remote_blocks_branch_in_linked_worktree() {
        let branch = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
        let cap = can_delete_local_and_remote(&branch, Some(linked_worktree(true)), true);
        assert!(!cap.available);
        assert_eq!(cap.reason, Some("checked out in worktree"));
    }

    // -- can_push --

    #[test]
    fn push_happy_path_when_remote_configured() {
        let branch = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
        assert!(can_push(&branch, true, true).available);
    }

    #[test]
    fn push_blocks_when_no_remote() {
        let branch = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
        let cap = can_push(&branch, false, true);
        assert!(!cap.available);
        assert_eq!(cap.reason, Some("no remote"));
    }

    #[test]
    fn push_ignores_worktree_state() {
        // can_push has no worktree parameter at all; a dirty, current branch
        // with a configured remote must still be pushable.
        let branch = test_branch("feature/x", true, false, TrackingStatus::Local, None, None);
        assert!(can_push(&branch, true, false).available);
    }

    #[test]
    fn push_blocks_when_tracked_and_not_ahead() {
        let tracked = TrackingStatus::Tracked {
            remote_ref: "origin/feature/x".to_string(),
            gone: false,
        };
        let branch = test_branch("feature/x", false, false, tracked, Some(0), None);
        let cap = can_push(&branch, true, true);
        assert!(!cap.available);
        assert_eq!(cap.reason, Some("not ahead"));
    }

    #[test]
    fn push_allows_when_tracked_and_ahead() {
        let tracked = TrackingStatus::Tracked {
            remote_ref: "origin/feature/x".to_string(),
            gone: false,
        };
        let branch = test_branch("feature/x", false, false, tracked, Some(3), None);
        assert!(can_push(&branch, true, true).available);
    }

    // -- can_pull --

    #[test]
    fn pull_allows_behind_current_branch() {
        let branch = test_branch(
            "feature/x",
            true,
            false,
            TrackingStatus::Local,
            None,
            Some(3),
        );
        assert!(can_pull(&branch, None, true).available);
    }

    #[test]
    fn pull_allows_non_current_branch_behind_and_not_checked_out() {
        let branch = test_branch(
            "feature/x",
            false,
            false,
            TrackingStatus::Local,
            None,
            Some(3),
        );
        assert!(can_pull(&branch, None, true).available);
    }

    #[test]
    fn pull_blocks_non_current_branch_when_not_behind() {
        let branch = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
        let cap = can_pull(&branch, None, true);
        assert!(!cap.available);
        assert_eq!(cap.reason, Some("not behind"));
    }

    #[test]
    fn pull_blocks_behind_branch_in_other_worktree() {
        let branch = test_branch(
            "feature/x",
            false,
            false,
            TrackingStatus::Local,
            None,
            Some(3),
        );
        let cap = can_pull(&branch, Some(linked_worktree(true)), true);
        assert!(!cap.available);
        assert_eq!(cap.reason, Some("checked out in worktree"));
    }

    #[test]
    fn pull_blocks_when_no_remote() {
        let branch = test_branch(
            "feature/x",
            true,
            false,
            TrackingStatus::Local,
            None,
            Some(3),
        );
        let cap = can_pull(&branch, None, false);
        assert!(!cap.available);
        assert_eq!(cap.reason, Some("no remote"));
    }

    // -- can_fast_forward --

    #[test]
    fn fast_forward_happy_path_when_behind_and_not_ahead() {
        let branch = test_branch(
            "feature/x",
            false,
            false,
            TrackingStatus::Local,
            Some(0),
            Some(2),
        );
        assert!(can_fast_forward(&branch, None, true).available);
    }

    #[test]
    fn fast_forward_blocks_when_in_sync() {
        let branch = test_branch(
            "feature/x",
            false,
            false,
            TrackingStatus::Local,
            Some(0),
            Some(0),
        );
        let cap = can_fast_forward(&branch, None, true);
        assert!(!cap.available);
        assert_eq!(cap.reason, Some("in sync"));
    }

    #[test]
    fn fast_forward_blocks_branch_in_other_worktree() {
        let branch = test_branch(
            "feature/x",
            false,
            false,
            TrackingStatus::Local,
            Some(0),
            Some(2),
        );
        let cap = can_fast_forward(&branch, Some(linked_worktree(true)), true);
        assert!(!cap.available);
        assert_eq!(cap.reason, Some("checked out in worktree"));
    }

    #[test]
    fn fast_forward_blocks_when_no_remote() {
        let branch = test_branch(
            "feature/x",
            false,
            false,
            TrackingStatus::Local,
            Some(0),
            Some(2),
        );
        let cap = can_fast_forward(&branch, None, false);
        assert!(!cap.available);
        assert_eq!(cap.reason, Some("no remote"));
    }

    // -- can_force_push --

    #[test]
    fn force_push_happy_path_when_ahead() {
        let branch = test_branch(
            "feature/x",
            false,
            false,
            TrackingStatus::Local,
            Some(2),
            Some(0),
        );
        assert!(can_force_push(&branch, true).available);
    }

    #[test]
    fn force_push_blocks_when_behind() {
        let branch = test_branch(
            "feature/x",
            false,
            false,
            TrackingStatus::Local,
            Some(0),
            Some(2),
        );
        let cap = can_force_push(&branch, true);
        assert!(!cap.available);
        assert_eq!(cap.reason, Some("behind or in sync"));
    }

    #[test]
    fn force_push_blocks_when_no_remote() {
        let branch = test_branch(
            "feature/x",
            false,
            false,
            TrackingStatus::Local,
            Some(2),
            Some(0),
        );
        let cap = can_force_push(&branch, false);
        assert!(!cap.available);
        assert_eq!(cap.reason, Some("no remote"));
    }

    // -- can_rebase --

    #[test]
    fn rebase_allows_current_branch_regardless_of_cleanliness() {
        let branch = test_branch("feature/x", true, false, TrackingStatus::Local, None, None);
        assert!(can_rebase(&branch, Some(linked_worktree(false)), false).available);
        assert!(can_rebase(&branch, None, false).available);
    }

    #[test]
    fn rebase_allows_branch_not_checked_out_anywhere() {
        let branch = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
        assert!(can_rebase(&branch, None, true).available);
    }

    #[test]
    fn rebase_blocks_base_branch() {
        let branch = test_branch("main", false, true, TrackingStatus::Local, None, None);
        let cap = can_rebase(&branch, None, true);
        assert!(!cap.available);
        assert_eq!(cap.reason, Some("base"));
    }

    #[test]
    fn rebase_blocks_branch_in_other_worktree() {
        let branch = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
        let cap = can_rebase(&branch, Some(linked_worktree(true)), true);
        assert!(!cap.available);
        assert_eq!(cap.reason, Some("checked out in other worktree"));
    }

    // -- can_merge_into_base --

    #[test]
    fn merge_into_base_allows_when_base_not_in_worktree() {
        let branch = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
        assert!(can_merge_into_base(&branch, None).available);
    }

    #[test]
    fn merge_into_base_allows_when_base_worktree_clean() {
        let branch = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
        assert!(can_merge_into_base(&branch, Some(linked_worktree(true))).available);
    }

    #[test]
    fn merge_into_base_blocks_when_base_worktree_dirty() {
        let branch = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
        let cap = can_merge_into_base(&branch, Some(linked_worktree(false)));
        assert!(!cap.available);
        assert_eq!(cap.reason, Some("base dirty"));
    }

    // -- can_squash_into_base --

    #[test]
    fn squash_into_base_allows_when_base_not_in_worktree() {
        let branch = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
        assert!(can_squash_into_base(&branch, None).available);
    }

    #[test]
    fn squash_into_base_blocks_when_base_worktree_dirty() {
        let branch = test_branch("feature/x", false, false, TrackingStatus::Local, None, None);
        let cap = can_squash_into_base(&branch, Some(linked_worktree(false)));
        assert!(!cap.available);
        assert_eq!(cap.reason, Some("base dirty"));
    }
}
