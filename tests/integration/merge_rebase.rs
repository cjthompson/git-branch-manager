
use git_branch_manager::git::operations;

use crate::common::*;

#[test]
fn test_merge_branch_success() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Create a feature branch with a unique file
    run_git(dir, &["checkout", "-b", "feature-to-merge"]);
    std::fs::write(dir.join("feature.txt"), "feature content\n").unwrap();
    run_git(dir, &["add", "feature.txt"]);
    run_git(dir, &["commit", "-m", "Add feature"]);
    run_git(dir, &["checkout", "main"]);

    let results = operations::merge_branch(dir, "feature-to-merge", "main", false, false);
    assert_eq!(results.len(), 1);
    assert!(
        results[0].success,
        "merge should succeed: {}",
        results[0].message
    );

    // Verify the feature file is present on main
    assert!(
        dir.join("feature.txt").exists(),
        "feature.txt should be on main after merge"
    );

    // The operation's contract is to leave HEAD on the base branch.
    let repo = git2::Repository::open(dir).expect("re-open repo");
    assert_eq!(
        repo.head().unwrap().shorthand().ok(),
        Some("main"),
        "HEAD should remain on the base branch after merge"
    );
}

#[test]
fn test_merge_branch_squash_success() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature-squash"]);
    std::fs::write(dir.join("squash.txt"), "squash content\n").unwrap();
    run_git(dir, &["add", "squash.txt"]);
    run_git(dir, &["commit", "-m", "Squash candidate"]);
    run_git(dir, &["checkout", "main"]);

    let results = operations::merge_branch(dir, "feature-squash", "main", true, false);
    assert_eq!(results.len(), 1);
    assert!(
        results[0].success,
        "squash merge should succeed: {}",
        results[0].message
    );

    // The squash content should exist on main
    assert!(
        dir.join("squash.txt").exists(),
        "squash.txt should be on main after squash merge"
    );

    // And main should have a single new commit (NOT a merge commit): the squash
    // commit must have exactly one parent.
    let repo = git2::Repository::open(dir).expect("re-open repo");
    let head_commit = repo.head().unwrap().peel_to_commit().unwrap();
    assert!(
        head_commit
            .summary()
            .ok()
            .flatten()
            .is_some_and(|s| s.contains("Squash merge feature-squash")),
        "HEAD should be the squash merge commit, got: {:?}",
        head_commit.summary()
    );
    assert_eq!(
        head_commit.parent_count(),
        1,
        "squash merge must be a single-parent commit, not a merge commit"
    );
}

#[test]
fn test_merge_branch_conflict_aborts() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Both branches create the same file with different content → conflict
    run_git(dir, &["checkout", "-b", "conflict-feature"]);
    std::fs::write(dir.join("conflict.txt"), "feature version\n").unwrap();
    run_git(dir, &["add", "conflict.txt"]);
    run_git(dir, &["commit", "-m", "Feature adds conflict.txt"]);

    run_git(dir, &["checkout", "main"]);
    std::fs::write(dir.join("conflict.txt"), "main version\n").unwrap();
    run_git(dir, &["add", "conflict.txt"]);
    run_git(dir, &["commit", "-m", "Main adds conflict.txt"]);

    let results = operations::merge_branch(dir, "conflict-feature", "main", false, false);
    assert_eq!(results.len(), 1);
    assert!(!results[0].success, "conflicting merge should fail");

    // Verify merge was aborted cleanly: no conflicts, no half-staged leftovers,
    // and HEAD restored to the base branch.
    let repo = git2::Repository::open(dir).expect("re-open repo");
    let statuses = repo.statuses(None).unwrap();
    let dirty = statuses.iter().any(|e| {
        e.status().intersects(
            git2::Status::CONFLICTED
                | git2::Status::INDEX_NEW
                | git2::Status::INDEX_MODIFIED
                | git2::Status::INDEX_DELETED,
        )
    });
    assert!(
        !dirty,
        "merge abort should leave no conflicts or staged leftovers"
    );
    assert_eq!(
        repo.head().unwrap().shorthand().ok(),
        Some("main"),
        "HEAD should be back on the base branch after abort"
    );
}

// ---------------------------------------------------------------------------
// Rebase operation tests
// ---------------------------------------------------------------------------

#[test]
fn test_rebase_branch_success() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Feature branch: add a unique file
    run_git(dir, &["checkout", "-b", "feature-rebase"]);
    std::fs::write(dir.join("rebase-feature.txt"), "feature content\n").unwrap();
    run_git(dir, &["add", "rebase-feature.txt"]);
    run_git(dir, &["commit", "-m", "Feature commit"]);

    // Main gets a new commit (so rebase is non-trivial)
    run_git(dir, &["checkout", "main"]);
    std::fs::write(dir.join("main-update.txt"), "main update\n").unwrap();
    run_git(dir, &["add", "main-update.txt"]);
    run_git(dir, &["commit", "-m", "Main update"]);

    // Rebase feature onto main (rebase checks out the feature branch internally)
    let results = operations::rebase_branch(dir, "feature-rebase", "main", false);
    assert_eq!(results.len(), 1);
    assert!(
        results[0].success,
        "rebase should succeed: {}",
        results[0].message
    );

    // After rebase, feature should be 1 commit ahead of main
    let repo = git2::Repository::open(dir).unwrap();
    let feature_oid = repo
        .find_branch("feature-rebase", git2::BranchType::Local)
        .unwrap()
        .get()
        .peel_to_commit()
        .unwrap()
        .id();
    let main_oid = repo
        .find_branch("main", git2::BranchType::Local)
        .unwrap()
        .get()
        .peel_to_commit()
        .unwrap()
        .id();
    let (ahead, behind) = repo.graph_ahead_behind(feature_oid, main_oid).unwrap();
    assert_eq!(
        ahead, 1,
        "feature should be 1 commit ahead of main after rebase"
    );
    assert_eq!(
        behind, 0,
        "feature should be 0 commits behind main after rebase onto it"
    );
}

#[test]
fn test_rebase_branch_conflict_aborts() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Create a shared file on main
    std::fs::write(dir.join("shared.txt"), "original\n").unwrap();
    run_git(dir, &["add", "shared.txt"]);
    run_git(dir, &["commit", "-m", "Add shared file"]);

    // Feature branch modifies shared.txt
    run_git(dir, &["checkout", "-b", "rebase-conflict"]);
    std::fs::write(dir.join("shared.txt"), "feature version\n").unwrap();
    run_git(dir, &["add", "shared.txt"]);
    run_git(dir, &["commit", "-m", "Feature modifies shared"]);

    // Main also modifies shared.txt (divergent history)
    run_git(dir, &["checkout", "main"]);
    std::fs::write(dir.join("shared.txt"), "main version\n").unwrap();
    run_git(dir, &["add", "shared.txt"]);
    run_git(dir, &["commit", "-m", "Main modifies shared"]);

    let results = operations::rebase_branch(dir, "rebase-conflict", "main", false);
    assert_eq!(results.len(), 1);
    assert!(!results[0].success, "conflicting rebase should fail");
    assert!(
        results[0].message.contains("Rebase conflict") && results[0].message.contains("aborted"),
        "message should report the conflict was aborted: {}",
        results[0].message
    );

    // Rebase should be aborted: no ongoing rebase state
    let rebase_head = dir.join(".git").join("REBASE_HEAD");
    assert!(
        !rebase_head.exists(),
        "REBASE_HEAD should not exist after abort"
    );
    // And the working tree must be clean — no conflict markers left behind, HEAD
    // restored to the branch that was being rebased.
    let repo = git2::Repository::open(dir).unwrap();
    let statuses = repo.statuses(None).unwrap();
    assert!(
        !statuses
            .iter()
            .any(|e| e.status().contains(git2::Status::CONFLICTED)),
        "rebase abort should leave no conflicted files"
    );
    assert_eq!(
        repo.head().unwrap().shorthand().ok(),
        Some("rebase-conflict"),
        "HEAD should be restored to the rebased branch after abort"
    );
}
