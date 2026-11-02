use std::path::Path;
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;

use git_branch_manager::git::operations;
use git_branch_manager::types::{BranchAction, ProgressUpdate};

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

// ---------------------------------------------------------------------------
// Shared helpers for the remote operation tests below
// ---------------------------------------------------------------------------

/// Create `branch` from the current HEAD, commit `file` with `content`, push it to `remote`, and return to main.
fn push_branch_to_remote(
    work_dir: &Path,
    remote: &str,
    branch: &str,
    file: &str,
    content: &str,
    subject: &str,
) {
    run_git(work_dir, &["checkout", "-b", branch]);
    std::fs::write(work_dir.join(file), content).unwrap();
    run_git(work_dir, &["add", file]);
    run_git(work_dir, &["commit", "-m", subject]);
    run_git(work_dir, &["push", "-u", remote, branch]);
    run_git(work_dir, &["checkout", "main"]);
}

/// Create `branch` from the current HEAD, push it to origin, and return to main.
fn push_branch_to_origin(work_dir: &Path, branch: &str, file: &str, content: &str, subject: &str) {
    push_branch_to_remote(work_dir, "origin", branch, file, content, subject);
}

/// Whether `name` resolves as a ref in `dir`. Panics only if git cannot be run.
fn ref_exists(dir: &Path, name: &str) -> bool {
    Command::new("git")
        .args(["rev-parse", "-q", "--verify", name])
        .current_dir(dir)
        .output()
        .expect("failed to run git rev-parse")
        .status
        .success()
}

// ---------------------------------------------------------------------------
// Checkout / fetch / pull
// ---------------------------------------------------------------------------

#[test]
fn test_checkout_remote_branch_missing_repo_reports_spawn_error() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();
    let missing = work_dir.join("missing-repo");
    assert!(!missing.exists(), "missing repository precondition");

    let result = operations::checkout_remote_branch(&missing, "origin", "feat");
    assert!(!result.success);
    assert_eq!(result.branch_name, "feat");
    assert_eq!(result.action, BranchAction::CheckoutRemote);
    assert!(!result.message.is_empty(), "spawn error should be reported");
}

#[test]
fn test_fetch_remote_success_reports_fetched() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    let results = operations::fetch_remote(&work_dir, "origin", &AtomicBool::new(false));
    assert_eq!(results.len(), 1);
    let result = &results[0];
    assert!(result.success, "fetch should succeed: {}", result.message);
    assert_eq!(result.message, "Fetched origin");
    assert_eq!(result.branch_name, "origin");
    assert_eq!(result.action, BranchAction::FetchRemote);
}

#[test]
fn test_delete_remotes_batch_for_non_origin_remote_only_updates_selected_remote() {
    let (tmpdir, work_dir, _repo) = setup_remote_test_repo();
    let origin_dir = work_dir.parent().unwrap().join("remote.git");
    let secondary_dir = tmpdir.path().join("secondary.git");
    run_git(
        tmpdir.path(),
        &[
            "init",
            "--bare",
            "-b",
            "main",
            secondary_dir.to_str().unwrap(),
        ],
    );
    run_git(
        &work_dir,
        &[
            "remote",
            "add",
            "secondary",
            secondary_dir.to_str().unwrap(),
        ],
    );
    run_git(&work_dir, &["push", "secondary", "main"]);

    push_branch_to_remote(
        &work_dir,
        "secondary",
        "secondary-only",
        "secondary-only.txt",
        "secondary\n",
        "Add secondary-only",
    );
    run_git(&work_dir, &["push", "origin", "secondary-only"]);

    let ref_name = "refs/heads/secondary-only";
    assert!(
        ref_exists(&secondary_dir, ref_name),
        "secondary branch setup"
    );
    assert!(ref_exists(&origin_dir, ref_name), "origin branch setup");

    let names = vec!["secondary-only".to_string()];
    let results = operations::delete_remotes_batch_for_remote(
        &work_dir,
        "secondary",
        &names,
        &AtomicBool::new(false),
    );

    assert_eq!(results.len(), 1);
    assert!(
        results[0].success,
        "secondary delete should succeed: {}",
        results[0].message
    );
    assert!(
        !ref_exists(&secondary_dir, ref_name),
        "the selected remote branch should be deleted"
    );
    assert!(
        ref_exists(&origin_dir, ref_name),
        "deleting from secondary must leave origin unchanged"
    );
}

#[test]
fn test_fetch_remote_unknown_remote_fails() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    let results = operations::fetch_remote(&work_dir, "no-such-remote", &AtomicBool::new(false));
    assert_eq!(results.len(), 1);
    let result = &results[0];
    assert!(!result.success);
    assert!(!result.message.is_empty(), "stderr should be reported");
}

#[test]
fn test_fetch_remote_precancelled_reports_cancelled() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    let results = operations::fetch_remote(&work_dir, "origin", &AtomicBool::new(true));
    assert_eq!(results.len(), 1);
    assert!(!results[0].success);
    assert_eq!(results[0].message, "Cancelled");
}

#[test]
fn test_fetch_remote_missing_repo_reports_spawn_error() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();
    let missing = work_dir.join("missing-repo");
    assert!(!missing.exists(), "missing repository precondition");

    let results = operations::fetch_remote(&missing, "origin", &AtomicBool::new(false));
    assert_eq!(results.len(), 1);
    assert!(!results[0].success);
    assert!(
        !results[0].message.is_empty(),
        "spawn error should be reported"
    );
}

#[test]
fn test_pull_remote_fast_forwards_non_current_branch() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    push_branch_to_origin(
        &work_dir,
        "pull-src",
        "pull.txt",
        "pull\n",
        "Pull source commit",
    );
    let tip = git_output(&work_dir, &["rev-parse", "pull-src"]);

    // Rewind the local branch so it is behind origin/pull-src
    run_git(&work_dir, &["branch", "-f", "pull-src", "main"]);

    let results = operations::pull_remote(&work_dir, "origin", "pull-src", &AtomicBool::new(false));
    assert_eq!(results.len(), 1);
    let result = &results[0];
    assert!(result.success, "pull should succeed: {}", result.message);
    assert_eq!(result.message, "Pulled origin/pull-src");
    assert_eq!(result.branch_name, "pull-src");
    assert_eq!(git_output(&work_dir, &["rev-parse", "pull-src"]), tip);
}

#[test]
fn test_pull_remote_missing_ref_fails() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    let results = operations::pull_remote(
        &work_dir,
        "origin",
        "no-such-branch",
        &AtomicBool::new(false),
    );
    assert_eq!(results.len(), 1);
    let result = &results[0];
    assert!(!result.success);
    assert_eq!(result.branch_name, "no-such-branch");
    assert!(!result.message.is_empty(), "stderr should be reported");
}

#[test]
fn test_pull_remote_precancelled_reports_cancelled() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    let results = operations::pull_remote(&work_dir, "origin", "main", &AtomicBool::new(true));
    assert_eq!(results.len(), 1);
    assert!(!results[0].success);
    assert_eq!(results[0].message, "Cancelled");
}

#[test]
fn test_pull_remote_missing_repo_reports_spawn_error() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();
    let missing = work_dir.join("missing-repo");
    assert!(!missing.exists(), "missing repository precondition");

    let results = operations::pull_remote(&missing, "origin", "main", &AtomicBool::new(false));
    assert_eq!(results.len(), 1);
    assert!(!results[0].success);
    assert!(
        !results[0].message.is_empty(),
        "spawn error should be reported"
    );
}

// ---------------------------------------------------------------------------
// Remote delete: batch, fallback, and progress paths
// ---------------------------------------------------------------------------

#[test]
fn test_delete_remotes_batch_precancelled_marks_all_cancelled() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();
    let remote_dir = work_dir.parent().unwrap().join("remote.git");
    push_branch_to_origin(&work_dir, "cancel-a", "cancel-a.txt", "a\n", "Add cancel-a");
    push_branch_to_origin(&work_dir, "cancel-b", "cancel-b.txt", "b\n", "Add cancel-b");

    let names = vec!["cancel-a".to_string(), "cancel-b".to_string()];
    let results = operations::delete_remotes_batch_for_remote(
        &work_dir,
        "origin",
        &names,
        &AtomicBool::new(true),
    );
    assert_eq!(results.len(), 2);
    for result in &results {
        assert!(!result.success);
        assert_eq!(result.message, "Cancelled");
    }
    for name in &names {
        assert!(
            !git_output(&remote_dir, &["branch", "--list", name]).is_empty(),
            "cancelled batch must leave remote branch {name} in place"
        );
    }
}

#[test]
fn test_delete_remotes_batch_mixed_ok_and_missing_reports_each_item() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();
    let remote_dir = work_dir.parent().unwrap().join("remote.git");
    push_branch_to_origin(&work_dir, "ok-a", "ok-a.txt", "ok\n", "Add ok-a");
    assert!(ref_exists(&remote_dir, "refs/heads/ok-a"));

    let names = vec!["ok-a".to_string(), "never-pushed".to_string()];
    let results = operations::delete_remotes_batch(&work_dir, &names, &AtomicBool::new(false));
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].branch_name, "ok-a");
    assert!(results[0].success, "ok-a delete: {}", results[0].message);
    assert_eq!(results[1].branch_name, "never-pushed");
    assert!(!results[1].success);
    assert!(!results[1].message.is_empty(), "stderr should be reported");
    assert!(
        !ref_exists(&remote_dir, "refs/heads/ok-a"),
        "the valid branch should be deleted from the bare remote"
    );
    assert!(
        !ref_exists(&remote_dir, "refs/heads/never-pushed"),
        "the nonexistent branch should remain absent"
    );
}

#[test]
fn test_delete_remotes_batch_missing_repo_reports_each_error() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();
    let missing = work_dir.join("missing-repo");
    assert!(!missing.exists(), "missing repository precondition");

    let names = vec!["a".to_string(), "b".to_string()];
    let results = operations::delete_remotes_batch(&missing, &names, &AtomicBool::new(false));
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].branch_name, "a");
    assert_eq!(results[1].branch_name, "b");
    for result in &results {
        assert!(!result.success);
        assert!(!result.message.is_empty(), "spawn error should be reported");
    }
}

#[test]
fn test_delete_remotes_with_progress_reports_each_item() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();
    push_branch_to_origin(&work_dir, "prog-a", "prog-a.txt", "a\n", "Add prog-a");
    push_branch_to_origin(&work_dir, "prog-b", "prog-b.txt", "b\n", "Add prog-b");

    let (tx, rx) = mpsc::channel::<ProgressUpdate>();
    let names = vec!["prog-a".to_string(), "prog-b".to_string()];
    let results =
        operations::delete_remotes_with_progress(&work_dir, &names, &tx, &AtomicBool::new(false));

    assert_eq!(results.len(), 2);
    assert!(results[0].success, "prog-a delete: {}", results[0].message);
    assert!(results[1].success, "prog-b delete: {}", results[1].message);

    let updates: Vec<ProgressUpdate> = rx.try_iter().collect();
    let tuples: Vec<(usize, usize, &str)> = updates
        .iter()
        .map(|u| (u.completed, u.total, u.current_item.as_str()))
        .collect();
    assert_eq!(tuples, vec![(0, 2, "prog-a"), (1, 2, "prog-b")]);
}

#[test]
fn test_delete_remotes_with_progress_mixed_results() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();
    push_branch_to_origin(&work_dir, "prog-ok", "prog-ok.txt", "ok\n", "Add prog-ok");

    let (tx, rx) = mpsc::channel::<ProgressUpdate>();
    let names = vec!["prog-ok".to_string(), "prog-missing".to_string()];
    let results = operations::delete_remotes_with_progress_for_remote(
        &work_dir,
        "origin",
        &names,
        &tx,
        &AtomicBool::new(false),
    );

    assert_eq!(results.len(), 2);
    assert!(results[0].success, "prog-ok delete: {}", results[0].message);
    assert!(!results[1].success);
    assert!(!results[1].message.is_empty(), "stderr should be reported");

    let updates: Vec<ProgressUpdate> = rx.try_iter().collect();
    assert_eq!(updates.len(), 2);
}

#[test]
fn test_delete_remotes_with_progress_precancelled_stops_first() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();
    let remote_dir = work_dir.parent().unwrap().join("remote.git");
    push_branch_to_origin(
        &work_dir,
        "prog-stop",
        "prog-stop.txt",
        "stop\n",
        "Add prog-stop",
    );

    let (tx, rx) = mpsc::channel::<ProgressUpdate>();
    let names = vec!["prog-stop".to_string()];
    let results = operations::delete_remotes_with_progress_for_remote(
        &work_dir,
        "origin",
        &names,
        &tx,
        &AtomicBool::new(true),
    );

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].message, "Cancelled");
    assert_eq!(rx.try_iter().count(), 0, "no progress before cancellation");
    assert!(
        !git_output(&remote_dir, &["branch", "--list", "prog-stop"]).is_empty(),
        "cancelled delete must leave the remote branch in place"
    );
}

// ---------------------------------------------------------------------------
// Merge / cherry-pick into current branch
// ---------------------------------------------------------------------------

#[test]
fn test_merge_remote_into_current_fast_forwards() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();
    push_branch_to_origin(&work_dir, "ff-src", "ff-src.txt", "ff\n", "Add ff-src");

    let results = operations::merge_remote_into_current(&work_dir, "origin/ff-src", "ff-src");
    assert_eq!(results.len(), 1);
    let result = &results[0];
    assert!(result.success, "merge should succeed: {}", result.message);
    assert_eq!(result.message, "Merged origin/ff-src into current");
    assert_eq!(result.branch_name, "ff-src");
    assert_eq!(
        git_output(&work_dir, &["rev-parse", "HEAD"]),
        git_output(&work_dir, &["rev-parse", "origin/ff-src"])
    );
}

#[test]
fn test_merge_remote_into_current_conflict_aborts() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();
    push_branch_to_origin(
        &work_dir,
        "conflict-src",
        "README.md",
        "branch side\n",
        "Branch side change",
    );
    std::fs::write(work_dir.join("README.md"), "main side\n").unwrap();
    run_git(&work_dir, &["add", "README.md"]);
    run_git(&work_dir, &["commit", "-m", "Main side change"]);

    let results =
        operations::merge_remote_into_current(&work_dir, "origin/conflict-src", "conflict-src");
    assert_eq!(results.len(), 1);
    let result = &results[0];
    assert!(!result.success);
    assert_eq!(result.message, "Merge conflict \u{2014} aborted");
    assert!(result.failure.is_none());

    assert_eq!(git_output(&work_dir, &["status", "--porcelain"]), "");
    assert!(
        !ref_exists(&work_dir, "MERGE_HEAD"),
        "MERGE_HEAD should be absent after abort"
    );
    assert_eq!(
        std::fs::read_to_string(work_dir.join("README.md")).unwrap(),
        "main side\n"
    );
}

#[test]
fn test_cherry_pick_remote_applies_commit() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();
    push_branch_to_origin(&work_dir, "cp-src", "cherry.txt", "cherry\n", "Add cherry");

    let results = operations::cherry_pick_remote(&work_dir, "origin/cp-src", "cp-src");
    assert_eq!(results.len(), 1);
    let result = &results[0];
    assert!(
        result.success,
        "cherry-pick should succeed: {}",
        result.message
    );
    assert_eq!(result.message, "Cherry-picked origin/cp-src");
    assert_eq!(result.branch_name, "cp-src");
    assert!(work_dir.join("cherry.txt").exists());
    assert_eq!(
        git_output(&work_dir, &["log", "-1", "--format=%s"]),
        "Add cherry"
    );
}

#[test]
fn test_cherry_pick_remote_conflict_aborts() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();
    push_branch_to_origin(
        &work_dir,
        "cp-conflict",
        "README.md",
        "branch side\n",
        "Branch side change",
    );
    std::fs::write(work_dir.join("README.md"), "main side\n").unwrap();
    run_git(&work_dir, &["add", "README.md"]);
    run_git(&work_dir, &["commit", "-m", "Main side change"]);

    let results = operations::cherry_pick_remote(&work_dir, "origin/cp-conflict", "cp-conflict");
    assert_eq!(results.len(), 1);
    let result = &results[0];
    assert!(!result.success);
    assert_eq!(result.message, "Cherry-pick conflict \u{2014} aborted");
    assert!(result.failure.is_none());

    assert_eq!(git_output(&work_dir, &["status", "--porcelain"]), "");
    assert!(
        !ref_exists(&work_dir, "CHERRY_PICK_HEAD"),
        "CHERRY_PICK_HEAD should be absent after abort"
    );
}
