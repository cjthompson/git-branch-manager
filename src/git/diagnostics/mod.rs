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

mod verify;
mod workers;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};

use git2::{Oid, Repository};

use crate::git::cache::BranchCache;
use crate::git::merge_detection::{build_reachable_set_from_repo, BaseReachable};
use crate::types::{CacheAudit, CacheFix, MergeStatus};

use verify::{local_branches, record_merge_status, verify_ahead_behind, verify_merge_base};
use workers::{run_cherry_candidates, run_squash_candidates, CherryCandidate, SquashCandidate};

/// Shared read-only context for one audit pass.
struct AuditCtx<'a> {
    repo: &'a Repository,
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
        base_branch,
        current_branch: repo
            .head()
            .ok()
            .and_then(|h| h.shorthand().ok().map(|s| s.to_string()))
            .unwrap_or_default(),
        base_oid: repo
            .find_branch(base_branch, git2::BranchType::Local)
            .ok()
            .and_then(|b| b.get().target()),
        reachable: build_reachable_set_from_repo(repo, base_branch),
        cache,
    };

    // Enumerate local branches with their current tips. Remote branches are
    // only needed for the orphan sweep below (cached squash results for
    // remote branches are keyed by their full "origin/<branch>" ref).
    let locals = local_branches(repo);
    let live_local: HashSet<&str> = locals.iter().map(|(name, _)| name.as_str()).collect();
    let live_remote = remote_branch_names(repo);
    let total = locals.len();

    // Phase 1 (sequential, cheap): ahead/behind and merge-base checks are
    // git2-native, not subprocess-based, so they stay on this thread. Merge
    // status resolves immediately for regularly-merged branches; anything
    // else needs a squash check and is queued for phase 2.
    let mut squash_candidates = Vec::new();
    for (i, (name, tip)) in locals.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return audit;
        }
        progress(i, total, name);

        verify_ahead_behind(&ctx, name, &mut audit);
        if let Some(base_oid) = ctx.base_oid {
            verify_merge_base(&ctx, name, *tip, base_oid, &mut audit);
        }

        match ctx.reachable.regular_merge_status(*tip) {
            Some(status) => record_merge_status(&ctx, name, *tip, status, &mut audit),
            None => squash_candidates.push(SquashCandidate {
                name: name.clone(),
                tip: *tip,
                merge_base: ctx
                    .base_oid
                    .and_then(|b| ctx.repo.merge_base(*tip, b).ok())
                    .map(|o| o.to_string()),
            }),
        }
    }

    // Phase 2 (parallel): squash-merge truth requires shelling out to `git`
    // twice per branch (local base + remote base) — the dominant cost of an
    // audit. Farm it out to a worker pool exactly like `squash_loader` does,
    // since `is_squash_merged` needs only `repo_path`/strings, not a
    // `Repository` handle.
    if !cancel.load(Ordering::Relaxed) {
        for result in run_squash_candidates(repo_path, base_branch, squash_candidates) {
            record_merge_status(&ctx, &result.name, result.tip, result.status, &mut audit);
        }

        // Phase 2b (parallel): cherry-pick truth. We only record discrepancies
        // for branches whose cache currently shows a cherry-related status —
        // the squash phase has already audited every other branch, and
        // double-reporting it here would create duplicate discrepancies.
        let cherry_candidates: Vec<CherryCandidate> = locals
            .iter()
            .filter_map(|(name, tip)| {
                let tip_str = tip.to_string();
                let cached = cache.lookup(name, &tip_str)?;
                let is_cherry = matches!(
                    cached,
                    MergeStatus::CherryPicked
                        | MergeStatus::LocalCherryPicked
                        | MergeStatus::RemoteCherryPicked
                );
                if !is_cherry {
                    return None;
                }
                Some(CherryCandidate {
                    branch_name: name.clone(),
                    commit_hash: tip_str,
                    merge_base: ctx
                        .base_oid
                        .and_then(|b| ctx.repo.merge_base(*tip, b).ok())
                        .map(|o| o.to_string()),
                })
            })
            .collect();
        for result in run_cherry_candidates(repo_path, base_branch, cherry_candidates) {
            let tip_oid = git2::Oid::from_str(&result.commit_hash).ok();
            if let Some(tip_oid) = tip_oid {
                record_merge_status(
                    &ctx,
                    &result.branch_name,
                    tip_oid,
                    result.recomputed_status,
                    &mut audit,
                );
            }
        }
    }

    // Orphans: cached merge-status rows whose branch (local or remote) no
    // longer exists.
    for cached_name in cache.cached_branch_names() {
        if !live_local.contains(cached_name.as_str()) && !live_remote.contains(&cached_name) {
            audit.orphans.push(cached_name);
        }
    }
    audit.orphans.sort();

    audit
}

/// All remote branch names (e.g. `"origin/feature-x"`) — the same short form
/// used as the cache key for remote-branch squash-merge results.
fn remote_branch_names(repo: &Repository) -> HashSet<String> {
    let mut out = HashSet::new();
    if let Ok(branches) = repo.branches(Some(git2::BranchType::Remote)) {
        for (branch, _) in branches.flatten() {
            if let Ok(Some(name)) = branch.name() {
                out.insert(name.to_string());
            }
        }
    }
    out
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

/// Run a silent, automatic cache audit-and-fix pass in the background, for
/// launch-time verification. Unlike the manual F2 flow, there is no review
/// step: any discrepancies/orphans found are applied and persisted before the
/// resulting [`CacheAudit`] is sent, purely so the caller can patch live UI
/// state. Opens its own `Repository`/`BranchCache` handles, matching every
/// other launch-time background thread (neither type is `Send`).
pub fn spawn_cache_verifier(
    repo_path: PathBuf,
    base_branch: String,
    cache_root: super::cache::CacheRoot,
) -> Receiver<CacheAudit> {
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        let Ok(repo) = Repository::open(&repo_path) else {
            return;
        };
        let mut cache = BranchCache::load_for_base(&repo_path, &base_branch, &cache_root);
        let cancel = AtomicBool::new(false);
        let audit = audit_cache(
            &repo,
            &repo_path,
            &base_branch,
            &cache,
            &cancel,
            |_, _, _| {},
        );

        if !audit.is_clean() {
            apply_fix(&mut cache, &audit);
        }

        let _ = tx.send(audit);
    });

    rx
}
