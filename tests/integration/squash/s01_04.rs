
use git_branch_manager::git::graph;
use git_branch_manager::types::MergeStatus;

use crate::common::*;

// ---------------------------------------------------------------------------
// Squash-merge detection test scenarios (plan P002)
//
// See docs/plans/2026-08-29-squash-merge-test-scenarios.md for the full
// scenario matrix and docs/plans/2026-08-29-squash-merge-algorithm-options.md
// for the parent algorithm-options doc these scenarios validate against.
//
// Naming: `test_squash_scenario_NN_...` (NN matches the plan doc's numbering;
// sub-scenarios like 8a/8b/8c and 20a-20d get their own suffixed test).
// ---------------------------------------------------------------------------

#[test]
fn test_squash_scenario_01_baseline_single_commit_clean_squash() {
    // Plan scenario 1: single-commit branch, squashed cleanly onto base.
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

    // Expect A: Graph flags the landing commit, with no fuzzy annotation
    // (Option 6 defers to the exact-match tier).
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
        "clean single-commit squash should be flagged (Algorithm A)"
    );
    assert_eq!(
        landing.possible_squash_merge_sources(),
        vec!["feature/baseline"],
        "the Graph landing commit should retain the exact-match source branch"
    );
    assert!(
        landing.fuzzy_squash_match().is_none(),
        "exact match must not also get a fuzzy annotation"
    );

    // Expect B: Branches view reports a squash-merged status family member.
    // No remote is configured in this test repo, so it's local-only.
    let repo = git2::Repository::open(dir).unwrap();
    let branches = test_branches(&tmpdir, &repo, "main").expect("list_branches failed");
    let feature = branches
        .iter()
        .find(|b| b.name == "feature/baseline")
        .unwrap();
    assert_eq!(feature.merge_status, MergeStatus::LocalSquashMerged);
}

#[test]
fn test_squash_landing_lists_all_matching_local_branch_names() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/login"]);
    std::fs::write(dir.join("login.txt"), "login content\n").unwrap();
    run_git(dir, &["add", "login.txt"]);
    run_git(dir, &["commit", "-m", "add login"]);
    run_git(dir, &["branch", "feature/auth"]);
    let source_oid = git_output(dir, &["rev-parse", "HEAD"]);

    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/login"]);
    run_git(dir, &["commit", "-m", "squash login"]);
    let squash_oid = git_output(dir, &["rev-parse", "HEAD"]);

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
        .find(|commit| commit.oid == squash_oid)
        .expect("squash landing should be displayed");

    assert_eq!(
        landing
            .relationships
            .iter()
            .filter(|r| r.kind == graph::RelationshipKind::SquashMerge)
            .count(),
        1
    );
    assert_eq!(landing.relationships[0].destination_oid, squash_oid);
    assert_eq!(landing.relationships[0].source_oid, source_oid);
    assert_eq!(
        landing.relationships[0].source_refs,
        ["feature/auth", "feature/login"]
    );
    assert_eq!(
        landing.possible_squash_merge_sources(),
        vec!["feature/auth", "feature/login"],
        "all local branches at a matching tip are useful candidates, in stable order"
    );
}

#[test]
fn test_squash_scenario_02_multi_commit_branch_squashed_into_one_base_commit() {
    // Plan scenario 2: multi-commit branch touching multiple files, squashed
    // as a single base commit.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/multi"]);
    std::fs::write(dir.join("a.txt"), "a content\n").unwrap();
    run_git(dir, &["add", "a.txt"]);
    run_git(dir, &["commit", "-m", "add a"]);
    std::fs::write(dir.join("b.txt"), "b content\n").unwrap();
    run_git(dir, &["add", "b.txt"]);
    run_git(dir, &["commit", "-m", "add b"]);
    std::fs::write(dir.join("a.txt"), "a content, revised\n").unwrap();
    run_git(dir, &["add", "a.txt"]);
    run_git(dir, &["commit", "-m", "revise a"]);

    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/multi"]);
    run_git(dir, &["commit", "-m", "squash landing (multi)"]);
    let squash_oid = git_output(dir, &["rev-parse", "HEAD"]);

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..tmpdir.graph_options()
        },
    )
    .expect("graph load should succeed");
    assert!(
        snapshot
            .commits
            .iter()
            .find(|c| c.oid == squash_oid)
            .expect("squash landing commit should be displayed")
            .is_possible_squash_merge(),
        "multi-commit branch squashed to one base commit should still match on aggregate diff"
    );

    let repo = git2::Repository::open(dir).unwrap();
    let branches = test_branches(&tmpdir, &repo, "main").expect("list_branches failed");
    let feature = branches.iter().find(|b| b.name == "feature/multi").unwrap();
    assert_eq!(feature.merge_status, MergeStatus::LocalSquashMerged);
}

#[test]
fn test_squash_scenario_03_regular_merge_is_not_flagged_as_squash() {
    // Plan scenario 3: a normal two-parent merge must not be flagged as squash
    // by either detector.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/regular-merge"]);
    std::fs::write(dir.join("regular.txt"), "regular content\n").unwrap();
    run_git(dir, &["add", "regular.txt"]);
    run_git(dir, &["commit", "-m", "regular feature commit"]);

    run_git(dir, &["checkout", "main"]);
    // Give main its own commit so the merge can't fast-forward and actually
    // produces a two-parent merge commit.
    std::fs::write(dir.join("main-change.txt"), "main change\n").unwrap();
    run_git(dir, &["add", "main-change.txt"]);
    run_git(dir, &["commit", "-m", "main change"]);
    run_git(
        dir,
        &["merge", "feature/regular-merge", "-m", "regular merge"],
    );
    let merge_oid = git_output(dir, &["rev-parse", "HEAD"]);

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..tmpdir.graph_options()
        },
    )
    .expect("graph load should succeed");
    assert!(
        !snapshot
            .commits
            .iter()
            .find(|c| c.oid == merge_oid)
            .expect("merge commit should be displayed")
            .is_possible_squash_merge(),
        "a regular merge commit must never be flagged as a possible squash merge"
    );

    let repo = git2::Repository::open(dir).unwrap();
    let branches = test_branches(&tmpdir, &repo, "main").expect("list_branches failed");
    let feature = branches
        .iter()
        .find(|b| b.name == "feature/regular-merge")
        .unwrap();
    assert_eq!(
        feature.merge_status,
        MergeStatus::Merged,
        "regular merges must be classified by the regular-merge detector, not fall through to squash"
    );
}

#[test]
fn test_squash_scenario_04_branch_with_internal_merge_commit_then_squash_merged() {
    // Plan scenario 4: the branch's own history contains a merge commit
    // before it is squash-merged into base. Algorithm A/B diff merge-base to
    // tip as a flat tree diff, so internal topology must not matter.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/topology"]);
    std::fs::write(dir.join("topology.txt"), "topology base\n").unwrap();
    run_git(dir, &["add", "topology.txt"]);
    run_git(dir, &["commit", "-m", "topology base commit"]);

    run_git(dir, &["checkout", "-b", "feature/topology-sub"]);
    std::fs::write(dir.join("sub.txt"), "sub content\n").unwrap();
    run_git(dir, &["add", "sub.txt"]);
    run_git(dir, &["commit", "-m", "sub commit"]);

    run_git(dir, &["checkout", "feature/topology"]);
    run_git(
        dir,
        &[
            "merge",
            "feature/topology-sub",
            "-m",
            "merge sub into topology",
        ],
    );

    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/topology"]);
    run_git(dir, &["commit", "-m", "squash landing (topology)"]);
    let squash_oid = git_output(dir, &["rev-parse", "HEAD"]);

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..tmpdir.graph_options()
        },
    )
    .expect("graph load should succeed");
    assert!(
        snapshot
            .commits
            .iter()
            .find(|c| c.oid == squash_oid)
            .expect("squash landing commit should be displayed")
            .is_possible_squash_merge(),
        "a merge commit inside the branch's own history must not prevent detection"
    );

    let repo = git2::Repository::open(dir).unwrap();
    let branches = test_branches(&tmpdir, &repo, "main").expect("list_branches failed");
    let feature = branches
        .iter()
        .find(|b| b.name == "feature/topology")
        .unwrap();
    assert_eq!(feature.merge_status, MergeStatus::LocalSquashMerged);
}
