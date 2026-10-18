use super::MergeStatus;

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
/// reality (`verified`) versus drifted from it (`mismatched`). Branches that
/// had no cache row contribute to `skipped` with a human-readable `skip_reasons`
/// entry — they are not discrepancies (the app recomputes those on demand).
#[derive(Debug, Clone, Default)]
pub struct CategoryStat {
    pub verified: usize,
    pub mismatched: usize,
    pub skipped: usize,
    pub skip_reasons: Vec<&'static str>,
}

impl CategoryStat {
    /// Number of cached entries actually compared against truth.
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
