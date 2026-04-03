use chrono::{DateTime, Utc};
use crate::git::github::PrStatus;

/// Merge status of a branch relative to the base branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeStatus {
    /// Branch is reachable from the base branch (regular merge)
    Merged,
    /// Branch content exists in base via squash merge
    SquashMerged,
    /// Branch has not been merged
    Unmerged,
    /// Squash-merge check has not completed yet (phase-1 placeholder)
    Pending,
}

/// Human-readable age string: "3 days ago", "2 months ago", etc.
pub fn format_age(date: &DateTime<Utc>) -> String {
    let duration = Utc::now() - *date;
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
pub fn format_age_short(date: &DateTime<Utc>) -> String {
    let duration = Utc::now() - *date;
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
        format_age(&self.last_commit_date)
    }

    /// Compact age string for narrow terminals: "3d", "2mo", etc.
    pub fn age_short(&self) -> String {
        format_age_short(&self.last_commit_date)
    }
}

/// All information about a single remote branch.
#[derive(Debug, Clone)]
pub struct RemoteBranchInfo {
    /// Full ref name, e.g. "origin/feature-x"
    pub full_ref: String,
    /// Remote name, e.g. "origin"
    pub remote: String,
    /// Branch name without remote prefix, e.g. "feature-x"
    pub short_name: String,
    /// Whether a local branch with the same name exists
    pub has_local: bool,
    /// Whether this is the base branch (e.g. origin/main)
    pub is_base: bool,
    pub last_commit_date: DateTime<Utc>,
    pub merge_status: MergeStatus,
    /// Commits ahead of base branch (None if not computed)
    pub ahead: Option<u32>,
    /// Commits behind base branch (None if not computed)
    pub behind: Option<u32>,
}

impl RemoteBranchInfo {
    /// Remote branches are pinned if they are the base branch.
    pub fn is_pinned(&self) -> bool {
        self.is_base
    }

    pub fn age_display(&self) -> String {
        format_age(&self.last_commit_date)
    }

    pub fn age_short(&self) -> String {
        format_age_short(&self.last_commit_date)
    }
}

/// All information about a single git worktree.
#[derive(Debug, Clone)]
pub struct WorktreeInfo {
    /// Absolute path to this worktree directory.
    pub path: std::path::PathBuf,
    /// Checked-out branch name, or `None` if detached HEAD.
    pub branch: Option<String>,
    /// True for the main (primary) worktree.
    pub is_main: bool,
    /// Short (7-char) HEAD commit SHA.
    pub commit_hash: String,
    /// Working tree status (staged/unstaged/untracked).
    pub wt_status: WorkingTreeStatus,
    /// Age date: newest mtime of dirty files if dirty, else HEAD commit date.
    pub age_date: DateTime<Utc>,
    // Fields below are populated by phase 2 (branch enrichment):
    /// Merge status relative to base branch (defaults to Unmerged until phase-2 enrichment).
    pub merge_status: MergeStatus,
    /// Commits ahead of remote tracking branch (None until phase-2 enrichment).
    pub ahead: Option<u32>,
    /// Commits behind remote tracking branch (None until phase-2 enrichment).
    pub behind: Option<u32>,
    /// Associated GitHub PR (None until phase-2 enrichment).
    pub pr: Option<PrStatus>,
}

impl WorktreeInfo {
    pub fn is_pinned(&self) -> bool {
        self.is_main
    }

    pub fn age_display(&self) -> String {
        format_age(&self.age_date)
    }

    pub fn age_short(&self) -> String {
        format_age_short(&self.age_date)
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
    ForcePush,
    Pull,
    DeleteTag,
    DeleteTagAndRemote,
    PushTag,
    DeleteRemoteBranch,
    DeleteRemoteAndLocal,
    CheckoutRemote,
    WorktreeRemove,
    WorktreeForceRemove,
    FetchRemote,
    PullRemote,
    MergeRemoteIntoCurrent,
    CherryPickRemote,
    ViewRemotePR,
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
            BranchAction::ForcePush => "Force push",
            BranchAction::Pull => "Pull",
            BranchAction::DeleteTag => "Delete tag",
            BranchAction::DeleteTagAndRemote => "Delete tag (local + remote)",
            BranchAction::PushTag => "Push tag",
            BranchAction::DeleteRemoteBranch => "Delete remote branch",
            BranchAction::DeleteRemoteAndLocal => "Delete remote + local",
            BranchAction::CheckoutRemote => "Checkout remote branch",
            BranchAction::WorktreeRemove => "Remove worktree",
            BranchAction::WorktreeForceRemove => "Force remove worktree",
            BranchAction::FetchRemote => "Fetch remote",
            BranchAction::PullRemote => "Pull remote",
            BranchAction::MergeRemoteIntoCurrent => "Merge into current",
            BranchAction::CherryPickRemote => "Cherry-pick latest",
            BranchAction::ViewRemotePR => "View PR in browser",
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

/// Per-item result sent from the remote-branch enrichment background thread.
#[derive(Debug, Clone)]
pub struct RemoteEnrichResult {
    /// Identifies which branch to update (matches `RemoteBranchInfo::full_ref`).
    pub full_ref: String,
    pub merge_status: MergeStatus,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
}

/// Per-item result sent from the worktree status-enrichment background thread.
#[derive(Debug, Clone)]
pub struct WorktreeEnrichResult {
    /// Index into `App::worktrees` (position in the phase-1 list).
    pub index: usize,
    pub wt_status: WorkingTreeStatus,
    pub age_date: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};

    fn ago(secs: i64) -> chrono::DateTime<Utc> {
        Utc::now() - Duration::seconds(secs)
    }

    // --- format_age ---

    #[test]
    fn format_age_just_now() {
        assert_eq!(format_age(&ago(30)), "just now");
    }

    #[test]
    fn format_age_singular_minute() {
        // 65 s → 1 min
        assert_eq!(format_age(&ago(65)), "1 min ago");
    }

    #[test]
    fn format_age_plural_minutes() {
        // 305 s → 5 mins
        assert_eq!(format_age(&ago(305)), "5 mins ago");
    }

    #[test]
    fn format_age_singular_hour() {
        // 3665 s → 1 hour
        assert_eq!(format_age(&ago(3_665)), "1 hour ago");
    }

    #[test]
    fn format_age_plural_hours() {
        // 7265 s → 2 hours
        assert_eq!(format_age(&ago(7_265)), "2 hours ago");
    }

    #[test]
    fn format_age_singular_day() {
        assert_eq!(format_age(&ago(86_465)), "1 day ago");
    }

    #[test]
    fn format_age_plural_days() {
        // 3 days
        assert_eq!(format_age(&ago(259_265)), "3 days ago");
    }

    #[test]
    fn format_age_singular_week() {
        assert_eq!(format_age(&ago(604_865)), "1 week ago");
    }

    #[test]
    fn format_age_plural_weeks() {
        assert_eq!(format_age(&ago(1_209_665)), "2 weeks ago");
    }

    #[test]
    fn format_age_singular_month() {
        // 30 days + 65 s — falls into months bucket (num_days=30, 30/30=1)
        assert_eq!(format_age(&ago(2_592_065)), "1 month ago");
    }

    #[test]
    fn format_age_plural_months() {
        // 90 days (3 months)
        assert_eq!(format_age(&ago(7_776_065)), "3 months ago");
    }

    #[test]
    fn format_age_singular_year() {
        // 365 days + 65 s
        assert_eq!(format_age(&ago(31_536_065)), "1 year ago");
    }

    #[test]
    fn format_age_plural_years() {
        // 730 days (2 years)
        assert_eq!(format_age(&ago(63_072_065)), "2 years ago");
    }

    // --- format_age_short ---

    #[test]
    fn format_age_short_now() {
        assert_eq!(format_age_short(&ago(30)), "now");
    }

    #[test]
    fn format_age_short_minutes() {
        assert_eq!(format_age_short(&ago(305)), "5m");
    }

    #[test]
    fn format_age_short_hours() {
        assert_eq!(format_age_short(&ago(7_265)), "2h");
    }

    #[test]
    fn format_age_short_days() {
        assert_eq!(format_age_short(&ago(172_865)), "2d");
    }

    #[test]
    fn format_age_short_weeks() {
        assert_eq!(format_age_short(&ago(1_209_665)), "2w");
    }

    #[test]
    fn format_age_short_months() {
        // 60 days → 2mo
        assert_eq!(format_age_short(&ago(5_184_065)), "2mo");
    }

    #[test]
    fn format_age_short_years() {
        // 730 days → 2y
        assert_eq!(format_age_short(&ago(63_072_065)), "2y");
    }

    // --- WorkingTreeStatus ---

    #[test]
    fn working_tree_status_clean_is_clean() {
        let s = WorkingTreeStatus::clean();
        assert!(s.is_clean());
        assert!(!s.has_staged);
        assert!(!s.has_unstaged);
        assert!(!s.has_untracked);
    }

    #[test]
    fn working_tree_status_not_clean_when_staged() {
        let s = WorkingTreeStatus { has_staged: true, has_unstaged: false, has_untracked: false };
        assert!(!s.is_clean());
    }

    #[test]
    fn working_tree_status_not_clean_when_unstaged() {
        let s = WorkingTreeStatus { has_staged: false, has_unstaged: true, has_untracked: false };
        assert!(!s.is_clean());
    }

    #[test]
    fn working_tree_status_not_clean_when_untracked() {
        let s = WorkingTreeStatus { has_staged: false, has_unstaged: false, has_untracked: true };
        assert!(!s.is_clean());
    }

    #[test]
    fn working_tree_status_summary_clean() {
        assert_eq!(WorkingTreeStatus::clean().summary(), "clean");
    }

    #[test]
    fn working_tree_status_summary_staged_only() {
        let s = WorkingTreeStatus { has_staged: true, has_unstaged: false, has_untracked: false };
        assert_eq!(s.summary(), "staged");
    }

    #[test]
    fn working_tree_status_summary_unstaged_only() {
        let s = WorkingTreeStatus { has_staged: false, has_unstaged: true, has_untracked: false };
        assert_eq!(s.summary(), "unstaged");
    }

    #[test]
    fn working_tree_status_summary_untracked_only() {
        let s = WorkingTreeStatus { has_staged: false, has_unstaged: false, has_untracked: true };
        assert_eq!(s.summary(), "untracked");
    }

    #[test]
    fn working_tree_status_summary_staged_and_unstaged() {
        let s = WorkingTreeStatus { has_staged: true, has_unstaged: true, has_untracked: false };
        assert_eq!(s.summary(), "staged+unstaged");
    }

    #[test]
    fn working_tree_status_summary_all() {
        let s = WorkingTreeStatus { has_staged: true, has_unstaged: true, has_untracked: true };
        assert_eq!(s.summary(), "staged+unstaged+untracked");
    }
}
