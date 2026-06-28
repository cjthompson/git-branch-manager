use crate::types::{BranchInfo, MergeStatus};
use git2::Repository;
use std::collections::HashSet;
use std::path::Path;
use std::process::Command;
use tracing::{field, instrument, Span};

/// Holds the reachable sets for both the local base branch and its remote tracking ref.
/// Used to determine whether a branch is merged, and if only into one side.
pub struct BaseReachable {
    pub local: HashSet<git2::Oid>,
    pub remote: HashSet<git2::Oid>,
}

impl BaseReachable {
    /// Returns the appropriate MergeStatus for a branch OID.
    /// When no remote tracking ref exists, local is treated as authoritative (returns Merged).
    pub fn regular_merge_status(&self, oid: git2::Oid) -> Option<MergeStatus> {
        let in_local = self.local.contains(&oid);
        let in_remote = self.remote.contains(&oid);
        let has_remote = !self.remote.is_empty();
        match (in_local, in_remote, has_remote) {
            (true, true, _) => Some(MergeStatus::Merged),
            (true, false, false) => Some(MergeStatus::Merged), // no remote — local is truth
            (false, true, _) => Some(MergeStatus::RemoteMerged),
            (true, false, true) => Some(MergeStatus::LocalMerged),
            (false, false, _) => None,
        }
    }
}

fn build_reachable_from_ref(repo: &Repository, base_branch: &str) -> HashSet<git2::Oid> {
    let oid = match repo
        .find_branch(base_branch, git2::BranchType::Local)
        .ok()
        .and_then(|b| b.get().target())
    {
        Some(oid) => oid,
        None => return HashSet::new(),
    };
    revwalk_from_oid(repo, oid)
}

fn build_reachable_from_remote_ref(repo: &Repository, base_branch: &str) -> HashSet<git2::Oid> {
    let remote_name = format!("origin/{base_branch}");
    let oid = match repo
        .find_branch(&remote_name, git2::BranchType::Remote)
        .ok()
        .and_then(|b| b.get().target())
    {
        Some(oid) => oid,
        None => return HashSet::new(),
    };
    revwalk_from_oid(repo, oid)
}

fn revwalk_from_oid(repo: &Repository, oid: git2::Oid) -> HashSet<git2::Oid> {
    let mut revwalk = match repo.revwalk() {
        Ok(r) => r,
        Err(_) => return HashSet::new(),
    };
    let _ = revwalk.set_sorting(git2::Sort::NONE);
    let _ = revwalk.push(oid);
    let mut set = HashSet::new();
    for oid in revwalk.flatten() {
        set.insert(oid);
    }
    set
}

/// Detect which branches have been regular-merged into the base branch using git2.
/// Modifies branch merge_status in place from Unmerged to Merged where applicable.
#[instrument(
    skip(repo, branches),
    fields(
        base_branch,
        branch_count = branches.len(),
        base_oid = field::Empty,
        base_lookup_result = field::Empty,
        candidate_count = field::Empty,
        checked_count = field::Empty,
        skipped_base_count = field::Empty,
        skipped_current_count = field::Empty,
        find_branch_error_count = field::Empty,
        missing_target_count = field::Empty,
        merged_count = field::Empty,
        unmerged_count = field::Empty,
    )
)]
/// Returns a BaseReachable with reachable sets for both local and remote base,
/// so callers can reuse it for merge-base computation and squash detection.
pub fn detect_merged_branches(
    repo: &Repository,
    base_branch: &str,
    branches: &mut [BranchInfo],
) -> anyhow::Result<BaseReachable> {
    let span = Span::current();

    let local_reachable = build_reachable_from_ref(repo, base_branch);
    let remote_reachable = build_reachable_from_remote_ref(repo, base_branch);

    if local_reachable.is_empty() && remote_reachable.is_empty() {
        span.record("base_lookup_result", "find_branch_error");
        return Err(anyhow::anyhow!("base branch not found: {base_branch}"));
    }
    span.record("base_lookup_result", "success");

    let base_reachable = BaseReachable {
        local: local_reachable,
        remote: remote_reachable,
    };

    let candidates: Vec<(usize, git2::Oid)> = branches
        .iter()
        .enumerate()
        .filter(|(_, b)| !b.is_base && !b.is_current)
        .filter_map(|(i, b)| {
            repo.find_branch(&b.name, git2::BranchType::Local)
                .and_then(|br| {
                    br.get()
                        .target()
                        .ok_or_else(|| git2::Error::from_str("no target"))
                })
                .ok()
                .map(|oid| (i, oid))
        })
        .collect();

    let skipped_base_count = branches.iter().filter(|b| b.is_base).count();
    let skipped_current_count = branches.iter().filter(|b| b.is_current).count();
    let candidate_count = branches
        .iter()
        .filter(|b| !b.is_base && !b.is_current)
        .count();
    let find_branch_error_count = candidate_count.saturating_sub(candidates.len());
    let missing_target_count = 0usize;

    span.record("candidate_count", candidate_count);
    span.record("skipped_base_count", skipped_base_count);
    span.record("skipped_current_count", skipped_current_count);
    span.record("find_branch_error_count", find_branch_error_count);
    span.record("missing_target_count", missing_target_count);

    if candidates.is_empty() {
        span.record("checked_count", 0u64);
        span.record("merged_count", 0u64);
        span.record("unmerged_count", 0u64);
        return Ok(base_reachable);
    }

    let mut merged_count = 0usize;
    let mut unmerged_count = 0usize;
    for (i, branch_oid) in &candidates {
        if let Some(status) = base_reachable.regular_merge_status(*branch_oid) {
            merged_count += 1;
            branches[*i].merge_status = status;
        } else {
            unmerged_count += 1;
        }
    }
    span.record("checked_count", candidates.len() as u64);
    span.record("merged_count", merged_count);
    span.record("unmerged_count", unmerged_count);
    Ok(base_reachable)
}

/// Build the reachable sets for both local and remote base using an already-open repository.
/// Call this when you already have a repo handle on the current thread.
#[instrument(skip(repo), fields(reachable_count = field::Empty))]
pub fn build_reachable_set_from_repo(repo: &Repository, base_branch: &str) -> BaseReachable {
    let local = build_reachable_from_ref(repo, base_branch);
    let remote = build_reachable_from_remote_ref(repo, base_branch);
    BaseReachable { local, remote }
}

/// Build the reachable sets for both local and remote base by opening a fresh Repository.
/// Intended for background-thread use: Repository is !Send, so callers open their own handle
/// rather than sharing the main thread's repo.
pub fn build_reachable_set(repo_path: &Path, base_branch: &str) -> BaseReachable {
    let repo = match git2::Repository::open(repo_path) {
        Ok(r) => r,
        Err(_) => {
            return BaseReachable {
                local: HashSet::new(),
                remote: HashSet::new(),
            }
        }
    };
    build_reachable_set_from_repo(&repo, base_branch)
}

/// Apply merge statuses to branches using a prebuilt BaseReachable.
/// Used when the reachable set was built in a parallel thread via build_reachable_set,
/// so we re-resolve each branch tip with the provided (main-thread) repo handle.
pub fn apply_merge_statuses(
    repo: &Repository,
    branches: &mut [BranchInfo],
    base_reachable: &BaseReachable,
) {
    if base_reachable.local.is_empty() && base_reachable.remote.is_empty() {
        return;
    }
    for branch in branches.iter_mut() {
        if branch.is_base || branch.is_current {
            continue;
        }
        if let Ok(b) = repo.find_branch(&branch.name, git2::BranchType::Local) {
            if let Some(oid) = b.get().target() {
                if let Some(status) = base_reachable.regular_merge_status(oid) {
                    branch.merge_status = status;
                }
            }
        }
    }
}

/// Detect if a branch was squash-merged into the base branch using git CLI.
/// Uses commit-tree + cherry to check if the branch's tree content already exists in base.
///
/// `merge_base` is the branch's already-known merge base with `base_branch` (the
/// value computed once from the in-memory reachable set in `fill_merge_base_commits`).
/// When `Some`, it is used directly and the per-call `git merge-base` subprocess is
/// skipped entirely — this avoids re-walking history for every candidate, which is
/// catastrophic on branches whose merge base is far back or absent (disjoint
/// histories force `git merge-base` to walk the full graph). When `None`, we fall
/// back to computing it via `git merge-base` (used by the remote path, which does
/// not precompute merge bases).
#[instrument(skip(repo_path))]
pub fn is_squash_merged(
    repo_path: &Path,
    base_branch: &str,
    branch_name: &str,
    commit_hash: Option<&str>,
    merge_base: Option<&str>,
) -> bool {
    let git = |args: &[&str]| -> Option<String> {
        let out = Command::new("git")
            .args(args)
            .current_dir(repo_path)
            .stdin(std::process::Stdio::null())
            .output()
            .ok()?;
        if out.status.success() {
            Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
        } else {
            None
        }
    };

    let branchish = commit_hash.unwrap_or(branch_name);

    // Step 1: find merge-base. Prefer the precomputed value; only shell out when absent.
    let ancestor = match merge_base {
        Some(mb) if !mb.is_empty() => mb.to_string(),
        _ => match git(&["merge-base", base_branch, branchish]) {
            Some(a) if !a.is_empty() => a,
            _ => return false,
        },
    };

    // Step 2: create temp commit-tree
    let tree_spec = format!("{branchish}^{{tree}}");
    let temp_commit = match git(&["commit-tree", &tree_spec, "-p", &ancestor, "-m", "_"]) {
        Some(c) if !c.is_empty() => c,
        _ => return false,
    };

    // Step 3: cherry check
    match git(&["cherry", base_branch, &temp_commit]) {
        Some(result) => result.starts_with('-'),
        None => false,
    }
}
