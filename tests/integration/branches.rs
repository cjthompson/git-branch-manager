
use git_branch_manager::git::{
    branch, merge_detection,
};
use git_branch_manager::types::MergeStatus;

use crate::common::*;

#[test]
fn test_detect_base_branch_main() {
    let (_tmpdir, repo) = setup_test_repo();

    let base = branch::detect_base_branch(&repo, None).expect("detect_base_branch failed");
    assert_eq!(base, "main");
}

#[test]
fn test_detect_base_branch_override() {
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Create a "develop" branch
    run_git(dir, &["branch", "develop"]);

    let base =
        branch::detect_base_branch(&repo, Some("develop")).expect("detect_base_branch failed");
    assert_eq!(base, "develop");
}

#[test]
fn test_detect_base_branch_override_nonexistent() {
    let (_tmpdir, repo) = setup_test_repo();

    let result = branch::detect_base_branch(&repo, Some("nonexistent"));
    assert!(
        result.is_err(),
        "expected error for nonexistent branch override"
    );
}

#[test]
fn test_list_branches() {
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Create two feature branches
    run_git(dir, &["branch", "feature-a"]);
    run_git(dir, &["branch", "feature-b"]);

    let branches = test_branches(&tmpdir, &repo, "main").expect("list_branches failed");

    // Should have 3 branches: main, feature-a, feature-b
    assert_eq!(
        branches.len(),
        3,
        "expected 3 branches, got: {:?}",
        branches.iter().map(|b| &b.name).collect::<Vec<_>>()
    );

    let names: Vec<&str> = branches.iter().map(|b| b.name.as_str()).collect();
    assert!(names.contains(&"main"), "missing 'main'");
    assert!(names.contains(&"feature-a"), "missing 'feature-a'");
    assert!(names.contains(&"feature-b"), "missing 'feature-b'");

    // main should be marked as base
    let main_branch = branches.iter().find(|b| b.name == "main").unwrap();
    assert!(main_branch.is_base, "main should be marked is_base");
}

#[test]
fn test_merged_branch_detection() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Create feature-merged branch with a commit
    run_git(dir, &["checkout", "-b", "feature-merged"]);
    let feature_file = dir.join("feature.txt");
    std::fs::write(&feature_file, "feature content\n").expect("failed to write feature file");
    run_git(dir, &["add", "feature.txt"]);
    run_git(dir, &["commit", "-m", "Add feature"]);

    // Switch back to main, add a commit so merge can't fast-forward, then merge
    run_git(dir, &["checkout", "main"]);
    let main_file = dir.join("main-change.txt");
    std::fs::write(&main_file, "main branch change\n").expect("failed to write main file");
    run_git(dir, &["add", "main-change.txt"]);
    run_git(dir, &["commit", "-m", "Main branch commit"]);
    run_git(
        dir,
        &["merge", "feature-merged", "-m", "Merge feature-merged"],
    );

    // Re-open the repo so git2 sees the merge commit
    let repo = git2::Repository::open(dir).expect("failed to re-open repo");
    let branches = test_branches(&tmpdir, &repo, "main").expect("list_branches failed");

    let feature = branches
        .iter()
        .find(|b| b.name == "feature-merged")
        .expect("feature-merged branch not found");
    assert_eq!(
        feature.merge_status,
        MergeStatus::Merged,
        "feature-merged should be detected as Merged"
    );
}

#[test]
fn test_squash_merged_branch_detection() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Create feature-squashed branch with a commit
    run_git(dir, &["checkout", "-b", "feature-squashed"]);
    let squash_file = dir.join("squash-feature.txt");
    std::fs::write(&squash_file, "squash feature content\n")
        .expect("failed to write squash feature file");
    run_git(dir, &["add", "squash-feature.txt"]);
    run_git(dir, &["commit", "-m", "Add squash feature"]);

    // Switch back to main and squash merge
    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature-squashed"]);
    run_git(dir, &["commit", "-m", "squash merge feature-squashed"]);

    // Re-open the repo so git2 sees the latest state
    let repo = git2::Repository::open(dir).expect("failed to re-open repo");
    let branches = test_branches(&tmpdir, &repo, "main").expect("list_branches failed");

    let feature = branches
        .iter()
        .find(|b| b.name == "feature-squashed")
        .expect("feature-squashed branch not found");
    assert_eq!(
        feature.merge_status,
        MergeStatus::LocalSquashMerged,
        "feature-squashed should be detected as LocalSquashMerged (no remote in test repo)"
    );
}

#[test]
fn test_in_sync_branch_detection() {
    // A branch cut from main with no commits added has HEAD == main HEAD.
    // That used to report as Merged, which is misleading — no integration
    // event occurred. It should report as InSync instead.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Cut two branches: one truly empty, one that adds a commit then resets
    // back to main so its tip also equals main's tip.
    run_git(dir, &["branch", "feature-fresh"]);

    let repo = git2::Repository::open(dir).expect("failed to open repo");
    let branches = test_branches(&tmpdir, &repo, "main").expect("list_branches failed");

    let fresh = branches
        .iter()
        .find(|b| b.name == "feature-fresh")
        .expect("feature-fresh branch not found");
    assert_eq!(
        fresh.merge_status,
        MergeStatus::InSync,
        "fresh branch (tip == base tip) should be InSync, not Merged"
    );
}

#[test]
fn test_in_sync_does_not_swallow_real_merges() {
    // Regression guard: an InSync check at the top of regular_merge_status
    // must not cause branches with unique merged commits to also report InSync
    // (they have a unique commit, so their tip differs from base tip and
    // should still be Merged).
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature-merged"]);
    std::fs::write(dir.join("f.txt"), "f").unwrap();
    run_git(dir, &["add", "f.txt"]);
    run_git(dir, &["commit", "-m", "add f"]);
    run_git(dir, &["checkout", "main"]);
    // Add a commit to main so merge can't fast-forward.
    std::fs::write(dir.join("main-change.txt"), "m").unwrap();
    run_git(dir, &["add", "main-change.txt"]);
    run_git(dir, &["commit", "-m", "main change"]);
    run_git(dir, &["merge", "feature-merged", "-m", "merge f"]);

    let repo = git2::Repository::open(dir).expect("failed to re-open repo");
    let branches = test_branches(&tmpdir, &repo, "main").expect("list_branches failed");

    let merged = branches
        .iter()
        .find(|b| b.name == "feature-merged")
        .expect("feature-merged branch not found");
    assert_eq!(
        merged.merge_status,
        MergeStatus::Merged,
        "regular-merged branch must still report Merged, not InSync"
    );
}

#[test]
fn test_unmerged_branch_detection() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Create an unmerged branch with a unique commit
    run_git(dir, &["checkout", "-b", "feature-wip"]);
    let wip_file = dir.join("wip.txt");
    std::fs::write(&wip_file, "work in progress\n").expect("failed to write wip file");
    run_git(dir, &["add", "wip.txt"]);
    run_git(dir, &["commit", "-m", "WIP commit"]);

    // Switch back to main (do NOT merge)
    run_git(dir, &["checkout", "main"]);

    // Re-open the repo so git2 sees the latest state
    let repo = git2::Repository::open(dir).expect("failed to re-open repo");
    let branches = test_branches(&tmpdir, &repo, "main").expect("list_branches failed");

    let feature = branches
        .iter()
        .find(|b| b.name == "feature-wip")
        .expect("feature-wip branch not found");
    assert_eq!(
        feature.merge_status,
        MergeStatus::Unmerged,
        "feature-wip should be detected as Unmerged"
    );
}

#[test]
fn test_list_branches_phase1() {
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/a"]);
    std::fs::write(dir.join("a.txt"), "a").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "feature a"]);
    run_git(dir, &["checkout", "main"]);

    let branches = branch::list_branches_phase1(&repo, "main").unwrap();
    assert!(branches.len() >= 2);
    let main_branch = branches.iter().find(|b| b.name == "main").unwrap();
    assert!(main_branch.is_base);
    let feature = branches.iter().find(|b| b.name == "feature/a").unwrap();
    assert!(!feature.is_base);
    // Phase 1 marks unmerged non-pinned as Pending
    assert_eq!(feature.merge_status, MergeStatus::Pending);
}

#[test]
fn test_is_squash_merged_direct() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/squashed"]);
    std::fs::write(dir.join("squash.txt"), "squash content").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "squash commit"]);
    run_git(dir, &["checkout", "main"]);

    run_git(dir, &["merge", "--squash", "feature/squashed"]);
    run_git(dir, &["commit", "-m", "squashed feature"]);

    assert!(merge_detection::is_squash_merged(
        dir,
        "main",
        "feature/squashed",
        None,
        None
    ));
}

#[test]
fn test_is_squash_merged_with_precomputed_merge_base() {
    // The fast path: a precomputed merge base is supplied, so is_squash_merged must
    // not need to derive it via `git merge-base` and still detect the squash.
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();

    // HEAD is the initial commit; it becomes the merge base after we branch.
    let merge_base = repo
        .head()
        .unwrap()
        .peel_to_commit()
        .unwrap()
        .id()
        .to_string();

    run_git(dir, &["checkout", "-b", "feature/squashed"]);
    std::fs::write(dir.join("squash.txt"), "squash content").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "squash commit"]);
    run_git(dir, &["checkout", "main"]);

    run_git(dir, &["merge", "--squash", "feature/squashed"]);
    run_git(dir, &["commit", "-m", "squashed feature"]);

    assert!(merge_detection::is_squash_merged(
        dir,
        "main",
        "feature/squashed",
        None,
        Some(&merge_base),
    ));
}

#[test]
fn test_is_not_squash_merged_direct() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/unmerged"]);
    std::fs::write(dir.join("unmerged.txt"), "unmerged").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "unmerged commit"]);
    run_git(dir, &["checkout", "main"]);

    assert!(!merge_detection::is_squash_merged(
        dir,
        "main",
        "feature/unmerged",
        None,
        None
    ));
}
