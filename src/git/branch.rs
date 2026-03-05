use anyhow::Result;
use chrono::DateTime;
use git2::{BranchType, Repository};
use thiserror::Error;

use crate::types::{BranchInfo, MergeStatus, TrackingStatus};

use super::cache::BranchCache;
use super::merge_detection::{detect_merged_branches, is_squash_merged};

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
}

/// Detect the base branch, either from an explicit override or by auto-detection.
pub fn detect_base_branch(repo: &Repository, override_base: Option<&str>) -> Result<String> {
    if let Some(name) = override_base {
        // Validate the override branch exists
        repo.find_branch(name, BranchType::Local)
            .map_err(|_| GitError::BaseBranchNotFound(name.to_string()))?;
        return Ok(name.to_string());
    }

    // Try refs/remotes/origin/HEAD symref
    if let Ok(reference) = repo.find_reference("refs/remotes/origin/HEAD")
        && let Ok(resolved) = reference.resolve()
        && let Some(name) = resolved.shorthand()
        && let Some(branch_name) = name.strip_prefix("origin/")
        && repo.find_branch(branch_name, BranchType::Local).is_ok()
    {
        return Ok(branch_name.to_string());
    }

    // Fallback chain
    for candidate in &["main", "master", "develop"] {
        if repo.find_branch(candidate, BranchType::Local).is_ok() {
            return Ok(candidate.to_string());
        }
    }

    Err(GitError::CannotDetectBase.into())
}

/// List all local branches with metadata and regular merge detection, sorted by date.
///
/// Does NOT run squash-merge detection — call `spawn_squash_checker` for that.
/// Used by the TUI path for instant startup.
pub fn list_branches_phase1(repo: &Repository, base_branch: &str) -> Result<Vec<BranchInfo>> {
    let mut branches = collect_branch_metadata(repo, base_branch)?;
    detect_merged_branches(repo, base_branch, &mut branches)?;
    branches.sort_by(|a, b| b.last_commit_date.cmp(&a.last_commit_date));
    Ok(branches)
}

/// List all local branches with full metadata including squash-merge detection.
///
/// Used by `--list` mode (synchronous) and integration tests.
/// Loads and updates the squash-merge cache automatically.
pub fn list_branches(repo: &Repository, base_branch: &str) -> Result<Vec<BranchInfo>> {
    let repo_path = repo.workdir().unwrap_or_else(|| repo.path());
    let mut cache = BranchCache::load(repo_path);
    let mut branches = collect_branch_metadata(repo, base_branch)?;
    detect_merged_branches(repo, base_branch, &mut branches)?;

    for branch in branches.iter_mut() {
        if branch.merge_status != MergeStatus::Unmerged || branch.is_base || branch.is_current {
            continue;
        }
        let Some(commit_hash) = get_commit_hash(repo, &branch.name) else {
            continue;
        };
        if let Some(status) = cache.lookup(&branch.name, &commit_hash) {
            branch.merge_status = status;
        } else if is_squash_merged(repo_path, base_branch, &branch.name) {
            branch.merge_status = MergeStatus::SquashMerged;
            cache.insert(&branch.name, &MergeStatus::SquashMerged, &commit_hash);
        } else {
            cache.insert(&branch.name, &MergeStatus::Unmerged, &commit_hash);
        }
    }

    cache.save();
    branches.sort_by(|a, b| b.last_commit_date.cmp(&a.last_commit_date));
    Ok(branches)
}

/// List all local branches with full metadata including cached squash-merge detection.
///
/// Used by `--list` mode when a cache is available.
pub fn list_branches_cached(
    repo: &Repository,
    base_branch: &str,
    cache: &mut BranchCache,
) -> Result<Vec<BranchInfo>> {
    let repo_path = repo.workdir().unwrap_or_else(|| repo.path());
    let mut branches = collect_branch_metadata(repo, base_branch)?;
    detect_merged_branches(repo, base_branch, &mut branches)?;

    for branch in branches.iter_mut() {
        if branch.merge_status != MergeStatus::Unmerged || branch.is_base || branch.is_current {
            continue;
        }
        let Some(commit_hash) = get_commit_hash(repo, &branch.name) else {
            continue;
        };
        if let Some(status) = cache.lookup(&branch.name, &commit_hash) {
            branch.merge_status = status;
        } else if is_squash_merged(repo_path, base_branch, &branch.name) {
            branch.merge_status = MergeStatus::SquashMerged;
            cache.insert(&branch.name, &MergeStatus::SquashMerged, &commit_hash);
        } else {
            cache.insert(&branch.name, &MergeStatus::Unmerged, &commit_hash);
        }
    }

    branches.sort_by(|a, b| b.last_commit_date.cmp(&a.last_commit_date));
    Ok(branches)
}

/// Get the HEAD commit hash of a local branch.
pub fn get_commit_hash(repo: &Repository, branch_name: &str) -> Option<String> {
    repo.find_branch(branch_name, BranchType::Local)
        .ok()
        .and_then(|b| b.get().peel_to_commit().ok())
        .map(|c| c.id().to_string())
}

/// Collect metadata for all local branches (no merge detection).
fn collect_branch_metadata(repo: &Repository, base_branch: &str) -> Result<Vec<BranchInfo>> {
    let branches_iter = repo.branches(Some(BranchType::Local))?;
    let mut branches: Vec<BranchInfo> = Vec::new();

    for branch_result in branches_iter {
        let (branch, _) = branch_result?;

        let name = match branch.name()? {
            Some(n) => n.to_string(),
            None => continue, // skip branches with non-UTF8 names
        };

        let is_current = branch.is_head();
        let is_base = name == base_branch;

        // Tracking status
        let tracking = match branch.upstream() {
            Ok(upstream) => {
                let remote_ref = upstream
                    .name()?
                    .unwrap_or("")
                    .to_string();
                TrackingStatus::Tracked {
                    remote_ref,
                    gone: false,
                }
            }
            Err(e) => {
                // Check if the upstream is configured but gone
                if e.code() == git2::ErrorCode::NotFound {
                    // Check if there's a configured upstream that's gone
                    let config = repo.config()?;
                    let merge_key = format!("branch.{}.merge", name);
                    if config.get_string(&merge_key).is_ok() {
                        // Upstream was configured but the remote branch is gone
                        let remote_key = format!("branch.{}.remote", name);
                        let remote = config.get_string(&remote_key).unwrap_or_default();
                        let merge_ref = config.get_string(&merge_key).unwrap_or_default();
                        let remote_ref = format!(
                            "{}/{}",
                            remote,
                            merge_ref.strip_prefix("refs/heads/").unwrap_or(&merge_ref)
                        );
                        TrackingStatus::Tracked {
                            remote_ref,
                            gone: true,
                        }
                    } else {
                        TrackingStatus::Local
                    }
                } else {
                    TrackingStatus::Local
                }
            }
        };

        // Last commit date
        let commit = branch.get().peel_to_commit()?;
        let time = commit.committer().when();
        let last_commit_date = DateTime::from_timestamp(time.seconds(), 0)
            .unwrap_or_default();

        // Ahead/behind counts (only for tracked, non-gone branches)
        let (ahead, behind) = match &tracking {
            TrackingStatus::Tracked { gone: false, .. } => {
                let branch_oid = commit.id();
                if let Ok(upstream) = branch.upstream() {
                    if let Ok(upstream_commit) = upstream.get().peel_to_commit() {
                        match repo.graph_ahead_behind(branch_oid, upstream_commit.id()) {
                            Ok((a, b)) => (Some(a as u32), Some(b as u32)),
                            Err(_) => (None, None),
                        }
                    } else {
                        (None, None)
                    }
                } else {
                    (None, None)
                }
            }
            _ => (None, None),
        };

        branches.push(BranchInfo {
            name,
            is_current,
            is_base,
            tracking,
            ahead,
            behind,
            last_commit_date,
            merge_status: MergeStatus::Unmerged,
        });
    }

    Ok(branches)
}
