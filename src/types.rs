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
    pub last_commit_date: DateTime<Utc>,
    pub merge_status: MergeStatus,
}

impl BranchInfo {
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
}

/// What the user wants to do with selected branches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchAction {
    DeleteLocal,
    DeleteLocalAndRemote,
}

impl BranchAction {
    pub fn label(&self) -> &'static str {
        match self {
            BranchAction::DeleteLocal => "Delete local",
            BranchAction::DeleteLocalAndRemote => "Delete local + remote",
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
