use crate::git::cache::BranchCache;
use crate::git::merge_detection::is_squash_merged;
use crate::types::{MergeStatus, SquashResult};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use tracing::instrument;

/// Spawn a background thread that checks each candidate branch for squash-merge status.
/// Uses the cache for previously computed results and updates it as new results arrive.
#[instrument(skip(candidates, cache), fields(base_branch, candidate_count = candidates.len()))]
pub fn spawn_squash_checker(
    repo_path: PathBuf,
    base_branch: String,
    candidates: Vec<(String, String)>, // (branch_name, commit_hash)
    mut cache: BranchCache,
) -> Receiver<SquashResult> {
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        for (branch_name, commit_hash) in &candidates {
            // Check cache first
            if let Some(status) = cache.lookup(branch_name, commit_hash) {
                let is_squash = matches!(status, MergeStatus::SquashMerged);
                if tx
                    .send(SquashResult {
                        branch_name: branch_name.clone(),
                        is_squash_merged: is_squash,
                    })
                    .is_err()
                {
                    return; // Receiver dropped
                }
                continue;
            }

            let is_squash = is_squash_merged(&repo_path, &base_branch, branch_name, None);

            let status = if is_squash {
                MergeStatus::SquashMerged
            } else {
                MergeStatus::Unmerged
            };
            cache.insert(branch_name, &status, commit_hash);

            if tx
                .send(SquashResult {
                    branch_name: branch_name.clone(),
                    is_squash_merged: is_squash,
                })
                .is_err()
            {
                return;
            }
        }

        cache.save();
    });

    rx
}
