use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

// --- Enums ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MergeStatus {
    Merged,
    SquashMerged,
    Unmerged,
    Pending,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrackingStatus {
    Tracked { remote_ref: String, gone: bool },
    Local,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrStatus {
    Draft,
    Open,
    Merged,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrInfo {
    pub number: u32,
    pub status: PrStatus,
}

pub type PrMap = HashMap<String, PrInfo>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchAction {
    // Local branch actions
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
    ViewRemotePR,
    // Worktree actions
    WorktreeRemove,
    WorktreeForceRemove,
}

impl BranchAction {
    pub fn label(&self) -> &'static str {
        match self {
            Self::DeleteLocal => "Delete local",
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
            Self::ViewRemotePR => "Open PR in browser",
            Self::WorktreeRemove => "Remove worktree",
            Self::WorktreeForceRemove => "Force remove worktree",
        }
    }
}

// --- Working Tree Status ---

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

    /// Single-letter abbreviation for narrow columns: staged→`s`, unstaged→`u`,
    /// untracked→`t`, joined by `+` (e.g. `s+u`). Empty when clean.
    pub fn short_summary(&self) -> String {
        let mut parts = Vec::new();
        if self.has_staged {
            parts.push("s");
        }
        if self.has_unstaged {
            parts.push("u");
        }
        if self.has_untracked {
            parts.push("t");
        }
        parts.join("+")
    }
}

// --- Data Model Structs ---

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
    /// Name of the detected base branch (e.g. "main")
    pub base_branch: String,
    /// Short (8-char) commit hash of the merge-base with the base branch
    pub merge_base_commit: Option<String>,
    pub pr: Option<PrInfo>,
}

impl BranchInfo {
    pub fn is_pinned(&self) -> bool {
        self.is_base || self.is_current
    }

    pub fn age_display(&self) -> String {
        format_age(&self.last_commit_date)
    }

    pub fn age_short(&self) -> String {
        format_age_short(&self.last_commit_date)
    }
}

#[derive(Debug, Clone)]
pub struct RemoteBranchInfo {
    pub full_ref: String,
    pub remote: String,
    pub short_name: String,
    pub has_local: bool,
    pub is_base: bool,
    pub last_commit_date: DateTime<Utc>,
    pub merge_status: MergeStatus,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
    /// True when this remote shares no history with the base branch (no merge
    /// base). ahead/behind then equal the full history sizes and are meaningless,
    /// so the A/B column shows the `disjoint` marker instead of the counts.
    pub disjoint: bool,
    pub pr: Option<PrInfo>,
}

impl RemoteBranchInfo {
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

#[derive(Debug, Clone)]
pub struct WorktreeInfo {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub is_main: bool,
    pub commit_hash: String,
    pub wt_status: WorkingTreeStatus,
    pub age_date: DateTime<Utc>,
    pub merge_status: MergeStatus,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
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

#[derive(Debug, Clone)]
pub struct TagInfo {
    pub name: String,
    pub commit_hash: String,
    pub date: DateTime<Utc>,
    pub message: Option<String>,
    pub is_annotated: bool,
}

impl TagInfo {
    pub fn age_display(&self) -> String {
        format_age(&self.date)
    }

    pub fn age_short(&self) -> String {
        format_age_short(&self.date)
    }
}

// --- Operation/Channel Types ---

#[derive(Debug, Clone)]
pub struct OperationResult {
    pub branch_name: String,
    pub action: BranchAction,
    pub success: bool,
    pub message: String,
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
    pub is_squash_merged: bool,
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
    pub index: usize,
    pub wt_status: WorkingTreeStatus,
    pub age_date: DateTime<Utc>,
}

// --- Diagnostics ---

/// A diagnostic tool selectable from the Diagnostics overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticAction {
    /// Recompute all cached git analysis fresh and diff it against the cache.
    VerifyCache,
}

impl DiagnosticAction {
    /// All diagnostics, in display order. The Diagnostics overlay renders one
    /// menu row per entry, so adding a tool is a matter of extending this slice.
    pub const ALL: &'static [DiagnosticAction] = &[DiagnosticAction::VerifyCache];

    pub fn label(&self) -> &'static str {
        match self {
            Self::VerifyCache => "Verify cache accuracy",
        }
    }
}

/// Which cached value category a [`Discrepancy`] belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagKind {
    MergeStatus,
    AheadBehind,
    MergeBase,
}

impl DiagKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::MergeStatus => "merge-status",
            Self::AheadBehind => "ahead/behind",
            Self::MergeBase => "merge-base",
        }
    }
}

/// Per-category tally from a cache audit: how many cached entries matched git
/// reality (`verified`) versus drifted from it (`mismatched`).
#[derive(Debug, Clone, Copy, Default)]
pub struct CategoryStat {
    pub verified: usize,
    pub mismatched: usize,
}

impl CategoryStat {
    pub fn checked(&self) -> usize {
        self.verified + self.mismatched
    }
}

/// The typed correction to write back when fixing a [`Discrepancy`]. Carries the
/// freshly-computed truth so the fix never has to recompute it.
#[derive(Debug, Clone)]
pub enum CacheFix {
    Status {
        commit_hash: String,
        status: MergeStatus,
    },
    AheadBehind {
        branch_oid: String,
        upstream_oid: String,
        ahead: u32,
        behind: u32,
    },
    MergeBase {
        branch_tip: String,
        base_tip: String,
        merge_base: Option<String>,
    },
}

/// A single cached value that disagrees with freshly-computed git reality.
#[derive(Debug, Clone)]
pub struct Discrepancy {
    pub branch: String,
    pub kind: DiagKind,
    /// What the cache currently serves (human-readable).
    pub cached: String,
    /// What git actually says (human-readable).
    pub actual: String,
    /// Typed correction to apply on fix.
    pub fix: CacheFix,
}

/// Result of verifying the on-disk cache against git reality.
#[derive(Debug, Clone, Default)]
pub struct CacheAudit {
    pub merge_status: CategoryStat,
    pub ahead_behind: CategoryStat,
    pub merge_base: CategoryStat,
    pub discrepancies: Vec<Discrepancy>,
    /// Cached merge-status rows whose branch no longer exists.
    pub orphans: Vec<String>,
}

impl CacheAudit {
    /// True when nothing needs fixing.
    pub fn is_clean(&self) -> bool {
        self.discrepancies.is_empty() && self.orphans.is_empty()
    }

    /// Total cached entries checked across all categories.
    pub fn total_checked(&self) -> usize {
        self.merge_status.checked() + self.ahead_behind.checked() + self.merge_base.checked()
    }
}

// --- Format Helpers ---

pub fn format_age(date: &DateTime<Utc>) -> String {
    let secs = Utc::now().signed_duration_since(*date).num_seconds().max(0);
    if secs < 60 {
        return "just now".to_string();
    }
    let mins = secs / 60;
    if mins < 60 {
        return plural(mins, "minute");
    }
    let hours = mins / 60;
    if hours < 24 {
        return plural(hours, "hour");
    }
    let days = hours / 24;
    if days < 7 {
        return plural(days, "day");
    }
    let weeks = days / 7;
    if weeks < 5 {
        return plural(weeks, "week");
    }
    let months = days / 30;
    if months < 12 {
        return plural(months, "month");
    }
    let years = days / 365;
    plural(years, "year")
}

pub fn format_age_short(date: &DateTime<Utc>) -> String {
    let secs = Utc::now().signed_duration_since(*date).num_seconds().max(0);
    if secs < 60 {
        return "now".to_string();
    }
    let mins = secs / 60;
    if mins < 60 {
        return format!("{mins}m");
    }
    let hours = mins / 60;
    if hours < 24 {
        return format!("{hours}h");
    }
    let days = hours / 24;
    if days < 7 {
        return format!("{days}d");
    }
    let weeks = days / 7;
    if weeks < 5 {
        return format!("{weeks}w");
    }
    let months = days / 30;
    if months < 12 {
        return format!("{months}mo");
    }
    let years = days / 365;
    format!("{years}y")
}

fn plural(n: i64, unit: &str) -> String {
    if n == 1 {
        format!("{n} {unit} ago")
    } else {
        format!("{n} {unit}s ago")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn format_age_just_now() {
        let now = Utc::now();
        assert_eq!(format_age(&now), "just now");
    }

    #[test]
    fn format_age_minutes() {
        let date = Utc::now() - chrono::Duration::minutes(5);
        assert_eq!(format_age(&date), "5 minutes ago");
    }

    #[test]
    fn format_age_singular_day() {
        let date = Utc::now() - chrono::Duration::days(1);
        assert_eq!(format_age(&date), "1 day ago");
    }

    #[test]
    fn format_age_plural_days() {
        let date = Utc::now() - chrono::Duration::days(3);
        assert_eq!(format_age(&date), "3 days ago");
    }

    #[test]
    fn format_age_short_days() {
        let date = Utc::now() - chrono::Duration::days(3);
        assert_eq!(format_age_short(&date), "3d");
    }

    #[test]
    fn format_age_short_weeks() {
        let date = Utc::now() - chrono::Duration::weeks(2);
        assert_eq!(format_age_short(&date), "2w");
    }

    #[test]
    fn format_age_short_months() {
        let date = Utc::now() - chrono::Duration::days(60);
        assert_eq!(format_age_short(&date), "2mo");
    }

    #[test]
    fn format_age_short_years() {
        let date = Utc::now() - chrono::Duration::days(400);
        assert_eq!(format_age_short(&date), "1y");
    }

    #[test]
    fn working_tree_status_clean() {
        let s = WorkingTreeStatus::clean();
        assert!(s.is_clean());
        assert_eq!(s.summary(), "");
    }

    #[test]
    fn working_tree_status_staged_only() {
        let s = WorkingTreeStatus {
            has_staged: true,
            has_unstaged: false,
            has_untracked: false,
        };
        assert!(!s.is_clean());
        assert_eq!(s.summary(), "staged");
    }

    #[test]
    fn working_tree_status_all_three() {
        let s = WorkingTreeStatus {
            has_staged: true,
            has_unstaged: true,
            has_untracked: true,
        };
        assert_eq!(s.summary(), "staged+unstaged+untracked");
    }

    #[test]
    fn branch_info_is_pinned() {
        let b = BranchInfo {
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
        };
        assert!(b.is_pinned());
    }

    #[test]
    fn branch_info_not_pinned() {
        let b = BranchInfo {
            name: "feature/x".into(),
            is_current: false,
            is_base: false,
            tracking: TrackingStatus::Local,
            ahead: None,
            behind: None,
            last_commit_date: Utc::now(),
            merge_status: MergeStatus::Unmerged,
            base_branch: "main".into(),
            merge_base_commit: None,
            pr: None,
        };
        assert!(!b.is_pinned());
    }

    #[test]
    fn tag_info_age_short() {
        let t = TagInfo {
            name: "v1.0".into(),
            commit_hash: "abc1234".into(),
            date: Utc::now() - chrono::Duration::days(5),
            message: Some("Release 1.0".into()),
            is_annotated: true,
        };
        assert_eq!(t.age_short(), "5d");
    }

    #[test]
    fn worktree_info_main_is_pinned() {
        let w = WorktreeInfo {
            path: PathBuf::from("/repo"),
            branch: Some("main".into()),
            is_main: true,
            commit_hash: "abc1234".into(),
            wt_status: WorkingTreeStatus::clean(),
            age_date: Utc::now(),
            merge_status: MergeStatus::Unmerged,
            ahead: None,
            behind: None,
            pr: None,
        };
        assert!(w.is_pinned());
    }

    #[test]
    fn branch_action_labels() {
        assert_eq!(BranchAction::DeleteLocal.label(), "Delete local");
        assert_eq!(BranchAction::Checkout.label(), "Checkout");
        assert_eq!(BranchAction::PushTag.label(), "Push tag");
        assert_eq!(BranchAction::WorktreeRemove.label(), "Remove worktree");
    }
}
