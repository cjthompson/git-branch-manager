use super::format::{format_age, format_age_short};
use super::{MergeStatus, PrInfo, PrStatus, SquashConfidence, TrackingStatus};
use chrono::{DateTime, Utc};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrimaryBranchCodeMatch {
    Merged,
    ContentEquivalent { commit_oid: String },
    NotFound,
}

// --- Working Tree Status ---

/// Category of a single changed file reported by `git status`: either a
/// tracked file with working-tree changes, or a new untracked file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangedFileKind {
    Staged,
    Modified,
    Untracked,
}

impl ChangedFileKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Staged => "staged",
            Self::Modified => "modified",
            Self::Untracked => "untracked",
        }
    }
}

/// One file reported as modified or untracked in a worktree's working tree,
/// for display in the Details view's "Changed Files" section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedFile {
    pub path: String,
    pub kind: ChangedFileKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkingTreeStatus {
    pub has_staged: bool,
    /// Tracked files with working-tree changes that are not staged. Named to
    /// match git's own "modified" wording in `git status`.
    pub has_modified: bool,
    pub has_untracked: bool,
    /// Modified and untracked files (excludes staged-only files), for the
    /// Details view's "Changed Files" section. Populated by
    /// `git::status::detect_working_tree_status`.
    pub changed_files: Vec<ChangedFile>,
}

impl WorkingTreeStatus {
    pub fn clean() -> Self {
        Self {
            has_staged: false,
            has_modified: false,
            has_untracked: false,
            changed_files: Vec::new(),
        }
    }

    pub fn is_clean(&self) -> bool {
        !self.has_staged && !self.has_modified && !self.has_untracked
    }

    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if self.has_staged {
            parts.push("staged");
        }
        if self.has_modified {
            parts.push("modified");
        }
        if self.has_untracked {
            parts.push("untracked");
        }
        parts.join("+")
    }

    /// Single-letter abbreviation for narrow columns: staged→`s`, modified→`m`,
    /// untracked→`t`, joined by `+` (e.g. `s+m`). Empty when clean.
    pub fn short_summary(&self) -> String {
        let mut parts = Vec::new();
        if self.has_staged {
            parts.push("s");
        }
        if self.has_modified {
            parts.push("m");
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
    pub squash_confidence: Option<SquashConfidence>,
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
    pub squash_confidence: Option<SquashConfidence>,
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
    /// True when this worktree is checked out on the base branch. The merge
    /// column is then blank — a branch can't be "merged into itself". Set by
    /// [`crate::git::worktree::apply_branch_merge_status`] from the branch list.
    pub is_base: bool,
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
