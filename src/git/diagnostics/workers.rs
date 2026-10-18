use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};

use git2::Oid;

use crate::git::merge_detection::{is_cherry_picked, is_squash_merged};
use crate::types::MergeStatus;

/// Number of worker threads used to run `is_squash_merged` concurrently
/// during an audit. Mirrors `squash_loader::SQUASH_WORKER_COUNT` — bound by
/// subprocess fork/exec overhead, not CPU parallelism.
const AUDIT_SQUASH_WORKER_COUNT: usize = 4;

/// Number of worker threads used to run `is_cherry_picked` concurrently
/// during an audit. Mirrors `cherry_loader::CHERRY_WORKER_COUNT`.
const AUDIT_CHERRY_WORKER_COUNT: usize = 4;

pub(super) struct SquashCandidate {
    pub(super) name: String,
    pub(super) tip: Oid,
    pub(super) merge_base: Option<String>,
}

pub(super) struct SquashCandidateResult {
    pub(super) name: String,
    pub(super) tip: Oid,
    pub(super) status: MergeStatus,
}

pub(super) struct CherryCandidate {
    pub(super) branch_name: String,
    pub(super) commit_hash: String,
    pub(super) merge_base: Option<String>,
}

pub(super) struct CherryCandidateResult {
    pub(super) branch_name: String,
    pub(super) commit_hash: String,
    pub(super) recomputed_status: MergeStatus,
}

/// Resolve squash-merge truth for every candidate using a fixed worker pool,
/// mirroring `squash_loader::spawn_squash_checker`'s queue-based dispatch.
pub(super) fn run_squash_candidates(
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

/// Resolve cherry-pick truth for every candidate using a fixed worker pool,
/// mirroring `cherry_loader::spawn_cherry_checker`'s queue-based dispatch.
pub(super) fn run_cherry_candidates(
    repo_path: &Path,
    base_branch: &str,
    candidates: Vec<CherryCandidate>,
) -> Vec<CherryCandidateResult> {
    if candidates.is_empty() {
        return Vec::new();
    }

    let queue: Arc<Mutex<VecDeque<CherryCandidate>>> =
        Arc::new(Mutex::new(VecDeque::from(candidates)));
    let (tx, rx): (_, Receiver<CherryCandidateResult>) = mpsc::channel();
    let repo_path: PathBuf = repo_path.to_path_buf();

    let mut handles = Vec::with_capacity(AUDIT_CHERRY_WORKER_COUNT);
    for _ in 0..AUDIT_CHERRY_WORKER_COUNT {
        let queue = Arc::clone(&queue);
        let tx = tx.clone();
        let repo_path = repo_path.clone();
        let base_branch = base_branch.to_string();
        handles.push(std::thread::spawn(move || loop {
            let next = queue.lock().unwrap().pop_front();
            let Some(candidate) = next else { break };

            let local_cherry = is_cherry_picked(
                &repo_path,
                &base_branch,
                &candidate.branch_name,
                Some(&candidate.commit_hash),
                candidate.merge_base.as_deref(),
            );

            let remote_base = format!("origin/{base_branch}");
            let remote_cherry = is_cherry_picked(
                &repo_path,
                &remote_base,
                &candidate.branch_name,
                Some(&candidate.commit_hash),
                None,
            );

            let recomputed_is_cherry = local_cherry || remote_cherry;
            let recomputed_status = if recomputed_is_cherry {
                MergeStatus::CherryPicked
            } else {
                MergeStatus::Unmerged
            };

            if tx
                .send(CherryCandidateResult {
                    branch_name: candidate.branch_name,
                    commit_hash: candidate.commit_hash,
                    recomputed_status,
                })
                .is_err()
            {
                break;
            }
        }));
    }
    drop(tx);

    let results: Vec<CherryCandidateResult> = rx.iter().collect();
    for handle in handles {
        let _ = handle.join();
    }
    results
}
