use std::sync::atomic::AtomicBool;

use git_branch_manager::git::operations;

use crate::common::*;

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
        commit.summary().ok().flatten().unwrap_or(""),
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
        commit.summary().ok().flatten().unwrap_or(""),
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
