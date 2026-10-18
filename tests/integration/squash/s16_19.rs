
use git_branch_manager::git::{
    graph, merge_detection,
};
use git_branch_manager::types::MergeStatus;

use crate::common::*;

#[test]
fn test_squash_scenario_16_shallow_out_of_window_history_max_count_boundary() {
    // Plan scenario 16: the actual squash-landing commit is older than the
    // Graph's configured `max_count` window.
    // Expect A: not flagged — Algorithm A fails closed outside its displayed
    // window (no false positive, no panic).
    // Expect B: still detects it — B is documented as searching beyond the
    // Graph display window (the concrete "A false, B true" disagreement case).
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/out-of-window"]);
    std::fs::write(dir.join("window.txt"), "windowed content\n").unwrap();
    run_git(dir, &["add", "window.txt"]);
    run_git(dir, &["commit", "-m", "windowed source"]);

    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/out-of-window"]);
    run_git(dir, &["commit", "-m", "squash landing (out of window)"]);
    let squash_oid = git_output(dir, &["rev-parse", "HEAD"]);

    // Push the squash landing commit outside the displayed window.
    run_git(dir, &["commit", "--allow-empty", "-m", "newer commit 1"]);
    run_git(dir, &["commit", "--allow-empty", "-m", "newer commit 2"]);

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            max_count: 2,
            base_branch: Some("main".into()),
            ..tmpdir.graph_options()
        },
    )
    .expect("bounded graph load should not error");
    assert_eq!(snapshot.commits.len(), 2);
    assert!(
        !snapshot.commits.iter().any(|c| c.oid == squash_oid),
        "the squash landing commit should be outside the displayed window"
    );
    assert!(
        snapshot
            .commits
            .iter()
            .all(|c| !c.is_possible_squash_merge()),
        "Algorithm A must fail closed (no false positive) outside its displayed window"
    );

    // Expect B: full history search still finds it.
    assert!(
        merge_detection::is_squash_merged(dir, "main", "feature/out-of-window", None, None),
        "Algorithm B is not bounded by the Graph's display window and should still detect it"
    );
}

#[test]
fn test_squash_scenario_17_local_base_vs_remote_base_divergence() {
    // Plan scenario 17: branch is squash-merged into origin/main, but local
    // main has not fetched/merged that commit yet. Detection against local
    // main must report false; detection against origin/main (after fetch)
    // must report true.
    let base = tempfile::tempdir().expect("failed to create tempdir");
    let remote_dir = base.path().join("remote.git");
    std::fs::create_dir_all(&remote_dir).unwrap();
    run_git(&remote_dir, &["init", "--bare", "-b", "main"]);

    let work_dir = base.path().join("work");
    run_git(
        base.path(),
        &["clone", remote_dir.to_str().unwrap(), "work"],
    );
    run_git(&work_dir, &["config", "user.name", "Test User"]);
    run_git(&work_dir, &["config", "user.email", "test@example.com"]);
    std::fs::write(work_dir.join("README.md"), "# Test\n").unwrap();
    run_git(&work_dir, &["add", "."]);
    run_git(&work_dir, &["commit", "-m", "Initial commit"]);
    run_git(&work_dir, &["push", "-u", "origin", "main"]);

    run_git(&work_dir, &["checkout", "-b", "feature/divergence"]);
    std::fs::write(work_dir.join("divergence.txt"), "divergence content\n").unwrap();
    run_git(&work_dir, &["add", "divergence.txt"]);
    run_git(&work_dir, &["commit", "-m", "feature commit"]);
    run_git(&work_dir, &["push", "-u", "origin", "feature/divergence"]);
    run_git(&work_dir, &["checkout", "main"]);

    // A second clone squash-merges the feature branch and pushes to origin,
    // without `work` ever fetching it locally into `main`.
    let second_dir = base.path().join("second");
    run_git(
        base.path(),
        &["clone", remote_dir.to_str().unwrap(), "second"],
    );
    run_git(&second_dir, &["config", "user.name", "Second User"]);
    run_git(&second_dir, &["config", "user.email", "second@example.com"]);
    run_git(&second_dir, &["fetch", "origin", "feature/divergence"]);
    run_git(
        &second_dir,
        &["merge", "--squash", "origin/feature/divergence"],
    );
    run_git(
        &second_dir,
        &["commit", "-m", "squash merge feature/divergence"],
    );
    run_git(&second_dir, &["push", "origin", "main"]);

    // `work`'s local main is still behind; only fetch (not merge/pull).
    run_git(&work_dir, &["fetch", "origin"]);

    assert!(
        !merge_detection::is_squash_merged(&work_dir, "main", "feature/divergence", None, None),
        "local main has not fetched the squash landing commit yet, so it must not be flagged"
    );
    assert!(
        merge_detection::is_squash_merged(
            &work_dir,
            "origin/main",
            "feature/divergence",
            None,
            None
        ),
        "origin/main (fetched) already contains the squash landing and must be flagged"
    );
}

#[test]
fn test_squash_scenario_18_branch_advances_after_cached_as_squash_merged() {
    // Plan scenario 18: branch is squash-merged and detected/cached as
    // SquashMerged; branch then gets a new commit on top (still local,
    // unpushed, unmerged). Re-running detection should no longer report
    // SquashMerged for the branch's *current* tip. `MergeStatus::SquashMerged`
    // (the local+remote-confirmed variant) is cached *permanently* in
    // `git/cache.rs::BranchCache::lookup` — it ignores `commit_hash` entirely
    // for that one status, unlike `LocalSquashMerged`/`RemoteSquashMerged`
    // (commit-hash-gated). This test exercises exactly that permanent-cache
    // branch by using a remote setup where both local and remote sides
    // confirm the squash (producing `SquashMerged`, not the volatile
    // `LocalSquashMerged`), then documents whether the known bug reproduces.
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    run_git(&work_dir, &["checkout", "-b", "feature/cache-staleness"]);
    std::fs::write(work_dir.join("stale.txt"), "stale content\n").unwrap();
    run_git(&work_dir, &["add", "stale.txt"]);
    run_git(&work_dir, &["commit", "-m", "feature commit"]);
    run_git(
        &work_dir,
        &["push", "-u", "origin", "feature/cache-staleness"],
    );

    run_git(&work_dir, &["checkout", "main"]);
    run_git(&work_dir, &["merge", "--squash", "feature/cache-staleness"]);
    run_git(
        &work_dir,
        &["commit", "-m", "squash merge feature/cache-staleness"],
    );
    run_git(&work_dir, &["push", "origin", "main"]);

    let repo = git2::Repository::open(&work_dir).unwrap();
    let branches = test_branches(&_tmpdir, &repo, "main").expect("list_branches failed");
    let feature = branches
        .iter()
        .find(|b| b.name == "feature/cache-staleness")
        .unwrap();
    assert_eq!(
        feature.merge_status,
        MergeStatus::SquashMerged,
        "both local and remote main confirm the squash, so this must be the fully-confirmed \
         (and thus permanently-cached) SquashMerged variant"
    );

    // Branch advances with a brand-new, unmerged, unpushed commit.
    run_git(&work_dir, &["checkout", "feature/cache-staleness"]);
    std::fs::write(work_dir.join("new-work.txt"), "new unmerged work\n").unwrap();
    run_git(&work_dir, &["add", "new-work.txt"]);
    run_git(&work_dir, &["commit", "-m", "new unmerged commit on top"]);
    run_git(&work_dir, &["checkout", "main"]);

    let repo = git2::Repository::open(&work_dir).unwrap();
    let branches = test_branches(&_tmpdir, &repo, "main").expect("list_branches failed");
    let feature = branches
        .iter()
        .find(|b| b.name == "feature/cache-staleness")
        .unwrap();
    // Known bug (see git/cache.rs BranchCache::lookup and the parent plan
    // doc's cache-limitation note): SquashMerged is cached permanently and
    // ignores the branch's now-advanced commit hash, so the stale status is
    // retained instead of being recomputed as Unmerged/Pending for the new
    // tip. This assertion documents today's actual (buggy) behavior; it is a
    // deliberate visible pin, not an endorsement of the behavior.
    assert_eq!(
        feature.merge_status,
        MergeStatus::SquashMerged,
        "KNOWN BUG (see docs/plans/2026-08-29-squash-merge-test-scenarios.md scenario 18 and \
         git/cache.rs's permanent-cache comment for SquashMerged/Merged): the cache does not \
         invalidate on branch advancement for this status, so a branch with new unmerged work \
         still reports the stale SquashMerged status from before it advanced"
    );
}

#[test]
#[ignore]
fn test_squash_scenario_19_large_history_performance_characterization() {
    // Plan scenario 19: not a correctness assertion. Generate a large base
    // history and many diverged local branches, most unrelated to any squash,
    // and record wall-clock time as a baseline. Ignored by default (expensive);
    // run explicitly with:
    //   cargo test test_squash_scenario_19 -- --ignored --nocapture
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    const BASE_COMMITS: usize = 500;
    const BRANCH_COUNT: usize = 50;

    for i in 0..BASE_COMMITS {
        std::fs::write(
            dir.join(format!("base-{i}.txt")),
            format!("base content {i}\n"),
        )
        .unwrap();
        run_git(dir, &["add", "."]);
        run_git(dir, &["commit", "-m", &format!("base commit {i}")]);
    }

    for i in 0..BRANCH_COUNT {
        let branch_name = format!("feature/perf-{i}");
        run_git(dir, &["checkout", "-b", &branch_name, "main"]);
        std::fs::write(
            dir.join(format!("feature-{i}.txt")),
            format!("feature content {i}\n"),
        )
        .unwrap();
        run_git(dir, &["add", "."]);
        run_git(dir, &["commit", "-m", &format!("feature commit {i}")]);
        run_git(dir, &["checkout", "main"]);
    }

    let start = std::time::Instant::now();
    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..tmpdir.graph_options()
        },
    )
    .expect("graph load should succeed even at this scale");
    let graph_elapsed = start.elapsed();
    eprintln!(
        "[scenario 19] Algorithm A (Graph): {} commits displayed, {} branches, elapsed {:?}",
        snapshot.commits.len(),
        BRANCH_COUNT,
        graph_elapsed
    );

    let repo = git2::Repository::open(dir).unwrap();
    let start = std::time::Instant::now();
    let branches = test_branches(&tmpdir, &repo, "main").expect("list_branches failed");
    let branch_elapsed = start.elapsed();
    eprintln!(
        "[scenario 19] Algorithm B (Branches list_branches): {} branches, elapsed {:?}",
        branches.len(),
        branch_elapsed
    );
}
