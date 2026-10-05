use std::collections::{HashMap, HashSet};
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

/// One per-commit update produced by asynchronous squash-merge enrichment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphEnrichmentUpdate {
    pub oid: String,
    pub is_possible_squash_merge: bool,
    /// Local branch names whose aggregate diff exactly matches this base
    /// commit. Empty unless `is_possible_squash_merge` is true.
    pub possible_squash_merge_sources: Vec<String>,
    pub fuzzy_squash_match: Option<FuzzySquashMatch>,
    pub is_cherry_picked_commit: bool,
}

/// Channel message carrying the full set of squash-merge enrichment updates
/// for a given `GraphSnapshot` reload generation. Delivered as a single
/// batch rather than per-commit so the App applies it atomically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphEnrichmentMsg {
    pub generation: u64,
    pub updates: Vec<GraphEnrichmentUpdate>,
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
    CherryLine,
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
    /// True when this displayed base-branch commit has the same stable Git
    /// patch ID as the aggregate patch of a displayed, non-merged local branch.
    pub is_possible_squash_merge: bool,
    /// Local branches whose aggregate diffs exactly match this base commit.
    /// The list is sorted for a stable compact Graph label and details-modal
    /// presentation. It is empty unless `is_possible_squash_merge` is true.
    pub possible_squash_merge_sources: Vec<String>,
    /// Set when this commit's diff is a *near*-match (not exact) for a
    /// displayed branch tip's aggregate diff, per the Option 6 fuzzy/possible
    /// tier (`git::fuzzy_match`). Never set on a commit that already has
    /// `is_possible_squash_merge == true` — Option 6 is additive and defers
    /// to the exact-match tier.
    pub fuzzy_squash_match: Option<FuzzySquashMatch>,
    /// True when this commit landed in base via an individual cherry-pick,
    /// as detected by `git cherry`. Defaults to false (none of the
    /// pre-cherry-detection snapshots set it).
    pub is_cherry_picked_commit: bool,
    /// Author name from `git2::Signature::name()` / `%an`.
    pub author_name: String,
    /// Author email from `git2::Signature::email()` / `%ae`.
    pub author_email: String,
    /// Author date (when the change was originally made) in UTC.
    /// Git's "%aI" / `commit.author().when()`. `None` means the loader could
    /// not parse or represent the author date.
    pub authored_at: Option<DateTime<Utc>>,
}

/// A fuzzy (non-exact) possible-squash-merge signal for a single base commit,
/// scored against the best-matching displayed branch tip. See
/// `docs/plans/2026-08-29-squash-merge-test-scenarios.md` "New: Option 6".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuzzySquashMatch {
    /// Jaccard similarity as an integer percent (0-100), rounded from the raw
    /// f32 score. Stored as u8 rather than f32 so GraphCommit/FuzzySquashMatch
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
                oid: oid_text, summary: commit.summary().ok().flatten().unwrap_or_default().to_string(),
                parents: commit.parent_ids().map(|parent| parent.to_string()).collect(), lane: None,
                branch: None, refs: Vec::new(), is_possible_squash_merge: false,
                possible_squash_merge_sources: Vec::new(), fuzzy_squash_match: None,
                is_cherry_picked_commit: false, author_name: commit.author().name().unwrap_or("").to_string(),
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
            record.is_possible_squash_merge = old.is_possible_squash_merge;
            record.possible_squash_merge_sources = old.possible_squash_merge_sources.clone();
            record.fuzzy_squash_match = old.fuzzy_squash_match.clone();
            record.is_cherry_picked_commit = old.is_cherry_picked_commit;
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
    let mut updates = compute_possible_squash_updates(
        repo_path,
        &snapshot,
        options.base_branch.as_deref(),
        &options.cache_root,
    );
    let observer: EnrichmentObserver = Arc::new(|_| {});
    let cherry_updates = compute_cherry_pick_updates(
        repo_path,
        &snapshot,
        options.base_branch.as_deref(),
        &EnrichmentCancel::never(),
        &observer,
    )
    .unwrap_or_default();

    merge_enrichment_updates(&mut updates, cherry_updates);
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
        let Some(mut updates) = compute_possible_squash_updates_with_cancel(
            &repo_path,
            &snapshot,
            requested_base.as_deref(),
            &cache_root,
            &cancel,
            &observer,
        ) else {
            return;
        };
        if cancel.is_cancelled() {
            return;
        }
        let Some(cherry_updates) = compute_cherry_pick_updates(
            &repo_path,
            &snapshot,
            requested_base.as_deref(),
            &cancel,
            &observer,
        ) else {
            return;
        };
        if cancel.is_cancelled() {
            return;
        }
        merge_enrichment_updates(&mut updates, cherry_updates);
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
                is_possible_squash_merge: false,
                possible_squash_merge_sources: Vec::new(),
                fuzzy_squash_match: None,
                is_cherry_picked_commit: false,
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
            is_possible_squash_merge: false,
            possible_squash_merge_sources: Vec::new(),
            fuzzy_squash_match: None,
            is_cherry_picked_commit: false,
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

/// Match only work represented by this bounded snapshot. Both base commits and
/// local branch tips are selected from `snapshot.commits`, so increasing
/// repository history or ref age cannot make Graph startup scan beyond the
/// configured `max_count` window.
///
/// Returns a list of per-commit updates that the caller applies to the
/// snapshot via `apply_squash_enrichment`. This split lets the App run the
/// heavy work on a background thread and the result stays purely owned
/// (no `&mut` over the App's snapshot) so it can be applied at any time.
pub fn compute_possible_squash_updates(
    repo_path: &Path,
    snapshot: &GraphSnapshot,
    requested_base: Option<&str>,
    cache_root: &CacheRoot,
) -> Vec<GraphEnrichmentUpdate> {
    let observer: EnrichmentObserver = Arc::new(|_| {});
    compute_possible_squash_updates_with_cancel(
        repo_path,
        snapshot,
        requested_base,
        cache_root,
        &EnrichmentCancel::never(),
        &observer,
    )
    .unwrap_or_default()
}

fn compute_possible_squash_updates_with_cancel(
    repo_path: &Path,
    snapshot: &GraphSnapshot,
    requested_base: Option<&str>,
    cache_root: &CacheRoot,
    cancel: &EnrichmentCancel,
    observer: &EnrichmentObserver,
) -> Option<Vec<GraphEnrichmentUpdate>> {
    if cancel.is_cancelled() {
        return None;
    }
    let base_branch = requested_base.map(str::to_string).or_else(|| {
        let repository = git2::Repository::open(repo_path).ok()?;
        crate::git::branch::detect_base_branch(&repository, None).ok()
    });
    let Some(base_branch) = base_branch else {
        return Some(Vec::new());
    };
    let Some(base_tip) = snapshot.commits.iter().find_map(|commit| {
        commit
            .refs
            .iter()
            .any(|reference| {
                reference.kind == GraphRefKind::LocalBranch && reference.name == base_branch
            })
            .then(|| commit.oid.clone())
    }) else {
        return Some(Vec::new());
    };

    if cancel.is_cancelled() {
        return None;
    }
    let mut cache = BranchCache::load_for_base(repo_path, &base_branch, cache_root);

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
        let merge_base = match displayed_branch_relation(&snapshot.commits, &base_tip, &tip) {
            DisplayedBranchRelation::Diverged { merge_base } => merge_base,
            DisplayedBranchRelation::RegularlyMerged | DisplayedBranchRelation::Ineligible => {
                continue;
            }
        };
        jobs.push(PatchJob {
            target: PatchTarget::BranchTip { source_names },
            old_oid: merge_base,
            new_oid: tip,
        });
    }

    let mut base_oids_by_patch = HashMap::<String, Vec<String>>::new();
    let mut source_names_by_patch = HashMap::<String, Vec<String>>::new();
    // Retained alongside patch IDs so pairs whose patch IDs don't exactly
    // match can still be scored by the Option 6 fuzzy tier below, without a
    // second `git diff` subprocess per job.
    let mut base_diffs = Vec::<(String, Vec<u8>)>::new();
    let mut branch_diffs = Vec::<Vec<u8>>::new();
    if cancel.is_cancelled() {
        return None;
    }
    for result in load_patch_ids(repo_path, jobs, &mut cache, cancel, observer)? {
        if cancel.is_cancelled() {
            return None;
        }
        match result.target {
            PatchTarget::BaseCommit(oid) => {
                if let Some(patch_id) = result.patch_id {
                    base_oids_by_patch
                        .entry(patch_id)
                        .or_default()
                        .push(oid.clone());
                }
                if let Some(diff_text) = result.diff_text {
                    base_diffs.push((oid, diff_text));
                }
            }
            PatchTarget::BranchTip { source_names } => {
                if let Some(patch_id) = result.patch_id {
                    let names = source_names_by_patch.entry(patch_id).or_default();
                    names.extend(source_names);
                    names.sort();
                    names.dedup();
                }
                if let Some(diff_text) = result.diff_text {
                    branch_diffs.push(diff_text);
                }
            }
        }
    }

    let mut source_names_by_base_oid = HashMap::<String, Vec<String>>::new();
    for (patch_id, source_names) in source_names_by_patch {
        if cancel.is_cancelled() {
            return None;
        }
        let Some(base_oids) = base_oids_by_patch.get(&patch_id) else {
            continue;
        };
        for oid in base_oids {
            if cancel.is_cancelled() {
                return None;
            }
            let names = source_names_by_base_oid.entry(oid.clone()).or_default();
            names.extend(source_names.iter().cloned());
            names.sort();
            names.dedup();
        }
    }
    let matching_base_oids = source_names_by_base_oid
        .keys()
        .cloned()
        .collect::<HashSet<_>>();

    // Option 6: for base commits that didn't get an exact patch-id match,
    // score their diff against every displayed branch tip's diff and keep
    // the best fuzzy classification, if any clears the threshold.
    let mut fuzzy_by_oid: HashMap<String, FuzzySquashMatch> = HashMap::new();
    for (oid, base_diff) in &base_diffs {
        if cancel.is_cancelled() {
            return None;
        }
        if matching_base_oids.contains(oid) {
            continue;
        }
        let mut best_percent = None;
        for branch_diff in &branch_diffs {
            if cancel.is_cancelled() {
                return None;
            }
            if let Some(percent) = crate::git::fuzzy_match::score(branch_diff, base_diff)
                .and_then(|score| crate::git::fuzzy_match::classify(&score))
            {
                best_percent = Some(best_percent.map_or(percent, |best: u8| best.max(percent)));
            }
        }
        let Some(percent) = best_percent else {
            continue;
        };
        fuzzy_by_oid.insert(
            oid.clone(),
            FuzzySquashMatch {
                similarity_percent: percent,
            },
        );
    }

    if cancel.is_cancelled() {
        return None;
    }
    let updates: Vec<GraphEnrichmentUpdate> = snapshot
        .commits
        .iter()
        .filter(|commit| {
            commit.branch.as_ref().is_some_and(|branch| {
                branch.kind == GraphRefKind::LocalBranch && branch.name == base_branch
            })
        })
        .map(|commit| GraphEnrichmentUpdate {
            oid: commit.oid.clone(),
            is_possible_squash_merge: matching_base_oids.contains(&commit.oid),
            possible_squash_merge_sources: source_names_by_base_oid
                .get(&commit.oid)
                .cloned()
                .unwrap_or_default(),
            fuzzy_squash_match: fuzzy_by_oid.get(&commit.oid).cloned(),
            is_cherry_picked_commit: false,
        })
        .collect();
    if cancel.is_cancelled() {
        return None;
    }
    cache.save();
    Some(updates)
}

/// Apply a batch of squash-merge enrichment updates to a snapshot. Unknown
/// OIDs are ignored — they may belong to a newer snapshot (stale enrichment)
/// or to commits the structural snapshot didn't include.
pub fn apply_squash_enrichment(snapshot: &mut GraphSnapshot, updates: &[GraphEnrichmentUpdate]) {
    for update in updates {
        if let Some(commit) = snapshot
            .commits
            .iter_mut()
            .find(|commit| commit.oid == update.oid)
        {
            commit.is_possible_squash_merge = update.is_possible_squash_merge;
            commit.possible_squash_merge_sources = update.possible_squash_merge_sources.clone();
            commit.fuzzy_squash_match = update.fuzzy_squash_match.clone();
            commit.is_cherry_picked_commit = update.is_cherry_picked_commit;
        }
    }
}

/// For every displayed, diverged non-base branch tip, run `git cherry` once
/// against its merge-base with `base_branch` and emit a
/// `GraphEnrichmentUpdate` per OID whose line starts with `-` (already
/// landed via cherry-pick).
///
/// One cheap `git cherry` subprocess per displayed branch tip, not a
/// per-commit diff. Mirrors the base-branch/branch-tip relationship
/// detection logic of `compute_possible_squash_updates`.
fn compute_cherry_pick_updates(
    repo_path: &Path,
    snapshot: &GraphSnapshot,
    requested_base: Option<&str>,
    cancel: &EnrichmentCancel,
    observer: &EnrichmentObserver,
) -> Option<Vec<GraphEnrichmentUpdate>> {
    if cancel.is_cancelled() {
        return None;
    }
    let base_branch = requested_base.map(str::to_string).or_else(|| {
        let repository = git2::Repository::open(repo_path).ok()?;
        crate::git::branch::detect_base_branch(&repository, None).ok()
    });
    let Some(base_branch) = base_branch else {
        return Some(Vec::new());
    };
    let Some(base_tip) = snapshot.commits.iter().find_map(|commit| {
        commit
            .refs
            .iter()
            .any(|reference| {
                reference.kind == GraphRefKind::LocalBranch && reference.name == base_branch
            })
            .then(|| commit.oid.clone())
    }) else {
        return Some(Vec::new());
    };

    let mut cherry_picked: HashSet<String> = HashSet::new();

    let displayed_branch_tips = snapshot.commits.iter().filter(|commit| {
        commit.refs.iter().any(|reference| {
            reference.kind == GraphRefKind::LocalBranch && reference.name != base_branch
        })
    });

    for tip in displayed_branch_tips {
        if cancel.is_cancelled() {
            return None;
        }
        let merge_base = match displayed_branch_relation(&snapshot.commits, &base_tip, &tip.oid) {
            DisplayedBranchRelation::Diverged { merge_base } => merge_base,
            DisplayedBranchRelation::RegularlyMerged | DisplayedBranchRelation::Ineligible => {
                continue;
            }
        };
        let tip_str = tip.oid.as_str();
        let git_cherry = |args: &[&str]| -> Option<String> {
            let out = Command::new("git")
                .args(args)
                .current_dir(repo_path)
                .stdin(Stdio::null())
                .output()
                .ok()?;
            if out.status.success() {
                Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
            } else {
                None
            }
        };
        if cancel.is_cancelled() {
            return None;
        }
        let result = match git_cherry(&["cherry", &base_branch, tip_str, &merge_base]) {
            Some(s) if !s.is_empty() => s,
            _ => continue,
        };
        for line in result.lines() {
            if cancel.is_cancelled() {
                return None;
            }
            observer(EnrichmentStage::CherryLine);
            if cancel.is_cancelled() {
                return None;
            }
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            // Format: "<status> <commit_hash> <subject>"
            let mut parts = line.splitn(3, char::is_whitespace);
            let status = parts.next().unwrap_or("");
            let oid = parts.next().unwrap_or("");
            if status.starts_with('-') && !oid.is_empty() {
                cherry_picked.insert(oid.to_string());
            }
        }
    }

    if cancel.is_cancelled() {
        return None;
    }
    Some(
        snapshot
            .commits
            .iter()
            .map(|commit| GraphEnrichmentUpdate {
                oid: commit.oid.clone(),
                is_possible_squash_merge: false,
                possible_squash_merge_sources: Vec::new(),
                fuzzy_squash_match: None,
                is_cherry_picked_commit: cherry_picked.contains(&commit.oid),
            })
            .collect(),
    )
}

fn merge_enrichment_updates(base: &mut Vec<GraphEnrichmentUpdate>, extra: Vec<GraphEnrichmentUpdate>) {
    use std::collections::HashMap;
    let mut by_oid: HashMap<String, GraphEnrichmentUpdate> =
        base.drain(..).map(|u| (u.oid.clone(), u)).collect();
    for u in extra {
        by_oid
            .entry(u.oid.clone())
            .and_modify(|existing| existing.is_cherry_picked_commit = u.is_cherry_picked_commit)
            .or_insert(u);
    }
    *base = by_oid.into_values().collect();
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

enum PatchTarget {
    BaseCommit(String),
    BranchTip { source_names: Vec<String> },
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
    fn started_enrichment_superseded_during_cherry_output_publishes_nothing() {
        assert_started_enrichment_cancelled(EnrichmentStage::CherryLine);
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
}
