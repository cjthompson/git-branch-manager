use super::info::WorkingTreeStatus;
use super::{MergeStatus, SquashConfidence};
use chrono::{DateTime, Utc};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchAction {
    /// Navigate to the selected ref or commit in the Graph view. This is a
    /// UI-only action and must never be enqueued as a Git operation.
    JumpToGraph,
    CheckPrimaryBranchCode,
    // Local branch actions
    DeleteLocal,
    /// Skip the merge-status gate; bypass `BranchInfo.merge_status` and
    /// call `git2::Branch::delete` unconditionally. Reached from the
    /// Confirm overlay's `!` extra key and from the menu's force entry.
    DeleteLocalForce,
    DeleteLocalAndRemote,
    Checkout,
    Fetch,
    FetchPrune,
    FastForward,
    Merge,
    SquashMerge,
    Rebase,
    Worktree,
    Push,
    ForcePush,
    Pull,
    /// Apply the branch tip (`refs/heads/<name>`) onto the current HEAD.
    CherryPickLocal,
    // Tag actions
    DeleteTag,
    DeleteTagAndRemote,
    PushTag,
    // Remote branch actions
    DeleteRemoteBranch,
    DeleteRemoteAndLocal,
    CheckoutRemote,
    FetchRemote,
    PullRemote,
    MergeRemoteIntoCurrent,
    CherryPickRemote,
    /// Cherry-pick this exact commit object onto the current checkout.
    CherryPickCommit,
    ViewRemotePR,
    // Worktree actions
    WorktreeRemove,
    WorktreeForceRemove,
    WorktreeRemoveAndDeleteBranch,
    WorktreeRemoveAndDeleteBranchRemote,
    /// Cascade by **branch name** (vs the path-based variants above):
    /// remove the linked worktree holding this branch, then delete the branch
    /// itself. The worker resolves the path via `git worktree list`.
    /// Reached from the Branches view's `r` recovery key in the
    /// Confirm/Results overlays.
    DeleteBranchAndRemoveWorktree,
    /// Same cascade, but force-removes the worktree first (handles dirty
    /// working trees) before deleting the branch.
    DeleteBranchAndRemoveWorktreeForce,
    /// Cascade plus remote push-delete of the now-gone branch.
    DeleteBranchAndRemoveWorktreeRemote,
}

impl BranchAction {
    pub fn label(&self) -> &'static str {
        match self {
            Self::JumpToGraph => "Jump to Graph",
            Self::CheckPrimaryBranchCode => "Check code in primary branch",
            Self::DeleteLocal => "Delete local",
            Self::DeleteLocalForce => "Force-delete local",
            Self::DeleteLocalAndRemote => "Delete local + remote",
            Self::Checkout => "Checkout",
            Self::Fetch => "Fetch all",
            Self::FetchPrune => "Fetch + prune",
            Self::FastForward => "Fast-forward",
            Self::Merge => "Merge into base",
            Self::SquashMerge => "Squash merge into base",
            Self::Rebase => "Rebase onto base",
            Self::Worktree => "Create worktree",
            Self::Push => "Push",
            Self::ForcePush => "Force push",
            Self::Pull => "Pull",
            Self::CherryPickLocal => "Cherry-pick onto current",
            Self::DeleteTag => "Delete tag",
            Self::DeleteTagAndRemote => "Delete tag (local + remote)",
            Self::PushTag => "Push tag",
            Self::DeleteRemoteBranch => "Delete remote branch",
            Self::DeleteRemoteAndLocal => "Delete remote + local",
            Self::CheckoutRemote => "Checkout remote",
            Self::FetchRemote => "Fetch remote",
            Self::PullRemote => "Pull remote",
            Self::MergeRemoteIntoCurrent => "Merge into current",
            Self::CherryPickRemote => "Cherry-pick latest",
            Self::CherryPickCommit => "Cherry-pick commit onto current",
            Self::ViewRemotePR => "Open PR in browser",
            Self::WorktreeRemove => "Remove worktree",
            Self::WorktreeForceRemove => "Force remove worktree",
            Self::WorktreeRemoveAndDeleteBranch => "Remove worktree + branch",
            Self::WorktreeRemoveAndDeleteBranchRemote => {
                "Remove worktree + branch (local + remote)"
            }
            Self::DeleteBranchAndRemoveWorktree => "Remove worktree + delete branch",
            Self::DeleteBranchAndRemoveWorktreeForce => "Force-remove worktree + delete branch",
            Self::DeleteBranchAndRemoveWorktreeRemote => {
                "Remove worktree + delete branch (local + remote)"
            }
        }
    }
}

// --- Operation/Channel Types ---

/// Why an operation failed, so the UI can offer targeted recovery instead of
/// only showing the raw git error. Populated by classification helpers added
/// in later tasks (e.g. a `classify_delete_error` in `git::operations`);
/// until then, failing operations report `Other` or leave `failure: None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailureCause {
    /// The branch did not exist when the operation looked it up.
    BranchNotFound,
    /// The branch has commits not yet merged into the base branch; a plain
    /// delete was refused.
    NotMerged,
    /// The branch is checked out in a worktree, so it cannot be deleted from
    /// this one. Primary worktrees are reported too, but cannot be removed as
    /// part of recovery.
    CheckedOutInWorktree {
        worktree_path: PathBuf,
        is_main: bool,
    },
    /// Catch-all for an error that hasn't been classified into a specific
    /// cause yet. Carries the raw message so nothing is lost.
    Other { raw_message: String },
}

#[derive(Debug, Clone)]
pub struct OperationResult {
    pub branch_name: String,
    pub action: BranchAction,
    pub success: bool,
    pub message: String,
    /// Typed classification of why the operation failed. `None` on success,
    /// or when the failure has not yet been classified (fall back to
    /// `message` in that case).
    pub failure: Option<FailureCause>,
}

impl OperationResult {
    /// Build a successful result. `name` is the branch/tag/remote/worktree
    /// identifier the operation targeted.
    pub fn success(
        name: impl Into<String>,
        action: BranchAction,
        message: impl Into<String>,
    ) -> Self {
        Self {
            branch_name: name.into(),
            action,
            success: true,
            message: message.into(),
            failure: None,
        }
    }

    /// Build a failed result with a typed cause. `fallback_message` populates
    /// `message` for the existing `ui/results.rs` renderer, which only reads
    /// `message` today.
    pub fn failure(
        name: impl Into<String>,
        action: BranchAction,
        cause: FailureCause,
        fallback_message: impl Into<String>,
    ) -> Self {
        Self {
            branch_name: name.into(),
            action,
            success: false,
            message: fallback_message.into(),
            failure: Some(cause),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProgressUpdate {
    pub completed: usize,
    pub total: usize,
    pub current_item: String,
}

#[derive(Debug, Clone)]
pub struct SquashResult {
    pub branch_name: String,
    pub status: MergeStatus,
    pub confidence: Option<SquashConfidence>,
}

#[derive(Debug, Clone)]
pub struct CherryResult {
    pub branch_name: String,
    pub status: MergeStatus,
}

#[derive(Debug, Clone)]
pub struct RemoteEnrichResult {
    pub full_ref: String,
    pub merge_status: MergeStatus,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
    pub disjoint: bool,
}

#[derive(Debug, Clone)]
pub struct WorktreeEnrichResult {
    pub path: PathBuf,
    pub wt_status: WorkingTreeStatus,
    pub age_date: DateTime<Utc>,
}
