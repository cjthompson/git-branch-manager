mod merge_base;
mod metadata;
mod remote;

pub use merge_base::{
    compute_merge_base_short, fill_merge_base_commits, fill_merge_base_commits_cached,
};
pub use remote::{list_remote_branches_phase1, spawn_remote_enricher};

use crate::types::*;
use git2::{BranchType, Repository};
use metadata::collect_branch_metadata;
use thiserror::Error;
use tracing::{field, info_span, instrument, Span};

#[derive(Error, Debug)]
pub enum GitError {
    #[error("not a git repository")]
    NotARepo,
    #[error("base branch not found: {0}")]
    BaseBranchNotFound(String),
    #[error("cannot auto-detect base branch")]
    CannotDetectBase,
    #[error("command failed: {command}: {stderr}")]
    CommandFailed { command: String, stderr: String },
    #[error("parse error: {0}")]
    ParseError(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Git2(#[from] git2::Error),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

type Result<T> = std::result::Result<T, GitError>;

/// Detect the base branch for the repository.
/// If override_base is provided and exists, use it.
/// Otherwise try remote HEAD symref, common names, then first branch.
#[instrument(skip(repo))]
pub fn detect_base_branch(repo: &Repository, override_base: Option<&str>) -> Result<String> {
    if let Some(base) = override_base {
        // Validate that the branch exists
        if repo.find_branch(base, BranchType::Local).is_ok() {
            return Ok(base.to_string());
        }
        return Err(GitError::BaseBranchNotFound(base.to_string()));
    }

    // Try remote HEAD symref
    if let Ok(reference) = repo.find_reference("refs/remotes/origin/HEAD") {
        if let Ok(Some(target)) = reference.symbolic_target() {
            if let Some(name) = target.strip_prefix("refs/remotes/origin/") {
                if repo.find_branch(name, BranchType::Local).is_ok() {
                    return Ok(name.to_string());
                }
            }
        }
    }

    // Fallback: try common names
    for name in &["main", "master", "develop"] {
        if repo.find_branch(name, BranchType::Local).is_ok() {
            return Ok(name.to_string());
        }
    }

    // Last resort: first branch
    let branches = repo.branches(Some(BranchType::Local))?;
    for (branch, _) in branches.flatten() {
        if let Some(name) = branch.name()? {
            return Ok(name.to_string());
        }
    }

    Err(GitError::CannotDetectBase)
}

/// List local branches with metadata (phase 1: synchronous, git2 only).
/// Detects regular merges and marks unmerged non-pinned branches as Pending.
#[instrument(skip(repo), fields(base_branch, result_count = field::Empty))]
pub fn list_branches_phase1(repo: &Repository, base_branch: &str) -> Result<Vec<BranchInfo>> {
    let span = Span::current();
    let mut branches = collect_branch_metadata(repo, base_branch, false, true)?;
    let base_reachable =
        super::merge_detection::detect_merged_branches(repo, base_branch, &mut branches)?;
    fill_merge_base_commits(repo, &mut branches, &base_reachable.local);

    // Mark unmerged non-pinned branches as Pending (for squash check)
    info_span!(
        "list_branches_phase1_mark_pending",
        branch_count = branches.len()
    )
    .in_scope(|| {
        for b in &mut branches {
            // Only connected branches are squash-check candidates. A branch with no
            // merge base is disjoint from base and can't have been squash-merged, so
            // leave it Unmerged rather than Pending (nothing will resolve it otherwise).
            if !b.is_pinned()
                && b.merge_status == MergeStatus::Unmerged
                && b.merge_base_commit.is_some()
            {
                b.merge_status = MergeStatus::Pending;
            }
        }
    });

    // Sort: pinned first, then by date descending
    info_span!("list_branches_phase1_sort", branch_count = branches.len()).in_scope(|| {
        branches.sort_by(|a, b| {
            b.is_pinned()
                .cmp(&a.is_pinned())
                .then(b.last_commit_date.cmp(&a.last_commit_date))
        });
    });

    span.record("result_count", branches.len() as u64);
    Ok(branches)
}

/// Fast metadata-only pass: collects branch info without merge detection.
/// All non-pinned branches start as Pending; merge statuses are filled in
/// asynchronously via a subsequent detect_merged_branches call.
#[instrument(
    skip(repo),
    fields(
        base_branch,
        skip_ahead_behind = true,
        result_count = field::Empty,
        pending_count = field::Empty,
    )
)]
pub fn list_branches_fast(repo: &Repository, base_branch: &str) -> Result<Vec<BranchInfo>> {
    let span = Span::current();
    let mut branches = collect_branch_metadata(repo, base_branch, true, true)?;
    let mut pending_count = 0usize;
    info_span!(
        "list_branches_fast_mark_pending",
        branch_count = branches.len()
    )
    .in_scope(|| {
        for b in &mut branches {
            if !b.is_pinned() {
                b.merge_status = MergeStatus::Pending;
                pending_count += 1;
            }
        }
    });
    info_span!("list_branches_fast_sort", branch_count = branches.len()).in_scope(|| {
        branches.sort_by(|a, b| {
            b.is_pinned()
                .cmp(&a.is_pinned())
                .then(b.last_commit_date.cmp(&a.last_commit_date))
        });
    });
    span.record("pending_count", pending_count as u64);
    span.record("result_count", branches.len() as u64);
    Ok(branches)
}

/// Get the commit hash for a local branch.
#[instrument(skip(repo))]
pub fn get_commit_hash(repo: &Repository, branch_name: &str) -> Option<String> {
    repo.find_branch(branch_name, BranchType::Local)
        .ok()
        .and_then(|b| b.get().peel_to_commit().ok())
        .map(|c| c.id().to_string())
}

/// List all local branches with full metadata including squash-merge detection.
/// Synchronous — runs squash checks inline. Used by `--list` mode and tests.
#[instrument(skip(repo), fields(base_branch, result_count = field::Empty))]
pub fn list_branches(repo: &Repository, base_branch: &str) -> Result<Vec<BranchInfo>> {
    list_branches_with_cache_root(repo, base_branch, &super::cache::CacheRoot::from_env())
}

#[instrument(skip(repo, cache_root), fields(base_branch, result_count = field::Empty))]
pub fn list_branches_with_cache_root(
    repo: &Repository,
    base_branch: &str,
    cache_root: &super::cache::CacheRoot,
) -> Result<Vec<BranchInfo>> {
    let span = Span::current();
    let repo_path = repo.workdir().unwrap_or_else(|| repo.path());
    let mut cache = super::cache::BranchCache::load_for_base(repo_path, base_branch, cache_root);

    let mut branches = collect_branch_metadata(repo, base_branch, false, true)?;
    let base_reachable =
        super::merge_detection::detect_merged_branches(repo, base_branch, &mut branches)?;

    // Compute merge bases first so squash detection can reuse them (and skip
    // disjoint branches) instead of re-deriving the merge base per branch.
    fill_merge_base_commits(repo, &mut branches, &base_reachable.local);

    for branch in branches.iter_mut() {
        if branch.merge_status != MergeStatus::Unmerged || branch.is_base || branch.is_current {
            continue;
        }
        // No merge base ⇒ no shared history ⇒ it can't have been squash-merged into
        // base. Skip it rather than letting `git merge-base` walk the whole graph.
        let Some(merge_base) = branch.merge_base_commit.clone() else {
            continue;
        };
        let Some(commit_hash) = get_commit_hash(repo, &branch.name) else {
            continue;
        };
        if let Some(status) = cache.lookup(&branch.name, &commit_hash) {
            branch.merge_status = status;
        } else {
            let local_squash = super::merge_detection::is_squash_merged(
                repo_path,
                base_branch,
                &branch.name,
                Some(&commit_hash),
                Some(&merge_base),
            );
            // Only check remote if origin/<base> actually exists
            let has_remote_base = repo
                .find_branch(&format!("origin/{base_branch}"), git2::BranchType::Remote)
                .is_ok();
            let remote_squash = if has_remote_base {
                super::merge_detection::is_squash_merged(
                    repo_path,
                    &format!("origin/{base_branch}"),
                    &branch.name,
                    Some(&commit_hash),
                    None,
                )
            } else {
                false
            };
            let status = match (local_squash, remote_squash) {
                (true, true) => MergeStatus::SquashMerged,
                (false, true) => MergeStatus::RemoteSquashMerged,
                (true, false) => MergeStatus::LocalSquashMerged,
                (false, false) => MergeStatus::Unmerged,
            };
            branch.merge_status = status;
            cache.insert(&branch.name, &status, &commit_hash);
        }
    }

    cache.log_stats("list_branches");
    cache.save();
    info_span!("list_branches_sort", branch_count = branches.len())
        .in_scope(|| branches.sort_by_key(|b| std::cmp::Reverse(b.last_commit_date)));
    span.record("result_count", branches.len() as u64);
    Ok(branches)
}
