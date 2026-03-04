use chrono::{DateTime, Utc};

/// Merge status of a branch relative to the base branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeStatus {
    /// Branch is reachable from the base branch (regular merge)
    Merged,
    /// Branch content exists in base via squash merge
    SquashMerged,
    /// Branch has not been merged
    Unmerged,
}

/// Remote tracking relationship.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrackingStatus {
    /// Tracks a remote branch
    Tracked { remote_ref: String, gone: bool },
    /// No upstream configured
    Local,
}

/// All information about a single local branch.
#[derive(Debug, Clone)]
pub struct BranchInfo {
    pub name: String,
    pub is_current: bool,
    pub is_base: bool,
    pub tracking: TrackingStatus,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
    pub last_commit_date: DateTime<Utc>,
    pub merge_status: MergeStatus,
}

impl BranchInfo {
    /// Whether this branch is pinned to the top (base or current branch).
    pub fn is_pinned(&self) -> bool {
        self.is_base || self.is_current
    }

    /// Human-readable age string: "3 days ago", "2 months ago", etc.
    pub fn age_display(&self) -> String {
        let duration = Utc::now() - self.last_commit_date;
        let seconds = duration.num_seconds();

        if seconds < 60 {
            "just now".to_string()
        } else if seconds < 3600 {
            let mins = duration.num_minutes();
            format!("{} min{} ago", mins, if mins == 1 { "" } else { "s" })
        } else if seconds < 86400 {
            let hours = duration.num_hours();
            format!("{} hour{} ago", hours, if hours == 1 { "" } else { "s" })
        } else if seconds < 604800 {
            let days = duration.num_days();
            format!("{} day{} ago", days, if days == 1 { "" } else { "s" })
        } else if seconds < 2_592_000 {
            let weeks = duration.num_weeks();
            format!("{} week{} ago", weeks, if weeks == 1 { "" } else { "s" })
        } else if seconds < 31_536_000 {
            let months = duration.num_days() / 30;
            format!("{} month{} ago", months, if months == 1 { "" } else { "s" })
        } else {
            let years = duration.num_days() / 365;
            format!("{} year{} ago", years, if years == 1 { "" } else { "s" })
        }
    }

    /// Compact age string for narrow terminals: "3d", "2mo", etc.
    pub fn age_short(&self) -> String {
        let duration = Utc::now() - self.last_commit_date;
        let seconds = duration.num_seconds();

        if seconds < 60 {
            "now".into()
        } else if seconds < 3600 {
            format!("{}m", duration.num_minutes())
        } else if seconds < 86400 {
            format!("{}h", duration.num_hours())
        } else if seconds < 604800 {
            format!("{}d", duration.num_days())
        } else if seconds < 2_592_000 {
            format!("{}w", duration.num_weeks())
        } else if seconds < 31_536_000 {
            format!("{}mo", duration.num_days() / 30)
        } else {
            format!("{}y", duration.num_days() / 365)
        }
    }
}

/// What the user wants to do with selected branches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchAction {
    DeleteLocal,
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
    Pull,
    DeleteTag,
    PushTag,
}

impl BranchAction {
    pub fn label(&self) -> &'static str {
        match self {
            BranchAction::DeleteLocal => "Delete local",
            BranchAction::DeleteLocalAndRemote => "Delete local + remote",
            BranchAction::Checkout => "Checkout",
            BranchAction::Fetch => "Fetch",
            BranchAction::FetchPrune => "Fetch + prune",
            BranchAction::FastForward => "Fast-forward",
            BranchAction::Merge => "Merge into base",
            BranchAction::SquashMerge => "Squash merge into base",
            BranchAction::Rebase => "Rebase onto base",
            BranchAction::Worktree => "Create worktree",
            BranchAction::Push => "Push",
            BranchAction::Pull => "Pull",
            BranchAction::DeleteTag => "Delete tag",
            BranchAction::PushTag => "Push tag",
        }
    }
}

/// Result of a single branch operation.
#[derive(Debug, Clone)]
pub struct OperationResult {
    pub branch_name: String,
    pub action: BranchAction,
    pub success: bool,
    pub message: String,
}

/// Working tree status: clean, staged, unstaged, untracked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkingTreeStatus {
    pub has_staged: bool,
    pub has_unstaged: bool,
    pub has_untracked: bool,
}

impl WorkingTreeStatus {
    pub fn clean() -> Self {
        Self {
            has_staged: false,
            has_unstaged: false,
            has_untracked: false,
        }
    }

    pub fn is_clean(&self) -> bool {
        !self.has_staged && !self.has_unstaged && !self.has_untracked
    }

    pub fn summary(&self) -> String {
        if self.is_clean() {
            return "clean".to_string();
        }
        let mut parts = Vec::new();
        if self.has_staged {
            parts.push("staged");
        }
        if self.has_unstaged {
            parts.push("unstaged");
        }
        if self.has_untracked {
            parts.push("untracked");
        }
        parts.join("+")
    }
}

/// Progress update sent from a background operation thread.
#[derive(Debug, Clone)]
pub struct ProgressUpdate {
    /// Number of items completed so far.
    pub completed: usize,
    /// Total number of items to process.
    pub total: usize,
    /// Name of the item currently being processed.
    pub current_item: String,
}

/// Result of a background squash-merge check for a single branch.
#[derive(Debug)]
pub struct SquashResult {
    pub branch_name: String,
    pub is_squash_merged: bool,
}
