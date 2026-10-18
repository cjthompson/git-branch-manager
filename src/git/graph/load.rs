use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

use chrono::{TimeZone, Utc};

use crate::git::cache::CacheRoot;

use super::backends::{gleisbau_settings, load_with_git_cli, load_with_gleisbau};
use super::model::{GraphCommit, GraphLine, GraphRefKind, GraphSource};
use super::refs::{assign_graph_branch_labels, collect_ref_data};
use super::relationships::{
    apply_squash_enrichment, compute_relationships, compute_relationships_with_observer,
    merge_relationships,
};
use super::state::{GraphLoadError, GraphLoadOptions, GraphRepositoryDelta, GraphRepositoryState};
use super::{EnrichmentCancel, EnrichmentObserver, EnrichmentStage, GraphEnrichmentMsg, GraphSnapshot};

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

pub(super) fn spawn_enrichment_with_observer(
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
