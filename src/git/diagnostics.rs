//! Cache-accuracy diagnostics.
//!
//! The TUI serves merge status, ahead/behind counts, and merge bases out of a
//! persistent SQLite cache (see [`crate::git::cache`]). Most of that data is
//! keyed by commit OID and so self-invalidates, but merged / squash-merged
//! statuses are *permanent* and keyed only by branch name — they can drift from
//! reality after a branch is reused, force-pushed, or gets new commits.
//!
//! [`audit_cache`] recomputes the truth directly from git (no cache) and diffs
//! it against what the cache would serve, producing a [`CacheAudit`].
//! [`apply_fix`] writes the freshly-computed truth back and removes orphan rows.

use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use git2::{Oid, Repository};

use crate::git::branch;
use crate::git::cache::BranchCache;
use crate::git::merge_detection::{build_reachable_set_from_repo, is_squash_merged, BaseReachable};
use crate::types::{CacheAudit, CacheFix, DiagKind, Discrepancy, MergeStatus};

/// Shared read-only context for one audit pass.
struct AuditCtx<'a> {
    repo: &'a Repository,
    repo_path: &'a Path,
    base_branch: &'a str,
    current_branch: String,
    reachable: BaseReachable,
    base_oid: Option<Oid>,
    cache: &'a BranchCache,
}

/// Verify the on-disk cache against current git reality.
///
/// For every cached entry that the app would actually serve, the corresponding
/// truth is recomputed from scratch and compared. Only entries the cache would
/// return are checked — a cache *miss* (e.g. a stale unmerged row whose commit
/// changed) is not a discrepancy, since the app recomputes those on demand.
///
/// `progress(completed, total, current_branch)` is invoked per branch so the
/// caller can drive a progress overlay. `cancel` is polled between branches;
/// when set, the audit returns whatever it has gathered so far.
pub fn audit_cache(
    repo: &Repository,
    repo_path: &Path,
    base_branch: &str,
    cache: &BranchCache,
    cancel: &AtomicBool,
    progress: impl Fn(usize, usize, &str),
) -> CacheAudit {
    let mut audit = CacheAudit::default();

    // Base tip + the set of commits reachable from base, computed once.
    let ctx = AuditCtx {
        repo,
        repo_path,
        base_branch,
        current_branch: repo
            .head()
            .ok()
            .and_then(|h| h.shorthand().map(|s| s.to_string()))
            .unwrap_or_default(),
        base_oid: repo
            .find_branch(base_branch, git2::BranchType::Local)
            .ok()
            .and_then(|b| b.get().target()),
        reachable: build_reachable_set_from_repo(repo, base_branch),
        cache,
    };

    // Enumerate local branches with their current tips.
    let locals = local_branches(repo);
    let live: HashSet<&str> = locals.iter().map(|(name, _)| name.as_str()).collect();
    let total = locals.len();

    for (i, (name, tip)) in locals.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return audit;
        }
        progress(i, total, name);

        verify_merge_status(&ctx, name, *tip, &mut audit);
        verify_ahead_behind(&ctx, name, &mut audit);
        if let Some(base_oid) = ctx.base_oid {
            verify_merge_base(&ctx, name, *tip, base_oid, &mut audit);
        }
    }

    // Orphans: cached merge-status rows whose branch no longer exists.
    for cached_name in cache.cached_branch_names() {
        if !live.contains(cached_name.as_str()) {
            audit.orphans.push(cached_name);
        }
    }
    audit.orphans.sort();

    audit
}

/// Apply the corrections from a [`CacheAudit`] to `cache` and persist them.
/// Overwrites each discrepant entry with the freshly-computed value and removes
/// orphan rows. The cache's existing correct entries are left untouched.
pub fn apply_fix(cache: &mut BranchCache, audit: &CacheAudit) {
    for d in &audit.discrepancies {
        match &d.fix {
            CacheFix::Status {
                commit_hash,
                status,
            } => {
                cache.insert(&d.branch, status, commit_hash);
            }
            CacheFix::AheadBehind {
                branch_oid,
                upstream_oid,
                ahead,
                behind,
            } => {
                if let (Ok(b), Ok(u)) = (Oid::from_str(branch_oid), Oid::from_str(upstream_oid)) {
                    cache.insert_ahead_behind(b, u, *ahead, *behind);
                }
            }
            CacheFix::MergeBase {
                branch_tip,
                base_tip,
                merge_base,
            } => {
                if let (Ok(t), Ok(b)) = (Oid::from_str(branch_tip), Oid::from_str(base_tip)) {
                    cache.insert_merge_base(t, b, merge_base.clone());
                }
            }
        }
    }
    for orphan in &audit.orphans {
        cache.delete_branch_entry(orphan);
    }
    cache.save();
}

/// All local branches with their current tip OIDs.
fn local_branches(repo: &Repository) -> Vec<(String, Oid)> {
    let mut out = Vec::new();
    if let Ok(branches) = repo.branches(Some(git2::BranchType::Local)) {
        for (branch, _) in branches.flatten() {
            if let (Ok(Some(name)), Some(oid)) = (branch.name(), branch.get().target()) {
                out.push((name.to_string(), oid));
            }
        }
    }
    out
}

fn verify_merge_status(ctx: &AuditCtx, name: &str, tip: Oid, audit: &mut CacheAudit) {
    let commit_hash = tip.to_string();

    // Always compute truth so the squash check genuinely runs for every
    // non-reachable branch, proportional to branch count.
    let truth = truth_merge_status(ctx, name, tip);

    match ctx.cache.lookup(name, &commit_hash) {
        Some(cached) => {
            // Cache hit: compare against truth.
            if cached == truth {
                audit.merge_status.verified += 1;
            } else {
                audit.merge_status.mismatched += 1;
                audit.discrepancies.push(Discrepancy {
                    branch: name.to_string(),
                    kind: DiagKind::MergeStatus,
                    cached: status_label(cached).to_string(),
                    actual: status_label(truth).to_string(),
                    fix: CacheFix::Status {
                        commit_hash,
                        status: truth,
                    },
                });
            }
        }
        None => {
            // No cache row — the app recomputes these on demand, so this
            // is not drift. Record it as skipped with a human-readable reason.
            let reason = if name == ctx.base_branch {
                "base branch"
            } else if name == ctx.current_branch {
                "current branch"
            } else {
                "no cached status"
            };
            audit.merge_status.skipped += 1;
            audit.merge_status.skip_reasons.push(reason);
        }
    }
}

/// Recompute a branch's merge status from scratch, mirroring the detection
/// pipeline: regular-merge via the reachable set, then squash-merge, else unmerged.
fn truth_merge_status(ctx: &AuditCtx, name: &str, tip: Oid) -> MergeStatus {
    // Check regular merge first (covers Merged / LocalMerged / RemoteMerged).
    if let Some(status) = ctx.reachable.regular_merge_status(tip) {
        return status;
    }
    // Not regularly merged — check squash-merge against local base.
    let merge_base = ctx
        .base_oid
        .and_then(|b| ctx.repo.merge_base(tip, b).ok())
        .map(|o| o.to_string());
    let tip_str = tip.to_string();
    let local_squash = is_squash_merged(
        ctx.repo_path,
        ctx.base_branch,
        name,
        Some(&tip_str),
        merge_base.as_deref(),
    );
    let remote_base = format!("origin/{}", ctx.base_branch);
    let remote_squash = is_squash_merged(ctx.repo_path, &remote_base, name, Some(&tip_str), None);
    match (local_squash, remote_squash) {
        (true, true) => MergeStatus::SquashMerged,
        (false, true) => MergeStatus::RemoteSquashMerged,
        (true, false) => MergeStatus::LocalSquashMerged,
        (false, false) => MergeStatus::Unmerged,
    }
}

fn verify_ahead_behind(ctx: &AuditCtx, name: &str, audit: &mut CacheAudit) {
    let Some((branch_oid, upstream_oid)) = branch_and_upstream_oid(ctx.repo, name) else {
        return;
    };
    let Some(cached) = ctx.cache.lookup_ahead_behind(branch_oid, upstream_oid) else {
        return;
    };
    let Ok((a, b)) = ctx.repo.graph_ahead_behind(branch_oid, upstream_oid) else {
        return;
    };
    let truth = (a as u32, b as u32);
    if cached == truth {
        audit.ahead_behind.verified += 1;
    } else {
        audit.ahead_behind.mismatched += 1;
        audit.discrepancies.push(Discrepancy {
            branch: name.to_string(),
            kind: DiagKind::AheadBehind,
            cached: format!("{}\u{2191} {}\u{2193}", cached.0, cached.1),
            actual: format!("{}\u{2191} {}\u{2193}", truth.0, truth.1),
            fix: CacheFix::AheadBehind {
                branch_oid: branch_oid.to_string(),
                upstream_oid: upstream_oid.to_string(),
                ahead: truth.0,
                behind: truth.1,
            },
        });
    }
}

fn verify_merge_base(ctx: &AuditCtx, name: &str, tip: Oid, base_oid: Oid, audit: &mut CacheAudit) {
    let Some(cached) = ctx.cache.lookup_merge_base(tip, base_oid) else {
        return;
    };
    // Recompute exactly as the cache was filled (short hash, bounded walk) so a
    // correct cache compares equal — comparing against the full unbounded
    // `repo.merge_base` would mismatch on representation alone.
    let truth = branch::compute_merge_base_short(ctx.repo, tip, &ctx.reachable.local).0;
    if cached == truth {
        audit.merge_base.verified += 1;
    } else {
        audit.merge_base.mismatched += 1;
        audit.discrepancies.push(Discrepancy {
            branch: name.to_string(),
            kind: DiagKind::MergeBase,
            cached: short_oid(cached.as_deref()),
            actual: short_oid(truth.as_deref()),
            fix: CacheFix::MergeBase {
                branch_tip: tip.to_string(),
                base_tip: base_oid.to_string(),
                merge_base: truth,
            },
        });
    }
}

/// Resolve a local branch's tip OID and its upstream's tip OID, if it tracks one.
fn branch_and_upstream_oid(repo: &Repository, name: &str) -> Option<(Oid, Oid)> {
    let local = repo.find_branch(name, git2::BranchType::Local).ok()?;
    let upstream = local.upstream().ok()?;
    let branch_oid = local.get().peel_to_commit().ok()?.id();
    let upstream_oid = upstream.get().peel_to_commit().ok()?.id();
    Some((branch_oid, upstream_oid))
}

fn status_label(status: MergeStatus) -> &'static str {
    match status {
        MergeStatus::Merged | MergeStatus::LocalMerged | MergeStatus::RemoteMerged => "merged",
        MergeStatus::InSync => "in-sync",
        MergeStatus::SquashMerged
        | MergeStatus::LocalSquashMerged
        | MergeStatus::RemoteSquashMerged => "squash-merged",
        MergeStatus::Unmerged => "unmerged",
        MergeStatus::Pending => "pending",
    }
}

fn short_oid(oid: Option<&str>) -> String {
    match oid {
        Some(o) => o.chars().take(8).collect(),
        None => "disconnected".to_string(),
    }
}
