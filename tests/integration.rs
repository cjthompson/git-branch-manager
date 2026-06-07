use std::process::Command;
use std::sync::atomic::AtomicBool;

use git_branch_manager::git::{
    branch, merge_detection, operations, squash_loader, status, tags, worktree,
};
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
        panic!("git {:?} failed in {}: {}", args, dir.display(), stderr);
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

    let branches = branch::list_branches(&repo, "main").expect("list_branches failed");

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
    run_git(dir, &["commit", "-m", "squash merge feature-squashed"]);

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
        repo.find_branch("to-delete", git2::BranchType::Local)
            .is_ok(),
        "branch should exist before deletion"
    );

    let result = operations::delete_local(&repo, "to-delete");
    assert!(
        result.success,
        "delete_local should succeed: {}",
        result.message
    );

    // Verify it's gone
    assert!(
        repo.find_branch("to-delete", git2::BranchType::Local)
            .is_err(),
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

#[test]
fn test_ahead_behind_indicators() {
    // Create a bare "remote" repo, clone it, push a branch, then add a local commit.
    // The branch should report ahead=1, behind=0.
    let tmpdir = tempfile::tempdir().expect("failed to create tmpdir");
    let base_dir = tmpdir.path();

    // 1. Create a bare remote repo
    let remote_dir = base_dir.join("remote.git");
    std::fs::create_dir_all(&remote_dir).unwrap();
    run_git(&remote_dir, &["init", "--bare", "-b", "main"]);

    // 2. Clone it into a working directory
    let work_dir = base_dir.join("work");
    run_git(base_dir, &["clone", remote_dir.to_str().unwrap(), "work"]);

    // 3. Configure user in the clone
    run_git(&work_dir, &["config", "user.name", "Test User"]);
    run_git(&work_dir, &["config", "user.email", "test@example.com"]);

    // 4. Create an initial commit on main and push
    let readme = work_dir.join("README.md");
    std::fs::write(&readme, "# Test\n").unwrap();
    run_git(&work_dir, &["add", "."]);
    run_git(&work_dir, &["commit", "-m", "Initial commit"]);
    run_git(&work_dir, &["push", "-u", "origin", "main"]);

    // 5. Create a feature branch and push it so it has a remote tracking branch
    run_git(&work_dir, &["checkout", "-b", "feature-ahead"]);
    let feature_file = work_dir.join("feature.txt");
    std::fs::write(&feature_file, "feature content\n").unwrap();
    run_git(&work_dir, &["add", "feature.txt"]);
    run_git(&work_dir, &["commit", "-m", "Feature commit 1"]);
    run_git(&work_dir, &["push", "-u", "origin", "feature-ahead"]);

    // 6. Add another local commit (not pushed) — this should make ahead=1
    let feature_file2 = work_dir.join("feature2.txt");
    std::fs::write(&feature_file2, "more feature content\n").unwrap();
    run_git(&work_dir, &["add", "feature2.txt"]);
    run_git(
        &work_dir,
        &["commit", "-m", "Feature commit 2 (local only)"],
    );

    // 7. Go back to main for listing
    run_git(&work_dir, &["checkout", "main"]);

    // 8. Open repo and list branches
    let repo = git2::Repository::open(&work_dir).expect("failed to open work repo");
    let branches = branch::list_branches(&repo, "main").expect("list_branches failed");

    let feature = branches
        .iter()
        .find(|b| b.name == "feature-ahead")
        .expect("feature-ahead branch not found");

    assert_eq!(
        feature.ahead,
        Some(1),
        "feature-ahead should be 1 commit ahead of its upstream"
    );
    assert_eq!(
        feature.behind,
        Some(0),
        "feature-ahead should be 0 commits behind its upstream"
    );

    // Also verify that main (which is in sync) reports ahead=0, behind=0
    let main_branch = branches.iter().find(|b| b.name == "main").unwrap();
    assert_eq!(main_branch.ahead, Some(0), "main should be 0 ahead");
    assert_eq!(main_branch.behind, Some(0), "main should be 0 behind");
}

#[test]
fn test_ahead_behind_local_only_branch() {
    // A branch with no upstream should have ahead=None, behind=None
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["branch", "local-only"]);

    let branches = branch::list_branches(&repo, "main").expect("list_branches failed");
    let local_branch = branches
        .iter()
        .find(|b| b.name == "local-only")
        .expect("local-only branch not found");

    assert_eq!(
        local_branch.ahead, None,
        "local-only branch should have ahead=None"
    );
    assert_eq!(
        local_branch.behind, None,
        "local-only branch should have behind=None"
    );
}

#[test]
fn test_checkout_branch() {
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();
    run_git(dir, &["branch", "feature-checkout"]);

    let result = operations::checkout_branch(&repo, dir, "feature-checkout", false);
    assert!(
        result.success,
        "checkout should succeed: {}",
        result.message
    );

    let repo = git2::Repository::open(dir).unwrap();
    let head = repo.head().unwrap();
    assert_eq!(head.shorthand().unwrap(), "feature-checkout");
}

#[test]
fn test_checkout_branch_with_stash() {
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();
    run_git(dir, &["branch", "feature-stash-checkout"]);

    // Create a dirty working tree (unstaged change)
    let dirty_file = dir.join("README.md");
    std::fs::write(&dirty_file, "# Modified\n").expect("failed to write dirty file");

    let result = operations::checkout_branch(&repo, dir, "feature-stash-checkout", true);
    assert!(
        result.success,
        "checkout with stash should succeed: {}",
        result.message
    );

    let repo = git2::Repository::open(dir).unwrap();
    let head = repo.head().unwrap();
    assert_eq!(head.shorthand().unwrap(), "feature-stash-checkout");

    // The stash should have been popped, so the working tree change should be present
    let contents = std::fs::read_to_string(&dirty_file).expect("failed to read file");
    assert_eq!(contents, "# Modified\n", "stash pop should restore changes");
}

// ---------------------------------------------------------------------------
// Remote branch tests — helper
// ---------------------------------------------------------------------------

/// Create a bare remote + clone setup with an initial commit on main,
/// plus remote-only and local+remote branches for testing.
///
/// Returns (tmpdir, work_dir path, Repository for the clone).
fn setup_remote_test_repo() -> (tempfile::TempDir, std::path::PathBuf, git2::Repository) {
    let tmpdir = tempfile::tempdir().expect("failed to create tmpdir");
    let base_dir = tmpdir.path();

    // Bare remote
    let remote_dir = base_dir.join("remote.git");
    std::fs::create_dir_all(&remote_dir).unwrap();
    run_git(&remote_dir, &["init", "--bare", "-b", "main"]);

    // Clone
    let work_dir = base_dir.join("work");
    run_git(base_dir, &["clone", remote_dir.to_str().unwrap(), "work"]);
    run_git(&work_dir, &["config", "user.name", "Test User"]);
    run_git(&work_dir, &["config", "user.email", "test@example.com"]);

    // Initial commit on main
    let readme = work_dir.join("README.md");
    std::fs::write(&readme, "# Test\n").unwrap();
    run_git(&work_dir, &["add", "."]);
    run_git(&work_dir, &["commit", "-m", "Initial commit"]);
    run_git(&work_dir, &["push", "-u", "origin", "main"]);

    let repo = git2::Repository::open(&work_dir).expect("failed to open work repo");
    (tmpdir, work_dir, repo)
}

// ---------------------------------------------------------------------------
// Remote branch tests
// ---------------------------------------------------------------------------

#[test]
fn test_list_remote_branches() {
    let (_tmpdir, work_dir, repo) = setup_remote_test_repo();

    // Create a feature branch and push it
    run_git(&work_dir, &["checkout", "-b", "feature-remote"]);
    let f = work_dir.join("feature.txt");
    std::fs::write(&f, "content\n").unwrap();
    run_git(&work_dir, &["add", "feature.txt"]);
    run_git(&work_dir, &["commit", "-m", "Feature commit"]);
    run_git(&work_dir, &["push", "-u", "origin", "feature-remote"]);
    run_git(&work_dir, &["checkout", "main"]);

    let remotes = branch::list_remote_branches_phase1(&repo, "main")
        .expect("list_remote_branches_phase1 failed");

    // Should have origin/main and origin/feature-remote
    let names: Vec<&str> = remotes.iter().map(|r| r.short_name.as_str()).collect();
    assert!(names.contains(&"main"), "should list origin/main");
    assert!(
        names.contains(&"feature-remote"),
        "should list origin/feature-remote"
    );

    // origin/main should be marked as base
    let main_remote = remotes.iter().find(|r| r.short_name == "main").unwrap();
    assert!(main_remote.is_base, "origin/main should be is_base");
    assert_eq!(main_remote.remote, "origin");
    assert_eq!(main_remote.full_ref, "origin/main");
}

#[test]
fn test_remote_branch_has_local() {
    let (_tmpdir, work_dir, repo) = setup_remote_test_repo();

    // Push a branch that has a local counterpart
    run_git(&work_dir, &["checkout", "-b", "has-local"]);
    let f = work_dir.join("local.txt");
    std::fs::write(&f, "content\n").unwrap();
    run_git(&work_dir, &["add", "local.txt"]);
    run_git(&work_dir, &["commit", "-m", "Local branch commit"]);
    run_git(&work_dir, &["push", "-u", "origin", "has-local"]);

    // Push a branch then delete the local copy (remote-only)
    run_git(&work_dir, &["checkout", "-b", "remote-only"]);
    let f2 = work_dir.join("remote-only.txt");
    std::fs::write(&f2, "content\n").unwrap();
    run_git(&work_dir, &["add", "remote-only.txt"]);
    run_git(&work_dir, &["commit", "-m", "Remote-only commit"]);
    run_git(&work_dir, &["push", "-u", "origin", "remote-only"]);
    run_git(&work_dir, &["checkout", "main"]);
    run_git(&work_dir, &["branch", "-D", "remote-only"]);

    let remotes = branch::list_remote_branches_phase1(&repo, "main")
        .expect("list_remote_branches_phase1 failed");

    let has_local_branch = remotes
        .iter()
        .find(|r| r.short_name == "has-local")
        .unwrap();
    assert!(
        has_local_branch.has_local,
        "has-local should have has_local=true"
    );

    let remote_only_branch = remotes
        .iter()
        .find(|r| r.short_name == "remote-only")
        .unwrap();
    assert!(
        !remote_only_branch.has_local,
        "remote-only should have has_local=false"
    );
}

#[test]
fn test_remote_branch_merged_detection() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    // Create and push a feature branch
    run_git(&work_dir, &["checkout", "-b", "feature-to-merge"]);
    let f = work_dir.join("merge-feature.txt");
    std::fs::write(&f, "merge content\n").unwrap();
    run_git(&work_dir, &["add", "merge-feature.txt"]);
    run_git(&work_dir, &["commit", "-m", "Feature to merge"]);
    run_git(&work_dir, &["push", "-u", "origin", "feature-to-merge"]);

    // Add a commit on main so merge is not a fast-forward
    run_git(&work_dir, &["checkout", "main"]);
    let main_file = work_dir.join("main-change.txt");
    std::fs::write(&main_file, "main branch change\n").unwrap();
    run_git(&work_dir, &["add", "main-change.txt"]);
    run_git(&work_dir, &["commit", "-m", "Main branch commit"]);

    // Merge and push
    run_git(
        &work_dir,
        &["merge", "feature-to-merge", "-m", "Merge feature"],
    );
    run_git(&work_dir, &["push", "origin", "main"]);

    // Re-open to see updated refs
    let repo = git2::Repository::open(&work_dir).unwrap();
    let mut remotes = branch::list_remote_branches_phase1(&repo, "main")
        .expect("list_remote_branches_phase1 failed");

    // Phase-1 now always returns Unmerged; enrichment runs in background thread.
    let rx = branch::spawn_remote_enricher(work_dir.clone(), "main".to_string(), remotes.clone());
    let index_map: std::collections::HashMap<String, usize> = remotes
        .iter()
        .enumerate()
        .map(|(i, b)| (b.full_ref.clone(), i))
        .collect();
    for result in rx {
        if let Some(&idx) = index_map.get(&result.full_ref) {
            remotes[idx].merge_status = result.merge_status;
        }
    }

    let merged = remotes
        .iter()
        .find(|r| r.short_name == "feature-to-merge")
        .unwrap();
    assert_eq!(
        merged.merge_status,
        MergeStatus::Merged,
        "feature-to-merge should be detected as Merged on remote"
    );
}

#[test]
fn test_remote_branch_unmerged_detection() {
    let (_tmpdir, work_dir, repo) = setup_remote_test_repo();

    // Create and push an unmerged feature branch
    run_git(&work_dir, &["checkout", "-b", "feature-unmerged"]);
    let f = work_dir.join("unmerged.txt");
    std::fs::write(&f, "unmerged content\n").unwrap();
    run_git(&work_dir, &["add", "unmerged.txt"]);
    run_git(&work_dir, &["commit", "-m", "Unmerged feature"]);
    run_git(&work_dir, &["push", "-u", "origin", "feature-unmerged"]);
    run_git(&work_dir, &["checkout", "main"]);

    let remotes = branch::list_remote_branches_phase1(&repo, "main")
        .expect("list_remote_branches_phase1 failed");

    // Phase-1 sets non-base branches to Pending; squash check resolves to Unmerged/SquashMerged.
    let unmerged = remotes
        .iter()
        .find(|r| r.short_name == "feature-unmerged")
        .unwrap();
    assert_eq!(
        unmerged.merge_status,
        MergeStatus::Pending,
        "feature-unmerged should be Pending after phase-1 (squash check not yet run)"
    );
}

#[test]
fn test_remote_branch_skips_head() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    // Set up origin/HEAD (some repos have this)
    run_git(&work_dir, &["remote", "set-head", "origin", "main"]);

    let repo = git2::Repository::open(&work_dir).unwrap();
    let remotes = branch::list_remote_branches_phase1(&repo, "main")
        .expect("list_remote_branches_phase1 failed");

    // Should not include any entry with short_name "HEAD"
    let head_entries: Vec<_> = remotes.iter().filter(|r| r.short_name == "HEAD").collect();
    assert!(
        head_entries.is_empty(),
        "origin/HEAD pseudo-ref should be filtered out"
    );
}

#[test]
fn test_checkout_remote_branch() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    // Create and push a branch, then delete local
    run_git(&work_dir, &["checkout", "-b", "remote-checkout-test"]);
    let f = work_dir.join("checkout-test.txt");
    std::fs::write(&f, "content\n").unwrap();
    run_git(&work_dir, &["add", "checkout-test.txt"]);
    run_git(&work_dir, &["commit", "-m", "Checkout test commit"]);
    run_git(&work_dir, &["push", "-u", "origin", "remote-checkout-test"]);
    run_git(&work_dir, &["checkout", "main"]);
    run_git(&work_dir, &["branch", "-D", "remote-checkout-test"]);

    // Now checkout from remote
    let result = operations::checkout_remote_branch(&work_dir, "origin", "remote-checkout-test");
    assert!(
        result.success,
        "checkout_remote_branch should succeed: {}",
        result.message
    );

    // Verify we're on the new local branch
    let repo = git2::Repository::open(&work_dir).unwrap();
    let head = repo.head().unwrap();
    assert_eq!(head.shorthand().unwrap(), "remote-checkout-test");

    // Verify the file from the remote branch exists
    assert!(
        work_dir.join("checkout-test.txt").exists(),
        "checked-out file should exist"
    );
}

#[test]
fn test_checkout_remote_branch_already_exists() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    // Create and push a branch, keep local copy
    run_git(&work_dir, &["checkout", "-b", "already-local"]);
    let f = work_dir.join("already.txt");
    std::fs::write(&f, "content\n").unwrap();
    run_git(&work_dir, &["add", "already.txt"]);
    run_git(&work_dir, &["commit", "-m", "Already local"]);
    run_git(&work_dir, &["push", "-u", "origin", "already-local"]);
    run_git(&work_dir, &["checkout", "main"]);

    // Trying to checkout remote when local already exists should fail
    let result = operations::checkout_remote_branch(&work_dir, "origin", "already-local");
    assert!(
        !result.success,
        "should fail when local branch already exists"
    );
}

#[test]
fn test_fetch_sync() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    // fetch_sync should succeed on a valid repo with a remote
    let success = operations::fetch_sync(&work_dir);
    assert!(success, "fetch_sync should succeed");
}

#[test]
fn test_remote_branches_sorted_by_date() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    // Create branches with different commit times (sequential commits)
    run_git(&work_dir, &["checkout", "-b", "older-branch"]);
    let f1 = work_dir.join("older.txt");
    std::fs::write(&f1, "older\n").unwrap();
    run_git(&work_dir, &["add", "older.txt"]);
    run_git(&work_dir, &["commit", "-m", "Older commit"]);
    run_git(&work_dir, &["push", "-u", "origin", "older-branch"]);

    run_git(&work_dir, &["checkout", "main"]);
    run_git(&work_dir, &["checkout", "-b", "newer-branch"]);
    let f2 = work_dir.join("newer.txt");
    std::fs::write(&f2, "newer\n").unwrap();
    run_git(&work_dir, &["add", "newer.txt"]);
    run_git(&work_dir, &["commit", "-m", "Newer commit"]);
    run_git(&work_dir, &["push", "-u", "origin", "newer-branch"]);
    run_git(&work_dir, &["checkout", "main"]);

    let repo = git2::Repository::open(&work_dir).unwrap();
    let remotes = branch::list_remote_branches_phase1(&repo, "main")
        .expect("list_remote_branches_phase1 failed");

    // Find positions — newer should come before older (sorted newest-first)
    let newer_pos = remotes
        .iter()
        .position(|r| r.short_name == "newer-branch")
        .unwrap();
    let older_pos = remotes
        .iter()
        .position(|r| r.short_name == "older-branch")
        .unwrap();
    assert!(
        newer_pos < older_pos,
        "newer-branch (pos {}) should come before older-branch (pos {}) in date-descending sort",
        newer_pos,
        older_pos
    );
}

#[test]
fn test_remote_branch_squash_merge_detection() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    // Create a feature branch with unique content
    run_git(&work_dir, &["checkout", "-b", "squash-feature"]);
    let f = work_dir.join("squash-feature.txt");
    std::fs::write(&f, "squash feature content\n").unwrap();
    run_git(&work_dir, &["add", "squash-feature.txt"]);
    run_git(&work_dir, &["commit", "-m", "Squash feature commit"]);
    run_git(&work_dir, &["push", "-u", "origin", "squash-feature"]);

    // Squash-merge into main (without a merge commit)
    run_git(&work_dir, &["checkout", "main"]);
    run_git(&work_dir, &["merge", "--squash", "squash-feature"]);
    run_git(&work_dir, &["commit", "-m", "Squash merge squash-feature"]);
    run_git(&work_dir, &["push", "origin", "main"]);

    // Reload repo after push
    let repo = git2::Repository::open(&work_dir).unwrap();
    let remotes = branch::list_remote_branches_phase1(&repo, "main")
        .expect("list_remote_branches_phase1 failed");

    // Build candidates for squash checker: (full_ref, commit_hash) for pending non-base branches
    let candidates: Vec<(String, String)> = remotes
        .iter()
        .filter(|b| b.merge_status == MergeStatus::Pending && !b.is_base)
        .filter_map(|b| {
            let refname = format!("refs/remotes/{}", b.full_ref);
            repo.find_reference(&refname)
                .ok()
                .and_then(|r| r.peel_to_commit().ok())
                .map(|c| (b.full_ref.clone(), c.id().to_string()))
        })
        .collect();

    let cache = git_branch_manager::git::cache::BranchCache::load(&work_dir);
    let rx = squash_loader::spawn_squash_checker(
        work_dir.clone(),
        "main".to_string(),
        candidates,
        cache,
    );

    let index_map: std::collections::HashMap<String, usize> = remotes
        .iter()
        .enumerate()
        .map(|(i, b)| (b.full_ref.clone(), i))
        .collect();

    let mut remotes = remotes;
    for result in rx {
        if let Some(&idx) = index_map.get(&result.branch_name) {
            remotes[idx].merge_status = if result.is_squash_merged {
                MergeStatus::SquashMerged
            } else {
                MergeStatus::Unmerged
            };
        }
    }

    let squashed = remotes
        .iter()
        .find(|r| r.short_name == "squash-feature")
        .unwrap();
    assert_eq!(
        squashed.merge_status,
        MergeStatus::SquashMerged,
        "squash-feature should be detected as SquashMerged on remote"
    );
}

// ---------------------------------------------------------------------------
// Working tree status detection
// ---------------------------------------------------------------------------

#[test]
fn test_wt_status_clean() {
    let (_tmpdir, repo) = setup_test_repo();
    let s = status::detect_working_tree_status(&repo);
    assert!(s.is_clean(), "fresh repo should be clean");
}

#[test]
fn test_wt_status_staged_only() {
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Add a new file to the index without committing
    std::fs::write(dir.join("new.txt"), "content\n").unwrap();
    run_git(dir, &["add", "new.txt"]);

    let s = status::detect_working_tree_status(&repo);
    assert!(s.has_staged, "should detect staged file");
    assert!(!s.has_unstaged, "should not detect unstaged changes");
    assert!(!s.has_untracked, "should not detect untracked files");
}

#[test]
fn test_wt_status_unstaged_only() {
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Modify a tracked file without staging
    std::fs::write(dir.join("README.md"), "# Modified\n").unwrap();

    let s = status::detect_working_tree_status(&repo);
    assert!(!s.has_staged, "should not detect staged changes");
    assert!(s.has_unstaged, "should detect unstaged modification");
    assert!(!s.has_untracked, "should not detect untracked files");
}

#[test]
fn test_wt_status_untracked_only() {
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Create a file that is not tracked by git
    std::fs::write(dir.join("untracked.txt"), "not tracked\n").unwrap();

    let s = status::detect_working_tree_status(&repo);
    assert!(!s.has_staged, "should not detect staged changes");
    assert!(!s.has_unstaged, "should not detect unstaged changes");
    assert!(s.has_untracked, "should detect untracked file");
}

#[test]
fn test_wt_status_all_three() {
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Staged: add a new file to index
    std::fs::write(dir.join("staged.txt"), "staged\n").unwrap();
    run_git(dir, &["add", "staged.txt"]);

    // Unstaged: modify a tracked file without staging
    std::fs::write(dir.join("README.md"), "# Modified\n").unwrap();

    // Untracked: a new file not added to index
    std::fs::write(dir.join("untracked.txt"), "not tracked\n").unwrap();

    let s = status::detect_working_tree_status(&repo);
    assert!(s.has_staged, "should detect staged file");
    assert!(s.has_unstaged, "should detect unstaged modification");
    assert!(s.has_untracked, "should detect untracked file");
    assert!(!s.is_clean());
}

// ---------------------------------------------------------------------------
// Merge operation tests
// ---------------------------------------------------------------------------

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

    // And main should have a single new commit (not a merge commit)
    let log = std::process::Command::new("git")
        .args(["log", "--oneline", "-3"])
        .current_dir(dir)
        .output()
        .unwrap();
    let log_str = String::from_utf8_lossy(&log.stdout);
    assert!(
        log_str.contains("Squash merge feature-squash"),
        "should have squash merge commit"
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

    // Verify merge was aborted: repo must not be in mid-merge state
    let status = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(dir)
        .output()
        .unwrap();
    let status_str = String::from_utf8_lossy(&status.stdout);
    assert!(
        !status_str.contains("UU"),
        "merge should have been aborted, no unresolved conflicts"
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
    let (ahead, _behind) = repo.graph_ahead_behind(feature_oid, main_oid).unwrap();
    assert_eq!(
        ahead, 1,
        "feature should be 1 commit ahead of main after rebase"
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
        results[0].message.contains("Rebase conflicts") || results[0].message.contains("conflict"),
        "message should mention conflict: {}",
        results[0].message
    );

    // Rebase should be aborted: no ongoing rebase state
    let rebase_head = dir.join(".git").join("REBASE_HEAD");
    assert!(
        !rebase_head.exists(),
        "REBASE_HEAD should not exist after abort"
    );
}

// ---------------------------------------------------------------------------
// Remote operations: push, pull, fast-forward, fetch-prune
// ---------------------------------------------------------------------------

#[test]
fn test_push_branch() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    // Create a local branch with a commit
    run_git(&work_dir, &["checkout", "-b", "push-test"]);
    std::fs::write(work_dir.join("push.txt"), "push content\n").unwrap();
    run_git(&work_dir, &["add", "push.txt"]);
    run_git(&work_dir, &["commit", "-m", "Push test commit"]);
    run_git(&work_dir, &["checkout", "main"]);

    let result = operations::push_branch(&work_dir, "push-test", &AtomicBool::new(false));
    assert!(
        result.success,
        "push_branch should succeed: {}",
        result.message
    );

    // Fetch and verify remote has the branch
    run_git(&work_dir, &["fetch", "origin"]);
    let repo = git2::Repository::open(&work_dir).unwrap();
    assert!(
        repo.find_branch("origin/push-test", git2::BranchType::Remote)
            .is_ok(),
        "origin/push-test should exist after push"
    );
}

#[test]
fn test_pull_branch_current() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    // Simulate another committer pushing to origin/main via a second clone
    let tmpdir2 = tempfile::tempdir().unwrap();
    let clone2 = tmpdir2.path().join("clone2");
    let remote_dir = work_dir.parent().unwrap().join("remote.git");
    run_git(
        tmpdir2.path(),
        &["clone", remote_dir.to_str().unwrap(), "clone2"],
    );
    run_git(&clone2, &["config", "user.name", "Other User"]);
    run_git(&clone2, &["config", "user.email", "other@example.com"]);
    std::fs::write(clone2.join("other.txt"), "other commit\n").unwrap();
    run_git(&clone2, &["add", "other.txt"]);
    run_git(&clone2, &["commit", "-m", "Other commit on main"]);
    run_git(&clone2, &["push", "origin", "main"]);

    // Pull in work_dir (main is current branch)
    let result = operations::pull_branch(&work_dir, "main", true, &AtomicBool::new(false));
    assert!(
        result.success,
        "pull_branch (current) should succeed: {}",
        result.message
    );

    // Verify the new file exists locally
    assert!(
        work_dir.join("other.txt").exists(),
        "pulled file should exist after pull"
    );
}

#[test]
fn test_pull_branch_non_current() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    // Create and push a feature branch
    run_git(&work_dir, &["checkout", "-b", "pull-non-current"]);
    std::fs::write(work_dir.join("pull-feature.txt"), "feature\n").unwrap();
    run_git(&work_dir, &["add", "pull-feature.txt"]);
    run_git(&work_dir, &["commit", "-m", "Feature commit"]);
    run_git(&work_dir, &["push", "-u", "origin", "pull-non-current"]);

    // Push another commit from a second clone
    let tmpdir2 = tempfile::tempdir().unwrap();
    let clone2 = tmpdir2.path().join("clone2");
    let remote_dir = work_dir.parent().unwrap().join("remote.git");
    run_git(
        tmpdir2.path(),
        &["clone", remote_dir.to_str().unwrap(), "clone2"],
    );
    run_git(&clone2, &["config", "user.name", "Other User"]);
    run_git(&clone2, &["config", "user.email", "other@example.com"]);
    run_git(&clone2, &["checkout", "pull-non-current"]);
    std::fs::write(clone2.join("extra.txt"), "extra\n").unwrap();
    run_git(&clone2, &["add", "extra.txt"]);
    run_git(&clone2, &["commit", "-m", "Extra commit"]);
    run_git(&clone2, &["push", "origin", "pull-non-current"]);

    // Fetch so work_dir knows about the remote update, then switch to main
    run_git(&work_dir, &["fetch", "origin"]);
    run_git(&work_dir, &["checkout", "main"]);

    let result = operations::pull_branch(
        &work_dir,
        "pull-non-current",
        false,
        &AtomicBool::new(false),
    );
    assert!(
        result.success,
        "pull_branch (non-current) should succeed: {}",
        result.message
    );

    // The local branch tip should now be at "Extra commit"
    let repo = git2::Repository::open(&work_dir).unwrap();
    let branch_oid = repo
        .find_branch("pull-non-current", git2::BranchType::Local)
        .unwrap()
        .get()
        .peel_to_commit()
        .unwrap()
        .id();
    let commit = repo.find_commit(branch_oid).unwrap();
    assert_eq!(
        commit.summary().unwrap_or(""),
        "Extra commit",
        "local branch should be updated to latest remote commit"
    );
}

#[test]
fn test_fast_forward_branch() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    // Push a feature branch
    run_git(&work_dir, &["checkout", "-b", "ff-branch"]);
    std::fs::write(work_dir.join("ff.txt"), "ff content\n").unwrap();
    run_git(&work_dir, &["add", "ff.txt"]);
    run_git(&work_dir, &["commit", "-m", "FF commit"]);
    run_git(&work_dir, &["push", "-u", "origin", "ff-branch"]);

    // Advance the remote via a second clone
    let tmpdir2 = tempfile::tempdir().unwrap();
    let clone2 = tmpdir2.path().join("clone2");
    let remote_dir = work_dir.parent().unwrap().join("remote.git");
    run_git(
        tmpdir2.path(),
        &["clone", remote_dir.to_str().unwrap(), "clone2"],
    );
    run_git(&clone2, &["config", "user.name", "Other User"]);
    run_git(&clone2, &["config", "user.email", "other@example.com"]);
    run_git(&clone2, &["checkout", "ff-branch"]);
    std::fs::write(clone2.join("ff2.txt"), "ff2 content\n").unwrap();
    run_git(&clone2, &["add", "ff2.txt"]);
    run_git(&clone2, &["commit", "-m", "FF commit 2"]);
    run_git(&clone2, &["push", "origin", "ff-branch"]);

    // Go back to main (ff-branch is not current)
    run_git(&work_dir, &["checkout", "main"]);

    // fast_forward fetches origin/ff-branch:ff-branch
    let result = operations::fast_forward(&work_dir, "ff-branch", &AtomicBool::new(false));
    assert!(
        result.success,
        "fast_forward should succeed: {}",
        result.message
    );

    // Verify the local branch was advanced
    let repo = git2::Repository::open(&work_dir).unwrap();
    let commit = repo
        .find_branch("ff-branch", git2::BranchType::Local)
        .unwrap()
        .get()
        .peel_to_commit()
        .unwrap();
    assert_eq!(
        commit.summary().unwrap_or(""),
        "FF commit 2",
        "local branch should be at the latest remote commit after fast-forward"
    );
}

#[test]
fn test_fetch_prune_removes_stale_remote() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();
    let remote_dir = work_dir.parent().unwrap().join("remote.git");

    // Push a branch so work_dir has a remote-tracking ref for it
    run_git(&work_dir, &["checkout", "-b", "prune-me"]);
    std::fs::write(work_dir.join("prune.txt"), "prune\n").unwrap();
    run_git(&work_dir, &["add", "prune.txt"]);
    run_git(&work_dir, &["commit", "-m", "Prune commit"]);
    run_git(&work_dir, &["push", "-u", "origin", "prune-me"]);
    run_git(&work_dir, &["checkout", "main"]);

    // Delete the branch directly on the bare remote (simulating someone else deleting it).
    // This leaves the local origin/prune-me tracking ref stale — fetch --prune should clean it up.
    run_git(&remote_dir, &["branch", "-D", "prune-me"]);

    // Confirm origin/prune-me is still in local tracking refs before pruning
    let repo = git2::Repository::open(&work_dir).unwrap();
    assert!(
        repo.find_branch("origin/prune-me", git2::BranchType::Remote)
            .is_ok(),
        "origin/prune-me should still exist in local refs before prune"
    );

    let result = operations::fetch_prune(&work_dir, &AtomicBool::new(false));
    assert!(
        result.success,
        "fetch_prune should succeed: {}",
        result.message
    );

    // After prune, stale tracking ref should be gone
    let repo2 = git2::Repository::open(&work_dir).unwrap();
    assert!(
        repo2
            .find_branch("origin/prune-me", git2::BranchType::Remote)
            .is_err(),
        "origin/prune-me should be removed after fetch --prune"
    );
}

// ---------------------------------------------------------------------------
// Remote batch delete tests
// ---------------------------------------------------------------------------

#[test]
fn test_delete_remotes_batch_success() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    // Create and push two branches
    for branch_name in &["batch-del-1", "batch-del-2"] {
        run_git(&work_dir, &["checkout", "-b", branch_name]);
        std::fs::write(work_dir.join(format!("{}.txt", branch_name)), "content\n").unwrap();
        run_git(&work_dir, &["add", &format!("{}.txt", branch_name)]);
        run_git(
            &work_dir,
            &["commit", "-m", &format!("Add {}", branch_name)],
        );
        run_git(&work_dir, &["push", "-u", "origin", branch_name]);
        run_git(&work_dir, &["checkout", "main"]);
    }

    let names: Vec<String> = vec!["batch-del-1".to_string(), "batch-del-2".to_string()];
    let results = operations::delete_remotes_batch(&work_dir, &names, &AtomicBool::new(false));

    assert_eq!(results.len(), 2);
    assert!(
        results[0].success,
        "first remote delete should succeed: {}",
        results[0].message
    );
    assert!(
        results[1].success,
        "second remote delete should succeed: {}",
        results[1].message
    );

    // Fetch to sync tracking refs, then verify branches are gone
    run_git(&work_dir, &["fetch", "--prune"]);
    let repo = git2::Repository::open(&work_dir).unwrap();
    assert!(
        repo.find_branch("origin/batch-del-1", git2::BranchType::Remote)
            .is_err(),
        "origin/batch-del-1 should be deleted"
    );
    assert!(
        repo.find_branch("origin/batch-del-2", git2::BranchType::Remote)
            .is_err(),
        "origin/batch-del-2 should be deleted"
    );
}

#[test]
fn test_delete_remotes_batch_empty() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    // Empty input should return empty results immediately (no git command run)
    let results = operations::delete_remotes_batch(&work_dir, &[], &AtomicBool::new(false));
    assert!(
        results.is_empty(),
        "empty input should produce empty results"
    );
}

// ---------------------------------------------------------------------------
// Worktree operation tests
// ---------------------------------------------------------------------------

#[test]
fn test_create_worktree_simple() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["branch", "wt-feature"]);

    let result = operations::create_worktree(dir, "wt-feature");
    assert!(
        result.success,
        "create_worktree should succeed: {}",
        result.message
    );

    let wt_path = dir.join(".worktrees").join("wt-feature");
    assert!(wt_path.exists(), ".worktrees/wt-feature should be created");
    // A worktree directory contains a .git file (not a directory like the main repo)
    assert!(
        wt_path.join(".git").exists(),
        "worktree directory should be a valid git working tree"
    );
}

#[test]
fn test_create_worktree_sanitizes_slash() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Branch names with slashes (e.g. "feature/foo") must be sanitized to "feature-foo"
    run_git(dir, &["branch", "feature/slash-test"]);

    let result = operations::create_worktree(dir, "feature/slash-test");
    assert!(
        result.success,
        "create_worktree with slash should succeed: {}",
        result.message
    );

    let wt_path = dir.join(".worktrees").join("feature-slash-test");
    assert!(
        wt_path.exists(),
        ".worktrees/feature-slash-test should be created (slash → dash)"
    );
}

#[test]
fn test_remove_worktree_clean() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["branch", "wt-remove"]);
    run_git(
        dir,
        &["worktree", "add", ".worktrees/wt-remove", "wt-remove"],
    );

    let wt_path = dir.join(".worktrees").join("wt-remove");
    assert!(wt_path.exists(), "worktree should exist before removal");

    let result = operations::remove_worktree(dir, &wt_path);
    assert!(
        result.success,
        "remove_worktree should succeed on clean worktree: {}",
        result.message
    );
    assert!(!wt_path.exists(), "worktree directory should be removed");
}

#[test]
fn test_force_remove_worktree_dirty() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["branch", "wt-dirty"]);
    run_git(dir, &["worktree", "add", ".worktrees/wt-dirty", "wt-dirty"]);

    let wt_path = dir.join(".worktrees").join("wt-dirty");

    // Create an uncommitted change in the worktree — regular remove should fail
    std::fs::write(wt_path.join("dirty.txt"), "uncommitted change\n").unwrap();

    let result = operations::remove_worktree(dir, &wt_path);
    assert!(
        !result.success,
        "remove_worktree should fail on dirty worktree"
    );

    // Force remove should succeed regardless
    let result = operations::force_remove_worktree(dir, &wt_path);
    assert!(
        result.success,
        "force_remove_worktree should succeed even when dirty: {}",
        result.message
    );
    assert!(
        !wt_path.exists(),
        "worktree directory should be removed after force-remove"
    );
}

// ---------------------------------------------------------------------------
// Worktree listing and enrichment tests
// ---------------------------------------------------------------------------

#[test]
fn test_list_worktrees_main_only() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    let worktrees = worktree::list_worktrees(dir);
    assert_eq!(worktrees.len(), 1, "should have exactly 1 worktree (main)");

    let main_wt = &worktrees[0];
    assert!(
        main_wt.is_main,
        "first worktree should be the main worktree"
    );
    assert_eq!(
        main_wt.branch.as_deref(),
        Some("main"),
        "main worktree should be on 'main'"
    );
    assert_eq!(
        main_wt.commit_hash.len(),
        7,
        "commit_hash should be 7 chars"
    );
}

#[test]
fn test_list_worktrees_with_additional() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["branch", "wt-list-test"]);
    run_git(
        dir,
        &["worktree", "add", ".worktrees/wt-list-test", "wt-list-test"],
    );

    let worktrees = worktree::list_worktrees(dir);
    assert_eq!(worktrees.len(), 2, "should have 2 worktrees");

    let main_wt = worktrees
        .iter()
        .find(|w| w.is_main)
        .expect("should have a main worktree");
    assert_eq!(main_wt.branch.as_deref(), Some("main"));

    let extra_wt = worktrees
        .iter()
        .find(|w| !w.is_main)
        .expect("should have a non-main worktree");
    assert_eq!(extra_wt.branch.as_deref(), Some("wt-list-test"));

    // Clean up before tmpdir drops
    run_git(dir, &["worktree", "remove", ".worktrees/wt-list-test"]);
}

#[test]
fn test_enrich_worktrees_clean() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    let worktrees = worktree::list_worktrees(dir);
    assert_eq!(worktrees.len(), 1);

    let rx = worktree::enrich_worktrees(worktrees);
    let results: Vec<_> = rx.iter().collect();

    assert_eq!(results.len(), 1, "should receive one enrichment result");
    assert_eq!(results[0].index, 0);
    assert!(
        results[0].wt_status.is_clean(),
        "clean repo worktree should report clean status"
    );
}

// ---------------------------------------------------------------------------
// Additional rewrite-specific tests (tag operations, phase1, direct merge detection)
// ---------------------------------------------------------------------------

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
fn test_create_and_list_worktree() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();
    run_git(dir, &["checkout", "-b", "feature/wt-test"]);
    run_git(dir, &["checkout", "main"]);

    let result = operations::create_worktree(dir, "feature/wt-test");
    assert!(result.success);

    let worktrees = worktree::list_worktrees(dir);
    assert_eq!(worktrees.len(), 2);
    let wt = worktrees.iter().find(|w| !w.is_main).unwrap();
    assert_eq!(wt.branch.as_deref(), Some("feature/wt-test"));
}

#[test]
fn test_list_tags_empty() {
    let (_tmpdir, repo) = setup_test_repo();
    let result = tags::list_tags(&repo);
    assert!(result.is_empty());
}

#[test]
fn test_list_tags_lightweight() {
    let (tmpdir, repo) = setup_test_repo();
    run_git(tmpdir.path(), &["tag", "v0.1"]);
    let result = tags::list_tags(&repo);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].name, "v0.1");
    assert!(!result[0].is_annotated);
    assert!(result[0].message.is_none());
}

#[test]
fn test_list_tags_with_annotated() {
    let (tmpdir, repo) = setup_test_repo();
    run_git(tmpdir.path(), &["tag", "-a", "v1.0", "-m", "Release 1.0"]);
    let result = tags::list_tags(&repo);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].name, "v1.0");
    assert!(result[0].is_annotated);
    assert_eq!(result[0].message.as_deref(), Some("Release 1.0"));
}

#[test]
fn test_delete_tag() {
    let (tmpdir, repo) = setup_test_repo();
    run_git(tmpdir.path(), &["tag", "v1.0"]);
    let result = tags::delete_tag(&repo, "v1.0");
    assert!(result.success);
    let remaining = tags::list_tags(&repo);
    assert!(remaining.is_empty());
}

#[test]
fn test_delete_tags_batch() {
    let (tmpdir, repo) = setup_test_repo();
    run_git(tmpdir.path(), &["tag", "v1.0"]);
    run_git(tmpdir.path(), &["tag", "v2.0"]);
    let names = vec!["v1.0".to_string(), "v2.0".to_string()];
    let results = tags::delete_tags_batch(&repo, &names);
    assert!(results.iter().all(|r| r.success));
    let remaining = tags::list_tags(&repo);
    assert!(remaining.is_empty());
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
        None
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
        None
    ));
}

#[test]
fn dump_branches_basic() {
    let (tmp, _repo) = setup_test_repo();
    let out = Command::new(env!("CARGO_BIN_EXE_git-branch-manager"))
        .args(["--repo", tmp.path().to_str().unwrap(), "--branches", "--color=never"])
        .output()
        .expect("failed to run binary");
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let s = String::from_utf8(out.stdout).unwrap();
    assert!(s.starts_with("base: main"), "got: {s:?}");
    assert!(s.contains("Branch"), "header missing: {s:?}");
    assert!(s.contains("main"), "base branch row missing: {s:?}");
    assert!(!s.contains('\x1b'), "--color=never must be plain: {s:?}");
}

#[test]
fn dump_rejects_two_view_flags() {
    let (tmp, _repo) = setup_test_repo();
    let out = Command::new(env!("CARGO_BIN_EXE_git-branch-manager"))
        .args(["--repo", tmp.path().to_str().unwrap(), "--branches", "--tags"])
        .output()
        .expect("failed to run binary");
    assert!(!out.status.success(), "two view flags must be a usage error");
}

#[test]
fn dump_list_is_branches_alias() {
    let (tmp, _repo) = setup_test_repo();
    let out = Command::new(env!("CARGO_BIN_EXE_git-branch-manager"))
        .args(["--repo", tmp.path().to_str().unwrap(), "--list", "--color=never"])
        .output()
        .expect("failed to run binary");
    assert!(out.status.success());
    let s = String::from_utf8(out.stdout).unwrap();
    assert!(s.starts_with("base: main"), "got: {s:?}");
    let err = String::from_utf8(out.stderr).unwrap();
    assert!(err.contains("deprecated"), "expected --list deprecation note on stderr, got: {err:?}");
}

#[test]
fn dump_remotes_basic() {
    let (tmp, _repo) = setup_test_repo();
    // A bare "remote" with one branch, fetched into the test repo.
    let remote = tempfile::tempdir().unwrap();
    run_git(remote.path(), &["init", "--bare", "-b", "main"]);
    run_git(tmp.path(), &["remote", "add", "origin", remote.path().to_str().unwrap()]);
    run_git(tmp.path(), &["push", "origin", "main"]);
    run_git(tmp.path(), &["fetch", "origin"]);

    let out = Command::new(env!("CARGO_BIN_EXE_git-branch-manager"))
        .args(["--repo", tmp.path().to_str().unwrap(), "--remotes", "--color=never"])
        .output()
        .expect("failed to run binary");
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let s = String::from_utf8(out.stdout).unwrap();
    assert!(s.contains("Name"), "header missing: {s:?}");
    assert!(s.contains("origin/main"), "remote row missing: {s:?}");
}
