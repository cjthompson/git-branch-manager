use crate::types::*;
use chrono::{TimeZone, Utc};
use git2::{BranchType, Repository};
use std::sync::mpsc;
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
#[instrument(skip(repo), fields(base_branch, result_count = field::Empty))]
pub fn list_branches_phase1(repo: &Repository, base_branch: &str) -> Result<Vec<BranchInfo>> {
    let span = Span::current();
    let mut branches = collect_branch_metadata(repo, base_branch, false, true)?;
    let reachable =
        super::merge_detection::detect_merged_branches(repo, base_branch, &mut branches)?;
    fill_merge_base_commits(repo, &mut branches, &reachable);

    // Mark unmerged non-pinned branches as Pending (for squash check)
    info_span!(
        "list_branches_phase1_mark_pending",
        branch_count = branches.len()
    )
    .in_scope(|| {
        for b in &mut branches {
            if !b.is_pinned() && b.merge_status == MergeStatus::Unmerged {
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

/// List remote branches with basic metadata (phase 1).
#[instrument(skip(repo), fields(base_branch))]
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
#[instrument(skip(branches), fields(base_branch, branch_count = branches.len()))]
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
#[instrument(skip(repo), fields(base_branch, result_count = field::Empty))]
pub fn list_branches(repo: &Repository, base_branch: &str) -> Result<Vec<BranchInfo>> {
    let span = Span::current();
    let repo_path = repo.workdir().unwrap_or_else(|| repo.path());
    let mut cache = super::cache::BranchCache::load(repo_path);
    let mut branches = collect_branch_metadata(repo, base_branch, false, true)?;
    let reachable =
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

    fill_merge_base_commits(repo, &mut branches, &reachable);
    cache.log_stats("list_branches");
    cache.save();
    info_span!("list_branches_sort", branch_count = branches.len())
        .in_scope(|| branches.sort_by_key(|b| std::cmp::Reverse(b.last_commit_date)));
    span.record("result_count", branches.len() as u64);
    Ok(branches)
}

/// For each branch, find its merge base using a bounded revwalk against the reachable set.
/// Connected branches find their divergence point in O(divergence_depth) iterations.
/// Disconnected branches bail out after LIMIT iterations and return None — avoiding the
/// full-history traversal that would happen with an unbounded walk or repo.merge_base().
const MERGE_BASE_WALK_LIMIT: usize = 1_000;

#[instrument(
    skip(repo, branches, reachable),
    fields(branch_count = branches.len(), filled_count = field::Empty, miss_count = field::Empty, limited_count = field::Empty)
)]
fn fill_merge_base_commits(
    repo: &Repository,
    branches: &mut [BranchInfo],
    reachable: &std::collections::HashSet<git2::Oid>,
) {
    if reachable.is_empty() {
        return;
    }
    let span = tracing::Span::current();
    let mut filled_count = 0usize;
    let mut miss_count = 0usize;
    let mut limited_count = 0usize;
    for branch in branches.iter_mut() {
        if branch.is_base || branch.merge_base_commit.is_some() {
            continue;
        }
        let tip_oid = match repo
            .find_branch(&branch.name, git2::BranchType::Local)
            .and_then(|b| {
                b.get()
                    .target()
                    .ok_or_else(|| git2::Error::from_str("no target"))
            }) {
            Ok(oid) => oid,
            Err(_) => {
                miss_count += 1;
                continue;
            }
        };
        // Fast path: tip itself is in reachable (branch was fast-forwarded into base).
        if reachable.contains(&tip_oid) {
            let s = tip_oid.to_string();
            branch.merge_base_commit = Some(s[..8].to_string());
            filled_count += 1;
            continue;
        }
        // Bounded walk: iterate ancestors until we hit a reachable commit (= merge base).
        // Cap at MERGE_BASE_WALK_LIMIT to avoid full traversal of disconnected histories.
        let mut found_oid: Option<git2::Oid> = None;
        let mut hit_limit = false;
        if let Ok(mut revwalk) = repo.revwalk() {
            let _ = revwalk.set_sorting(git2::Sort::NONE);
            let _ = revwalk.push(tip_oid);
            for (n, oid_result) in (&mut revwalk).enumerate() {
                if n >= MERGE_BASE_WALK_LIMIT {
                    hit_limit = true;
                    break;
                }
                if let Ok(oid) = oid_result {
                    if reachable.contains(&oid) {
                        found_oid = Some(oid);
                        break;
                    }
                }
            }
        }
        match found_oid {
            Some(oid) => {
                let s = oid.to_string();
                branch.merge_base_commit = Some(s[..8].to_string());
                filled_count += 1;
            }
            None => {
                if hit_limit {
                    limited_count += 1;
                } else {
                    miss_count += 1;
                }
            }
        }
    }
    span.record("filled_count", filled_count as u64);
    span.record("miss_count", miss_count as u64);
    span.record("limited_count", limited_count as u64);
}

#[instrument(
    skip(repo),
    fields(
        base_branch,
        skip_ahead_behind,
        skip_merge_base,
        current_branch = field::Empty,
        base_oid = field::Empty,
        result_count = field::Empty,
        branch_iter_error_count = field::Empty,
        missing_name_count = field::Empty,
        local_branch_count = field::Empty,
        tracked_branch_count = field::Empty,
        gone_branch_count = field::Empty,
        commit_peel_error_count = field::Empty,
        ahead_behind_success_count = field::Empty,
        ahead_behind_error_count = field::Empty,
        ahead_behind_skip_count = field::Empty,
        merge_base_success_count = field::Empty,
        merge_base_error_count = field::Empty,
        merge_base_skip_count = field::Empty,
        base_oid_missing_count = field::Empty,
    )
)]
fn collect_branch_metadata(
    repo: &Repository,
    base_branch: &str,
    skip_ahead_behind: bool,
    skip_merge_base: bool,
) -> Result<Vec<BranchInfo>> {
    let span = Span::current();
    let head = repo.head().ok();
    let current_branch = head
        .as_ref()
        .and_then(|h| h.shorthand().map(|s| s.to_string()));
    if let Some(current_branch) = &current_branch {
        span.record("current_branch", current_branch.as_str());
    }

    // Resolve the base branch OID once for merge-base computation
    let base_oid = info_span!("collect_branch_metadata_base_lookup", base_branch).in_scope(|| {
        repo.find_branch(base_branch, BranchType::Local)
            .ok()
            .and_then(|b| b.get().target())
    });
    if let Some(base_oid) = base_oid {
        let base_oid_string = base_oid.to_string();
        span.record("base_oid", base_oid_string.as_str());
    }

    let mut branches = Vec::new();
    let branch_iter = info_span!(
        "collect_branch_metadata_branch_iter_init",
        branch_type = "local"
    )
    .in_scope(|| repo.branches(Some(BranchType::Local)))?;

    let mut branch_iter_error_count = 0usize;
    let mut missing_name_count = 0usize;
    let mut local_branch_count = 0usize;
    let mut tracked_branch_count = 0usize;
    let mut gone_branch_count = 0usize;
    let mut commit_peel_error_count = 0usize;
    let mut ahead_behind_success_count = 0usize;
    let mut ahead_behind_error_count = 0usize;
    let mut ahead_behind_skip_count = 0usize;
    let mut merge_base_success_count = 0usize;
    let mut merge_base_error_count = 0usize;
    let mut merge_base_skip_count = 0usize;
    let mut base_oid_missing_count = 0usize;
    for branch_result in branch_iter {
        let (branch, _) = match branch_result {
            Ok(branch) => branch,
            Err(err) => {
                branch_iter_error_count += 1;
                span.record("branch_iter_error_count", branch_iter_error_count as u64);
                return Err(err.into());
            }
        };
        let name = match branch.name()? {
            Some(n) => n.to_string(),
            None => {
                missing_name_count += 1;
                continue;
            }
        };

        let is_current = current_branch.as_deref() == Some(&name);
        let is_base = name == base_branch;
        let branch_span = info_span!(
            "collect_branch_metadata_branch",
            branch_name = %name,
            is_current,
            is_base,
            tracking_status = field::Empty,
            remote_ref = field::Empty,
            branch_tip = field::Empty,
            upstream_oid = field::Empty,
            ahead = field::Empty,
            behind = field::Empty,
            merge_base_oid = field::Empty,
            result_state = field::Empty,
        );
        let _branch_enter = branch_span.enter();

        // Tracking status
        let tracking = match info_span!("collect_branch_metadata_upstream", branch_name = %name)
            .in_scope(|| branch.upstream())
        {
            Ok(upstream) => {
                let remote_ref = upstream.name()?.unwrap_or_default().to_string();
                let gone = upstream.get().target().is_none()
                    || repo
                        .find_reference(upstream.get().name().unwrap_or_default())
                        .is_err();
                tracked_branch_count += 1;
                if gone {
                    gone_branch_count += 1;
                    branch_span.record("tracking_status", "gone");
                } else {
                    branch_span.record("tracking_status", "tracked");
                }
                branch_span.record("remote_ref", remote_ref.as_str());
                TrackingStatus::Tracked { remote_ref, gone }
            }
            Err(_) => {
                local_branch_count += 1;
                branch_span.record("tracking_status", "local");
                TrackingStatus::Local
            }
        };

        // Commit date
        let commit = match info_span!("collect_branch_metadata_peel_commit", branch_name = %name)
            .in_scope(|| branch.get().peel_to_commit())
        {
            Ok(commit) => commit,
            Err(err) => {
                commit_peel_error_count += 1;
                branch_span.record("result_state", "commit_peel_error");
                span.record("commit_peel_error_count", commit_peel_error_count as u64);
                return Err(err.into());
            }
        };
        let branch_tip = commit.id().to_string();
        branch_span.record("branch_tip", branch_tip.as_str());
        let time = commit.committer().when();
        let date = Utc
            .timestamp_opt(time.seconds(), 0)
            .single()
            .unwrap_or_else(Utc::now);

        // Ahead/behind (only for tracked, non-gone branches; skipped in fast path)
        let (ahead, behind) = if skip_ahead_behind {
            ahead_behind_skip_count += 1;
            (None, None)
        } else {
            match &tracking {
                TrackingStatus::Tracked { gone: false, .. } => {
                    let branch_oid = commit.id();
                    match info_span!(
                        "collect_branch_metadata_ahead_behind_upstream",
                        branch_name = %name,
                    )
                    .in_scope(|| branch.upstream())
                    {
                        Ok(upstream) => {
                            let upstream_oid = match info_span!(
                                "collect_branch_metadata_ahead_behind_peel_upstream",
                                branch_name = %name,
                            )
                            .in_scope(|| upstream.get().peel_to_commit())
                            {
                                Ok(commit) => commit.id(),
                                Err(err) => {
                                    ahead_behind_error_count += 1;
                                    branch_span
                                        .record("result_state", "ahead_behind_upstream_peel_error");
                                    span.record(
                                        "ahead_behind_error_count",
                                        ahead_behind_error_count as u64,
                                    );
                                    return Err(err.into());
                                }
                            };
                            let upstream_oid_string = upstream_oid.to_string();
                            branch_span.record("upstream_oid", upstream_oid_string.as_str());
                            match info_span!(
                                "collect_branch_metadata_ahead_behind_graph",
                                branch_name = %name,
                                branch_tip = %branch_oid,
                                upstream_oid = %upstream_oid,
                            )
                            .in_scope(|| repo.graph_ahead_behind(branch_oid, upstream_oid))
                            {
                                Ok((a, b)) => {
                                    ahead_behind_success_count += 1;
                                    branch_span.record("ahead", a);
                                    branch_span.record("behind", b);
                                    (Some(a as u32), Some(b as u32))
                                }
                                Err(err) => {
                                    ahead_behind_error_count += 1;
                                    branch_span.record("result_state", "ahead_behind_graph_error");
                                    span.record(
                                        "ahead_behind_error_count",
                                        ahead_behind_error_count as u64,
                                    );
                                    return Err(err.into());
                                }
                            }
                        }
                        Err(_) => {
                            ahead_behind_skip_count += 1;
                            branch_span.record("result_state", "ahead_behind_upstream_missing");
                            (None, None)
                        }
                    }
                }
                _ => {
                    ahead_behind_skip_count += 1;
                    (None, None)
                }
            }
        };

        // Compute merge-base commit for non-base branches.
        // skip_merge_base=true defers this to fill_merge_base_commits (reachable-set walk).
        let merge_base_commit = if skip_ahead_behind || skip_merge_base || is_base {
            merge_base_skip_count += 1;
            None
        } else if let Some(base_oid) = base_oid {
            let branch_oid = commit.id();
            match info_span!(
                "collect_branch_metadata_merge_base",
                branch_name = %name,
                base_oid = %base_oid,
                branch_tip = %branch_oid,
            )
            .in_scope(|| repo.merge_base(base_oid, branch_oid))
            {
                Ok(oid) => {
                    merge_base_success_count += 1;
                    let oid = oid.to_string();
                    branch_span.record("merge_base_oid", oid.as_str());
                    Some(oid[..8].to_string())
                }
                Err(_) => {
                    merge_base_error_count += 1;
                    None
                }
            }
        } else {
            base_oid_missing_count += 1;
            merge_base_skip_count += 1;
            None
        };
        if !matches!(&tracking, TrackingStatus::Tracked { gone: false, .. }) && !skip_ahead_behind {
            branch_span.record("result_state", "success_without_graph");
        } else {
            branch_span.record("result_state", "success");
        }

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

    span.record("result_count", branches.len() as u64);
    span.record("branch_iter_error_count", branch_iter_error_count as u64);
    span.record("missing_name_count", missing_name_count as u64);
    span.record("local_branch_count", local_branch_count as u64);
    span.record("tracked_branch_count", tracked_branch_count as u64);
    span.record("gone_branch_count", gone_branch_count as u64);
    span.record("commit_peel_error_count", commit_peel_error_count as u64);
    span.record(
        "ahead_behind_success_count",
        ahead_behind_success_count as u64,
    );
    span.record("ahead_behind_error_count", ahead_behind_error_count as u64);
    span.record("ahead_behind_skip_count", ahead_behind_skip_count as u64);
    span.record("merge_base_success_count", merge_base_success_count as u64);
    span.record("merge_base_error_count", merge_base_error_count as u64);
    span.record("merge_base_skip_count", merge_base_skip_count as u64);
    span.record("base_oid_missing_count", base_oid_missing_count as u64);
    Ok(branches)
}
