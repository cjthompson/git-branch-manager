use crate::types::*;
use chrono::{TimeZone, Utc};
use git2::{BranchType, Repository};
use std::sync::mpsc;
use thiserror::Error;
use tracing::instrument;

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
        if let Some(target) = reference.symbolic_target() {
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
pub fn list_branches_phase1(repo: &Repository, base_branch: &str) -> Result<Vec<BranchInfo>> {
    let mut branches = collect_branch_metadata(repo, base_branch)?;
    super::merge_detection::detect_merged_branches(repo, base_branch, &mut branches)?;

    // Mark unmerged non-pinned branches as Pending (for squash check)
    for b in &mut branches {
        if !b.is_pinned() && b.merge_status == MergeStatus::Unmerged {
            b.merge_status = MergeStatus::Pending;
        }
    }

    // Sort: pinned first, then by date descending
    branches.sort_by(|a, b| {
        b.is_pinned()
            .cmp(&a.is_pinned())
            .then(b.last_commit_date.cmp(&a.last_commit_date))
    });

    Ok(branches)
}

/// Fast metadata-only pass: collects branch info without merge detection.
/// All non-pinned branches start as Pending; merge statuses are filled in
/// asynchronously via a subsequent detect_merged_branches call.
#[instrument(skip(repo), fields(base_branch))]
pub fn list_branches_fast(repo: &Repository, base_branch: &str) -> Result<Vec<BranchInfo>> {
    let mut branches = collect_branch_metadata(repo, base_branch)?;
    for b in &mut branches {
        if !b.is_pinned() {
            b.merge_status = MergeStatus::Pending;
        }
    }
    branches.sort_by(|a, b| {
        b.is_pinned()
            .cmp(&a.is_pinned())
            .then(b.last_commit_date.cmp(&a.last_commit_date))
    });
    Ok(branches)
}

/// Get the commit hash for a local branch.
pub fn get_commit_hash(repo: &Repository, branch_name: &str) -> Option<String> {
    repo.find_branch(branch_name, BranchType::Local)
        .ok()
        .and_then(|b| b.get().peel_to_commit().ok())
        .map(|c| c.id().to_string())
}

/// List remote branches with basic metadata (phase 1).
pub fn list_remote_branches_phase1(
    repo: &Repository,
    base_branch: &str,
) -> Result<Vec<RemoteBranchInfo>> {
    let mut remotes = Vec::new();
    let branches = repo.branches(Some(BranchType::Remote))?;

    for branch_result in branches {
        let (branch, _) = branch_result?;
        let name = match branch.name()? {
            Some(n) => n.to_string(),
            None => continue,
        };

        // Skip HEAD pseudo-refs
        if name.ends_with("/HEAD") {
            continue;
        }

        let (remote, short_name) = match name.split_once('/') {
            Some((r, s)) => (r.to_string(), s.to_string()),
            None => continue,
        };

        let commit = branch.get().peel_to_commit()?;
        let time = commit.committer().when();
        let date = Utc
            .timestamp_opt(time.seconds(), 0)
            .single()
            .unwrap_or_else(Utc::now);

        let has_local = repo.find_branch(&short_name, BranchType::Local).is_ok();
        let is_base = short_name == base_branch;

        remotes.push(RemoteBranchInfo {
            full_ref: name,
            remote,
            short_name,
            has_local,
            is_base,
            last_commit_date: date,
            merge_status: if is_base {
                MergeStatus::Merged
            } else {
                MergeStatus::Pending
            },
            ahead: None,
            behind: None,
            pr: None,
        });
    }

    remotes.sort_by(|a, b| {
        b.is_pinned()
            .cmp(&a.is_pinned())
            .then(b.last_commit_date.cmp(&a.last_commit_date))
    });

    Ok(remotes)
}

/// Spawn a background thread that enriches remote branches with ahead/behind and merge status.
pub fn spawn_remote_enricher(
    repo_path: std::path::PathBuf,
    base_branch: String,
    branches: Vec<RemoteBranchInfo>,
) -> mpsc::Receiver<RemoteEnrichResult> {
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        let repo = match Repository::open(&repo_path) {
            Ok(r) => r,
            Err(_) => return,
        };

        // Prefer local base ref, fall back to remote tracking
        let base_oid = repo
            .find_branch(&base_branch, BranchType::Local)
            .or_else(|_| repo.find_branch(&format!("origin/{base_branch}"), BranchType::Remote))
            .and_then(|b| {
                b.get()
                    .target()
                    .ok_or_else(|| git2::Error::from_str("no target"))
            })
            .ok();

        let base_oid = match base_oid {
            Some(oid) => oid,
            None => return,
        };

        for branch in &branches {
            if branch.is_base {
                continue;
            }

            let branch_ref = format!("refs/remotes/{}", branch.full_ref);
            let branch_oid = match repo
                .find_reference(&branch_ref)
                .and_then(|r| r.target().ok_or_else(|| git2::Error::from_str("no target")))
            {
                Ok(oid) => oid,
                Err(_) => continue,
            };

            let merge_status = if repo
                .graph_descendant_of(base_oid, branch_oid)
                .unwrap_or(false)
            {
                MergeStatus::Merged
            } else {
                MergeStatus::Unmerged
            };

            let (ahead, behind) = repo
                .graph_ahead_behind(branch_oid, base_oid)
                .map(|(a, b)| (Some(a as u32), Some(b as u32)))
                .unwrap_or((None, None));

            let result = RemoteEnrichResult {
                full_ref: branch.full_ref.clone(),
                merge_status,
                ahead,
                behind,
            };
            if tx.send(result).is_err() {
                return;
            }
        }
    });

    rx
}

/// List all local branches with full metadata including squash-merge detection.
/// Synchronous — runs squash checks inline. Used by `--list` mode and tests.
pub fn list_branches(repo: &Repository, base_branch: &str) -> Result<Vec<BranchInfo>> {
    let repo_path = repo.workdir().unwrap_or_else(|| repo.path());
    let mut cache = super::cache::BranchCache::load(repo_path);
    let mut branches = collect_branch_metadata(repo, base_branch)?;
    super::merge_detection::detect_merged_branches(repo, base_branch, &mut branches)?;

    for branch in branches.iter_mut() {
        if branch.merge_status != MergeStatus::Unmerged || branch.is_base || branch.is_current {
            continue;
        }
        let Some(commit_hash) = get_commit_hash(repo, &branch.name) else {
            continue;
        };
        if let Some(status) = cache.lookup(&branch.name, &commit_hash) {
            branch.merge_status = status;
        } else if super::merge_detection::is_squash_merged(
            repo_path,
            base_branch,
            &branch.name,
            None,
        ) {
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

#[instrument(skip(repo), fields(base_branch))]
fn collect_branch_metadata(repo: &Repository, base_branch: &str) -> Result<Vec<BranchInfo>> {
    let head = repo.head().ok();
    let current_branch = head
        .as_ref()
        .and_then(|h| h.shorthand().map(|s| s.to_string()));

    // Resolve the base branch OID once for merge-base computation
    let base_oid = repo
        .find_branch(base_branch, BranchType::Local)
        .ok()
        .and_then(|b| b.get().target());

    let mut branches = Vec::new();
    let branch_iter = repo.branches(Some(BranchType::Local))?;

    for branch_result in branch_iter {
        let (branch, _) = branch_result?;
        let name = match branch.name()? {
            Some(n) => n.to_string(),
            None => continue,
        };

        let is_current = current_branch.as_deref() == Some(&name);
        let is_base = name == base_branch;

        // Tracking status
        let tracking = match branch.upstream() {
            Ok(upstream) => {
                let remote_ref = upstream.name()?.unwrap_or_default().to_string();
                let gone = upstream.get().target().is_none()
                    || repo
                        .find_reference(upstream.get().name().unwrap_or_default())
                        .is_err();
                TrackingStatus::Tracked { remote_ref, gone }
            }
            Err(_) => TrackingStatus::Local,
        };

        // Commit date
        let commit = branch.get().peel_to_commit()?;
        let time = commit.committer().when();
        let date = Utc
            .timestamp_opt(time.seconds(), 0)
            .single()
            .unwrap_or_else(Utc::now);

        // Ahead/behind (only for tracked, non-gone branches)
        let (ahead, behind) = match &tracking {
            TrackingStatus::Tracked { gone: false, .. } => {
                let branch_oid = commit.id();
                match branch.upstream() {
                    Ok(upstream) => {
                        let upstream_oid = upstream.get().peel_to_commit()?.id();
                        let (a, b) = repo.graph_ahead_behind(branch_oid, upstream_oid)?;
                        (Some(a as u32), Some(b as u32))
                    }
                    Err(_) => (None, None),
                }
            }
            _ => (None, None),
        };

        // Compute merge-base commit for non-base branches
        let merge_base_commit = if !is_base {
            if let Some(base_oid) = base_oid {
                let branch_oid = commit.id();
                repo.merge_base(base_oid, branch_oid)
                    .ok()
                    .map(|oid| oid.to_string()[..8].to_string())
            } else {
                None
            }
        } else {
            None
        };

        branches.push(BranchInfo {
            name,
            is_current,
            is_base,
            tracking,
            ahead,
            behind,
            last_commit_date: date,
            merge_status: MergeStatus::Unmerged, // detect_merged_branches fills this in
            base_branch: base_branch.to_string(),
            merge_base_commit,
            pr: None,
        });
    }

    Ok(branches)
}
