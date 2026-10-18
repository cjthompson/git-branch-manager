use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use crate::git::cache::BranchCache;

use super::model::{GraphRefKind, GraphRelationship, RelationshipKind, RelationshipMatch};
use super::patch::git2_diff_patch;
use super::relationships::RelationshipAccumulator;
use super::{EnrichmentCancel, EnrichmentObserver, EnrichmentStage, GraphSnapshot};

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
pub(super) fn compute_cherry_pick_relationships(
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
pub(super) fn compute_cherry_pick_relationships_observed(
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
