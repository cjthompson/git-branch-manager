mod action;
mod diagnostics;
mod format;
mod info;
#[cfg(test)]
mod tests;

pub use action::*;
pub use diagnostics::*;
pub use format::*;
pub use info::*;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// --- Enums ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MergeStatus {
    Merged,
    /// Branch tip OID is literally equal to the base branch tip OID — the branch
    /// was cut from base with no unique commits, so no integration event has
    /// occurred. Distinct from `Merged`, which now implies at least one unique
    /// commit was regular-merged.
    InSync,
    SquashMerged,
    LocalMerged,        // merged into local base, not yet in origin/<base>
    RemoteMerged,       // merged into origin/<base>, local base not fast-forwarded
    LocalSquashMerged,  // squash-merged into local base only
    RemoteSquashMerged, // squash-merged into origin/<base> only
    LikelySquashMerged,
    CherryPicked,
    LocalCherryPicked,
    RemoteCherryPicked,
    Unmerged,
    Pending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SquashConfidence {
    MergeTreeConfirmed,
    FuzzyMatch { similarity_percent: u8 },
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
