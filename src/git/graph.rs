use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};

use thiserror::Error;

use chrono::{DateTime, TimeZone, Utc};

use crate::git::cache::{BranchCache, CacheRoot};

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

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GraphRepositoryState {
    pub refs: HashMap<GraphRefId, String>,
    /// Non-branch/tag history roots (for example refs/stash and custom refs).
    /// Full remote-inclusive loaders use these as traversal roots too.
    pub additional_roots: HashMap<String, String>,
    pub head_ref: Option<String>,
    pub head_oid: Option<String>,
    pub worktrees: HashMap<String, Option<String>>,
    pub tracking: HashMap<String, (String, Option<u32>, Option<u32>)>,
}

impl GraphRepositoryState {
    pub fn capture(repo_path: &Path) -> Result<Self, String> {
        Self::capture_with_head(repo_path, repo_path)
    }

    pub fn capture_with_head(repo_path: &Path, head_repo_path: &Path) -> Result<Self, String> {
        let repo = git2::Repository::open(repo_path).map_err(|error| error.message().to_string())?;
        let head_repo = git2::Repository::open(head_repo_path).map_err(|error| error.message().to_string())?;
        let mut state = Self::default();
        let references = repo.references().map_err(|error| error.message().to_string())?;
        for reference in references {
            let reference = reference.map_err(|error| error.message().to_string())?;
            let Ok(full_name) = reference.name().map(str::to_string) else { continue };
            let kind = if full_name.starts_with("refs/heads/") {
                GraphRefKind::LocalBranch
            } else if full_name.starts_with("refs/remotes/") {
                GraphRefKind::RemoteBranch
            } else if full_name.starts_with("refs/tags/") {
                GraphRefKind::Tag
            } else {
                if let Ok(commit) = reference.peel_to_commit() {
                    state.additional_roots.insert(full_name, commit.id().to_string());
                }
                continue
            };
            let oid = reference.peel_to_commit().map(|commit| commit.id().to_string()).ok();
            if let Some(oid) = oid {
                state.refs.insert(GraphRefId { kind, full_name }, oid);
            }
        }
        for branch in repo.branches(Some(git2::BranchType::Local)).map_err(|error| error.message().to_string())? {
            let (branch, _) = branch.map_err(|error| error.message().to_string())?;
            let Some(name) = branch.name().map_err(|error| error.message().to_string())?.map(str::to_string) else { continue };
            let Ok(upstream) = branch.upstream() else { continue };
            let Some(upstream_name) = upstream.name().ok().flatten().map(str::to_string) else { continue };
            let tracking = match (branch.get().target(), upstream.get().target()) {
                (Some(local), Some(remote)) => repo.graph_ahead_behind(local, remote).ok().map(|(ahead, behind)| (Some(ahead.try_into().unwrap_or(u32::MAX)), Some(behind.try_into().unwrap_or(u32::MAX)))),
                _ => None,
            }.unwrap_or((None, None));
            state.tracking.insert(name, (upstream_name, tracking.0, tracking.1));
        }
        if let Ok(head) = head_repo.head() {
            state.head_ref = head.name().ok().map(str::to_string);
            state.head_oid = head.peel_to_commit().ok().map(|commit| commit.id().to_string());
        }
        for worktree in crate::git::worktree::try_list_worktrees(repo_path)? {
            let path = std::fs::canonicalize(&worktree.path).unwrap_or(worktree.path);
            state.worktrees.insert(path.to_string_lossy().into_owned(), worktree.branch);
        }
        Ok(state)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphRepositoryDelta {
    pub before: GraphRepositoryState,
    pub after: GraphRepositoryState,
    pub added_refs: Vec<(GraphRefId, String)>,
    pub removed_refs: Vec<(GraphRefId, String)>,
    pub moved_refs: Vec<(GraphRefId, String, String)>,
    pub head_changed: bool,
    pub worktrees_changed: bool,
    pub tracking_changed: bool,
    pub additional_roots_changed: bool,
}

impl GraphRepositoryDelta {
    pub fn between(before: GraphRepositoryState, after: GraphRepositoryState) -> Self {
        let mut added_refs = Vec::new();
        let mut removed_refs = Vec::new();
        let mut moved_refs = Vec::new();
        for (id, old_oid) in &before.refs {
            match after.refs.get(id) {
                None => removed_refs.push((id.clone(), old_oid.clone())),
                Some(new_oid) if new_oid != old_oid => moved_refs.push((id.clone(), old_oid.clone(), new_oid.clone())),
                _ => {}
            }
        }
        for (id, oid) in &after.refs {
            if !before.refs.contains_key(id) { added_refs.push((id.clone(), oid.clone())); }
        }
        Self {
            head_changed: before.head_ref != after.head_ref || before.head_oid != after.head_oid,
            worktrees_changed: before.worktrees != after.worktrees,
            tracking_changed: before.tracking != after.tracking,
            additional_roots_changed: before.additional_roots != after.additional_roots,
            before,
            after,
            added_refs,
            removed_refs,
            moved_refs,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphLoadOptions {
    pub max_count: usize,
    pub include_remotes: bool,
    pub line_style: GraphLineStyle,
    pub base_branch: Option<String>,
    pub cache_root: CacheRoot,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum GraphLineStyle {
    #[default]
    Thin,
    Round,
}

impl GraphLineStyle {
    pub fn from_symbol_name(name: &str) -> Self {
        if name == "powerline" {
            Self::Round
        } else {
            Self::Thin
        }
    }
}

impl Default for GraphLoadOptions {
    fn default() -> Self {
        Self {
            max_count: 500,
            include_remotes: false,
            line_style: GraphLineStyle::default(),
            base_branch: None,
            cache_root: CacheRoot::from_env(),
        }
    }
}

#[derive(Debug, Error)]
pub enum GraphLoadError {
    #[error("Gleisbau failed ({gleisbau}) and the git CLI fallback failed ({fallback})")]
    Both { gleisbau: String, fallback: String },
}

pub fn load_graph(
    repo_path: &Path,
    options: GraphLoadOptions,
) -> Result<GraphSnapshot, GraphLoadError> {
    let gleisbau_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        load_with_gleisbau(repo_path, options.clone())
    }));

    match gleisbau_result {
        Ok(Ok(snapshot)) => Ok(snapshot),
        Ok(Err(cause)) => load_with_git_cli(repo_path, options, cause.clone()).map_err(|fallback| {
            GraphLoadError::Both {
                gleisbau: cause,
                fallback,
            }
        }),
        Err(_) => {
            let cause = "Gleisbau panicked while building the graph".to_string();
            load_with_git_cli(repo_path, options, cause.clone()).map_err(|fallback| {
                GraphLoadError::Both {
                    gleisbau: cause,
                    fallback,
                }
            })
        }
    }
}

/// Rebuilds topology over only the configured display window from the
/// resulting ref roots. The repository history is never loaded through the
/// normal full Graph loaders on this path; `revwalk.take(max_count)` bounds
/// commit discovery and the resulting owned records are laid out directly.
pub fn update_graph_incrementally(
    repo_path: &Path,
    previous: GraphSnapshot,
    delta: &GraphRepositoryDelta,
    options: GraphLoadOptions,
) -> Result<GraphSnapshot, GraphLoadError> {
    if delta.added_refs.is_empty() && delta.removed_refs.is_empty() && delta.moved_refs.is_empty()
        && !delta.additional_roots_changed {
        let mut patched = previous;
        let refs = collect_ref_data(repo_path, options.include_remotes, options.base_branch.as_deref())
            .map_err(|cause| GraphLoadError::Both { gleisbau: cause.clone(), fallback: cause })?;
        for commit in &mut patched.commits {
            commit.refs = refs.refs_by_oid.get(&commit.oid).cloned().unwrap_or_default();
        }
        patched.ref_counts = refs.ref_counts;
        assign_graph_branch_labels(&mut patched.commits, &refs.branch_labels, refs.base_branch.as_deref());
        return Ok(patched);
    }

    let repository = git2::Repository::open(repo_path)
        .map_err(|error| GraphLoadError::Both { gleisbau: error.message().to_string(), fallback: error.message().to_string() })?;
    let resulting_state = GraphRepositoryState::capture(repo_path)
        .map_err(|cause| GraphLoadError::Both { gleisbau: cause.clone(), fallback: cause })?;
    let previous_generation = previous.generation;
    let previous_topology = previous.commits.iter().map(|commit| (commit.oid.clone(), commit.parents.clone())).collect::<Vec<_>>();
    let old_by_oid = previous.commits.iter().map(|commit| (commit.oid.clone(), commit.clone())).collect::<HashMap<_, _>>();
    let ref_data = collect_ref_data(repo_path, options.include_remotes, options.base_branch.as_deref())
        .map_err(|cause| GraphLoadError::Both { gleisbau: cause.clone(), fallback: cause })?;
    let mut eligible_roots = resulting_state.refs.iter().filter_map(|(id, oid)| {
        (id.kind == GraphRefKind::LocalBranch || options.include_remotes)
            .then(|| git2::Oid::from_str(oid).ok()).flatten()
    }).collect::<Vec<_>>();
    if options.include_remotes {
        eligible_roots.extend(resulting_state.additional_roots.values().filter_map(|oid| git2::Oid::from_str(oid).ok()));
    }
    let mut cached_by_oid = old_by_oid.clone();
    let mut records = previous.commits.clone();
    records.retain(|commit| {
        git2::Oid::from_str(&commit.oid).ok().is_some_and(|commit_oid| {
            eligible_roots.iter().any(|root| *root == commit_oid || repository.graph_descendant_of(*root, commit_oid).unwrap_or(false))
        })
    });
    // Traverse all eligible resulting roots together. This gives Git one
    // deterministic global ordering and lets cached commits count toward the
    // window without hiding their ancestry from refill discovery.
    eligible_roots.sort_by_key(|oid| oid.to_string());
    eligible_roots.dedup();
    let mut walk = repository.revwalk().map_err(|error| GraphLoadError::Both { gleisbau: error.message().to_string(), fallback: error.message().to_string() })?;
    walk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME)
        .map_err(|error| GraphLoadError::Both { gleisbau: error.message().to_string(), fallback: error.message().to_string() })?;
    for root in &eligible_roots {
        walk.push(*root).map_err(|error| GraphLoadError::Both { gleisbau: error.message().to_string(), fallback: error.message().to_string() })?;
    }
    records.clear();
    let mut included = HashSet::new();
    for oid in walk {
        let oid = oid.map_err(|error| GraphLoadError::Both { gleisbau: error.message().to_string(), fallback: error.message().to_string() })?;
        let oid_text = oid.to_string();
        if !included.insert(oid_text.clone()) { continue; }
        if let Some(cached) = cached_by_oid.remove(&oid_text) {
            records.push(cached);
        } else {
            let commit = repository.find_commit(oid).map_err(|error| GraphLoadError::Both { gleisbau: error.message().to_string(), fallback: error.message().to_string() })?;
            records.push(GraphCommit {
                oid: oid_text,
                summary: commit
                    .summary()
                    .ok()
                    .flatten()
                    .unwrap_or_default()
                    .to_string(),
                parents: commit
                    .parent_ids()
                    .map(|parent| parent.to_string())
                    .collect(),
                lane: None,
                branch: None,
                refs: Vec::new(),
                relationships: vec![],
                author_name: commit.author().name().unwrap_or("").to_string(),
                author_email: commit.author().email().unwrap_or("").to_string(),
                authored_at: Utc.timestamp_opt(commit.author().when().seconds(), 0).single(),
            });
        }
        if records.len() >= options.max_count.max(1) { break; }
    }
    for record in &mut records {
        record.refs = ref_data.refs_by_oid.get(&record.oid).cloned().unwrap_or_default();
        record.branch = None;
        if let Some(old) = old_by_oid.get(&record.oid) {
            merge_relationships(&mut record.relationships, &old.relationships);
        }
    }
    records = topological_commit_order(records);
    records.truncate(options.max_count.max(1));
    let same_topology = previous_topology == records.iter().map(|commit| (commit.oid.clone(), commit.parents.clone())).collect::<Vec<_>>();
    if same_topology {
        assign_graph_branch_labels(&mut records, &ref_data.branch_labels, ref_data.base_branch.as_deref());
        let mut patched = previous;
        patched.commits = records;
        patched.ref_counts = ref_data.ref_counts;
        patched.max_count = options.max_count;
        patched.includes_remotes = options.include_remotes;
        return Ok(patched);
    }
    assign_graph_branch_labels(&mut records, &ref_data.branch_labels, ref_data.base_branch.as_deref());

    let settings = gleisbau_settings(options.include_remotes, options.line_style, options.base_branch.as_deref())
        .map_err(|cause| GraphLoadError::Both { gleisbau: cause.clone(), fallback: cause })?;
    let mut commits = records.iter().map(|record| {
        let oid = git2::Oid::from_str(&record.oid).expect("record OID was read from repository");
        gleisbau::backend::git2::CommitInfo {
            oid,
            parents: record.parents.iter().filter_map(|parent| git2::Oid::from_str(parent).ok()).collect(),
            children: Vec::new(),
            branch_trace: None,
        }
    }).collect::<Vec<_>>();
    let indices = commits.iter().enumerate().map(|(index, commit)| (commit.oid, index)).collect::<HashMap<_, _>>();
    gleisbau::backend::git2::assign_children(&mut commits, &indices);
    let mut branches = gleisbau::backend::git2::assign_branches(&repository, &mut commits, &indices, &settings)
        .map_err(|cause| GraphLoadError::Both { gleisbau: cause.clone(), fallback: cause })?;
    gleisbau::backend::git2::correct_fork_merges(&commits, &indices, &mut branches)
        .map_err(|cause| GraphLoadError::Both { gleisbau: cause.clone(), fallback: cause })?;
    gleisbau::backend::git2::assign_sources_targets(&commits, &indices, &mut branches);
    let tracks = gleisbau::graph::TrackMap { commits, indices, all_branches: branches };
    let remote_base = if options.include_remotes {
        options.base_branch.as_deref().and_then(|base| tracks.all_branches.iter().position(|branch| branch.is_remote && branch.name == format!("origin/{base}")))
    } else { None };
    let saved_remote_name = remote_base.map(|index| (index, tracks.all_branches[index].name.clone()));
    let mut tracks = tracks;
    if let Some((index, name)) = &saved_remote_name { tracks.all_branches[*index].name = format!("__gbm_remote_base__/{name}"); }
    let layout = gleisbau::layout::layout_track_range(&tracks, 0..tracks.commits.len(), &settings)
        .map_err(|cause| GraphLoadError::Both { gleisbau: cause.clone(), fallback: cause })?;
    if let Some((index, name)) = saved_remote_name { tracks.all_branches[index].name = name; }
    let rendered = gleisbau::print::unicode::print_graph_terminal(&settings, &tracks, &layout, &vec![1; layout.commit_count()]);
    let mut line_to_commit = HashMap::new();
    for (relative, line) in rendered.commit2line.iter().copied().enumerate() {
        line_to_commit.insert(line, layout.commit_index_start() + relative);
    }
    for (index, track_commit) in tracks.commits.iter().enumerate() {
        records[index].lane = track_commit.branch_trace.and_then(|trace| layout.track_visual(trace)).and_then(|visual| visual.column);
    }
    Ok(GraphSnapshot {
        source: GraphSource::Gleisbau,
        commits: records,
        lines: rendered.graph_lines.into_iter().enumerate().map(|(index, graph)| GraphLine { graph, commit_index: line_to_commit.get(&index).copied() }).collect(),
        ref_counts: ref_data.ref_counts,
        max_count: options.max_count,
        includes_remotes: options.include_remotes,
        generation: previous_generation,
    })
}

fn topological_commit_order(records: Vec<GraphCommit>) -> Vec<GraphCommit> {
    let mut by_oid = records.into_iter().map(|record| (record.oid.clone(), record)).collect::<HashMap<_, _>>();
    let mut child_counts = by_oid.keys().map(|oid| (oid.clone(), 0usize)).collect::<HashMap<_, _>>();
    for record in by_oid.values() {
        for parent in &record.parents {
            if let Some(count) = child_counts.get_mut(parent) { *count += 1; }
        }
    }
    let mut ordered = Vec::with_capacity(by_oid.len());
    while !by_oid.is_empty() {
        let next = child_counts.iter().filter(|(_, count)| **count == 0)
            .filter_map(|(oid, _)| by_oid.get(oid).map(|record| (oid.clone(), record.authored_at.map(|time| time.timestamp()).unwrap_or_default())))
            .max_by(|(oid_a, time_a), (oid_b, time_b)| time_a.cmp(time_b).then_with(|| oid_b.cmp(oid_a)))
            .map(|(oid, _)| oid);
        let Some(oid) = next else { break };
        let Some(record) = by_oid.remove(&oid) else { break };
        for parent in &record.parents {
            if let Some(count) = child_counts.get_mut(parent) { *count = count.saturating_sub(1); }
        }
        child_counts.remove(&oid);
        ordered.push(record);
    }
    ordered.extend(by_oid.into_values());
    ordered
}

/// Synchronous, single-call helper for tests: loads the structural snapshot
/// and applies squash-merge enrichment on the calling thread, returning the
/// fully annotated snapshot. The app uses `spawn_possible_squash_enrichment`
/// instead so the Graph view becomes usable before annotation completes.
#[doc(hidden)]
pub fn load_graph_with_squash_annotations(
    repo_path: &Path,
    options: GraphLoadOptions,
) -> Result<GraphSnapshot, GraphLoadError> {
    let mut snapshot = load_graph(repo_path, options.clone())?;
    let updates = compute_relationships(
        repo_path,
        &snapshot,
        options.base_branch.as_deref(),
        &options.cache_root,
        &EnrichmentCancel::never(),
    );
    apply_squash_enrichment(&mut snapshot, &updates);
    Ok(snapshot)
}

/// Spawn a background thread that computes squash-merge enrichment for the
/// given snapshot and returns a receiver for the single batched result. The
/// caller (App) is expected to compare `msg.generation` against its current
/// reload generation before applying — enrichment that completes after a
/// newer load started must NOT overwrite the newer snapshot.
pub fn spawn_possible_squash_enrichment(
    snapshot: GraphSnapshot,
    repo_path: PathBuf,
    requested_base: Option<String>,
    generation: u64,
    cache_root: CacheRoot,
    latest_generation: Arc<AtomicU64>,
) -> Receiver<GraphEnrichmentMsg> {
    spawn_enrichment_with_observer(
        snapshot,
        repo_path,
        requested_base,
        generation,
        cache_root,
        EnrichmentCancel::new(latest_generation, generation),
        |_| {},
    )
}

fn spawn_enrichment_with_observer(
    snapshot: GraphSnapshot,
    repo_path: PathBuf,
    requested_base: Option<String>,
    generation: u64,
    cache_root: CacheRoot,
    cancel: EnrichmentCancel,
    observer: impl Fn(EnrichmentStage) + Send + Sync + 'static,
) -> Receiver<GraphEnrichmentMsg> {
    let (tx, rx) = mpsc::channel();
    let observer: EnrichmentObserver = Arc::new(observer);
    std::thread::spawn(move || {
        if cancel.is_cancelled() {
            return;
        }
        let updates = compute_relationships_with_observer(
            &repo_path,
            &snapshot,
            requested_base.as_deref(),
            &cache_root,
            &cancel,
            Some(&observer),
        );
        if cancel.is_cancelled() {
            return;
        }
        let _ = tx.send(GraphEnrichmentMsg {
            generation,
            updates,
        });
    });
    rx
}

/// Load a graph in a worker thread. The receiver carries only the owned
/// [`GraphSnapshot`] result, never a repository or Gleisbau type.
pub fn spawn_graph_loader(
    repo_path: PathBuf,
    options: GraphLoadOptions,
) -> Receiver<Result<GraphSnapshot, GraphLoadError>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(load_graph(&repo_path, options));
    });
    rx
}

fn load_with_gleisbau(
    repo_path: &Path,
    options: GraphLoadOptions,
) -> Result<GraphSnapshot, String> {
    let ref_data = collect_ref_data(
        repo_path,
        options.include_remotes,
        options.base_branch.as_deref(),
    )?;
    let settings = gleisbau_settings(
        options.include_remotes,
        options.line_style,
        options.base_branch.as_deref(),
    )?;
    let repository =
        gleisbau::Repository::open(repo_path).map_err(|error| error.message().to_string())?;
    let mut graph = gleisbau::graph::Builder::new()
        .with_repository(repository)
        .with_settings(Rc::clone(&settings))
        .with_max_count(options.max_count)
        .build()?;

    // Gleisbau intentionally strips "origin/" before applying branch order
    // patterns, so ^main$ puts both main and origin/main in the same order
    // group. When those refs diverge, ShortestFirst can then place the remote
    // track in column zero. Keep the remote commits in the graph, but make
    // the matching remote base track non-matching for this layout pass.
    let remote_base_index = if options.include_remotes {
        options.base_branch.as_deref().and_then(|base| {
            let remote_name = format!("origin/{base}");
            graph
                .tracks
                .all_branches
                .iter()
                .position(|branch| branch.is_remote && branch.name == remote_name)
        })
    } else {
        None
    };
    let layout = if let Some(index) = remote_base_index {
        let original_name = graph.tracks.all_branches[index].name.clone();
        let layout_name = format!("__gbm_remote_base__/{original_name}");
        graph.tracks.all_branches[index].name = layout_name;
        let layout = gleisbau::layout::layout_track_range(
            &graph.tracks,
            0..graph.tracks.commits.len(),
            &settings,
        );
        graph.tracks.all_branches[index].name = original_name;
        layout?
    } else {
        graph.layout
    };

    let heights = vec![1; layout.commit_count()];
    let rendered = gleisbau::print::unicode::print_graph_terminal(
        &settings,
        &graph.tracks,
        &layout,
        &heights,
    );
    let mut line_to_commit = HashMap::new();
    for (relative_index, line_index) in rendered.commit2line.iter().copied().enumerate() {
        line_to_commit.insert(
            line_index,
            layout.commit_index_start() + relative_index,
        );
    }

    let mut commits = graph
        .tracks
        .commits
        .iter()
        .map(|info| {
            let oid = info.oid.to_string();
            let commit = graph
                .repository
                .find_commit(info.oid)
                .map_err(|error| error.message().to_string())?;
            let summary = commit
                .summary()
                .map_err(|error| error.message().to_string())?
                .unwrap_or_default()
                .to_string();
            let author = commit.author();
            let author_time = author.when();
            let authored_at = Utc.timestamp_opt(author_time.seconds(), 0).single();
            let lane = info
                .branch_trace
                .and_then(|trace| layout.track_visual(trace))
                .and_then(|visual| visual.column);

            Ok(GraphCommit {
                oid: oid.clone(),
                summary,
                parents: info.parents.iter().map(ToString::to_string).collect(),
                lane,
                branch: None,
                refs: ref_data.refs_by_oid.get(&oid).cloned().unwrap_or_default(),
                relationships: vec![],
                author_name: author.name().unwrap_or("").to_string(),
                author_email: author.email().unwrap_or("").to_string(),
                authored_at,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    assign_graph_branch_labels(
        &mut commits,
        &ref_data.branch_labels,
        ref_data.base_branch.as_deref(),
    );
    Ok(GraphSnapshot {
        source: GraphSource::Gleisbau,
        commits,
        lines: rendered
            .graph_lines
            .into_iter()
            .enumerate()
            .map(|(index, graph)| GraphLine {
                graph,
                commit_index: line_to_commit.get(&index).copied(),
            })
            .collect(),
        ref_counts: ref_data.ref_counts,
        max_count: options.max_count,
        includes_remotes: options.include_remotes,
        generation: None,
    })
}

fn load_with_git_cli(
    repo_path: &Path,
    options: GraphLoadOptions,
    cause: String,
) -> Result<GraphSnapshot, String> {
    let ref_data = collect_ref_data(
        repo_path,
        options.include_remotes,
        options.base_branch.as_deref(),
    )?;
    let max_count = format!("--max-count={}", options.max_count);
    let mut command = Command::new("git");
    command.current_dir(repo_path).args([
        "-c",
        "color.ui=false",
        "log",
        "--graph",
        "--topo-order",
        "--decorate",
        "--oneline",
        "--no-color",
        // NUL cannot occur in Git commit headers, while control bytes such as
        // unit separator are valid in author names and email addresses.
        "--format=%x1e%H%x00%P%x00%an%x00%ae%x00%aI%x00%s",
        &max_count,
    ]);
    if options.include_remotes {
        // Gleisbau uses `push_glob("*")` when remotes are included, which
        // covers tag-only and other non-head refs as well as heads/remotes.
        command.arg("--all");
    } else {
        command.arg("--branches");
    }
    let output = command
        .output()
        .map_err(|error| format!("failed to run git log: {error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }

    let mut commits = Vec::new();
    let mut lines = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let Some(record_start) = line.find('\x1e') else {
            lines.push(GraphLine {
                graph: line.to_string(),
                commit_index: None,
            });
            continue;
        };

        let graph = line[..record_start].to_string();
        let fields: Vec<_> = line[record_start + 1..].splitn(6, '\0').collect();
        if fields.len() != 6 || fields[0].is_empty() {
            return Err(format!("could not parse git log record: {line}"));
        }

        let oid = fields[0].to_string();
        let commit_index = commits.len();
        let lane = graph
            .chars()
            .position(|character| character == '*')
            .map(|index| index / 2);
        let authored_at = DateTime::parse_from_rfc3339(fields[4])
            .ok()
            .map(|dt| dt.with_timezone(&Utc));
        commits.push(GraphCommit {
            oid: oid.clone(),
            summary: fields[5].to_string(),
            parents: fields[1].split_whitespace().map(str::to_string).collect(),
            lane,
            branch: None,
            refs: ref_data.refs_by_oid.get(&oid).cloned().unwrap_or_default(),
            relationships: vec![],
            author_name: fields[2].to_string(),
            author_email: fields[3].to_string(),
            authored_at,
        });
        lines.push(GraphLine {
            graph,
            commit_index: Some(commit_index),
        });
    }
    assign_graph_branch_labels(
        &mut commits,
        &ref_data.branch_labels,
        ref_data.base_branch.as_deref(),
    );
    Ok(GraphSnapshot {
        source: GraphSource::GitCliFallback { cause },
        commits,
        lines,
        ref_counts: ref_data.ref_counts,
        max_count: options.max_count,
        includes_remotes: options.include_remotes,
        generation: None,
    })
}

/// Detect bounded squash and cherry-pick relationships in a worker.
pub fn compute_relationships(
    repo_path: &Path,
    snapshot: &GraphSnapshot,
    requested_base: Option<&str>,
    cache_root: &CacheRoot,
    cancel: &EnrichmentCancel,
) -> Vec<GraphRelationship> {
    compute_relationships_with_observer(
        repo_path,
        snapshot,
        requested_base,
        cache_root,
        cancel,
        None,
    )
}

fn resolve_base_name(repo_path: &Path, requested_base: Option<&str>) -> Option<String> {
    requested_base.map(str::to_owned).or_else(|| {
        let repo = git2::Repository::open(repo_path).ok()?;
        crate::git::branch::detect_base_branch(&repo, None).ok()
    })
}

fn resolve_base(
    repo_path: &Path,
    snapshot: &GraphSnapshot,
    requested_base: Option<&str>,
) -> Option<(String, String)> {
    let name = resolve_base_name(repo_path, requested_base)?;
    let tip = snapshot
        .commits
        .iter()
        .find(|commit| {
            commit
                .refs
                .iter()
                .any(|r| r.kind == GraphRefKind::LocalBranch && r.name == name)
        })?
        .oid
        .clone();
    Some((name, tip))
}

fn compute_relationships_with_observer(
    repo_path: &Path,
    snapshot: &GraphSnapshot,
    requested_base: Option<&str>,
    cache_root: &CacheRoot,
    cancel: &EnrichmentCancel,
    observer: Option<&EnrichmentObserver>,
) -> Vec<GraphRelationship> {
    if cancel.is_cancelled() {
        return Vec::new();
    }
    let Some((base_branch, base_tip)) = resolve_base(repo_path, snapshot, requested_base) else {
        return Vec::new();
    };
    if cancel.is_cancelled() {
        return Vec::new();
    }
    let mut cache = BranchCache::load_for_base(repo_path, &base_branch, cache_root);
    let noop: EnrichmentObserver = Arc::new(|_| {});
    let Some(squash_relationships) = compute_squash_relationships(
        repo_path,
        snapshot,
        &base_branch,
        &base_tip,
        &mut cache,
        cancel,
        observer.unwrap_or(&noop),
    ) else {
        return Vec::new();
    };
    cache.save();
    let mut relationships = RelationshipAccumulator::default();
    for relationship in squash_relationships {
        relationships.insert(&relationship);
    }
    if !cancel.is_cancelled() {
        let cherry = if let Some(observer) = observer {
            compute_cherry_pick_relationships_observed(
                repo_path,
                snapshot,
                &base_branch,
                &base_tip,
                &cache,
                cancel,
                MAX_CHERRY_SOURCE_COMMITS,
                MAX_CHERRY_DESTINATION_SCAN,
                observer,
            )
        } else {
            compute_cherry_pick_relationships(
                repo_path,
                snapshot,
                &base_branch,
                &base_tip,
                &cache,
                cancel,
            )
        };
        for relationship in cherry {
            relationships.insert(&relationship);
        }
    }
    relationships.into_relationships()
}

#[allow(clippy::too_many_arguments)]
fn compute_squash_relationships(
    repo_path: &Path,
    snapshot: &GraphSnapshot,
    base_branch: &str,
    base_tip: &str,
    cache: &mut BranchCache,
    cancel: &EnrichmentCancel,
    observer: &EnrichmentObserver,
) -> Option<Vec<GraphRelationship>> {
    if cancel.is_cancelled() {
        return None;
    }
    let mut jobs = snapshot
        .commits
        .iter()
        .filter(|commit| {
            commit.parents.len() == 1
                && commit.branch.as_ref().is_some_and(|branch| {
                    branch.kind == GraphRefKind::LocalBranch && branch.name == base_branch
                })
        })
        .map(|commit| PatchJob {
            target: PatchTarget::BaseCommit(commit.oid.clone()),
            old_oid: commit.parents[0].clone(),
            new_oid: commit.oid.clone(),
        })
        .collect::<Vec<_>>();

    let mut displayed_branch_names_by_tip = HashMap::<String, Vec<String>>::new();
    for commit in &snapshot.commits {
        if cancel.is_cancelled() {
            return None;
        }
        let names = commit
            .refs
            .iter()
            .filter(|reference| {
                reference.kind == GraphRefKind::LocalBranch && reference.name != base_branch
            })
            .map(|reference| reference.name.clone());
        displayed_branch_names_by_tip
            .entry(commit.oid.clone())
            .or_default()
            .extend(names);
    }
    displayed_branch_names_by_tip.retain(|_, names| {
        names.sort();
        names.dedup();
        !names.is_empty()
    });

    for (tip, source_names) in displayed_branch_names_by_tip {
        if cancel.is_cancelled() {
            return None;
        }
        let merge_base = match displayed_branch_relation(&snapshot.commits, base_tip, &tip) {
            DisplayedBranchRelation::Diverged { merge_base } => merge_base,
            DisplayedBranchRelation::RegularlyMerged | DisplayedBranchRelation::Ineligible => {
                continue;
            }
        };
        let source = SourceTip {
            tip_oid: tip.clone(),
            merge_base,
            source_names,
        };
        jobs.push(PatchJob {
            old_oid: source.merge_base.clone(),
            new_oid: tip,
            target: PatchTarget::BranchTip(source),
        });
    }

    let mut base_oids_by_patch = HashMap::<String, Vec<String>>::new();
    let mut sources_by_patch = HashMap::<String, Vec<SourceTip>>::new();
    // Retained alongside patch IDs so pairs whose patch IDs don't exactly
    // match can still be scored by the Option 6 fuzzy tier below, without a
    // second `git diff` subprocess per job.
    let mut base_diffs = Vec::<(String, Vec<u8>)>::new();
    let mut branch_diffs = Vec::<(SourceTip, Vec<u8>)>::new();
    if cancel.is_cancelled() {
        return None;
    }
    for result in load_patch_ids(repo_path, jobs, cache, cancel, observer)? {
        if cancel.is_cancelled() {
            return None;
        }
        match result.target {
            PatchTarget::BaseCommit(oid) => {
                if let Some(patch_id) = result.patch_id {
                    base_oids_by_patch.entry(patch_id).or_default().push(oid.clone());
                }
                if let Some(diff_text) = result.diff_text {
                    base_diffs.push((oid, diff_text));
                }
            }
            PatchTarget::BranchTip(source) => {
                if let Some(patch_id) = result.patch_id {
                    sources_by_patch.entry(patch_id).or_default().push(source.clone());
                }
                if let Some(diff_text) = result.diff_text {
                    branch_diffs.push((source, diff_text));
                }
            }
        }
    }

    let mut relationships = Vec::<GraphRelationship>::new();
    let mut exact_pairs = HashSet::<(String, String)>::new();
    for (patch_id, sources) in &sources_by_patch {
        if cancel.is_cancelled() {
            return None;
        }
        let Some(base_oids) = base_oids_by_patch.get(patch_id) else {
            continue;
        };
        for destination in base_oids {
            for source in sources {
                if cancel.is_cancelled() {
                    return None;
                }
                exact_pairs.insert((destination.clone(), source.tip_oid.clone()));
                relationships.push(GraphRelationship {
                    kind: RelationshipKind::SquashMerge,
                    matching: RelationshipMatch::Exact,
                    destination_oid: destination.clone(),
                    destination_refs: vec![base_branch.to_owned()],
                    source_oid: source.tip_oid.clone(),
                    source_refs: source.source_names.clone(),
                });
            }
        }
    }
    for (destination, base_diff) in &base_diffs {
        if cancel.is_cancelled() {
            return None;
        }
        for (source, branch_diff) in &branch_diffs {
            if cancel.is_cancelled() {
                return None;
            }
            if exact_pairs.contains(&(destination.clone(), source.tip_oid.clone())) {
                continue;
            }
            let Some(percent) = crate::git::fuzzy_match::score(branch_diff, base_diff)
                .and_then(|score| crate::git::fuzzy_match::classify(&score))
            else {
                continue;
            };
            relationships.push(GraphRelationship {
                kind: RelationshipKind::SquashMerge,
                matching: RelationshipMatch::Fuzzy { similarity_percent: percent },
                destination_oid: destination.clone(),
                destination_refs: vec![base_branch.to_owned()],
                source_oid: source.tip_oid.clone(),
                source_refs: source.source_names.clone(),
            });
        }
    }
    // Patch workers and hash maps finish in arbitrary order; publish stable pairs.
    relationships.sort_by(|left, right| left.key().cmp(&right.key()));
    relationships.dedup_by(|left, right| left.key() == right.key());

    if cancel.is_cancelled() {
        return None;
    }
    if cancel.is_cancelled() {
        return None;
    }
    Some(relationships)
}

type RelationshipKey = (RelationshipKind, String, String);

#[derive(Default)]
struct RelationshipAccumulator(BTreeMap<RelationshipKey, GraphRelationship>);

impl RelationshipAccumulator {
    fn insert(&mut self, incoming: &GraphRelationship) {
        let key = (
            incoming.kind,
            incoming.destination_oid.clone(),
            incoming.source_oid.clone(),
        );
        if let Some(current) = self.0.get_mut(&key) {
            current.matching = current.matching.stronger(incoming.matching);
            current
                .source_refs
                .extend(incoming.source_refs.iter().cloned());
            current
                .destination_refs
                .extend(incoming.destination_refs.iter().cloned());
            current.source_refs.sort();
            current.source_refs.dedup();
            current.destination_refs.sort();
            current.destination_refs.dedup();
        } else {
            let mut relationship = incoming.clone();
            relationship.source_refs.sort();
            relationship.source_refs.dedup();
            relationship.destination_refs.sort();
            relationship.destination_refs.dedup();
            self.0.insert(key, relationship);
        }
    }

    fn into_relationships(self) -> Vec<GraphRelationship> {
        self.0.into_values().collect()
    }
}

/// Merge immutable relationship facts without removing captured provenance.
fn merge_relationships(existing: &mut Vec<GraphRelationship>, incoming: &[GraphRelationship]) {
    let mut relationships = RelationshipAccumulator::default();
    for relationship in existing.iter().chain(incoming) {
        relationships.insert(relationship);
    }
    *existing = relationships.into_relationships();
}

/// Attach each fact to every displayed endpoint, retaining prior facts.
pub fn apply_squash_enrichment(snapshot: &mut GraphSnapshot, updates: &[GraphRelationship]) {
    let mut positions = HashMap::<&str, Vec<usize>>::new();
    for (position, commit) in snapshot.commits.iter().enumerate() {
        positions.entry(&commit.oid).or_default().push(position);
    }
    let mut changes = HashMap::<usize, RelationshipAccumulator>::new();
    for relationship in updates {
        let endpoints = [
            relationship.destination_oid.as_str(),
            relationship.source_oid.as_str(),
        ];
        for (index, oid) in endpoints.iter().enumerate() {
            if index == 1 && endpoints[0] == *oid {
                continue;
            }
            if let Some(indices) = positions.get(*oid) {
                for &position in indices {
                    let relationships = changes.entry(position).or_insert_with(|| {
                        let mut accumulator = RelationshipAccumulator::default();
                        for existing in &snapshot.commits[position].relationships {
                            accumulator.insert(existing);
                        }
                        accumulator
                    });
                    relationships.insert(relationship);
                }
            }
        }
    }
    for (position, relationships) in changes {
        snapshot.commits[position].relationships = relationships.into_relationships();
    }
}

/// Maximum branch-only revwalk yields accepted for one displayed tip.
pub const MAX_CHERRY_SOURCE_COMMITS: usize = 500;
/// Maximum base-only revwalk yields inspected for one displayed tip.
pub const MAX_CHERRY_DESTINATION_SCAN: usize = 2_000;
const CHERRY_PATCH_ID_VERSION: u32 = 1;

/// Patch identity for a single-parent commit; roots, merges and empty diffs have none.
pub(crate) fn git2_patch_id(repo: &git2::Repository, oid: git2::Oid) -> Option<String> {
    let commit = repo.find_commit(oid).ok()?;
    if commit.parent_count() != 1 {
        return None;
    }
    git2_diff_patch(repo, commit.parent_id(0).ok()?, oid).0
}

#[allow(clippy::too_many_arguments)]
fn compute_cherry_pick_relationships(
    repo_path: &Path,
    snapshot: &GraphSnapshot,
    base_branch: &str,
    base_tip: &str,
    cache: &BranchCache,
    cancel: &EnrichmentCancel,
) -> Vec<GraphRelationship> {
    compute_cherry_pick_relationships_with_bounds(
        repo_path,
        snapshot,
        base_branch,
        base_tip,
        cache,
        cancel,
        MAX_CHERRY_SOURCE_COMMITS,
        MAX_CHERRY_DESTINATION_SCAN,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn compute_cherry_pick_relationships_with_bounds(
    repo_path: &Path,
    snapshot: &GraphSnapshot,
    base_branch: &str,
    base_tip: &str,
    cache: &BranchCache,
    cancel: &EnrichmentCancel,
    source_limit: usize,
    destination_limit: usize,
) -> Vec<GraphRelationship> {
    let observer: EnrichmentObserver = Arc::new(|_| {});
    compute_cherry_pick_relationships_observed(
        repo_path,
        snapshot,
        base_branch,
        base_tip,
        cache,
        cancel,
        source_limit,
        destination_limit,
        &observer,
    )
}

struct CherryPatchIds<'a> {
    repo: &'a git2::Repository,
    cache: &'a BranchCache,
    cancel: &'a EnrichmentCancel,
    observer: &'a EnrichmentObserver,
    memo: HashMap<git2::Oid, Option<String>>,
    pending: Vec<(String, Option<String>)>,
}

impl CherryPatchIds<'_> {
    fn load(&mut self, oids: &[git2::Oid]) -> bool {
        if self.cancel.is_cancelled() {
            return false;
        }
        let missing = oids
            .iter()
            .copied()
            .filter(|oid| !self.memo.contains_key(oid))
            .collect::<HashSet<_>>();
        let mut missing = missing.into_iter().collect::<Vec<_>>();
        missing.sort();
        let keys = missing
            .iter()
            .map(|oid| format!("{oid}:v{CHERRY_PATCH_ID_VERSION}"))
            .collect::<Vec<_>>();
        let cached = self.cache.lookup_commit_patch_ids(&keys);
        for (oid, key) in missing.into_iter().zip(keys) {
            if self.cancel.is_cancelled() {
                return false;
            }
            let patch_id = if let Some(value) = cached.get(&key) {
                value.clone()
            } else {
                (self.observer)(EnrichmentStage::CherryPatchCompute);
                if self.cancel.is_cancelled() {
                    return false;
                }
                let value = git2_patch_id(self.repo, oid);
                self.pending.push((key, value.clone()));
                value
            };
            self.memo.insert(oid, patch_id);
        }
        true
    }

    fn flush(&mut self) {
        if !self.cancel.is_cancelled() {
            self.cache.store_commit_patch_ids(&self.pending);
        }
        self.pending.clear();
    }
}

/// Collect only budgeted single-parent commits, polling even on merge yields.
/// Source overflow rejects the entire tip; destination overflow retains its prefix.
fn cherry_walk(
    repo: &git2::Repository,
    push: git2::Oid,
    hide: git2::Oid,
    limit: usize,
    stage: EnrichmentStage,
    cancel: &EnrichmentCancel,
    observer: &EnrichmentObserver,
) -> Option<(Vec<git2::Oid>, bool)> {
    let mut walk = repo.revwalk().ok()?;
    walk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME)
        .ok()?;
    walk.push(push).ok()?;
    walk.hide(hide).ok()?;
    let mut commits = Vec::new();
    for (index, yielded) in walk.enumerate() {
        if cancel.is_cancelled() {
            return None;
        }
        observer(stage);
        if cancel.is_cancelled() {
            return None;
        }
        if index >= limit {
            return Some((commits, true));
        }
        let oid = yielded.ok()?;
        let commit = repo.find_commit(oid).ok()?;
        if commit.parent_count() == 1 {
            commits.push(oid);
        }
    }
    Some((commits, false))
}

#[allow(clippy::too_many_arguments)]
fn compute_cherry_pick_relationships_observed(
    repo_path: &Path,
    snapshot: &GraphSnapshot,
    base_branch: &str,
    base_tip: &str,
    cache: &BranchCache,
    cancel: &EnrichmentCancel,
    source_limit: usize,
    destination_limit: usize,
    observer: &EnrichmentObserver,
) -> Vec<GraphRelationship> {
    let mut relationships = RelationshipAccumulator::default();
    if cancel.is_cancelled() {
        return Vec::new();
    }
    let (Ok(repo), Ok(base)) = (
        git2::Repository::open(repo_path),
        git2::Oid::from_str(base_tip),
    ) else {
        return Vec::new();
    };
    let mut patches = CherryPatchIds {
        repo: &repo,
        cache,
        cancel,
        observer,
        memo: HashMap::new(),
        pending: Vec::new(),
    };
    let mut seen = HashSet::new();
    for commit in &snapshot.commits {
        if cancel.is_cancelled() {
            break;
        }
        let mut names = commit
            .refs
            .iter()
            .filter(|r| r.kind == GraphRefKind::LocalBranch && r.name != base_branch)
            .map(|r| r.name.clone())
            .collect::<Vec<_>>();
        names.sort();
        names.dedup();
        if names.is_empty() {
            continue;
        }
        let Ok(tip) = git2::Oid::from_str(&commit.oid) else {
            continue;
        };
        if !seen.insert(tip) {
            continue;
        }
        let Some((sources, source_overflow)) = cherry_walk(
            &repo,
            tip,
            base,
            source_limit,
            EnrichmentStage::CherrySourceYield,
            cancel,
            observer,
        ) else {
            continue;
        };
        if source_overflow {
            tracing::debug!(tip = %tip, "scan_incomplete: source bound");
            continue;
        }
        if sources.is_empty() {
            continue;
        }
        if !patches.load(&sources) {
            break;
        }
        let mut buckets = HashMap::<String, Vec<git2::Oid>>::new();
        for source in sources {
            if let Some(Some(patch_id)) = patches.memo.get(&source) {
                buckets.entry(patch_id.clone()).or_default().push(source);
            }
        }
        let Some((destinations, destination_overflow)) = cherry_walk(
            &repo,
            base,
            tip,
            destination_limit,
            EnrichmentStage::CherryDestinationYield,
            cancel,
            observer,
        ) else {
            patches.flush();
            continue;
        };
        if destination_overflow {
            tracing::debug!(tip = %tip, "scan_incomplete: destination bound");
        }
        if !patches.load(&destinations) {
            break;
        }
        for destination in destinations {
            if cancel.is_cancelled() {
                break;
            }
            let Some(Some(patch_id)) = patches.memo.get(&destination) else {
                continue;
            };
            let Some(sources) = buckets.get(patch_id) else {
                continue;
            };
            for source in sources {
                if cancel.is_cancelled() {
                    break;
                }
                relationships.insert(&GraphRelationship {
                    kind: RelationshipKind::CherryPick,
                    matching: RelationshipMatch::Exact,
                    destination_oid: destination.to_string(),
                    destination_refs: vec![base_branch.to_owned()],
                    source_oid: source.to_string(),
                    source_refs: names.clone(),
                });
            }
        }
        patches.flush();
    }
    relationships.into_relationships()
}

#[derive(Debug, PartialEq, Eq)]
enum DisplayedBranchRelation {
    RegularlyMerged,
    Diverged { merge_base: String },
    Ineligible,
}

/// Resolve ancestry only through commits already owned by the snapshot. A
/// merge base outside the displayed window is intentionally ineligible rather
/// than triggering an unbounded repository walk during Graph startup.
fn displayed_branch_relation(
    commits: &[GraphCommit],
    base_tip: &str,
    branch_tip: &str,
) -> DisplayedBranchRelation {
    let commits_by_oid = commits
        .iter()
        .map(|commit| (commit.oid.as_str(), commit))
        .collect::<HashMap<_, _>>();
    let base_ancestors = displayed_ancestors(&commits_by_oid, base_tip);
    if base_ancestors.contains(branch_tip) {
        return DisplayedBranchRelation::RegularlyMerged;
    }
    let branch_ancestors = displayed_ancestors(&commits_by_oid, branch_tip);
    let common = base_ancestors
        .intersection(&branch_ancestors)
        .copied()
        .collect::<HashSet<_>>();
    if common.is_empty() {
        return DisplayedBranchRelation::Ineligible;
    }

    let ancestors_by_common = common
        .iter()
        .map(|oid| (*oid, displayed_ancestors(&commits_by_oid, oid)))
        .collect::<HashMap<_, _>>();
    let mut best = common.iter().copied().filter(|candidate| {
        !common.iter().any(|other| {
            other != candidate
                && ancestors_by_common
                    .get(other)
                    .is_some_and(|ancestors| ancestors.contains(candidate))
        })
    });
    let Some(merge_base) = best.next() else {
        return DisplayedBranchRelation::Ineligible;
    };
    if best.next().is_some() {
        return DisplayedBranchRelation::Ineligible;
    }
    DisplayedBranchRelation::Diverged {
        merge_base: merge_base.to_string(),
    }
}

fn displayed_ancestors<'a>(
    commits_by_oid: &HashMap<&'a str, &'a GraphCommit>,
    start: &str,
) -> HashSet<&'a str> {
    let Some(start) = commits_by_oid.get(start) else {
        return HashSet::new();
    };
    let mut ancestors = HashSet::new();
    let mut pending = vec![start.oid.as_str()];
    while let Some(oid) = pending.pop() {
        if !ancestors.insert(oid) {
            continue;
        }
        if let Some(commit) = commits_by_oid.get(oid) {
            pending.extend(
                commit
                    .parents
                    .iter()
                    .filter_map(|parent| commits_by_oid.get(parent.as_str()))
                    .map(|parent| parent.oid.as_str()),
            );
        }
    }
    ancestors
}

/// Like the established squash loader, Graph patch matching uses four workers
/// regardless of CPU count. Each worker runs at most one Git subprocess at a
/// time, so both worker and active-subprocess concurrency are capped at four.
const GRAPH_PATCH_WORKER_COUNT: usize = 4;

/// Bump whenever `compute_patch`'s `git diff`/`git patch-id` invocation
/// (its flags, or what gets hashed) changes in a way that could yield a
/// different (patch_id, diff_text) pair for the same (old_oid, new_oid).
/// Embedded in every `graph_patch` cache key so a version bump
/// invalidates all previously cached entries at once, with no schema
/// migration needed.
const GRAPH_DIFF_VERSION: u32 = 1;

struct PatchJob {
    target: PatchTarget,
    old_oid: String,
    new_oid: String,
}

#[derive(Clone, Debug)]
struct SourceTip {
    tip_oid: String,
    merge_base: String,
    source_names: Vec<String>,
}

enum PatchTarget {
    BaseCommit(String),
    BranchTip(SourceTip),
}

struct PatchResult {
    target: PatchTarget,
    patch_id: Option<String>,
    /// Raw `git diff` stdout for this job, retained alongside the patch-id so
    /// `annotate_possible_squash_merges` can run the Option 6 fuzzy-match tier
    /// on pairs whose patch IDs don't exactly match, without a second `git
    /// diff` subprocess per job. `Some` whenever the diff subprocess produced
    /// non-empty output (regardless of whether patch-id computation itself
    /// succeeded); `None` for empty/net-zero diffs or subprocess failures.
    diff_text: Option<Vec<u8>>,
}

fn load_patch_ids(
    repo_path: &Path,
    jobs: Vec<PatchJob>,
    cache: &mut BranchCache,
    cancel: &EnrichmentCancel,
    observer: &EnrichmentObserver,
) -> Option<Vec<PatchResult>> {
    if cancel.is_cancelled() {
        return None;
    }
    if jobs.is_empty() {
        return Some(Vec::new());
    }

    // Cache-hit fast path: resolve every job whose (old_oid, new_oid)
    // diff is already known, with no subprocess cost, before dispatching
    // the remainder to the worker pool. Mirrors the cache-hit/worker-pool
    // split in `squash_loader::spawn_squash_checker`.
    let mut results = Vec::with_capacity(jobs.len());
    let mut misses = Vec::new();
    for job in jobs {
        if cancel.is_cancelled() {
            return None;
        }
        match cache.lookup_graph_patch(&job.old_oid, &job.new_oid, GRAPH_DIFF_VERSION) {
            Some((patch_id, diff_text)) => results.push(PatchResult {
                target: job.target,
                patch_id,
                diff_text,
            }),
            None => misses.push(job),
        }
    }
    if misses.is_empty() {
        return Some(results);
    }

    // This span measures only uncached patch work: from dispatch through all
    // workers finishing. Cache-only calls return above without emitting it.
    let span = tracing::info_span!(
        "graph_squash_patches",
        jobs = results.len() + misses.len(),
        misses = misses.len()
    );
    let _entered = span.enter();

    let worker_count = misses.len().min(GRAPH_PATCH_WORKER_COUNT);
    let queue = Arc::new(Mutex::new(
        misses.into_iter().collect::<std::collections::VecDeque<_>>(),
    ));
    let (tx, rx) = mpsc::channel();
    let mut handles = Vec::with_capacity(worker_count);
    for _ in 0..worker_count {
        let queue = Arc::clone(&queue);
        let tx = tx.clone();
        let cancel = cancel.clone();
        let observer = Arc::clone(observer);
        let repo_path = repo_path.to_path_buf();
        handles.push(std::thread::spawn(move || {
            while let Some(job) = next_patch_job(&queue, &cancel, &observer) {
                if cancel.is_cancelled() {
                    break;
                }
                let (patch_id, diff_text) = compute_patch(&repo_path, &job.old_oid, &job.new_oid);
                if cancel.is_cancelled() {
                    break;
                }
                let sent = tx.send((
                    job.old_oid,
                    job.new_oid,
                    PatchResult {
                        target: job.target,
                        patch_id,
                        diff_text,
                    },
                ));
                if sent.is_err() {
                    break;
                }
            }
        }));
    }
    drop(tx);

    // Cache owner: only this (calling) thread ever touches `cache`,
    // mirroring squash_loader's single-writer rule.
    for (old_oid, new_oid, result) in rx {
        if cancel.is_cancelled() {
            break;
        }
        cache.insert_graph_patch(
            &old_oid,
            &new_oid,
            GRAPH_DIFF_VERSION,
            result.patch_id.clone(),
            result.diff_text.clone(),
        );
        results.push(result);
    }
    for handle in handles {
        let _ = handle.join();
    }

    if cancel.is_cancelled() {
        None
    } else {
        Some(results)
    }
}

fn next_patch_job(
    queue: &Mutex<std::collections::VecDeque<PatchJob>>,
    cancel: &EnrichmentCancel,
    observer: &EnrichmentObserver,
) -> Option<PatchJob> {
    if cancel.is_cancelled() {
        return None;
    }
    let mut queue = queue.lock().ok()?;
    if cancel.is_cancelled() || queue.is_empty() {
        return None;
    }
    observer(EnrichmentStage::PatchDispatch);
    if cancel.is_cancelled() {
        return None;
    }
    let job = queue.pop_front()?;
    if cancel.is_cancelled() {
        return None;
    }
    Some(job)
}

/// Run a single `git diff` between `old_oid` and `new_oid`, then feed its
/// output through `git patch-id --stable`. Returns `(patch_id, diff_text)`:
/// `diff_text` is the raw diff bytes (for the Option 6 fuzzy-match tier),
/// `patch_id` is the stable patch identity (for the existing exact-match
/// tier). Both share the single `git diff` invocation below rather than
/// re-running it per caller.
fn compute_patch(repo_path: &Path, old_oid: &str, new_oid: &str) -> (Option<String>, Option<Vec<u8>>) {
    let diff = Command::new("git")
        .current_dir(repo_path)
        .args([
            "diff",
            "--binary",
            "--full-index",
            "--no-ext-diff",
            "--no-textconv",
            old_oid,
            new_oid,
            "--",
        ])
        .stdin(Stdio::null())
        .output();
    let Ok(diff) = diff else {
        return (None, None);
    };
    if !diff.status.success() || diff.stdout.is_empty() {
        return (None, None);
    }
    let diff_text = Some(diff.stdout.clone());

    let patch_id = (|| {
        let mut patch_id = Command::new("git")
            .current_dir(repo_path)
            .args(["patch-id", "--stable"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        patch_id.stdin.as_mut()?.write_all(&diff.stdout).ok()?;
        let output = patch_id.wait_with_output().ok()?;
        if !output.status.success() {
            return None;
        }
        String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .next()
            .map(str::to_string)
    })();

    (patch_id, diff_text)
}

/// Compute a stable patch ID and the corresponding diff bytes with libgit2.
///
/// This helper is public only so integration tests can compare its output
/// with Git's CLI implementation.
#[doc(hidden)]
pub fn git2_diff_patch(
    repo: &git2::Repository,
    old: git2::Oid,
    new: git2::Oid,
) -> (Option<String>, Option<Vec<u8>>) {
    let (Ok(old_tree), Ok(new_tree)) = (
        repo.find_commit(old).and_then(|commit| commit.tree()),
        repo.find_commit(new).and_then(|commit| commit.tree()),
    ) else {
        return (None, None);
    };

    let mut options = git2::DiffOptions::new();
    options.show_binary(true).id_abbrev(40);
    let Ok(mut diff) = repo.diff_tree_to_tree(Some(&old_tree), Some(&new_tree), Some(&mut options))
    else {
        return (None, None);
    };
    let mut find_options = git2::DiffFindOptions::new();
    find_options.renames(true);
    if diff.find_similar(Some(&mut find_options)).is_err() || diff.deltas().len() == 0 {
        return (None, None);
    }

    let mut text = Vec::new();
    let printed = diff.print(git2::DiffFormat::Patch, |_, _, line| {
        if matches!(line.origin(), '+' | '-' | ' ') {
            text.push(line.origin() as u8);
        }
        text.extend_from_slice(line.content());
        true
    });
    if printed.is_err() || text.is_empty() {
        return (None, None);
    }

    (diff.patchid(None).ok().map(|id| id.to_string()), Some(text))
}

fn gleisbau_settings(
    include_remotes: bool,
    line_style: GraphLineStyle,
    base_branch: Option<&str>,
) -> Result<Rc<gleisbau::settings::Settings>, String> {
    use gleisbau::print::format::CommitFormat;
    use gleisbau::settings::{
        BranchOrder, BranchSettings, BranchSettingsDef, Characters, MergePatterns, Settings,
    };

    let characters = match line_style {
        GraphLineStyle::Thin => Characters::thin(),
        GraphLineStyle::Round => Characters::round(),
    };

    let mut def = BranchSettingsDef::none();
    if let Some(name) = base_branch {
        let pattern = format!("^{}$", regex::escape(name));
        def.persistence.insert(0, pattern.clone());
        def.order.insert(0, pattern);
    }

    Ok(Rc::new(Settings {
        reverse_commit_order: false,
        debug: false,
        compact: true,
        colored: false,
        include_remote: include_remotes,
        format: CommitFormat::OneLine,
        wrapping: None,
        characters,
        branch_order: BranchOrder::ShortestFirst(true),
        branches: BranchSettings::from(def).map_err(|error| error.to_string())?,
        merge_patterns: MergePatterns::default(),
    }))
}

#[derive(Default)]
struct RefData {
    refs_by_oid: HashMap<String, Vec<GraphRef>>,
    ref_counts: GraphRefCounts,
    branch_labels: HashMap<String, GraphBranchLabel>,
    base_branch: Option<String>,
}

fn collect_ref_data(
    repo_path: &Path,
    include_remotes: bool,
    requested_base: Option<&str>,
) -> Result<RefData, String> {
    let repository =
        git2::Repository::open(repo_path).map_err(|error| error.message().to_string())?;
    let head_shorthand = repository
        .head()
        .ok()
        .and_then(|head| head.shorthand().ok().map(str::to_string));
    let mut data = RefData::default();
    let base_branch = requested_base
        .map(str::to_string)
        .or_else(|| crate::git::branch::detect_base_branch(&repository, None).ok());
    data.base_branch = base_branch.clone();
    let linked_worktrees = crate::git::worktree::branches_checked_out_in_worktrees(repo_path);
    let mut local_refs = Vec::new();
    let mut remote_refs = Vec::new();

    for branch in repository
        .branches(Some(git2::BranchType::Local))
        .map_err(|error| error.message().to_string())?
    {
        let (branch, _) = branch.map_err(|error| error.message().to_string())?;
        let Some(name) = branch
            .name()
            .map_err(|error| error.message().to_string())?
            .map(str::to_string)
        else {
            continue;
        };
        let Some(target) = branch.get().target() else {
            continue;
        };
        let target_oid = target.to_string();
        local_refs.push((name.clone(), target_oid.clone()));
        data.ref_counts.local += 1;
        let label = GraphBranchLabel {
            name: name.clone(),
            target_oid: target_oid.clone(),
            kind: GraphRefKind::LocalBranch,
        };
        data.branch_labels.insert(format!("refs/heads/{name}"), label);
    }

    for branch in repository
        .branches(Some(git2::BranchType::Remote))
        .map_err(|error| error.message().to_string())?
    {
        let (branch, _) = branch.map_err(|error| error.message().to_string())?;
        let Some(name) = branch
            .name()
            .map_err(|error| error.message().to_string())?
            .map(str::to_string)
        else {
            continue;
        };
        if name.ends_with("/HEAD") {
            continue;
        }
        let Some(target) = branch.get().target() else {
            continue;
        };
        let target_oid = target.to_string();
        remote_refs.push((name.clone(), target_oid.clone()));
        if include_remotes {
            data.ref_counts.remote += 1;
            data.branch_labels.insert(
                format!("refs/remotes/{name}"),
                GraphBranchLabel {
                    name: name.clone(),
                    target_oid: target_oid.clone(),
                    kind: GraphRefKind::RemoteBranch,
                },
            );
        }
    }

    for (name, target_oid) in &local_refs {
        let tracking = repository.find_branch(name, git2::BranchType::Local).ok()
            .and_then(|branch| branch.upstream().ok())
            .and_then(|upstream| {
                let upstream_oid = upstream.get().target()?;
                let (ahead, behind) = repository.graph_ahead_behind(git2::Oid::from_str(target_oid).ok()?, upstream_oid).ok()?;
                Some(GraphRefTracking { ahead: ahead.try_into().unwrap_or(u32::MAX), behind: behind.try_into().unwrap_or(u32::MAX) })
            });
        insert_ref(
            &mut data.refs_by_oid,
            target_oid,
            GraphRef {
                name: name.clone(),
                kind: GraphRefKind::LocalBranch,
                has_linked_worktree: linked_worktrees.contains(name),
                is_current: head_shorthand.as_deref() == Some(name.as_str()),
                tracking,
            },
        );
    }

    if include_remotes {
        for (name, target_oid) in &remote_refs {
            insert_ref(
                &mut data.refs_by_oid,
                target_oid,
                GraphRef {
                    name: name.clone(),
                    kind: GraphRefKind::RemoteBranch,
                    has_linked_worktree: false,
                    is_current: false,
                    tracking: None,
                },
            );
        }
    } else {
        // With `include_remotes=false` we deliberately do not pull every
        // remote ref into the snapshot (the git log command in the loaders
        // already limits which commits are visible). However, when a tracked
        // local branch shares an OID with its upstream — e.g. `main` and
        // `origin/main` point at the same commit because nothing diverged —
        // we still want the remote ref attached so the LRT 'R' column shows
        // the cloud icon and the user can see the commit is on the remote.
        // The downstream ref-pane renderer collapses `origin/<X>` into its
        // matching local `<X>` label so no duplicate text appears.
        for (name, target_oid) in &remote_refs {
            let short = remote_short_name(name);
            let matches_local = local_refs.iter().any(|(local_name, _)| local_name == short);
            if !matches_local {
                continue;
            }
            insert_ref(
                &mut data.refs_by_oid,
                target_oid,
                GraphRef {
                    name: name.clone(),
                    kind: GraphRefKind::RemoteBranch,
                    has_linked_worktree: false,
                    is_current: false,
                    tracking: None,
                },
            );
        }
    }

    for name in repository
        .tag_names(None)
        .map_err(|error| error.message().to_string())?
        .iter()
        .filter_map(|name| name.ok().flatten())
    {
        let Ok(target) = repository
            .revparse_single(&format!("refs/tags/{name}"))
            .and_then(|object| object.peel_to_commit())
        else {
            continue;
        };
        insert_ref(
            &mut data.refs_by_oid,
            &target.id().to_string(),
            GraphRef {
                name: name.to_string(),
                kind: GraphRefKind::Tag,
                has_linked_worktree: false,
                is_current: false,
                tracking: None,
            },
        );
    }

    Ok(data)
}

fn remote_short_name(name: &str) -> &str {
    name.split_once('/').map(|(_, short)| short).unwrap_or(name)
}

fn assign_graph_branch_labels(
    commits: &mut [GraphCommit],
    branch_labels: &HashMap<String, GraphBranchLabel>,
    base_branch: Option<&str>,
) {
    for commit in commits.iter_mut() {
        commit.branch = None;
    }
    assign_first_parent_branch_labels(commits, branch_labels, base_branch);
    assign_merge_subject_branch_labels(commits);
}

fn assign_first_parent_branch_labels(
    commits: &mut [GraphCommit],
    branch_labels: &HashMap<String, GraphBranchLabel>,
    base_branch: Option<&str>,
) {
    let index_by_oid = commits
        .iter()
        .enumerate()
        .map(|(index, commit)| (commit.oid.clone(), index))
        .collect::<HashMap<_, _>>();
    let mut labels = branch_labels.values().cloned().collect::<Vec<_>>();
    labels.sort_by(|left, right| {
        let priority = |label: &GraphBranchLabel| {
            if label.kind == GraphRefKind::LocalBranch && Some(label.name.as_str()) == base_branch {
                0
            } else {
                match label.kind {
                    GraphRefKind::LocalBranch => 1,
                    GraphRefKind::RemoteBranch => 2,
                    GraphRefKind::Tag => 3,
                }
            }
        };
        priority(left)
            .cmp(&priority(right))
            .then_with(|| left.name.cmp(&right.name))
    });
    for label in labels {
        let mut oid = label.target_oid.clone();
        let mut visited = HashSet::new();
        while visited.insert(oid.clone()) {
            let Some(&index) = index_by_oid.get(&oid) else {
                break;
            };
            if commits[index].branch.is_none() {
                commits[index].branch = Some(label.clone());
            }
            let Some(parent) = commits[index].parents.first() else {
                break;
            };
            oid = parent.clone();
        }
    }
}

fn assign_merge_subject_branch_labels(commits: &mut [GraphCommit]) {
    let index_by_oid = commits
        .iter()
        .enumerate()
        .map(|(index, commit)| (commit.oid.clone(), index))
        .collect::<HashMap<_, _>>();

    for merge_index in 0..commits.len() {
        let Some(branch_name) = parse_merge_branch_name(&commits[merge_index].summary) else {
            continue;
        };
        let Some(first_parent) = commits[merge_index].parents.first() else {
            continue;
        };
        let Some(second_parent) = commits[merge_index].parents.get(1) else {
            continue;
        };

        let receiving_branch_history = first_parent_history(commits, &index_by_oid, first_parent);
        let label = GraphBranchLabel {
            name: branch_name,
            target_oid: second_parent.clone(),
            kind: GraphRefKind::LocalBranch,
        };
        let mut oid = second_parent.clone();
        let mut visited = HashSet::new();
        while visited.insert(oid.clone()) {
            if receiving_branch_history.contains(&oid) {
                break;
            }
            let Some(&index) = index_by_oid.get(&oid) else {
                break;
            };
            if commits[index].branch.is_none() {
                commits[index].branch = Some(label.clone());
            }
            let Some(parent) = commits[index].parents.first() else {
                break;
            };
            oid = parent.clone();
        }
    }
}

fn first_parent_history(
    commits: &[GraphCommit],
    index_by_oid: &HashMap<String, usize>,
    start_oid: &str,
) -> HashSet<String> {
    let mut history = HashSet::new();
    let mut oid = start_oid.to_string();
    while history.insert(oid.clone()) {
        let Some(&index) = index_by_oid.get(&oid) else {
            break;
        };
        let Some(parent) = commits[index].parents.first() else {
            break;
        };
        oid = parent.clone();
    }
    history
}

fn parse_merge_branch_name(summary: &str) -> Option<String> {
    let marker = "Merge branch '";
    let start = summary.find(marker)? + marker.len();
    let rest = &summary[start..];
    let end = rest.find('\'')?;
    let name = &rest[..end];
    (!name.is_empty()).then(|| name.to_string())
}

fn insert_ref(refs_by_oid: &mut HashMap<String, Vec<GraphRef>>, oid: &str, reference: GraphRef) {
    refs_by_oid
        .entry(oid.to_string())
        .or_default()
        .push(reference);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn squash(
        dest: &str,
        source: &str,
        names: &[&str],
        matching: RelationshipMatch,
    ) -> GraphRelationship {
        GraphRelationship {
            kind: RelationshipKind::SquashMerge,
            matching,
            destination_oid: dest.into(),
            destination_refs: vec!["main".into()],
            source_oid: source.into(),
            source_refs: names.iter().map(|n| n.to_string()).collect(),
        }
    }

    #[test]
    fn squash_confidence_prefers_exact_and_unions_its_sources() {
        let commit = GraphCommit {
            oid: "d".into(),
            relationships: vec![
                squash("d", "t1", &["feature/b"], RelationshipMatch::Exact),
                squash("d", "t2", &["feature/a"], RelationshipMatch::Exact),
                squash(
                    "d",
                    "t3",
                    &["feature/c"],
                    RelationshipMatch::Fuzzy {
                        similarity_percent: 97,
                    },
                ),
            ],
            ..GraphCommit::default()
        };
        let confidence = commit.squash_match_confidence().unwrap();
        assert_eq!(confidence.similarity_percent, 100);
        assert_eq!(confidence.sources, ["feature/a", "feature/b"]);
        assert!(commit.is_possible_squash_merge());
        assert_eq!(commit.fuzzy_squash_match(), None);
    }

    #[test]
    fn squash_confidence_reports_best_fuzzy_source() {
        let commit = GraphCommit {
            oid: "d".into(),
            relationships: vec![
                squash(
                    "d",
                    "t1",
                    &["feature/low"],
                    RelationshipMatch::Fuzzy {
                        similarity_percent: 80,
                    },
                ),
                squash(
                    "d",
                    "t2",
                    &["feature/high"],
                    RelationshipMatch::Fuzzy {
                        similarity_percent: 97,
                    },
                ),
            ],
            ..GraphCommit::default()
        };
        assert_eq!(
            commit.squash_match_confidence(),
            Some(SquashMatchConfidence {
                sources: vec!["feature/high".into()],
                similarity_percent: 97
            })
        );
        assert!(!commit.is_possible_squash_merge());
        assert_eq!(
            commit.fuzzy_squash_match(),
            Some(FuzzySquashMatch {
                similarity_percent: 97
            })
        );
    }

    #[test]
    fn squash_confidence_ignores_relationships_where_commit_is_the_source() {
        let commit = GraphCommit {
            oid: "t1".into(),
            relationships: vec![squash("d", "t1", &["feature/x"], RelationshipMatch::Exact)],
            ..GraphCommit::default()
        };
        assert_eq!(commit.squash_match_confidence(), None);
    }

    #[test]
    fn stronger_match_prefers_exact_then_higher_percent() {
        use RelationshipMatch::*;
        assert_eq!(
            Exact.stronger(Fuzzy {
                similarity_percent: 99
            }),
            Exact
        );
        assert_eq!(
            Fuzzy {
                similarity_percent: 80
            }
            .stronger(Fuzzy {
                similarity_percent: 90
            }),
            Fuzzy {
                similarity_percent: 90
            }
        );
    }


    #[test]
    fn enrichment_cancel_tracks_latest_generation() {
        let latest = Arc::new(AtomicU64::new(3));
        let cancel = EnrichmentCancel::new(Arc::clone(&latest), 3);
        assert!(!cancel.is_cancelled());
        latest.store(4, Ordering::Release);
        assert!(cancel.is_cancelled());
        assert!(!EnrichmentCancel::never().is_cancelled());
    }

    fn enrichment_git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap().trim().to_owned()
    }

    fn enrichment_fixture() -> (tempfile::TempDir, GraphSnapshot, CacheRoot) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        enrichment_git(path, &["init", "-b", "main"]);
        enrichment_git(path, &["config", "user.name", "Test"]);
        enrichment_git(path, &["config", "user.email", "test@example.com"]);
        enrichment_git(path, &["commit", "--allow-empty", "-m", "root"]);
        enrichment_git(path, &["checkout", "-b", "feature"]);
        for index in 0..3 {
            let name = format!("feature-{index}.txt");
            std::fs::write(path.join(&name), format!("content {index}\n")).unwrap();
            enrichment_git(path, &["add", &name]);
            enrichment_git(path, &["commit", "-m", &name]);
        }
        enrichment_git(path, &["checkout", "main"]);
        enrichment_git(path, &["merge", "--squash", "feature"]);
        enrichment_git(path, &["commit", "-m", "squash landing"]);
        let cache_root = CacheRoot::at(path.join("cache"));
        let snapshot = load_graph(
            path,
            GraphLoadOptions {
                base_branch: Some("main".into()),
                cache_root: cache_root.clone(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            enrichment_git(path, &["cherry", "main", "feature"])
                .lines()
                .count(),
            3
        );
        (dir, snapshot, cache_root)
    }

    fn assert_started_enrichment_cancelled(stage: EnrichmentStage) {
        let (dir, snapshot, cache_root) = enrichment_fixture();
        let latest = Arc::new(AtomicU64::new(1));
        let cancel = EnrichmentCancel::new(Arc::clone(&latest), 1);
        let (started_tx, started_rx) = mpsc::sync_channel(0);
        let (resume_tx, resume_rx) = mpsc::sync_channel(0);
        let resume_rx = Mutex::new(resume_rx);
        let observed = Arc::new(AtomicU64::new(0));
        let observed_worker = Arc::clone(&observed);
        let rx = spawn_enrichment_with_observer(
            snapshot,
            dir.path().to_path_buf(),
            Some("main".into()),
            1,
            cache_root,
            cancel,
            move |current| {
                if current == stage {
                    observed_worker.fetch_add(1, Ordering::Relaxed);
                    started_tx.send(()).unwrap();
                    resume_rx.lock().unwrap().recv().unwrap();
                }
            },
        );
        let timeout = std::time::Duration::from_secs(60);
        started_rx
            .recv_timeout(timeout)
            .expect("worker reached cancellation boundary");
        latest.store(2, Ordering::Release);
        resume_tx.send(()).unwrap();
        assert_eq!(
            rx.recv_timeout(timeout),
            Err(mpsc::RecvTimeoutError::Disconnected)
        );
        assert_eq!(
            observed.load(Ordering::Relaxed),
            1,
            "no further work at this boundary"
        );
    }

    #[test]
    fn started_enrichment_superseded_during_patch_dispatch_publishes_nothing() {
        assert_started_enrichment_cancelled(EnrichmentStage::PatchDispatch);
    }

    #[test]
    fn started_enrichment_superseded_during_cherry_source_revwalk_publishes_nothing() {
        assert_started_enrichment_cancelled(EnrichmentStage::CherrySourceYield);
    }

    #[test]
    fn repository_state_uses_canonical_typed_refs_and_observes_head_changes() {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let mut index = repo.index().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let sig = git2::Signature::now("test", "test@example.com").unwrap();
        let oid = repo.commit(Some("refs/heads/main"), &sig, &sig, "root", &tree, &[]).unwrap();
        repo.reference("refs/heads/feature/foo", oid, true, "test").unwrap();
        repo.reference("refs/remotes/origin/feature/foo", oid, true, "test").unwrap();
        repo.reference("refs/remotes/origin/origin/topic", oid, true, "test").unwrap();
        repo.reference("refs/remotes/upstream/topic", oid, true, "test").unwrap();
        repo.reference("refs/tags/feature/foo", oid, true, "test").unwrap();
        repo.set_head("refs/heads/main").unwrap();

        let before = GraphRepositoryState::capture(dir.path()).unwrap();
        assert!(before.refs.keys().any(|id| id.full_name == "refs/heads/feature/foo"));
        assert!(before.refs.keys().any(|id| id.full_name == "refs/remotes/origin/feature/foo"));
        assert!(before.refs.keys().any(|id| id.full_name == "refs/tags/feature/foo"));
        assert_eq!(before.head_ref.as_deref(), Some("refs/heads/main"));

        repo.set_head("refs/heads/feature/foo").unwrap();
        repo.find_reference("refs/remotes/origin/origin/topic").unwrap().delete().unwrap();
        let after = GraphRepositoryState::capture(dir.path()).unwrap();
        let delta = GraphRepositoryDelta::between(before, after);
        assert!(delta.head_changed);
        assert!(delta.added_refs.is_empty());
        assert!(delta.removed_refs.iter().any(|(id, _)| id.full_name == "refs/remotes/origin/origin/topic"));
        assert!(!delta.removed_refs.iter().any(|(id, _)| id.full_name == "refs/remotes/upstream/topic"));
    }

    #[test]
    fn incremental_update_adds_new_tip_history_without_dropping_cached_commits() {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let tree_id = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let sig = git2::Signature::now("test", "test@example.com").unwrap();
        let root = repo.commit(Some("refs/heads/main"), &sig, &sig, "root", &tree, &[]).unwrap();
        repo.set_head("refs/heads/main").unwrap();
        let options = GraphLoadOptions { max_count: 10, ..GraphLoadOptions::default() };
        let snapshot = load_graph(dir.path(), options.clone()).unwrap();
        let before = GraphRepositoryState::capture(dir.path()).unwrap();
        let parent = repo.find_commit(root).unwrap();
        let next = repo.commit(None, &sig, &sig, "feature", &tree, &[&parent]).unwrap();
        repo.reference("refs/heads/feature/new", next, true, "test").unwrap();
        let after = GraphRepositoryState::capture(dir.path()).unwrap();
        let updated = update_graph_incrementally(
            dir.path(), snapshot, &GraphRepositoryDelta::between(before, after), options,
        ).unwrap();
        assert!(updated.commits.iter().any(|commit| commit.oid == next.to_string()));
        assert!(updated.commits.iter().any(|commit| commit.oid == root.to_string()));
        assert!(updated.commits.iter().flat_map(|commit| &commit.refs).any(|reference| reference.name == "feature/new"));
    }

    #[test]
    fn remote_inclusive_incremental_update_keeps_history_from_additional_roots() {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let tree_id = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let sig = git2::Signature::now("test", "test@example.com").unwrap();
        let main = repo.commit(Some("refs/heads/main"), &sig, &sig, "main root", &tree, &[]).unwrap();
        let stash = main;
        repo.reference("refs/stash", stash, true, "test stash").unwrap();
        repo.set_head("refs/heads/main").unwrap();
        let options = GraphLoadOptions { max_count: 10, include_remotes: true, ..GraphLoadOptions::default() };
        let before = GraphRepositoryState::capture(dir.path()).unwrap();
        assert_eq!(before.additional_roots.get("refs/stash"), Some(&stash.to_string()));
        let snapshot = load_graph(dir.path(), options.clone()).unwrap();

        let parent = repo.find_commit(main).unwrap();
        let next = repo.commit(None, &sig, &sig, "new branch tip", &tree, &[&parent]).unwrap();
        repo.reference("refs/heads/feature/new", next, true, "test").unwrap();
        let after = GraphRepositoryState::capture(dir.path()).unwrap();
        let updated = update_graph_incrementally(
            dir.path(), snapshot, &GraphRepositoryDelta::between(before, after), options,
        ).unwrap();

        assert!(updated.commits.iter().any(|commit| commit.oid == stash.to_string()));
        assert!(updated.commits.iter().any(|commit| commit.oid == next.to_string()));
    }

    #[test]
    fn local_tracking_metadata_uses_configured_upstream_not_matching_short_name() {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let tree_id = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let sig = git2::Signature::now("test", "test@example.com").unwrap();
        let root = repo.commit(Some("refs/heads/main"), &sig, &sig, "root", &tree, &[]).unwrap();
        let parent = repo.find_commit(root).unwrap();
        let local = repo.commit(Some("refs/heads/topic"), &sig, &sig, "local", &tree, &[&parent]).unwrap();
        let origin = repo.commit(None, &sig, &sig, "origin", &tree, &[&parent]).unwrap();
        repo.reference("refs/remotes/origin/topic", origin, true, "test").unwrap();
        repo.reference("refs/remotes/upstream/topic", root, true, "test").unwrap();
        repo.remote("upstream", "https://example.invalid/upstream.git").unwrap();
        repo.find_branch("topic", git2::BranchType::Local).unwrap().set_upstream(Some("upstream/topic")).unwrap();
        repo.set_head("refs/heads/topic").unwrap();

        let refs = collect_ref_data(dir.path(), true, Some("main")).unwrap();
        let local_ref = refs.refs_by_oid.get(&local.to_string()).unwrap().iter()
            .find(|reference| reference.kind == GraphRefKind::LocalBranch).unwrap();
        let tracking = local_ref.tracking.as_ref().unwrap();
        assert_eq!((tracking.ahead, tracking.behind), (1, 0));
    }

    fn commit(oid: &str, parents: &[&str]) -> GraphCommit {
        GraphCommit {
            oid: oid.into(),
            summary: oid.into(),
            parents: parents.iter().map(|parent| (*parent).into()).collect(),
            ..GraphCommit::default()
        }
    }

    #[test]
    fn graph_commit_default_has_empty_author_fields_and_unknown_date() {
        let c = GraphCommit::default();
        assert_eq!(c.author_name, "");
        assert_eq!(c.author_email, "");
        assert_eq!(c.authored_at, None);
    }

    #[test]
    fn git_cli_loader_parses_author_and_author_date_from_format() {
        use crate::git::graph::{load_graph_with_squash_annotations, GraphLoadOptions};

        // Build a repo where gleisbau fails (shallow). Forces CLI fallback.
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::process::Command::new("git")
            .args(["init", "-q", "-b", "main"])
            .current_dir(dir)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "cli@example.com"])
            .current_dir(dir)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "CLI Bot"])
            .current_dir(dir)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "--allow-empty", "-q", "-m", "tip"])
            .current_dir(dir)
            .output()
            .unwrap();
        let head = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(dir)
            .output()
            .unwrap();
        let head_oid = String::from_utf8(head.stdout).unwrap().trim().to_string();
        std::fs::write(dir.join(".git/shallow"), format!("{head_oid}\n")).unwrap();

        let test_cache = tempfile::tempdir().unwrap();
        let snapshot = load_graph_with_squash_annotations(
            dir,
            GraphLoadOptions {
                cache_root: CacheRoot::at(test_cache.path()),
                ..GraphLoadOptions::default()
            },
        )
        .unwrap();
        assert!(matches!(
            snapshot.source,
            crate::git::graph::GraphSource::GitCliFallback { .. }
        ));
        let tip = snapshot.commits.last().expect("graph non-empty");
        assert_eq!(tip.author_name, "CLI Bot");
        assert_eq!(tip.author_email, "cli@example.com");
        assert!(tip.authored_at.is_some());
    }

    #[test]
    fn gleisbau_loader_populates_author_and_author_date() {
        use crate::git::graph::{load_graph_with_squash_annotations, GraphLoadOptions};

        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let init = std::process::Command::new("git")
            .args(["init", "-q", "-b", "main"])
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(init.status.success(), "git init failed");
        std::process::Command::new("git")
            .args(["config", "user.email", "gleisbau@example.com"])
            .current_dir(dir)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Gleisbau Bot"])
            .current_dir(dir)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "--allow-empty", "-q", "-m", "tip"])
            .current_dir(dir)
            .output()
            .unwrap();

        let test_cache = tempfile::tempdir().unwrap();
        let snapshot = load_graph_with_squash_annotations(
            dir,
            GraphLoadOptions {
                cache_root: CacheRoot::at(test_cache.path()),
                ..GraphLoadOptions::default()
            },
        )
        .unwrap();
        assert!(matches!(
            snapshot.source,
            crate::git::graph::GraphSource::Gleisbau
        ));
        let tip = snapshot.commits.last().expect("graph non-empty");
        assert_eq!(tip.author_name, "Gleisbau Bot");
        assert_eq!(tip.author_email, "gleisbau@example.com");
        assert!(tip.authored_at.is_some());
    }

    #[test]
    fn git_cli_loader_preserves_control_characters_in_author_fields() {
        use crate::git::graph::{load_graph_with_squash_annotations, GraphLoadOptions};

        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let init = std::process::Command::new("git")
            .args(["init", "-q", "-b", "main"])
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(init.status.success(), "git init failed");

        let author_name = "CLI\x1fBot";
        let author_email = "cli\x1f@example.com";
        for (key, value) in [("user.email", author_email), ("user.name", author_name)] {
            let configured = std::process::Command::new("git")
                .args(["config", key, value])
                .current_dir(dir)
                .output()
                .unwrap();
            assert!(configured.status.success(), "git config {key} failed");
        }
        let committed = std::process::Command::new("git")
            .args(["commit", "--allow-empty", "-q", "-m", "tip"])
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(committed.status.success(), "git commit failed");

        let head = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(dir)
            .output()
            .unwrap();
        let head_oid = String::from_utf8(head.stdout).unwrap().trim().to_string();
        std::fs::write(dir.join(".git/shallow"), format!("{head_oid}\n")).unwrap();

        let test_cache = tempfile::tempdir().unwrap();
        let snapshot = load_graph_with_squash_annotations(
            dir,
            GraphLoadOptions {
                cache_root: CacheRoot::at(test_cache.path()),
                ..GraphLoadOptions::default()
            },
        )
        .unwrap();
        assert!(matches!(
            snapshot.source,
            crate::git::graph::GraphSource::GitCliFallback { .. }
        ));
        let tip = snapshot.commits.last().expect("graph non-empty");
        assert_eq!(tip.author_name, author_name);
        assert_eq!(tip.author_email, author_email);
    }

    #[test]
    fn round_line_style_uses_gleisbau_round_characters() {
        use gleisbau::settings::Characters;

        let settings = gleisbau_settings(false, GraphLineStyle::Round, None).unwrap();

        assert_eq!(settings.characters.chars, Characters::round().chars);
    }

    #[test]
    fn gleisbau_settings_with_base_branch_puts_base_in_order_group_zero() {
        let settings = gleisbau_settings(false, GraphLineStyle::Thin, Some("main")).unwrap();

        assert_eq!(settings.branches.order[0].as_str(), "^main$");
    }

    #[test]
    fn gleisbau_settings_without_base_branch_keeps_none_semantics() {
        let settings = gleisbau_settings(false, GraphLineStyle::Thin, None).unwrap();

        assert!(settings.branches.order.is_empty());
    }

    #[test]
    fn gleisbau_settings_with_special_chars_escapes_base_branch_name() {
        let settings =
            gleisbau_settings(false, GraphLineStyle::Thin, Some("release/1.0")).unwrap();

        let pattern = &settings.branches.order[0];
        assert!(pattern.is_match("release/1.0"));
        assert!(!pattern.is_match("release/10"));
    }

    #[test]
    fn symbol_sets_only_select_rounded_lines_for_powerline() {
        assert_eq!(
            GraphLineStyle::from_symbol_name("powerline"),
            GraphLineStyle::Round
        );
        assert_eq!(
            GraphLineStyle::from_symbol_name("unicode"),
            GraphLineStyle::Thin
        );
        assert_eq!(
            GraphLineStyle::from_symbol_name("ascii"),
            GraphLineStyle::Thin
        );
    }

    #[test]
    fn displayed_history_bounds_branch_relationships() {
        let commits = vec![
            commit("base-tip", &["base-parent"]),
            commit("branch-tip", &["shared"]),
            commit("base-parent", &["shared"]),
            commit("shared", &["root"]),
            commit("root", &[]),
        ];
        assert_eq!(
            displayed_branch_relation(&commits, "base-tip", "branch-tip"),
            DisplayedBranchRelation::Diverged {
                merge_base: "shared".into()
            }
        );

        let regular = vec![
            commit("base-tip", &["branch-tip"]),
            commit("branch-tip", &["root"]),
            commit("root", &[]),
        ];
        assert_eq!(
            displayed_branch_relation(&regular, "base-tip", "branch-tip"),
            DisplayedBranchRelation::RegularlyMerged
        );

        let truncated = vec![
            commit("base-tip", &["missing-base-parent"]),
            commit("branch-tip", &["missing-branch-parent"]),
        ];
        assert_eq!(
            displayed_branch_relation(&truncated, "base-tip", "branch-tip"),
            DisplayedBranchRelation::Ineligible
        );
    }

    fn relationship_snapshot(commits: Vec<GraphCommit>) -> GraphSnapshot {
        GraphSnapshot {
            source: GraphSource::Gleisbau,
            commits,
            lines: vec![],
            ref_counts: GraphRefCounts::default(),
            max_count: 50,
            includes_remotes: false,
            generation: None,
        }
    }

    #[test]
    fn enrichment_never_erases_and_attaches_every_endpoint() {
        let existing = squash(
            "d",
            "s",
            &["feature/old"],
            RelationshipMatch::Fuzzy {
                similarity_percent: 80,
            },
        );
        let mut snapshot = relationship_snapshot(vec![
            GraphCommit {
                oid: "d".into(),
                relationships: vec![existing.clone()],
                ..Default::default()
            },
            GraphCommit {
                oid: "s".into(),
                ..Default::default()
            },
        ]);
        apply_squash_enrichment(&mut snapshot, &[]);
        assert_eq!(snapshot.commits[0].relationships.as_slice(), std::slice::from_ref(&existing));
        let fresh = GraphRelationship {
            matching: RelationshipMatch::Exact,
            destination_refs: vec!["develop".into(), "develop".into()],
            source_refs: vec!["feature/new".into()],
            ..existing.clone()
        };
        apply_squash_enrichment(&mut snapshot, std::slice::from_ref(&fresh));
        let merged = &snapshot.commits[0].relationships[0];
        assert_eq!(merged.matching, RelationshipMatch::Exact);
        assert_eq!(merged.source_refs, ["feature/new", "feature/old"]);
        assert_eq!(merged.destination_refs, ["develop", "main"]);
        let mut normalized = fresh;
        normalized.destination_refs.dedup();
        assert_eq!(snapshot.commits[1].relationships, [normalized]);
        apply_squash_enrichment(&mut snapshot, &[existing]);
        assert_eq!(
            snapshot.commits[0].relationships[0].matching,
            RelationshipMatch::Exact
        );
        assert_eq!(
            snapshot.commits[0].relationships,
            snapshot.commits[1].relationships
        );
        let before = snapshot.clone();
        apply_squash_enrichment(&mut snapshot, &[]);
        assert_eq!(snapshot, before);
        assert!(!snapshot.commits[1].is_possible_squash_merge());
    }

    fn native_commit(
        repo: &git2::Repository,
        parent: Option<git2::Oid>,
        files: &[(&str, &str)],
        label: &str,
    ) -> git2::Oid {
        let mut builder = repo.treebuilder(None).unwrap();
        for (name, content) in files {
            let blob = repo.blob(content.as_bytes()).unwrap();
            builder.insert(name, blob, 0o100644).unwrap();
        }
        let tree_id = builder.write().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let sig = git2::Signature::now("test", "test@example.com").unwrap();
        let parents = parent
            .map(|id| repo.find_commit(id).unwrap())
            .into_iter()
            .collect::<Vec<_>>();
        let refs = parents.iter().collect::<Vec<_>>();
        repo.commit(None, &sig, &sig, label, &tree, &refs).unwrap()
    }

    fn native_pair_fixture() -> (
        tempfile::TempDir,
        git2::Repository,
        git2::Oid,
        git2::Oid,
        GraphSnapshot,
        CacheRoot,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let root = native_commit(&repo, None, &[], "root");
        let source = native_commit(&repo, Some(root), &[("picked", "picked\n")], "source");
        let base = native_commit(
            &repo,
            Some(root),
            &[("unrelated", "base\n")],
            "advance base",
        );
        let destination = native_commit(
            &repo,
            Some(base),
            &[("unrelated", "base\n"), ("picked", "picked\n")],
            "destination",
        );
        repo.reference("refs/heads/main", destination, true, "fixture")
            .unwrap();
        repo.reference("refs/heads/feature", source, true, "fixture")
            .unwrap();
        let reference = |name: &str| GraphRef {
            name: name.into(),
            kind: GraphRefKind::LocalBranch,
            has_linked_worktree: false,
            is_current: false,
            tracking: None,
        };
        // Deliberately omit the merge base and historical destination from the window.
        let snapshot = relationship_snapshot(vec![
            GraphCommit {
                oid: destination.to_string(),
                refs: vec![reference("main")],
                ..Default::default()
            },
            GraphCommit {
                oid: source.to_string(),
                refs: vec![reference("feature")],
                ..Default::default()
            },
        ]);
        let cache_root = CacheRoot::at(dir.path().join("isolated/cache"));
        (dir, repo, source, destination, snapshot, cache_root)
    }

    #[test]
    fn cherry_scan_bounds_count_all_yields_and_keep_complete_pairs() {
        let (dir, repo, source, destination, mut snapshot, root) = native_pair_fixture();
        let cache = BranchCache::load_for_base(dir.path(), "main", &root);
        let scan = |snapshot: &GraphSnapshot, source_limit, destination_limit| {
            compute_cherry_pick_relationships_with_bounds(
                dir.path(),
                snapshot,
                "main",
                &snapshot.commits[0].oid,
                &cache,
                &EnrichmentCancel::never(),
                source_limit,
                destination_limit,
            )
        };
        assert_eq!(
            scan(&snapshot, 1, 1).len(),
            1,
            "exact bounds accept the complete pair outside displayed merge base"
        );
        assert!(
            scan(&snapshot, 0, 10).is_empty(),
            "source overflow skips entire tip"
        );
        assert!(scan(&snapshot, 1, 0).is_empty());
        let mut base_tip = destination;
        for index in 0..2_000 {
            base_tip = native_commit(
                &repo,
                Some(base_tip),
                &[("unrelated", "base\n"), ("picked", "picked\n")],
                &format!("base filler {index}"),
            );
        }
        snapshot.commits[0].oid = base_tip.to_string();
        assert!(
            scan(&snapshot, 500, MAX_CHERRY_DESTINATION_SCAN).is_empty(),
            "destination 2001 is outside production bound"
        );
        let pairs = scan(&snapshot, 500, 2_001);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].destination_oid, destination.to_string());
        let mut source_tip = source;
        for index in 0..499 {
            source_tip = native_commit(
                &repo,
                Some(source_tip),
                &[("picked", "picked\n")],
                &format!("source filler {index}"),
            );
        }
        snapshot.commits[1].oid = source_tip.to_string();
        assert_eq!(scan(&snapshot, MAX_CHERRY_SOURCE_COMMITS, 2_002).len(), 1);
        source_tip = native_commit(
            &repo,
            Some(source_tip),
            &[("picked", "picked\n")],
            "source 501",
        );
        snapshot.commits[1].oid = source_tip.to_string();
        assert!(scan(&snapshot, MAX_CHERRY_SOURCE_COMMITS, 2_002).is_empty());
        let mut valid = snapshot.commits[1].clone();
        valid.oid = source.to_string();
        valid.refs[0].name = "feature/valid".into();
        snapshot.commits.insert(1, valid);
        let retained = scan(&snapshot, MAX_CHERRY_SOURCE_COMMITS, 2_002);
        assert_eq!(
            retained.len(),
            1,
            "over-bound later tip does not erase earlier complete pairs"
        );
        assert_eq!(retained[0].source_refs, ["feature/valid"]);
        let fresh_root = CacheRoot::at(dir.path().join("fresh-bound-cache"));
        let fresh_cache = BranchCache::load_for_base(dir.path(), "main", &fresh_root);
        assert!(compute_cherry_pick_relationships_with_bounds(
            dir.path(),
            &snapshot,
            "main",
            &snapshot.commits[0].oid,
            &fresh_cache,
            &EnrichmentCancel::never(),
            500,
            2_000
        )
        .is_empty());
        assert!(
            !fresh_cache
                .lookup_commit_patch_ids(&[format!("{destination}:v1")])
                .contains_key(&format!("{destination}:v1")),
            "no patch computation beyond destination budget"
        );
    }

    #[test]
    fn cherry_scan_preserves_equal_patch_source_buckets_and_ref_unions() {
        let (dir, repo, first_source, first_dest, mut snapshot, root) = native_pair_fixture();
        let remove_source = native_commit(&repo, Some(first_source), &[], "source revert");
        let second_source = native_commit(
            &repo,
            Some(remove_source),
            &[("picked", "picked\n")],
            "source repick",
        );
        let remove_dest = native_commit(
            &repo,
            Some(first_dest),
            &[("unrelated", "base\n")],
            "destination revert",
        );
        let second_dest = native_commit(
            &repo,
            Some(remove_dest),
            &[("unrelated", "base\n"), ("picked", "picked\n")],
            "destination repick",
        );
        snapshot.commits[0].oid = second_dest.to_string();
        snapshot.commits[1].oid = second_source.to_string();
        let mut older = snapshot.commits[1].clone();
        older.oid = first_source.to_string();
        older.refs[0].name = "feature/older".into();
        snapshot.commits.push(older);
        let cache = BranchCache::load_for_base(dir.path(), "main", &root);
        let pairs = compute_cherry_pick_relationships_with_bounds(
            dir.path(),
            &snapshot,
            "main",
            &second_dest.to_string(),
            &cache,
            &EnrichmentCancel::never(),
            500,
            2_000,
        );
        let insertions = pairs
            .iter()
            .filter(|r| {
                r.source_oid == first_source.to_string()
                    || r.source_oid == second_source.to_string()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            insertions.len(),
            4,
            "two equal source patches times two destinations"
        );
        for source in [first_source, second_source] {
            assert_eq!(
                insertions
                    .iter()
                    .filter(|r| r.source_oid == source.to_string())
                    .map(|r| r.destination_oid.clone())
                    .collect::<std::collections::BTreeSet<_>>(),
                [first_dest.to_string(), second_dest.to_string()]
                    .into_iter()
                    .collect()
            );
        }
        for pair in insertions
            .iter()
            .filter(|r| r.source_oid == first_source.to_string())
        {
            assert_eq!(pair.source_refs, ["feature", "feature/older"]);
        }
        assert!(
            pairs.windows(2).all(|w| w[0].key() < w[1].key()),
            "deduplicated deterministic order"
        );
    }

    fn native_merge(
        repo: &git2::Repository,
        first: git2::Oid,
        other: git2::Oid,
        label: &str,
    ) -> git2::Oid {
        let first = repo.find_commit(first).unwrap();
        let other = repo.find_commit(other).unwrap();
        let tree = first.tree().unwrap();
        let sig = git2::Signature::now("test", "test@example.com").unwrap();
        repo.commit(None, &sig, &sig, label, &tree, &[&first, &other])
            .unwrap()
    }

    fn merge_pair_fixture() -> (tempfile::TempDir, GraphSnapshot, CacheRoot) {
        let (dir, repo, source, destination, mut snapshot, root) = native_pair_fixture();
        let ancestor = repo.find_commit(source).unwrap().parent_id(0).unwrap();
        snapshot.commits[1].oid = native_merge(&repo, source, ancestor, "source merge").to_string();
        snapshot.commits[0].oid =
            native_merge(&repo, destination, ancestor, "destination merge").to_string();
        (dir, snapshot, root)
    }

    #[test]
    fn cherry_scan_merge_yields_consume_source_and_destination_budgets() {
        let (dir, snapshot, root) = merge_pair_fixture();
        let cache = BranchCache::load_for_base(dir.path(), "main", &root);
        let scan = |sources, destinations| {
            compute_cherry_pick_relationships_with_bounds(
                dir.path(),
                &snapshot,
                "main",
                &snapshot.commits[0].oid,
                &cache,
                &EnrichmentCancel::never(),
                sources,
                destinations,
            )
        };
        assert!(
            scan(1, 10).is_empty(),
            "merge consumes first source yield so whole tip exceeds 1"
        );
        assert!(
            scan(2, 1).is_empty(),
            "merge consumes destination slot before the matching commit"
        );
        assert_eq!(
            scan(2, 2).len(),
            1,
            "complete match in bounded prefix survives destination overflow"
        );
        assert_eq!(
            scan(2, 3).len(),
            1,
            "exactly exhausted source/destination scans preserve pair"
        );
        let latest = Arc::new(AtomicU64::new(2));
        assert!(compute_cherry_pick_relationships_with_bounds(
            dir.path(),
            &snapshot,
            "main",
            &snapshot.commits[0].oid,
            &cache,
            &EnrichmentCancel::new(latest, 1),
            2,
            3
        )
        .is_empty());
    }

    fn assert_merge_worker_cancelled(stage: EnrichmentStage) {
        let (dir, snapshot, root) = merge_pair_fixture();
        let latest = Arc::new(AtomicU64::new(1));
        let (started_tx, started_rx) = mpsc::sync_channel(0);
        let (resume_tx, resume_rx) = mpsc::sync_channel(0);
        let resume_rx = Mutex::new(resume_rx);
        let observed = Arc::new(AtomicU64::new(0));
        let worker_observed = Arc::clone(&observed);
        let rx = spawn_enrichment_with_observer(
            snapshot,
            dir.path().to_path_buf(),
            Some("main".into()),
            1,
            root,
            EnrichmentCancel::new(Arc::clone(&latest), 1),
            move |current| {
                if current == stage {
                    worker_observed.fetch_add(1, Ordering::Relaxed);
                    started_tx.send(()).unwrap();
                    resume_rx.lock().unwrap().recv().unwrap();
                }
            },
        );
        let timeout = std::time::Duration::from_secs(60);
        started_rx
            .recv_timeout(timeout)
            .expect("actual worker reached real merge revwalk yield");
        latest.store(2, Ordering::Release);
        resume_tx.send(()).unwrap();
        assert_eq!(
            rx.recv_timeout(timeout),
            Err(mpsc::RecvTimeoutError::Disconnected)
        );
        assert_eq!(
            observed.load(Ordering::Relaxed),
            1,
            "cancellation before merge skip stops subsequent work"
        );
    }

    #[test]
    fn started_enrichment_merge_source_revwalk_cancelled_publishes_nothing() {
        assert_merge_worker_cancelled(EnrichmentStage::CherrySourceYield);
    }

    #[test]
    fn started_enrichment_merge_destination_revwalk_cancelled_publishes_nothing() {
        assert_merge_worker_cancelled(EnrichmentStage::CherryDestinationYield);
    }

    #[test]
    fn started_enrichment_cherry_patch_computation_cancelled_publishes_nothing() {
        assert_merge_worker_cancelled(EnrichmentStage::CherryPatchCompute);
    }

    #[test]
    fn cherry_scan_reuses_commit_cache_including_none() {
        let (dir, repo, source, destination, mut snapshot, root) = native_pair_fixture();
        snapshot.commits[1].oid = native_commit(
            &repo,
            Some(source),
            &[("picked", "picked\n")],
            "empty source",
        )
        .to_string();
        let cache = BranchCache::load_for_base(dir.path(), "main", &root);
        let calls = Arc::new(AtomicU64::new(0));
        let observer_calls = Arc::clone(&calls);
        let observer: EnrichmentObserver = Arc::new(move |stage| {
            if stage == EnrichmentStage::CherryPatchCompute {
                observer_calls.fetch_add(1, Ordering::Relaxed);
            }
        });
        let first = compute_cherry_pick_relationships_observed(
            dir.path(),
            &snapshot,
            "main",
            &destination.to_string(),
            &cache,
            &EnrichmentCancel::never(),
            500,
            2_000,
            &observer,
        );
        assert_eq!(first.len(), 1);
        assert!(calls.load(Ordering::Relaxed) > 0);
        let key = format!("{}:v1", snapshot.commits[1].oid);
        assert_eq!(
            cache
                .lookup_commit_patch_ids(std::slice::from_ref(&key))
                .get(&key),
            Some(&None)
        );
        calls.store(0, Ordering::Relaxed);
        let second = compute_cherry_pick_relationships_observed(
            dir.path(),
            &snapshot,
            "main",
            &destination.to_string(),
            &cache,
            &EnrichmentCancel::never(),
            500,
            2_000,
            &observer,
        );
        assert_eq!(second, first);
        assert_eq!(
            calls.load(Ordering::Relaxed),
            0,
            "cached None must skip empty diff recomputation"
        );
    }

    #[derive(Clone)]
    struct ScanLog(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for ScanLog {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn cherry_scan_logs_only_actual_overflow_on_default_target() {
        // Isolate tracing's global callsite-interest cache from parallel tests.
        if std::env::var_os("GBM_P64_SCAN_LOG_CHILD").is_none() {
            let output = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "git::graph::tests::cherry_scan_logs_only_actual_overflow_on_default_target",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env("GBM_P64_SCAN_LOG_CHILD", "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        let (dir, snapshot, root) = merge_pair_fixture();
        let cache = BranchCache::load_for_base(dir.path(), "main", &root);
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let writer = ScanLog(Arc::clone(&bytes));
        let subscriber = tracing_subscriber::fmt()
            .with_env_filter("git_branch_manager=debug")
            .with_ansi(false)
            .without_time()
            .with_writer(move || writer.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            let scan = |sources, destinations| {
                compute_cherry_pick_relationships_with_bounds(
                    dir.path(),
                    &snapshot,
                    "main",
                    &snapshot.commits[0].oid,
                    &cache,
                    &EnrichmentCancel::never(),
                    sources,
                    destinations,
                )
            };
            assert_eq!(scan(2, 3).len(), 1);
            assert!(
                !String::from_utf8(bytes.lock().unwrap().clone())
                    .unwrap()
                    .contains("scan_incomplete"),
                "exact exhaustion is not overflow"
            );
            bytes.lock().unwrap().clear();
            assert!(scan(1, 3).is_empty());
            assert_eq!(scan(2, 2).len(), 1);
        });
        let text = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
        assert!(text.contains("git_branch_manager::git::graph"));
        assert!(text.contains("scan_incomplete: source bound"));
        assert!(text.contains("scan_incomplete: destination bound"));
    }

    #[test]
    fn enrichment_distinguishes_kinds_missing_endpoints_self_pairs_and_idempotence() {
        let mut snapshot = relationship_snapshot(vec![
            GraphCommit {
                oid: "d".into(),
                ..Default::default()
            },
            GraphCommit {
                oid: "d".into(),
                ..Default::default()
            },
        ]);
        let squash = squash("d", "missing", &["feature/gone"], RelationshipMatch::Exact);
        let cherry = GraphRelationship {
            kind: RelationshipKind::CherryPick,
            ..squash.clone()
        };
        let self_pair = GraphRelationship {
            source_oid: "d".into(),
            ..cherry.clone()
        };
        let updates = [squash, cherry, self_pair];
        apply_squash_enrichment(&mut snapshot, &updates);
        for commit in &snapshot.commits {
            assert_eq!(commit.relationships.len(), 3);
            assert_eq!(commit.cherry_pick_sources().len(), 2);
            assert_eq!(commit.cherry_pick_destinations().len(), 1);
        }
        let before = snapshot.clone();
        apply_squash_enrichment(&mut snapshot, &updates);
        assert_eq!(snapshot, before);
    }

    #[test]
    fn cherry_scan_contained_tip_emits_nothing() {
        let (dir, _repo, _source, _dest, mut snapshot, root) = native_pair_fixture();
        snapshot.commits.truncate(1);
        let mut alias = snapshot.commits[0].refs[0].clone();
        alias.name = "feature/contained".into();
        snapshot.commits[0].refs.push(alias);
        let cache = BranchCache::load_for_base(dir.path(), "main", &root);
        assert!(compute_cherry_pick_relationships(
            dir.path(),
            &snapshot,
            "main",
            &snapshot.commits[0].oid,
            &cache,
            &EnrichmentCancel::never()
        )
        .is_empty());
    }

    #[test]
    fn graph_state_reports_relationship_changes_without_erasure() {
        let relationship = squash("d", "s", &["feature/source"], RelationshipMatch::Exact);
        let mut state = crate::view::graph::GraphState::new();
        assert!(!state.apply_squash_enrichment(std::slice::from_ref(&relationship)));
        state.apply_result(Ok(relationship_snapshot(vec![GraphCommit {
            oid: "d".into(),
            ..Default::default()
        }])));
        assert!(state.apply_squash_enrichment(std::slice::from_ref(&relationship)));
        assert!(!state.apply_squash_enrichment(std::slice::from_ref(&relationship)));
        assert!(!state.apply_squash_enrichment(&[]));
        assert_eq!(
            state.snapshot().unwrap().commits[0].relationships,
            [relationship]
        );
    }
}
