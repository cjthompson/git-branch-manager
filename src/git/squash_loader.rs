use crate::git::cache::BranchCache;
use crate::git::merge_detection::is_squash_merged;
use crate::types::{MergeStatus, SquashResult};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use tracing::instrument;

/// Number of worker threads used to run `is_squash_merged` concurrently.
/// Hardcoded rather than derived from core count: each worker shells out to
/// several `git` subprocesses per candidate, so this is bound by subprocess
/// fork/exec overhead, not CPU parallelism — going wider doesn't reliably
/// help and risks overwhelming small machines/CI.
const SQUASH_WORKER_COUNT: usize = 4;

/// Raw (uncached) result of computing squash status for one candidate branch.
/// Carries `commit_hash` alongside so the cache-owner thread can call
/// `BranchCache::insert` without re-deriving it.
struct WorkerResult {
    branch_name: String,
    commit_hash: String,
    status: MergeStatus,
}

/// Spawn a background thread that checks each candidate branch for squash-merge status.
/// Uses the cache for previously computed results and updates it as new results arrive.
/// Runs is_squash_merged twice per branch — once against the local base and once against
/// origin/<base> — to distinguish LocalSquashMerged / RemoteSquashMerged / SquashMerged.
///
/// Cache-hit candidates are resolved on the calling (cache-owner) thread with no
/// subprocess cost. Cache-miss candidates are handed to a fixed pool of
/// `SQUASH_WORKER_COUNT` worker threads pulling from a shared queue; workers run
/// `is_squash_merged` purely (no cache access) and send raw results back over an
/// internal channel to this thread, which is the sole owner of `cache`
/// (inserts + saves) and the sole sender on the returned channel, so external
/// streaming behavior for downstream consumers (app.rs, dump.rs) is unaffected.
#[instrument(skip(candidates, cache), fields(base_branch, candidate_count = candidates.len()))]
pub fn spawn_squash_checker(
    repo_path: PathBuf,
    base_branch: String,
    candidates: Vec<(String, String, Option<String>)>, // (branch_name, commit_hash, merge_base)
    mut cache: BranchCache,
) -> Receiver<SquashResult> {
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        const CACHE_SAVE_INTERVAL: usize = 100;
        let mut unsaved_inserts = 0usize;

        // --- Cache-hit fast path: resolve everything already known, with no
        // subprocess cost, before dispatching the remainder to the worker pool.
        let mut misses: VecDeque<(String, String, Option<String>)> = VecDeque::new();
        for (branch_name, commit_hash, merge_base) in candidates {
            let span = tracing::info_span!(
                "squash_candidate",
                branch_name = %branch_name,
                cache_hit = tracing::field::Empty,
                squash = tracing::field::Empty,
            );
            let _entered = span.enter();

            if let Some(cached_status) = cache.lookup(&branch_name, &commit_hash) {
                span.record("cache_hit", true);
                let is_squash = !matches!(cached_status, MergeStatus::Unmerged);
                span.record("squash", is_squash);
                if tx
                    .send(SquashResult {
                        branch_name,
                        status: cached_status,
                    })
                    .is_err()
                {
                    if unsaved_inserts > 0 {
                        cache.save();
                    }
                    return; // Receiver dropped
                }
                continue;
            }
            span.record("cache_hit", false);
            misses.push_back((branch_name, commit_hash, merge_base));
        }

        if misses.is_empty() {
            cache.log_stats("squash_checker");
            cache.save();
            return;
        }

        // --- Worker pool: SQUASH_WORKER_COUNT threads pull from a shared queue
        // and run the pure is_squash_merged checks. A shared queue (rather than
        // static up-front chunks) is used because per-branch cost varies a lot —
        // the remote-base check always re-derives merge-base via `git merge-base`
        // (merge_base is None on that path), and some branches have deep or
        // disjoint histories — so a queue lets fast workers absorb slack from
        // slow ones instead of idling on an exhausted static chunk.
        let queue = Arc::new(Mutex::new(misses));
        let (worker_tx, worker_rx) = mpsc::channel::<WorkerResult>();

        let mut handles = Vec::with_capacity(SQUASH_WORKER_COUNT);
        for _ in 0..SQUASH_WORKER_COUNT {
            let queue = Arc::clone(&queue);
            let worker_tx = worker_tx.clone();
            let repo_path = repo_path.clone();
            let base_branch = base_branch.clone();
            handles.push(std::thread::spawn(move || {
                loop {
                    let next = {
                        let mut q = queue.lock().unwrap();
                        q.pop_front()
                    };
                    let Some((branch_name, commit_hash, merge_base)) = next else {
                        break;
                    };

                    let span = tracing::info_span!(
                        "squash_candidate",
                        branch_name = %branch_name,
                        cache_hit = false,
                        squash = tracing::field::Empty,
                    );
                    let _entered = span.enter();

                    // Run squash detection against local base
                    let local_squash = is_squash_merged(
                        &repo_path,
                        &base_branch,
                        &branch_name,
                        Some(&commit_hash),
                        merge_base.as_deref(),
                    );

                    // Run squash detection against remote base (origin/<base>).
                    // Don't reuse local merge_base — remote base may differ.
                    let remote_base = format!("origin/{base_branch}");
                    let remote_squash = is_squash_merged(
                        &repo_path,
                        &remote_base,
                        &branch_name,
                        Some(&commit_hash),
                        None,
                    );

                    let status = match (local_squash, remote_squash) {
                        (true, true) => MergeStatus::SquashMerged,
                        (false, true) => MergeStatus::RemoteSquashMerged,
                        (true, false) => MergeStatus::LocalSquashMerged,
                        (false, false) => MergeStatus::Unmerged,
                    };
                    span.record("squash", !matches!(status, MergeStatus::Unmerged));
                    drop(_entered);

                    if worker_tx
                        .send(WorkerResult {
                            branch_name,
                            commit_hash,
                            status,
                        })
                        .is_err()
                    {
                        // Cache-owner thread gave up (downstream Receiver dropped
                        // upstream); stop pulling more work.
                        break;
                    }
                }
            }));
        }
        // Drop this thread's own sender so `worker_rx` disconnects once every
        // worker has finished (each worker holds and drops its own clone).
        drop(worker_tx);

        // --- Cache owner: single-threaded consumption of worker results. Only
        // this thread ever calls cache.insert/cache.save, exactly as before.
        while let Ok(WorkerResult {
            branch_name,
            commit_hash,
            status,
        }) = worker_rx.recv()
        {
            cache.insert(&branch_name, &status, &commit_hash);
            unsaved_inserts += 1;
            if unsaved_inserts >= CACHE_SAVE_INTERVAL {
                cache.save();
                unsaved_inserts = 0;
            }

            if tx
                .send(SquashResult {
                    branch_name,
                    status,
                })
                .is_err()
            {
                // Receiver dropped: stop dispatching further results, and clear
                // the queue so in-flight/idle workers wind down quickly.
                queue.lock().unwrap().clear();
                if unsaved_inserts > 0 {
                    cache.save();
                }
                return; // matches original early-exit semantics; worker threads
                        // self-terminate shortly since the queue is now empty.
            }
        }

        for handle in handles {
            let _ = handle.join();
        }

        cache.log_stats("squash_checker");
        cache.save();
    });

    rx
}
