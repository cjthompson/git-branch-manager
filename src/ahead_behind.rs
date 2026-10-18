use tracing::{field, info_span, instrument, Span};

#[instrument(
    skip(repo, branches, cache),
    fields(
        branch_count = branches.len(),
        tracked_branch_count = field::Empty,
        untracked_branch_count = field::Empty,
        gone_branch_count = field::Empty,
        result_count = field::Empty,
        skipped_count = field::Empty,
        cache_hit_count = field::Empty,
        find_branch_error_count = field::Empty,
        upstream_error_count = field::Empty,
        peel_error_count = field::Empty,
        graph_success_count = field::Empty,
        graph_error_count = field::Empty,
    )
)]
pub(crate) fn compute_ahead_behind(
    repo: &git2::Repository,
    branches: &[git_branch_manager::types::BranchInfo],
    cache: &mut git_branch_manager::git::cache::BranchCache,
) -> Vec<(String, Option<u32>, Option<u32>)> {
    use git_branch_manager::types::TrackingStatus;
    let span = Span::current();
    let mut updates: Vec<(String, Option<u32>, Option<u32>)> = Vec::new();
    let mut tracked_branch_count = 0usize;
    let mut untracked_branch_count = 0usize;
    let mut gone_branch_count = 0usize;
    let mut skipped_count = 0usize;
    let mut find_branch_error_count = 0usize;
    let mut upstream_error_count = 0usize;
    let mut peel_error_count = 0usize;
    let mut graph_success_count = 0usize;
    let mut graph_error_count = 0usize;
    let mut cache_hit_count = 0usize;

    for b in branches {
        let branch_span = info_span!(
            "compute_ahead_behind_branch",
            branch_name = %b.name,
            tracking_status = field::Empty,
            remote_ref = field::Empty,
            branch_tip = field::Empty,
            upstream_oid = field::Empty,
            ahead = field::Empty,
            behind = field::Empty,
            result_state = field::Empty,
        );
        let _branch_enter = branch_span.enter();

        let remote_ref = match &b.tracking {
            TrackingStatus::Tracked {
                remote_ref,
                gone: false,
            } => {
                tracked_branch_count += 1;
                branch_span.record("tracking_status", "tracked");
                branch_span.record("remote_ref", remote_ref.as_str());
                remote_ref
            }
            TrackingStatus::Tracked {
                remote_ref,
                gone: true,
            } => {
                gone_branch_count += 1;
                skipped_count += 1;
                branch_span.record("tracking_status", "gone");
                branch_span.record("remote_ref", remote_ref.as_str());
                branch_span.record("result_state", "skipped_gone");
                continue;
            }
            TrackingStatus::Local => {
                untracked_branch_count += 1;
                skipped_count += 1;
                branch_span.record("tracking_status", "local");
                branch_span.record("result_state", "skipped_untracked");
                continue;
            }
        };

        let local_branch =
            match info_span!("compute_ahead_behind_find_branch", branch_name = %b.name)
                .in_scope(|| repo.find_branch(&b.name, git2::BranchType::Local))
            {
                Ok(local_branch) => local_branch,
                Err(_) => {
                    find_branch_error_count += 1;
                    branch_span.record("result_state", "find_branch_error");
                    continue;
                }
            };

        let upstream = match info_span!(
            "compute_ahead_behind_find_upstream",
            branch_name = %b.name,
            remote_ref = %remote_ref,
        )
        .in_scope(|| local_branch.upstream())
        {
            Ok(upstream) => upstream,
            Err(_) => {
                upstream_error_count += 1;
                branch_span.record("result_state", "upstream_error");
                continue;
            }
        };

        let branch_oid = match info_span!("compute_ahead_behind_peel_branch", branch_name = %b.name)
            .in_scope(|| local_branch.get().peel_to_commit())
        {
            Ok(c) => c.id(),
            Err(_) => {
                peel_error_count += 1;
                branch_span.record("result_state", "branch_peel_error");
                continue;
            }
        };
        let branch_oid_string = branch_oid.to_string();
        branch_span.record("branch_tip", branch_oid_string.as_str());

        let upstream_oid = match info_span!(
            "compute_ahead_behind_peel_upstream",
            branch_name = %b.name,
            remote_ref = %remote_ref,
        )
        .in_scope(|| upstream.get().peel_to_commit())
        {
            Ok(c) => c.id(),
            Err(_) => {
                peel_error_count += 1;
                branch_span.record("result_state", "upstream_peel_error");
                continue;
            }
        };
        let upstream_oid_string = upstream_oid.to_string();
        branch_span.record("upstream_oid", upstream_oid_string.as_str());

        let (ahead, behind) =
            if let Some((a, bh)) = cache.lookup_ahead_behind(branch_oid, upstream_oid) {
                cache_hit_count += 1;
                branch_span.record("ahead", a);
                branch_span.record("behind", bh);
                branch_span.record("result_state", "cache_hit");
                (Some(a), Some(bh))
            } else {
                match info_span!(
                    "compute_ahead_behind_graph",
                    branch_name = %b.name,
                    branch_tip = %branch_oid,
                    upstream_oid = %upstream_oid,
                )
                .in_scope(|| repo.graph_ahead_behind(branch_oid, upstream_oid))
                {
                    Ok((a, bh)) => {
                        graph_success_count += 1;
                        let (a, bh) = (a as u32, bh as u32);
                        cache.insert_ahead_behind(branch_oid, upstream_oid, a, bh);
                        branch_span.record("ahead", a);
                        branch_span.record("behind", bh);
                        branch_span.record("result_state", "success");
                        (Some(a), Some(bh))
                    }
                    Err(_) => {
                        graph_error_count += 1;
                        branch_span.record("result_state", "graph_error");
                        (None, None)
                    }
                }
            };
        updates.push((b.name.clone(), ahead, behind));
    }

    span.record("tracked_branch_count", tracked_branch_count);
    span.record("untracked_branch_count", untracked_branch_count);
    span.record("gone_branch_count", gone_branch_count);
    span.record("result_count", updates.len());
    span.record("skipped_count", skipped_count);
    span.record("cache_hit_count", cache_hit_count);
    span.record("find_branch_error_count", find_branch_error_count);
    span.record("upstream_error_count", upstream_error_count);
    span.record("peel_error_count", peel_error_count);
    span.record("graph_success_count", graph_success_count);
    span.record("graph_error_count", graph_error_count);
    updates
}
