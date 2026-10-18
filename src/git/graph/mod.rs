mod backends;
mod cherry;
mod load;
mod model;
mod patch;
mod refs;
mod relationships;
mod squash;
mod state;
#[cfg(test)]
mod tests;

pub use cherry::{MAX_CHERRY_DESTINATION_SCAN, MAX_CHERRY_SOURCE_COMMITS};
pub use load::{
    load_graph, load_graph_with_squash_annotations, spawn_graph_loader,
    spawn_possible_squash_enrichment, update_graph_incrementally,
};
pub use model::{
    FuzzySquashMatch, GraphBranchLabel, GraphCommit, GraphLine, GraphRef, GraphRefCounts,
    GraphRefId, GraphRefKind, GraphRefTracking, GraphRelationship, GraphSource, RelationshipKind,
    RelationshipMatch, SquashMatchConfidence,
};
pub use patch::git2_diff_patch;
pub use relationships::{apply_squash_enrichment, compute_relationships};
pub use state::{
    GraphLineStyle, GraphLoadError, GraphLoadOptions, GraphRepositoryDelta, GraphRepositoryState,
};

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

/// An app-owned graph payload. It deliberately contains no repository handles
/// or Gleisbau values, so it can move across a background channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphSnapshot {
    pub source: GraphSource,
    pub commits: Vec<GraphCommit>,
    pub lines: Vec<GraphLine>,
    pub ref_counts: GraphRefCounts,
    pub max_count: usize,
    pub includes_remotes: bool,
    /// Reload-generation tag. `None` for snapshots that bypass the enrichment
    /// channel (test fixtures, the CLI dump path). The App stamps every
    /// `load_graph` result with `Some(generation)` so it can match enrichment
    /// messages to the snapshot they belong to and drop stale ones.
    pub generation: Option<u64>,
}

/// Channel message carrying the full set of squash-merge enrichment updates
/// for a given `GraphSnapshot` reload generation. Delivered as a single
/// batch rather than per-commit so the App applies it atomically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphEnrichmentMsg {
    pub generation: u64,
    pub updates: Vec<GraphRelationship>,
}

/// Cooperative cancellation owned by the App's graph generation. Already
/// running Git subprocesses finish; superseded workers stop at work boundaries.
#[derive(Clone, Debug)]
pub struct EnrichmentCancel {
    generation: u64,
    latest: Arc<AtomicU64>,
}

impl EnrichmentCancel {
    pub fn new(latest: Arc<AtomicU64>, generation: u64) -> Self {
        Self { generation, latest }
    }

    pub fn never() -> Self {
        Self::new(Arc::new(AtomicU64::new(0)), 0)
    }

    pub fn is_cancelled(&self) -> bool {
        self.latest.load(Ordering::Acquire) != self.generation
    }
}

// Local observer lets worker tests rendezvous at the same boundaries that
// production polls. The public worker uses a no-op observer.
#[derive(Clone, Copy, PartialEq, Eq)]
enum EnrichmentStage {
    PatchDispatch,
    CherrySourceYield,
    CherryDestinationYield,
    CherryPatchCompute,
}

type EnrichmentObserver = Arc<dyn Fn(EnrichmentStage) + Send + Sync>;

#[derive(Debug)]
pub struct GraphUpdateMsg {
    pub revision: u64,
    pub options: GraphLoadOptions,
    pub delta: GraphRepositoryDelta,
    pub result: Result<GraphSnapshot, GraphLoadError>,
}

pub fn spawn_graph_updater(
    repo_path: PathBuf,
    snapshot: GraphSnapshot,
    delta: GraphRepositoryDelta,
    options: GraphLoadOptions,
    revision: u64,
) -> Receiver<GraphUpdateMsg> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = update_graph_incrementally(&repo_path, snapshot, &delta, options.clone());
        let _ = tx.send(GraphUpdateMsg { revision, options, delta, result });
    });
    rx
}
