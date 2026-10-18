use chrono::{DateTime, Utc};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphSource {
    Gleisbau,
    GitCliFallback { cause: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GraphCommit {
    pub oid: String,
    pub summary: String,
    pub parents: Vec<String>,
    pub lane: Option<usize>,
    /// The live branch whose first-parent history owns this commit. The base
    /// branch claims its chain first so retained merged refs cannot relabel it.
    pub branch: Option<GraphBranchLabel>,
    pub refs: Vec<GraphRef>,
    /// Captured source/destination pairings, including every accepted confidence.
    pub relationships: Vec<GraphRelationship>,
    /// Author name from `git2::Signature::name()` / `%an`.
    pub author_name: String,
    /// Author email from `git2::Signature::email()` / `%ae`.
    pub author_email: String,
    /// Author date (when the change was originally made) in UTC.
    /// Git's "%aI" / `commit.author().when()`. `None` means the loader could
    /// not parse or represent the author date.
    pub authored_at: Option<DateTime<Utc>>,
}

/// The operation that paired a source commit with its destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RelationshipKind {
    SquashMerge,
    CherryPick,
}

impl RelationshipKind {
    pub fn code(self) -> &'static str {
        match self {
            Self::SquashMerge => "sm",
            Self::CherryPick => "c-p",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "sm" => Some(Self::SquashMerge),
            "c-p" => Some(Self::CherryPick),
            _ => None,
        }
    }
}

/// Exact patch identity or an accepted fuzzy similarity below 100 percent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationshipMatch {
    Exact,
    Fuzzy { similarity_percent: u8 },
}

impl RelationshipMatch {
    pub fn similarity_percent(self) -> u8 {
        match self {
            Self::Exact => 100,
            Self::Fuzzy { similarity_percent } => similarity_percent,
        }
    }

    pub fn stronger(self, other: Self) -> Self {
        if self.similarity_percent() >= other.similarity_percent() {
            self
        } else {
            other
        }
    }
}

/// One detected pairing, with sorted, deduplicated ref snapshots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphRelationship {
    pub kind: RelationshipKind,
    pub matching: RelationshipMatch,
    pub destination_oid: String,
    pub destination_refs: Vec<String>,
    pub source_oid: String,
    pub source_refs: Vec<String>,
}

impl GraphRelationship {
    pub fn key(&self) -> (RelationshipKind, &str, &str) {
        (
            self.kind,
            self.destination_oid.as_str(),
            self.source_oid.as_str(),
        )
    }
}

/// Strongest incoming squash confidence and all equally strong source names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SquashMatchConfidence {
    pub sources: Vec<String>,
    pub similarity_percent: u8,
}

impl GraphCommit {
    /// Whether this commit is a source of an exact cherry-pick pairing.
    pub fn is_cherry_picked_commit(&self) -> bool {
        !self.cherry_pick_destinations().is_empty()
    }

    /// Captured destinations reached by picking this source commit.
    pub fn cherry_pick_destinations(&self) -> Vec<&GraphRelationship> {
        self.relationships
            .iter()
            .filter(|r| r.kind == RelationshipKind::CherryPick && r.source_oid == self.oid)
            .collect()
    }

    /// Captured sources picked into this destination commit.
    pub fn cherry_pick_sources(&self) -> Vec<&GraphRelationship> {
        self.relationships
            .iter()
            .filter(|r| r.kind == RelationshipKind::CherryPick && r.destination_oid == self.oid)
            .collect()
    }

    pub fn squash_match_confidence(&self) -> Option<SquashMatchConfidence> {
        let incoming = || {
            self.relationships.iter().filter(|relationship| {
                relationship.kind == RelationshipKind::SquashMerge
                    && relationship.destination_oid == self.oid
            })
        };
        let best = incoming()
            .map(|relationship| relationship.matching.similarity_percent())
            .max()?;
        let mut sources = incoming()
            .filter(|relationship| relationship.matching.similarity_percent() == best)
            .flat_map(|relationship| relationship.source_refs.iter().cloned())
            .collect::<Vec<_>>();
        sources.sort();
        sources.dedup();
        Some(SquashMatchConfidence {
            sources,
            similarity_percent: best,
        })
    }

    pub fn is_possible_squash_merge(&self) -> bool {
        self.squash_match_confidence()
            .is_some_and(|confidence| confidence.similarity_percent == 100)
    }

    pub fn possible_squash_merge_sources(&self) -> Vec<String> {
        self.squash_match_confidence()
            .filter(|confidence| confidence.similarity_percent == 100)
            .map(|confidence| confidence.sources)
            .unwrap_or_default()
    }

    pub fn fuzzy_squash_match(&self) -> Option<FuzzySquashMatch> {
        self.squash_match_confidence()
            .filter(|confidence| confidence.similarity_percent < 100)
            .map(|confidence| FuzzySquashMatch {
                similarity_percent: confidence.similarity_percent,
            })
    }
}

/// A fuzzy (non-exact) possible-squash-merge signal for a single base commit,
/// scored against the best-matching displayed branch tip. See
/// `docs/plans/2026-08-29-squash-merge-test-scenarios.md` "New: Option 6".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuzzySquashMatch {
    /// Jaccard similarity floored from the raw f32 score; fuzzy is always <= 99.
    /// Stored as u8 rather than f32 so GraphCommit/FuzzySquashMatch
    /// can keep deriving Eq (f32 has no Eq impl).
    pub similarity_percent: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphBranchLabel {
    pub name: String,
    pub target_oid: String,
    pub kind: GraphRefKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphLine {
    pub graph: String,
    pub commit_index: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphRef {
    pub name: String,
    pub kind: GraphRefKind,
    pub has_linked_worktree: bool,
    pub is_current: bool,
    pub tracking: Option<GraphRefTracking>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphRefTracking {
    pub ahead: u32,
    pub behind: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GraphRefCounts {
    pub local: usize,
    pub remote: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GraphRefKind {
    LocalBranch,
    RemoteBranch,
    Tag,
}

/// Canonical identity for a repository ref. `full_name` retains the namespace
/// (for example `refs/remotes/origin/topic`) so equal display names cannot
/// alias across local, remote, and tag refs.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GraphRefId {
    pub kind: GraphRefKind,
    pub full_name: String,
}
