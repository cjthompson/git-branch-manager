
use git_branch_manager::git::{
    cache, graph, squash_loader,
};
use git_branch_manager::types::MergeStatus;

use crate::common::*;

// ---------------------------------------------------------------------------
// Graph patch cache tests
//
// These exercise `git::graph::compute_relationships`'s internal
// cache (the `graph_patch` SQLite table on `BranchCache`). They prove the
// cache path is consulted by injecting fabricated cached values, that the
// cache is populated after a normal load, that different (old_oid, new_oid)
// pairs get independent cache keys, and that the Graph loader and the
// Branches-view squash loader can write to the same SQLite file safely when
// run concurrently.
// ---------------------------------------------------------------------------

#[test]
fn test_graph_patch_cache_hit_avoids_recomputation() {
    // Build scenario-01-style clean squash, then fabricate a bogus cached
    // patch_id for the squash-landing (old_oid, new_oid) pair. After loading
    // the graph again, the landing commit must NOT be flagged — proving the
    // cache hit was used in place of the real computation.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/baseline"]);
    std::fs::write(dir.join("baseline.txt"), "baseline content\n").unwrap();
    run_git(dir, &["add", "baseline.txt"]);
    run_git(dir, &["commit", "-m", "baseline feature commit"]);

    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/baseline"]);
    run_git(dir, &["commit", "-m", "squash landing"]);
    let squash_oid = git_output(dir, &["rev-parse", "HEAD"]);
    let parent_oid = git_output(dir, &["rev-parse", "HEAD^"]);

    // Fabricate a cached (patch_id, diff_text) that won't match any real
    // branch tip's patch id, so any exact match the loader reports after this
    // point is the cache, not a fresh compute.
    {
        let mut cache = cache::BranchCache::load_with_root(
            dir,
            &cache::CacheRoot::at(tmpdir.cache_root().to_path_buf()),
        );
        cache.insert_graph_patch(
            &parent_oid,
            &squash_oid,
            1,
            Some("bogus-never-matches".to_string()),
            None,
        );
        cache.save();
    }

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..tmpdir.graph_options()
        },
    )
    .expect("graph load should succeed");
    let landing = snapshot
        .commits
        .iter()
        .find(|c| c.oid == squash_oid)
        .expect("squash landing commit should be displayed");
    assert!(
        !landing.is_possible_squash_merge(),
        "fabricated cached patch_id must suppress the real match (cache hit proved)"
    );
}

#[test]
fn test_graph_patch_cache_populated_after_load() {
    // Same scenario without fabrication: after a real load, the cache must
    // hold an entry for the landing commit's (old_oid, new_oid) pair.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/baseline"]);
    std::fs::write(dir.join("baseline.txt"), "baseline content\n").unwrap();
    run_git(dir, &["add", "baseline.txt"]);
    run_git(dir, &["commit", "-m", "baseline feature commit"]);

    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/baseline"]);
    run_git(dir, &["commit", "-m", "squash landing"]);
    let squash_oid = git_output(dir, &["rev-parse", "HEAD"]);
    let parent_oid = git_output(dir, &["rev-parse", "HEAD^"]);

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..tmpdir.graph_options()
        },
    )
    .expect("graph load should succeed");
    let landing = snapshot
        .commits
        .iter()
        .find(|c| c.oid == squash_oid)
        .expect("squash landing commit should be displayed");
    assert!(
        landing.is_possible_squash_merge(),
        "real match should still be flagged before asserting cache state"
    );

    let cache = cache::BranchCache::load_with_root(
        dir,
        &cache::CacheRoot::at(tmpdir.cache_root().to_path_buf()),
    );
    let cached = cache
        .lookup_graph_patch(&parent_oid, &squash_oid, 1)
        .expect("cache must hold an entry after a successful graph load");
    let (cached_patch_id, _cached_diff_text) = cached;
    assert!(
        cached_patch_id.is_some(),
        "cached patch_id should be populated for the squash landing OID pair"
    );
}

#[test]
fn test_graph_patch_cache_branch_change_invalidates() {
    // After the first squash lands and the cache populates, create and squash
    // a SECOND feature branch producing a new landing commit. Reload the
    // graph — the new landing commit must still be flagged (its OID pair was
    // never in the cache), and the first commit must remain flagged.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    // First squash scenario.
    run_git(dir, &["checkout", "-b", "feature/first"]);
    std::fs::write(dir.join("first.txt"), "first content\n").unwrap();
    run_git(dir, &["add", "first.txt"]);
    run_git(dir, &["commit", "-m", "first feature commit"]);
    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/first"]);
    run_git(dir, &["commit", "-m", "first squash landing"]);
    let first_squash_oid = git_output(dir, &["rev-parse", "HEAD"]);

    // Prime the cache with the first squash.
    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..tmpdir.graph_options()
        },
    )
    .expect("first graph load should succeed");
    assert!(
        snapshot
            .commits
            .iter()
            .find(|c| c.oid == first_squash_oid)
            .unwrap()
            .is_possible_squash_merge(),
        "first squash landing should be flagged"
    );

    // Second squash scenario — produces a fresh OID pair that was never cached.
    run_git(dir, &["checkout", "-b", "feature/second"]);
    std::fs::write(dir.join("second.txt"), "second content\n").unwrap();
    run_git(dir, &["add", "second.txt"]);
    run_git(dir, &["commit", "-m", "second feature commit"]);
    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/second"]);
    run_git(dir, &["commit", "-m", "second squash landing"]);
    let second_squash_oid = git_output(dir, &["rev-parse", "HEAD"]);
    let second_parent_oid = git_output(dir, &["rev-parse", "HEAD^"]);

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..tmpdir.graph_options()
        },
    )
    .expect("second graph load should succeed");

    let second_landing = snapshot
        .commits
        .iter()
        .find(|c| c.oid == second_squash_oid)
        .expect("second landing commit should be displayed");
    assert!(
        second_landing.is_possible_squash_merge(),
        "second landing commit must be freshly flagged (its OID pair was never in the cache)"
    );

    // First landing should still be flagged too — cache hit returned true.
    let first_landing = snapshot
        .commits
        .iter()
        .find(|c| c.oid == first_squash_oid)
        .expect("first landing commit should still be displayed");
    assert!(
        first_landing.is_possible_squash_merge(),
        "first landing commit must remain flagged (cache hit preserved its match)"
    );

    // And the second landing's OID pair should now itself be in the cache.
    let cache = cache::BranchCache::load_with_root(
        dir,
        &cache::CacheRoot::at(tmpdir.cache_root().to_path_buf()),
    );
    assert!(
        cache
            .lookup_graph_patch(&second_parent_oid, &second_squash_oid, 1)
            .is_some(),
        "second landing's OID pair must now be cached after the second load"
    );
}

#[test]
fn test_graph_and_branches_squash_loaders_concurrent_cache_access() {
    // Build a squash scenario with a real remote so the branches-view squash
    // loader's local-vs-remote base comparison returns a definite status.
    // Run the Graph loader and the Branches-view squash loader concurrently
    // against the same SQLite cache file. Both must produce correct results
    // with no panic or error — proving the two independent BranchCache
    // instances writing to the same file are safe when run concurrently.
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    run_git(&work_dir, &["checkout", "-b", "feature/concurrent"]);
    std::fs::write(work_dir.join("concurrent.txt"), "concurrent content\n").unwrap();
    run_git(&work_dir, &["add", "concurrent.txt"]);
    run_git(&work_dir, &["commit", "-m", "concurrent feature commit"]);
    run_git(&work_dir, &["push", "-u", "origin", "feature/concurrent"]);
    // Capture the feature branch tip BEFORE the squash merge so the
    // branches-view squash loader checks the right commit.
    let feature_tip_oid = git_output(&work_dir, &["rev-parse", "HEAD"]);
    run_git(&work_dir, &["checkout", "main"]);
    run_git(&work_dir, &["merge", "--squash", "feature/concurrent"]);
    run_git(&work_dir, &["commit", "-m", "concurrent squash landing"]);
    run_git(&work_dir, &["push", "origin", "main"]);
    let concurrent_oid = git_output(&work_dir, &["rev-parse", "HEAD"]);
    let _concurrent_parent = git_output(&work_dir, &["rev-parse", "HEAD^"]);

    let dir_clone = work_dir.clone();
    let graph_cache_root = cache::CacheRoot::at(_tmpdir.cache_root().to_path_buf());
    let graph_thread = std::thread::spawn(move || {
        graph::load_graph_with_squash_annotations(
            &dir_clone,
            graph::GraphLoadOptions {
                base_branch: Some("main".into()),
                cache_root: graph_cache_root,
                ..graph::GraphLoadOptions::default()
            },
        )
    });

    // The Branches-view squash loader uses (branch_name, commit_hash, Option)
    // candidates. We feed it the same feature branch as a real candidate,
    // using the feature branch tip (NOT the squash landing commit).
    let work_dir_for_squash = work_dir.clone();
    let candidates = vec![(
        "feature/concurrent".to_string(),
        feature_tip_oid.clone(),
        None,
    )];
    let squash_cache = cache::BranchCache::load_for_base(
        &work_dir,
        "main",
        &cache::CacheRoot::at(_tmpdir.cache_root().to_path_buf()),
    );
    let squash_thread = std::thread::spawn(move || {
        let rx = squash_loader::spawn_squash_checker(
            work_dir_for_squash,
            "main".to_string(),
            candidates,
            squash_cache,
        );
        rx.into_iter().collect::<Vec<_>>()
    });

    let graph_result = graph_thread.join().expect("graph thread panicked");
    let squash_results = squash_thread.join().expect("squash thread panicked");

    let snapshot = graph_result.expect("graph load should succeed");
    let landing = snapshot
        .commits
        .iter()
        .find(|c| c.oid == concurrent_oid)
        .expect("squash landing commit should be displayed");
    assert!(
        landing.is_possible_squash_merge(),
        "concurrent graph load should still flag the squash landing"
    );

    // With both local and remote bases present and the squash pushed, the
    // branches-view squash loader should report SquashMerged (both local
    // and remote-base checks true).
    assert!(
        squash_results.iter().any(|r| {
            r.branch_name == "feature/concurrent"
                && matches!(
                    r.status,
                    MergeStatus::SquashMerged | MergeStatus::LocalSquashMerged
                )
        }),
        "concurrent branches-view squash loader should report a squash status, got: {:?}",
        squash_results
    );
}
