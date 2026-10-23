use std::collections::{HashMap, HashSet};
use std::path::Path;

use super::model::{GraphBranchLabel, GraphCommit, GraphRef, GraphRefCounts, GraphRefKind, GraphRefTracking};

#[derive(Default)]
pub(super) struct RefData {
    pub(super) refs_by_oid: HashMap<String, Vec<GraphRef>>,
    pub(super) ref_counts: GraphRefCounts,
    pub(super) branch_labels: HashMap<String, GraphBranchLabel>,
    pub(super) base_branch: Option<String>,
}

pub(super) fn collect_ref_data(
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

    let base_oid = base_branch.as_deref().and_then(|base| {
        repository
            .find_branch(base, git2::BranchType::Local)
            .ok()?
            .get()
            .target()
    });
    for (name, target_oid) in &local_refs {
        let ahead_of_base = match base_oid {
            Some(base_oid) if base_branch.as_deref() != Some(name.as_str()) => {
                git2::Oid::from_str(target_oid)
                    .ok()
                    .and_then(|tip| repository.graph_ahead_behind(tip, base_oid).ok())
                    .map(|(ahead, _)| u32::try_from(ahead).unwrap_or(u32::MAX))
            }
            _ => None,
        };
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
                ahead_of_base,
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
                    ahead_of_base: None,
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
                    ahead_of_base: None,
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
                ahead_of_base: None,
            },
        );
    }

    Ok(data)
}

fn remote_short_name(name: &str) -> &str {
    name.split_once('/').map(|(_, short)| short).unwrap_or(name)
}

pub(super) fn assign_graph_branch_labels(
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
