use crate::git::cache::BranchCache;
use crate::git::merge_detection::is_squash_merged;
use crate::types::{MergeStatus, SquashResult};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use tracing::instrument;

/// Spawn a background thread that checks each candidate branch for squash-merge status.
/// Uses the cache for previously computed results and updates it as new results arrive.
/// Runs is_squash_merged twice per branch — once against the local base and once against
/// origin/<base> — to distinguish LocalSquashMerged / RemoteSquashMerged / SquashMerged.
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
            if let Some(cached_status) = cache.lookup(branch_name, commit_hash) {
                span.record("cache_hit", true);
                let is_squash = !matches!(cached_status, MergeStatus::Unmerged);
                span.record("squash", is_squash);
                if tx
                    .send(SquashResult {
                        branch_name: branch_name.clone(),
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

            // Run squash detection against local base
            let local_squash = is_squash_merged(
                &repo_path,
                &base_branch,
                branch_name,
                Some(commit_hash),
                merge_base.as_deref(),
            );

            // Run squash detection against remote base (origin/<base>).
            // Don't reuse local merge_base — remote base may differ.
            let remote_base = format!("origin/{base_branch}");
            let remote_squash = is_squash_merged(
                &repo_path,
                &remote_base,
                branch_name,
                Some(commit_hash),
                None,
            );

            let status = match (local_squash, remote_squash) {
                (true, true) => MergeStatus::SquashMerged,
                (false, true) => MergeStatus::RemoteSquashMerged,
                (true, false) => MergeStatus::LocalSquashMerged,
                (false, false) => MergeStatus::Unmerged,
            };
            let is_squash = !matches!(status, MergeStatus::Unmerged);
            span.record("squash", is_squash);

            cache.insert(branch_name, &status, commit_hash);
            unsaved_inserts += 1;
            if unsaved_inserts >= CACHE_SAVE_INTERVAL {
                cache.save();
                unsaved_inserts = 0;
            }

            if tx
                .send(SquashResult {
                    branch_name: branch_name.clone(),
                    status,
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
