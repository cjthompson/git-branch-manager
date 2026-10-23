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

pub fn can_cherry_pick_local(branch: &BranchInfo) -> Capability {
    if branch.is_base {
        Capability::no("base")
    } else if branch.is_current {
        Capability::no("current")
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
mod tests;
