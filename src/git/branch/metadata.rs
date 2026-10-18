use super::Result;
use crate::types::*;
use chrono::{TimeZone, Utc};
use git2::{BranchType, Repository};
use tracing::{field, info_span, instrument, Span};

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
pub(super) fn collect_branch_metadata(
    repo: &Repository,
    base_branch: &str,
    skip_ahead_behind: bool,
    skip_merge_base: bool,
) -> Result<Vec<BranchInfo>> {
    let span = Span::current();
    let head = repo.head().ok();
    let current_branch = head
        .as_ref()
        .and_then(|h| h.shorthand().ok().map(|s| s.to_string()));
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
            squash_confidence: None,
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
