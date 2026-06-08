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
    candidates: Vec<(String, String, Option<String>)>, // (branch_name, commit_hash, merge_base)
    mut cache: BranchCache,
) -> Receiver<SquashResult> {
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        const CACHE_SAVE_INTERVAL: usize = 100;
        let mut unsaved_inserts = 0usize;

        for (branch_name, commit_hash, merge_base) in &candidates {
            let span = tracing::info_span!(
                "squash_candidate",
                branch_name = %branch_name,
                cache_hit = tracing::field::Empty,
                squash = tracing::field::Empty,
            );
            let _entered = span.enter();

            // Check cache first
            if let Some(status) = cache.lookup(branch_name, commit_hash) {
                span.record("cache_hit", true);
                let is_squash = matches!(status, MergeStatus::SquashMerged);
                span.record("squash", is_squash);
                if tx
                    .send(SquashResult {
                        branch_name: branch_name.clone(),
                        is_squash_merged: is_squash,
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

            let is_squash = is_squash_merged(
                &repo_path,
                &base_branch,
                branch_name,
                Some(commit_hash),
                merge_base.as_deref(),
            );
            span.record("squash", is_squash);

            let status = if is_squash {
                MergeStatus::SquashMerged
            } else {
                MergeStatus::Unmerged
            };
            cache.insert(branch_name, &status, commit_hash);
            unsaved_inserts += 1;
            if unsaved_inserts >= CACHE_SAVE_INTERVAL {
                cache.save();
                unsaved_inserts = 0;
            }

            if tx
                .send(SquashResult {
                    branch_name: branch_name.clone(),
                    is_squash_merged: is_squash,
                })
                .is_err()
            {
                if unsaved_inserts > 0 {
                    cache.save();
                }
                return;
            }
        }

        cache.log_stats("squash_checker");
        cache.save();
    });

    rx
}
