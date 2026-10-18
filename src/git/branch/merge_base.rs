use crate::types::*;
use git2::Repository;
use tracing::{field, instrument};

/// For each branch, find its merge base using a bounded revwalk against the reachable set.
/// Connected branches find their divergence point in O(divergence_depth) iterations.
/// Disconnected branches bail out after LIMIT iterations and return None — avoiding the
/// full-history traversal that would happen with an unbounded walk or repo.merge_base().
const MERGE_BASE_WALK_LIMIT: usize = 1_000;

#[instrument(
    skip(repo, branches, reachable),
    fields(branch_count = branches.len(), filled_count = field::Empty, miss_count = field::Empty, limited_count = field::Empty)
)]
pub fn fill_merge_base_commits(
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

/// Compute a branch's merge base with the base branch exactly as the cache
/// stores it: the **short (8-char)** hash, or `None` when disconnected. Uses the
/// precomputed `reachable` set and a revwalk bounded by `MERGE_BASE_WALK_LIMIT`,
/// so it never traverses the full history of a disjoint branch.
///
/// Returns `(merge_base, hit_limit)`; `hit_limit` is true when the bounded walk
/// gave up without finding a base, letting callers distinguish "limited" from a
/// genuine miss. The diagnostics audit reuses this so its recomputed truth
/// matches the cached value byte-for-byte.
pub fn compute_merge_base_short(
    repo: &Repository,
    tip_oid: git2::Oid,
    reachable: &std::collections::HashSet<git2::Oid>,
) -> (Option<String>, bool) {
    if reachable.is_empty() {
        return (None, false);
    }
    if reachable.contains(&tip_oid) {
        return (Some(tip_oid.to_string()[..8].to_string()), false);
    }
    let Ok(mut revwalk) = repo.revwalk() else {
        return (None, false);
    };
    let _ = revwalk.set_sorting(git2::Sort::NONE);
    let _ = revwalk.push(tip_oid);
    for (n, oid_result) in (&mut revwalk).enumerate() {
        if n >= MERGE_BASE_WALK_LIMIT {
            return (None, true);
        }
        if let Ok(oid) = oid_result {
            if reachable.contains(&oid) {
                return (Some(oid.to_string()[..8].to_string()), false);
            }
        }
    }
    (None, false)
}

/// Like fill_merge_base_commits, but uses BranchCache to skip the bounded revwalk on
/// cache hits. Results are stored back into the cache for future calls.
/// base_tip_oid is the current tip of the base branch (used as part of the cache key).
#[instrument(
    skip(repo, branches, reachable, cache),
    fields(
        branch_count = branches.len(),
        filled_count = field::Empty,
        cache_hit_count = field::Empty,
        miss_count = field::Empty,
        limited_count = field::Empty,
    )
)]
pub fn fill_merge_base_commits_cached(
    repo: &Repository,
    branches: &mut [BranchInfo],
    reachable: &std::collections::HashSet<git2::Oid>,
    base_tip_oid: git2::Oid,
    cache: &mut crate::git::cache::BranchCache,
) {
    let span = tracing::Span::current();
    let mut filled_count = 0usize;
    let mut cache_hit_count = 0usize;
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
        // Cache lookup: None = miss, Some(None) = disconnected, Some(Some(h)) = found
        if let Some(cached) = cache.lookup_merge_base(tip_oid, base_tip_oid) {
            branch.merge_base_commit = cached;
            cache_hit_count += 1;
            if branch.merge_base_commit.is_some() {
                filled_count += 1;
            }
            continue;
        }
        // Cache miss: compute via the same bounded walk the diagnostics audit uses.
        let (mb, hit_limit) = compute_merge_base_short(repo, tip_oid, reachable);
        match mb {
            Some(hash) => {
                branch.merge_base_commit = Some(hash.clone());
                cache.insert_merge_base(tip_oid, base_tip_oid, Some(hash));
                filled_count += 1;
            }
            None => {
                cache.insert_merge_base(tip_oid, base_tip_oid, None);
                if hit_limit {
                    limited_count += 1;
                } else {
                    miss_count += 1;
                }
            }
        }
    }
    span.record("filled_count", filled_count as u64);
    span.record("cache_hit_count", cache_hit_count as u64);
    span.record("miss_count", miss_count as u64);
    span.record("limited_count", limited_count as u64);
}
