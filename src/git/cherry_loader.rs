use crate::git::cache::BranchCache;
use crate::git::merge_detection::is_cherry_picked;
use crate::types::{CherryResult, MergeStatus};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use tracing::instrument;

const CHERRY_WORKER_COUNT: usize = 4;

struct WorkerResult {
    branch_name: String,
    commit_hash: String,
    status: MergeStatus,
}

#[instrument(skip(candidates, cache), fields(base_branch, candidate_count = candidates.len()))]
pub fn spawn_cherry_checker(
    repo_path: PathBuf,
    base_branch: String,
    candidates: Vec<(String, String, Option<String>)>,
    mut cache: BranchCache,
) -> Receiver<CherryResult> {
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        const CACHE_SAVE_INTERVAL: usize = 100;
        let mut unsaved_inserts = 0usize;

        let mut misses: VecDeque<(String, String, Option<String>)> = VecDeque::new();
        for (branch_name, commit_hash, merge_base) in candidates {
            if let Some(cached_status) = cache.lookup(&branch_name, &commit_hash) {
                if tx
                    .send(CherryResult {
                        branch_name,
                        status: cached_status,
                    })
                    .is_err()
                {
                    if unsaved_inserts > 0 {
                        cache.save();
                    }
                    return;
                }
                continue;
            }
            misses.push_back((branch_name, commit_hash, merge_base));
        }

        if misses.is_empty() {
            cache.log_stats("cherry_checker");
            cache.save();
            return;
        }

        let queue = Arc::new(Mutex::new(misses));
        let (worker_tx, worker_rx) = mpsc::channel::<WorkerResult>();

        let mut handles = Vec::with_capacity(CHERRY_WORKER_COUNT);
        for _ in 0..CHERRY_WORKER_COUNT {
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

                    let local_cherry = is_cherry_picked(
                        &repo_path,
                        &base_branch,
                        &branch_name,
                        Some(&commit_hash),
                        merge_base.as_deref(),
                    );

                    let remote_base = format!("origin/{base_branch}");
                    let remote_cherry = is_cherry_picked(
                        &repo_path,
                        &remote_base,
                        &branch_name,
                        Some(&commit_hash),
                        None,
                    );

                    let status = match (local_cherry, remote_cherry) {
                        (true, true) => MergeStatus::CherryPicked,
                        (false, true) => MergeStatus::RemoteCherryPicked,
                        (true, false) => MergeStatus::LocalCherryPicked,
                        (false, false) => MergeStatus::Unmerged,
                    };

                    if worker_tx
                        .send(WorkerResult {
                            branch_name,
                            commit_hash,
                            status,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            }));
        }
        drop(worker_tx);

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
                .send(CherryResult {
                    branch_name,
                    status,
                })
                .is_err()
            {
                queue.lock().unwrap().clear();
                if unsaved_inserts > 0 {
                    cache.save();
                }
                return;
            }
        }

        for handle in handles {
            let _ = handle.join();
        }

        cache.log_stats("cherry_checker");
        cache.save();
    });

    rx
}
