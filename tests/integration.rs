use std::process::Command;

use git_branch_manager::git::{branch, operations};
use git_branch_manager::types::MergeStatus;

/// Create a temporary git repository with an initial commit on the "main" branch.
///
/// Returns the tempdir (must be kept alive for the duration of the test) and the
/// git2::Repository handle.
fn setup_test_repo() -> (tempfile::TempDir, git2::Repository) {
    let tmpdir = tempfile::tempdir().expect("failed to create tempdir");
    let dir = tmpdir.path();

    // git init
    run_git(dir, &["init", "-b", "main"]);

    // Configure user (required for commits)
    run_git(dir, &["config", "user.name", "Test User"]);
    run_git(dir, &["config", "user.email", "test@example.com"]);

    // Create an initial commit so that HEAD and "main" exist
    let initial_file = dir.join("README.md");
    std::fs::write(&initial_file, "# Test Repo\n").expect("failed to write initial file");
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "Initial commit"]);

    let repo = git2::Repository::open(dir).expect("failed to open repo");
    (tmpdir, repo)
}

/// Run a git command in the given directory, panicking on failure.
fn run_git(dir: &std::path::Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap_or_else(|e| panic!("failed to run git {:?}: {}", args, e));
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!(
            "git {:?} failed in {}: {}",
            args,
            dir.display(),
            stderr
        );
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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
    assert!(result.is_err(), "expected error for nonexistent branch override");
}

#[test]
fn test_list_branches() {
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Create two feature branches
    run_git(dir, &["branch", "feature-a"]);
    run_git(dir, &["branch", "feature-b"]);

    // set_current_dir is needed because squash-merge detection shells out to git
    std::env::set_current_dir(dir).expect("failed to set current dir");

    let branches = branch::list_branches(&repo, "main").expect("list_branches failed");

    // Should have 3 branches: main, feature-a, feature-b
    assert_eq!(branches.len(), 3, "expected 3 branches, got: {:?}",
        branches.iter().map(|b| &b.name).collect::<Vec<_>>());

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
    run_git(dir, &["merge", "feature-merged", "-m", "Merge feature-merged"]);

    std::env::set_current_dir(dir).expect("failed to set current dir");

    // Re-open the repo so git2 sees the merge commit
    let repo = git2::Repository::open(dir).expect("failed to re-open repo");
    let branches = branch::list_branches(&repo, "main").expect("list_branches failed");

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
    run_git(
        dir,
        &["commit", "-m", "squash merge feature-squashed"],
    );

    // The squash-merge detection shells out to git in the current working directory
    std::env::set_current_dir(dir).expect("failed to set current dir");

    // Re-open the repo so git2 sees the latest state
    let repo = git2::Repository::open(dir).expect("failed to re-open repo");
    let branches = branch::list_branches(&repo, "main").expect("list_branches failed");

    let feature = branches
        .iter()
        .find(|b| b.name == "feature-squashed")
        .expect("feature-squashed branch not found");
    assert_eq!(
        feature.merge_status,
        MergeStatus::SquashMerged,
        "feature-squashed should be detected as SquashMerged"
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

    std::env::set_current_dir(dir).expect("failed to set current dir");

    // Re-open the repo so git2 sees the latest state
    let repo = git2::Repository::open(dir).expect("failed to re-open repo");
    let branches = branch::list_branches(&repo, "main").expect("list_branches failed");

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
fn test_delete_local_branch() {
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Create a branch to delete
    run_git(dir, &["branch", "to-delete"]);

    // Verify it exists
    assert!(
        repo.find_branch("to-delete", git2::BranchType::Local).is_ok(),
        "branch should exist before deletion"
    );

    let result = operations::delete_local(&repo, "to-delete");
    assert!(result.success, "delete_local should succeed: {}", result.message);

    // Verify it's gone
    assert!(
        repo.find_branch("to-delete", git2::BranchType::Local).is_err(),
        "branch should not exist after deletion"
    );
}

#[test]
fn test_delete_local_nonexistent() {
    let (_tmpdir, repo) = setup_test_repo();

    let result = operations::delete_local(&repo, "does-not-exist");
    assert!(
        !result.success,
        "delete_local on nonexistent branch should return success: false"
    );
}
