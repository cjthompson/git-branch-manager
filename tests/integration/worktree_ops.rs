use std::sync::atomic::AtomicBool;

use git_branch_manager::git::{
    operations, worktree,
};

use crate::common::*;

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
    // It must be checked out on the requested branch, not left detached.
    let wts = worktree::list_worktrees(dir);
    assert!(
        wts.iter()
            .any(|w| w.branch.as_deref() == Some("wt-feature")),
        "worktree should be checked out on wt-feature: {wts:?}"
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
    // The slash must be sanitized to a dash, NOT create a nested directory.
    assert!(
        !dir.join(".worktrees")
            .join("feature")
            .join("slash-test")
            .exists(),
        "slash should be replaced with a dash, not create nested dirs"
    );
    // And the worktree is checked out on the real (slashed) branch name.
    let wts = worktree::list_worktrees(dir);
    assert!(
        wts.iter()
            .any(|w| w.branch.as_deref() == Some("feature/slash-test")),
        "worktree should be on feature/slash-test: {wts:?}"
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

    let (prog_tx, _prog_rx) = std::sync::mpsc::channel();
    let cancel = AtomicBool::new(false);
    let partial = AtomicBool::new(false);
    let result = operations::remove_worktree(dir, &wt_path, (0, 1), &prog_tx, &cancel, &partial);
    assert!(
        result.success,
        "remove_worktree should succeed on clean worktree: {}",
        result.message
    );
    assert!(!wt_path.exists(), "worktree directory should be removed");
    // It must also be deregistered from git's worktree list, not just unlinked
    // on disk — only the main worktree should remain.
    let wts = worktree::list_worktrees(dir);
    assert_eq!(
        wts.len(),
        1,
        "only the main worktree should remain: {wts:?}"
    );
    assert!(wts[0].is_main, "the remaining worktree is the main one");
}

#[test]
fn test_force_remove_worktree_dirty() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["branch", "wt-dirty"]);
    run_git(dir, &["worktree", "add", ".worktrees/wt-dirty", "wt-dirty"]);

    let wt_path = dir.join(".worktrees").join("wt-dirty");

    // Modify a TRACKED file in the worktree. `git worktree remove` reliably
    // refuses on tracked-file modifications across git versions; an untracked
    // file alone is not a dependable refusal trigger.
    std::fs::write(wt_path.join("README.md"), "# Modified in worktree\n").unwrap();

    let (prog_tx, _prog_rx) = std::sync::mpsc::channel();
    let cancel = AtomicBool::new(false);
    let partial = AtomicBool::new(false);
    let result = operations::remove_worktree(dir, &wt_path, (0, 1), &prog_tx, &cancel, &partial);
    assert!(
        !result.success,
        "remove_worktree should fail on dirty worktree"
    );

    // Force remove should succeed regardless
    let (prog_tx, _prog_rx) = std::sync::mpsc::channel();
    let result =
        operations::force_remove_worktree(dir, &wt_path, (0, 1), &prog_tx, &cancel, &partial);
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
