use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use crate::types::{MergeStatus, SquashResult};
use super::cache::BranchCache;
use super::merge_detection::is_squash_merged;

/// Spawn a background thread that checks each candidate branch for squash-merge
/// status and sends results back one-by-one.
///
/// Uses the cache to skip branches whose status is already known. Updates and
/// saves the cache when all candidates are processed.
///
/// The channel closes naturally when the thread completes (Sender is dropped).
pub fn spawn_squash_checker(
    repo_path: PathBuf,
    base_branch: String,
    candidates: Vec<(String, String)>,
    mut cache: BranchCache,
) -> Receiver<SquashResult> {
    let (tx, rx) = mpsc::channel::<SquashResult>();

    thread::spawn(move || {
        for (branch_name, commit_hash) in candidates {
            let is_squash = match cache.lookup(&branch_name, &commit_hash) {
                Some(MergeStatus::SquashMerged) => true,
                Some(MergeStatus::Unmerged) => false,
                _ => {
                    let result = is_squash_merged(&repo_path, &base_branch, &branch_name);
                    let status = if result {
                        MergeStatus::SquashMerged
                    } else {
                        MergeStatus::Unmerged
                    };
                    cache.insert(&branch_name, &status, &commit_hash);
                    result
                }
            };

            if tx
                .send(SquashResult {
                    branch_name,
                    is_squash_merged: is_squash,
                })
                .is_err()
            {
                break; // Receiver dropped (app exited)
            }
        }
        cache.save();
    });

    rx
}
