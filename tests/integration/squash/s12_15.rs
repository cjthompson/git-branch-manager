
use git_branch_manager::git::{
    fuzzy_match, graph, merge_detection,
};
use git_branch_manager::types::MergeStatus;

use crate::common::*;

#[test]
fn test_squash_scenario_12_empty_net_zero_branch_and_base_commit() {
    // Plan scenario 12: a branch whose commits fully cancel out (branch-tip
    // tree == merge-base tree) produces an empty diff. `git patch-id --stable`
    // on an empty diff produces no output at all (confirmed empirically: this
    // codebase's `compute_patch`/`stable_patch_id` early-returns `None` when
    // `git diff`'s stdout is empty, *before* ever invoking `patch-id`). A
    // second, unrelated empty-diff base commit is also constructed to confirm
    // the current implementation does not treat two "empty" patch-ids as
    // equal to each other (a real false-positive risk if `None == None` were
    // ever compared) — it can't, since only `Some(patch_id)` entries are
    // inserted into the match tables (see `annotate_possible_squash_merges`
    // in `git/graph.rs`).
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/net-zero"]);
    std::fs::write(dir.join("temp.txt"), "temporary content\n").unwrap();
    run_git(dir, &["add", "temp.txt"]);
    run_git(dir, &["commit", "-m", "add temp content"]);
    run_git(dir, &["rm", "temp.txt"]);
    run_git(dir, &["commit", "-m", "remove temp content (net zero)"]);

    run_git(dir, &["checkout", "main"]);
    // An unrelated, independent net-zero commit on base: add then remove a
    // different file in a single commit sequence.
    std::fs::write(dir.join("other-temp.txt"), "other temporary\n").unwrap();
    run_git(dir, &["add", "other-temp.txt"]);
    run_git(dir, &["commit", "-m", "add other temp content"]);
    run_git(dir, &["rm", "other-temp.txt"]);
    run_git(
        dir,
        &[
            "commit",
            "-m",
            "remove other temp content (net zero, unrelated)",
        ],
    );
    let empty_base_oid = git_output(dir, &["rev-parse", "HEAD"]);

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
            .find(|c| c.oid == empty_base_oid)
            .expect("unrelated empty base commit should be displayed")
            .is_possible_squash_merge(),
        "an unrelated empty/net-zero base commit must never be flagged, including against \
         another empty-diff branch — empty diffs are explicitly excluded (None), never matched"
    );
    assert!(
        snapshot
            .commits
            .iter()
            .all(|c| !c.is_possible_squash_merge()),
        "no commit in this fixture should be flagged: both diffs involved are empty"
    );

    assert!(!merge_detection::is_squash_merged(
        dir,
        "main",
        "feature/net-zero",
        None,
        None
    ));
}

#[test]
fn test_squash_scenario_13_reverted_branch_net_zero_but_no_specific_squash_point() {
    // Plan scenario 13: branch adds a commit, then a second commit that
    // reverts it (net-zero from base's perspective, but the commits still
    // exist as distinct history). Two separate assertions per the plan doc:
    // (a) the branch's tip tree is trivially content-identical to base's tree
    // (the "already integrated" OR-claim, checked directly via tree OIDs
    // since this codebase has no dedicated `already_integrated` field yet);
    // (b) no *specific* base commit is flagged as the squash point (the
    // `possible_squash(commit)` AND-claim), since there is nothing for a
    // base commit's diff to match against.
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();
    let base_tree = repo
        .head()
        .unwrap()
        .peel_to_tree()
        .unwrap()
        .id()
        .to_string();

    run_git(dir, &["checkout", "-b", "feature/reverted"]);
    std::fs::write(dir.join("temp2.txt"), "temp content 2\n").unwrap();
    run_git(dir, &["add", "temp2.txt"]);
    run_git(dir, &["commit", "-m", "add temp2 content"]);
    run_git(dir, &["rm", "temp2.txt"]);
    run_git(dir, &["commit", "-m", "revert temp2 content"]);

    // (a) content-equivalence check.
    let branch_tree = git_output(dir, &["rev-parse", "HEAD^{tree}"]);
    assert_eq!(
        branch_tree, base_tree,
        "reverted branch's tip tree must equal base's tree (already integrated, content-wise)"
    );

    // (b) no specific base commit is the squash point.
    run_git(dir, &["checkout", "main"]);
    assert!(
        !merge_detection::is_squash_merged(dir, "main", "feature/reverted", None, None),
        "an empty aggregate diff must not report as squash-merged into any base commit"
    );

    let repo = git2::Repository::open(dir).unwrap();
    let branches = test_branches(&tmpdir, &repo, "main").expect("list_branches failed");
    let feature = branches
        .iter()
        .find(|b| b.name == "feature/reverted")
        .unwrap();
    assert_eq!(
        feature.merge_status,
        MergeStatus::Unmerged,
        "observed: current status classification has no distinct \"already integrated but no \
         specific squash point\" state — a reverted branch reports plain Unmerged, matching \
         neither the regular-merge reachable-set check (different OID) nor the squash check \
         (no diff to match)"
    );
}

#[test]
fn test_squash_scenario_14_duplicate_independently_recreated_patch() {
    // Plan scenario 14: branch makes a one-line change; independently, a
    // *later, unrelated* commit on base makes the exact same one-line change
    // to the same file (two developers writing the identical fix). This is
    // the documented, accepted false positive for patch-id-based detection:
    // content equivalence, not historical provenance, is all patch-id can
    // observe.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    std::fs::write(dir.join("dup.txt"), "before\n").unwrap();
    run_git(dir, &["add", "dup.txt"]);
    run_git(dir, &["commit", "-m", "add dup.txt"]);

    run_git(dir, &["checkout", "-b", "feature/duplicate-fix"]);
    std::fs::write(dir.join("dup.txt"), "after\n").unwrap();
    run_git(dir, &["commit", "-am", "branch: fix dup.txt"]);

    run_git(dir, &["checkout", "main"]);
    std::fs::write(dir.join("dup.txt"), "after\n").unwrap();
    run_git(
        dir,
        &[
            "commit",
            "-am",
            "main: independently make the identical fix",
        ],
    );
    let independent_base_oid = git_output(dir, &["rev-parse", "HEAD"]);

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..tmpdir.graph_options()
        },
    )
    .expect("graph load should succeed");
    let independent = snapshot
        .commits
        .iter()
        .find(|c| c.oid == independent_base_oid)
        .expect("independent base commit should be displayed");
    assert!(
        independent.is_possible_squash_merge(),
        "documented, accepted false positive: an independently recreated identical one-line \
         fix is indistinguishable from a real squash landing under patch-id equality \
         (content equivalence, not historical provenance) — see plan doc scenario 14"
    );
    assert!(merge_detection::is_squash_merged(
        dir,
        "main",
        "feature/duplicate-fix",
        None,
        None
    ));

    // Calibration data point for Option 6: directly score the two diffs
    // (branch's own diff vs. the independent base commit's diff) with
    // `fuzzy_match::score`/`classify`. Since these are the exact same
    // one-line, one-file change, similarity is 1.0 — and `classify` already
    // defers `similarity >= 1.0` to the exact-match tier (which fired above),
    // so no *additional* fuzzy signal is expected here regardless of the
    // small-diff `MIN_UNION_SIZE_FOR_FUZZY` gate.
    let branch_diff = git_output(
        dir,
        &[
            "diff",
            "--binary",
            "--full-index",
            "main~1",
            "feature/duplicate-fix",
            "--",
        ],
    );
    let base_diff = git_output(
        dir,
        &["diff", "--binary", "--full-index", "main~1", "main", "--"],
    );
    let fscore = fuzzy_match::score(branch_diff.as_bytes(), base_diff.as_bytes())
        .expect("identical single-file diffs should pass the cheap prefilter");
    assert_eq!(
        fscore.similarity, 1.0,
        "observed: the coincidental duplicate fix scores as perfectly similar"
    );
    assert!(
        fuzzy_match::classify(&fscore).is_none(),
        "similarity == 1.0 always defers to the exact-match tier, regardless of diff size"
    );
}

#[test]
fn test_squash_scenario_15_criss_cross_multiple_merge_bases() {
    // Plan scenario 15: construct a criss-cross merge (two branches that have
    // merged each other using each other's *pre-merge* tips), so `main` and
    // `feature` have two co-equal merge bases. `git merge-base --all` should
    // report both; a plain `git merge-base` picks exactly one of them
    // (officially unspecified by git). This test pins down current behavior
    // for this fixture rather than asserting a "correct" choice git itself
    // doesn't guarantee.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "criss-a"]);
    std::fs::write(dir.join("a.txt"), "a content\n").unwrap();
    run_git(dir, &["add", "a.txt"]);
    run_git(dir, &["commit", "-m", "criss-a commit"]);
    let a_tip = git_output(dir, &["rev-parse", "HEAD"]);

    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["checkout", "-b", "criss-b"]);
    std::fs::write(dir.join("b.txt"), "b content\n").unwrap();
    run_git(dir, &["add", "b.txt"]);
    run_git(dir, &["commit", "-m", "criss-b commit"]);
    let b_tip = git_output(dir, &["rev-parse", "HEAD"]);

    // Merge b into a normally: parents [a_tip, b_tip].
    run_git(dir, &["checkout", "criss-a"]);
    run_git(dir, &["merge", "criss-b", "-m", "merge b into a"]);
    let merged_tree = git_output(dir, &["rev-parse", "HEAD^{tree}"]);

    // Construct the *other* merge directly via commit-tree, with parents
    // [b_tip, a_tip] (the PRE-merge tips) so neither merge commit is an
    // ancestor of the other — a genuine criss-cross, not a fast-forwarded
    // chain.
    let criss_merge_2 = git_output(
        dir,
        &[
            "commit-tree",
            &merged_tree,
            "-p",
            &b_tip,
            "-p",
            &a_tip,
            "-m",
            "merge a into b",
        ],
    );
    run_git(dir, &["branch", "-f", "criss-b", &criss_merge_2]);

    let all_bases = git_output(dir, &["merge-base", "--all", "criss-a", "criss-b"]);
    let base_count = all_bases.lines().filter(|l| !l.trim().is_empty()).count();
    assert_eq!(
        base_count, 2,
        "expected a genuine criss-cross fixture with two co-equal merge bases, got: {all_bases}"
    );
    let base_set: std::collections::HashSet<&str> = all_bases.lines().collect();
    assert!(base_set.contains(a_tip.as_str()));
    assert!(base_set.contains(b_tip.as_str()));

    // Record which single base `git merge-base` (no --all) actually picks in
    // this codebase's exact invocation pattern.
    let picked = git_output(dir, &["merge-base", "criss-a", "criss-b"]);
    assert!(
        base_set.contains(picked.as_str()),
        "the single-base pick must be one of the --all candidates, got: {picked}"
    );

    // Confirm using this merge-base does not panic and produces a boolean.
    // criss-a and criss-b have identical trees at this point (both built from
    // `merged_tree`), so a squash-style comparison should find the branch
    // fully represented already (this is a smoke check of the fixture, not a
    // squash-detection claim about a landed commit).
    let result = merge_detection::is_squash_merged(dir, "criss-a", "criss-b", None, Some(&picked));
    // No assertion on the boolean value itself beyond it not panicking —
    // per the plan doc, only the merge-base pick and non-panicking behavior
    // are being characterized here.
    let _ = result;
}
