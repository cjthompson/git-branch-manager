use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::Arc;

use crate::git::cache::{BranchCache, CacheRoot};

use super::cherry::{
    compute_cherry_pick_relationships, compute_cherry_pick_relationships_observed,
    MAX_CHERRY_DESTINATION_SCAN, MAX_CHERRY_SOURCE_COMMITS,
};
use super::model::{GraphRefKind, GraphRelationship, RelationshipKind};
use super::squash::compute_squash_relationships;
use super::{EnrichmentCancel, EnrichmentObserver, GraphSnapshot};

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

pub(super) fn compute_relationships_with_observer(
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

type RelationshipKey = (RelationshipKind, String, String);

#[derive(Default)]
pub(super) struct RelationshipAccumulator(BTreeMap<RelationshipKey, GraphRelationship>);

impl RelationshipAccumulator {
    pub(super) fn insert(&mut self, incoming: &GraphRelationship) {
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

    pub(super) fn into_relationships(self) -> Vec<GraphRelationship> {
        self.0.into_values().collect()
    }
}

/// Merge immutable relationship facts without removing captured provenance.
pub(super) fn merge_relationships(existing: &mut Vec<GraphRelationship>, incoming: &[GraphRelationship]) {
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
