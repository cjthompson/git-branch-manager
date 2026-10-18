use git2::{Oid, Repository};

use super::AuditCtx;
use crate::git::branch;
use crate::types::{CacheAudit, CacheFix, DiagKind, Discrepancy, MergeStatus};

/// All local branches with their current tip OIDs.
pub(super) fn local_branches(repo: &Repository) -> Vec<(String, Oid)> {
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

/// Compare a freshly-computed merge-status truth against what the cache
/// serves for `name`, recording a verified/mismatched/skipped tally and — on
/// mismatch — a [`Discrepancy`] carrying the typed fix.
pub(super) fn record_merge_status(
    ctx: &AuditCtx,
    name: &str,
    tip: Oid,
    truth: MergeStatus,
    audit: &mut CacheAudit,
) {
    let commit_hash = tip.to_string();

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

pub(super) fn verify_ahead_behind(ctx: &AuditCtx, name: &str, audit: &mut CacheAudit) {
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

pub(super) fn verify_merge_base(
    ctx: &AuditCtx,
    name: &str,
    tip: Oid,
    base_oid: Oid,
    audit: &mut CacheAudit,
) {
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
        MergeStatus::LikelySquashMerged => "possible-squash-merged",
        MergeStatus::CherryPicked
        | MergeStatus::LocalCherryPicked
        | MergeStatus::RemoteCherryPicked => "cherry-picked",
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
