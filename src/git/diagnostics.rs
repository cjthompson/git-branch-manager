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

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};

use git2::{Oid, Repository};

use crate::git::branch;
use crate::git::cache::BranchCache;
use crate::git::merge_detection::{build_reachable_set_from_repo, is_squash_merged, BaseReachable};
use crate::types::{CacheAudit, CacheFix, DiagKind, Discrepancy, MergeStatus};

/// Number of worker threads used to run `is_squash_merged` concurrently
/// during an audit. Mirrors `squash_loader::SQUASH_WORKER_COUNT` — bound by
/// subprocess fork/exec overhead, not CPU parallelism.
const AUDIT_SQUASH_WORKER_COUNT: usize = 4;

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

struct SquashCandidate {
    name: String,
    tip: Oid,
    merge_base: Option<String>,
}

struct SquashCandidateResult {
    name: String,
    tip: Oid,
    status: MergeStatus,
}

/// Resolve squash-merge truth for every candidate using a fixed worker pool,
/// mirroring `squash_loader::spawn_squash_checker`'s queue-based dispatch.
fn run_squash_candidates(
    repo_path: &Path,
    base_branch: &str,
    candidates: Vec<SquashCandidate>,
) -> Vec<SquashCandidateResult> {
    if candidates.is_empty() {
        return Vec::new();
    }

    let queue: Arc<Mutex<VecDeque<SquashCandidate>>> =
        Arc::new(Mutex::new(VecDeque::from(candidates)));
    let (tx, rx): (_, Receiver<SquashCandidateResult>) = mpsc::channel();
    let repo_path: PathBuf = repo_path.to_path_buf();

    let mut handles = Vec::with_capacity(AUDIT_SQUASH_WORKER_COUNT);
    for _ in 0..AUDIT_SQUASH_WORKER_COUNT {
        let queue = Arc::clone(&queue);
        let tx = tx.clone();
        let repo_path = repo_path.clone();
        let base_branch = base_branch.to_string();
        handles.push(std::thread::spawn(move || loop {
            let next = queue.lock().unwrap().pop_front();
            let Some(candidate) = next else { break };

            let tip_str = candidate.tip.to_string();
            let local_squash = is_squash_merged(
                &repo_path,
                &base_branch,
                &candidate.name,
                Some(&tip_str),
                candidate.merge_base.as_deref(),
            );
            let remote_base = format!("origin/{base_branch}");
            let remote_squash = is_squash_merged(
                &repo_path,
                &remote_base,
                &candidate.name,
                Some(&tip_str),
                None,
            );
            let status = match (local_squash, remote_squash) {
                (true, true) => MergeStatus::SquashMerged,
                (false, true) => MergeStatus::RemoteSquashMerged,
                (true, false) => MergeStatus::LocalSquashMerged,
                (false, false) => MergeStatus::Unmerged,
            };

            if tx
                .send(SquashCandidateResult {
                    name: candidate.name,
                    tip: candidate.tip,
                    status,
                })
                .is_err()
            {
                break;
            }
        }));
    }
    drop(tx);

    let results: Vec<SquashCandidateResult> = rx.iter().collect();
    for handle in handles {
        let _ = handle.join();
    }
    results
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
pub fn spawn_cache_verifier(repo_path: PathBuf, base_branch: String) -> Receiver<CacheAudit> {
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        let Ok(repo) = Repository::open(&repo_path) else {
            return;
        };
        let mut cache = BranchCache::load(&repo_path);
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

/// Compare a freshly-computed merge-status truth against what the cache
/// serves for `name`, recording a verified/mismatched/skipped tally and — on
/// mismatch — a [`Discrepancy`] carrying the typed fix.
fn record_merge_status(
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
