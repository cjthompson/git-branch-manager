use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::git::cache::BranchCache;

use super::model::{GraphCommit, GraphRefKind, GraphRelationship, RelationshipKind, RelationshipMatch};
use super::patch::{load_patch_ids, PatchJob, PatchTarget, SourceTip};
use super::relationships::is_source_ref;
use super::{EnrichmentCancel, EnrichmentObserver, GraphSnapshot};

#[allow(clippy::too_many_arguments)]
pub(super) fn compute_squash_relationships(
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
                is_source_ref(reference, base_branch, snapshot.includes_remotes)
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

#[derive(Debug, PartialEq, Eq)]
pub(super) enum DisplayedBranchRelation {
    RegularlyMerged,
    Diverged { merge_base: String },
    Ineligible,
}

/// Resolve ancestry only through commits already owned by the snapshot. A
/// merge base outside the displayed window is intentionally ineligible rather
/// than triggering an unbounded repository walk during Graph startup.
pub(super) fn displayed_branch_relation(
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
