use crate::ahead_behind::compute_ahead_behind;
use crate::app;
use git_branch_manager::git::{branch, cache, merge_detection};
use git_branch_manager::types::MergeStatus;
use std::path::Path;
use std::sync::mpsc::Receiver;
use tracing::{field, info_span};

pub(crate) fn spawn_phase1_loader(
    repo_path: &Path,
    base_branch: &str,
    cache_root: &cache::CacheRoot,
) -> Receiver<app::Phase1Msg> {
    let (phase1_tx, phase1_rx) = std::sync::mpsc::channel::<app::Phase1Msg>();
    let repo_path_bg = repo_path.to_path_buf();
    let base_branch_bg = base_branch.to_string();
    let cache_root_bg = cache_root.clone();
    std::thread::spawn(move || {
        let branch_load_span = info_span!(
            "branch_load",
            trigger = "startup",
            path = "phase1_async",
            inputs_changed = true,
        );
        let _branch_load_enter = branch_load_span.enter();
        let phase1_span = info_span!(
            "phase1_load_worker",
            trigger = "initial_load",
            repo_path = ?repo_path_bg,
            base_branch = %base_branch_bg,
            fast_branch_count = field::Empty,
            ahead_behind_update_count = field::Empty,
            merge_base_update_count = field::Empty,
            merge_status_update_count = field::Empty,
            result_state = field::Empty,
        );
        let _phase1_enter = phase1_span.enter();
        let Ok(repo) = git2::Repository::open(&repo_path_bg) else {
            phase1_span.record("result_state", "open_repo_error");
            return;
        };

        // Fast: metadata only, no merge detection
        let Ok(branches) = branch::list_branches_fast(&repo, &base_branch_bg) else {
            phase1_span.record("result_state", "list_branches_fast_error");
            return;
        };
        phase1_span.record("fast_branch_count", branches.len() as u64);
        let cache_for_app =
            cache::BranchCache::load_for_base(&repo_path_bg, &base_branch_bg, &cache_root_bg);
        let cache_for_squash =
            cache::BranchCache::load_for_base(&repo_path_bg, &base_branch_bg, &cache_root_bg);
        if phase1_tx
            .send(app::Phase1Msg::Fast(
                branches.clone(),
                Box::new(cache_for_app),
                Box::new(cache_for_squash),
            ))
            .is_err()
        {
            phase1_span.record("result_state", "fast_send_error");
            return;
        }

        // Spawn a secondary thread for the entire merge-detection pipeline
        // (build reachable set → apply merge statuses → fill merge bases) using
        // its own Repository handle. This runs fully in parallel with compute_ahead_behind
        // below, so the wall time becomes max(ahead_behind, detection) instead of their sum.
        //
        // Only spawn when there are non-base local branches to check — avoids a full
        // revwalk on repos used purely for remote tracking (no local branches).
        let has_candidates = branches.iter().any(|b| !b.is_base && !b.is_current);
        if has_candidates {
            let branches_clone = branches.clone();
            let phase1_tx_clone = phase1_tx.clone();
            let secondary_path = repo_path_bg.clone();
            let secondary_base = base_branch_bg.clone();
            let secondary_cache_root = cache_root_bg.clone();
            std::thread::spawn(move || {
                let Ok(repo2) = git2::Repository::open(&secondary_path) else {
                    return;
                };
                // Resolve current base tip for cache key and shortcut check.
                let base_tip_oid = repo2
                    .find_branch(&secondary_base, git2::BranchType::Local)
                    .ok()
                    .and_then(|b| b.get().target());
                let mut mb_cache = cache::BranchCache::load_for_base(
                    &secondary_path,
                    &secondary_base,
                    &secondary_cache_root,
                );
                let mut branches2 = branches_clone;

                // Fast path: if base tip matches the cache, skip the full revwalk and
                // restore merge statuses + merge bases from cache. Base tip unchanged
                // means nothing new was merged, so cached statuses are still valid.
                let base_tip_str = base_tip_oid.map(|o| o.to_string());
                let cache_hit =
                    base_tip_str.is_some() && base_tip_str == mb_cache.mb_data.base_tip;

                if cache_hit {
                    // Restore merge statuses from merge-status cache.
                    for branch in branches2.iter_mut() {
                        if branch.is_base || branch.is_current {
                            continue;
                        }
                        if let Some(tip_oid) = repo2
                            .find_branch(&branch.name, git2::BranchType::Local)
                            .ok()
                            .and_then(|b| b.get().target())
                        {
                            let tip_str = tip_oid.to_string();
                            if let Some(status) = mb_cache.lookup(&branch.name, &tip_str) {
                                branch.merge_status = status;
                            }
                            // Unknown tip (new branch since last run) stays Pending; squash
                            // loader will check it. This is correct — base unchanged means
                            // the branch can't be regularly merged yet.
                        }
                    }
                    // Restore merge bases from mb_cache; hits are O(1) map lookups.
                    if let Some(base_tip) = base_tip_oid {
                        branch::fill_merge_base_commits_cached(
                            &repo2,
                            &mut branches2,
                            &std::collections::HashSet::new(),
                            base_tip,
                            &mut mb_cache,
                        );
                    }
                } else {
                    // Full recompute: build reachable set, apply statuses, fill bases.
                    let base_reachable =
                        merge_detection::build_reachable_set_from_repo(&repo2, &secondary_base);
                    merge_detection::apply_merge_statuses(
                        &repo2,
                        &mut branches2,
                        &base_reachable,
                    );
                    // Cache the newly computed merge statuses.
                    for branch in branches2.iter() {
                        if !branch.is_base && !branch.is_current {
                            if let Some(tip_oid) = repo2
                                .find_branch(&branch.name, git2::BranchType::Local)
                                .ok()
                                .and_then(|b| b.get().target())
                            {
                                mb_cache.insert(
                                    &branch.name,
                                    &branch.merge_status,
                                    &tip_oid.to_string(),
                                );
                            }
                        }
                    }
                    if let Some(base_tip) = base_tip_oid {
                        branch::fill_merge_base_commits_cached(
                            &repo2,
                            &mut branches2,
                            &base_reachable.local,
                            base_tip,
                            &mut mb_cache,
                        );
                        mb_cache.set_base_tip(base_tip_str);
                    } else {
                        branch::fill_merge_base_commits(
                            &repo2,
                            &mut branches2,
                            &base_reachable.local,
                        );
                    }
                    mb_cache.save();
                }

                // Send merge bases BEFORE merge statuses: the squash checker is
                // spawned in the MergeStatuses handler and filters candidates on
                // merge_base_commit, so that field must already be populated.
                let merge_base_updates: Vec<(String, String)> = branches2
                    .iter()
                    .filter_map(|b| b.merge_base_commit.clone().map(|h| (b.name.clone(), h)))
                    .collect();
                let _ =
                    phase1_tx_clone.send(app::Phase1Msg::MergeBaseCommits(merge_base_updates));
                let merge_status_updates: Vec<(String, MergeStatus)> = branches2
                    .into_iter()
                    .map(|b| (b.name, b.merge_status))
                    .collect();
                let _ =
                    phase1_tx_clone.send(app::Phase1Msg::MergeStatuses(merge_status_updates));
            });
        }

        // Ahead/behind on this thread (needs the !Send repo from the fast load).
        // Runs fully in parallel with the secondary thread above.
        // Cache avoids re-traversing O(N) commit graph when tips haven't changed.
        let mut ab_cache =
            cache::BranchCache::load_for_base(&repo_path_bg, &base_branch_bg, &cache_root_bg);
        let ahead_behind_updates = compute_ahead_behind(&repo, &branches, &mut ab_cache);
        ab_cache.save();
        phase1_span.record(
            "ahead_behind_update_count",
            ahead_behind_updates.len() as u64,
        );
        let _ = phase1_tx.send(app::Phase1Msg::AheadBehind(ahead_behind_updates));
        phase1_span.record("result_state", "success");
    });
    phase1_rx
}
