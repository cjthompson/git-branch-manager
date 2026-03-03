use anyhow::Result;
use chrono::DateTime;
use git2::{BranchType, Repository};
use thiserror::Error;

use crate::types::{BranchInfo, MergeStatus, TrackingStatus};

use super::merge_detection::{detect_merged_branches, detect_squash_merged_branches};

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

/// List all local branches with metadata, merge status detection, sorted by date descending.
pub fn list_branches(repo: &Repository, base_branch: &str) -> Result<Vec<BranchInfo>> {
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

        branches.push(BranchInfo {
            name,
            is_current,
            is_base,
            tracking,
            last_commit_date,
            merge_status: MergeStatus::Unmerged,
        });
    }

    // Detect merge statuses
    detect_merged_branches(repo, base_branch, &mut branches)?;
    detect_squash_merged_branches(base_branch, &mut branches);

    // Sort by last_commit_date descending (most recent first)
    branches.sort_by(|a, b| b.last_commit_date.cmp(&a.last_commit_date));

    Ok(branches)
}
