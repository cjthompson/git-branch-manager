
use git_branch_manager::git::{
    operations, worktree,
};

use crate::common::*;

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
fn test_worktree_lookup_includes_primary_and_reports_command_errors() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    let primary = worktree::try_worktree_for_branch(dir, "main")
        .expect("primary worktree lookup should run")
        .expect("primary worktree should be found");
    assert!(primary.is_main);
    assert_eq!(primary.branch.as_deref(), Some("main"));

    let missing =
        worktree::try_list_worktrees(std::path::Path::new("/definitely/not/a/git/repository"));
    assert!(
        missing.is_err(),
        "worktree command failures must be observable"
    );
}

#[test]
fn test_branches_checked_out_in_worktrees_excludes_primary() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["branch", "feature/linked-checkout"]);
    run_git(
        dir,
        &[
            "worktree",
            "add",
            ".worktrees/feature-linked-checkout",
            "feature/linked-checkout",
        ],
    );

    let checked_out = worktree::branches_checked_out_in_worktrees(dir);
    assert!(
        checked_out.contains("feature/linked-checkout"),
        "linked worktree's branch should be reported"
    );
    assert!(
        !checked_out.contains("main"),
        "the primary worktree's branch is excluded, even though it is checked out"
    );

    run_git(
        dir,
        &["worktree", "remove", ".worktrees/feature-linked-checkout"],
    );
}

#[test]
fn test_branches_checked_out_in_worktrees_empty_with_only_primary() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    let checked_out = worktree::branches_checked_out_in_worktrees(dir);
    assert!(
        checked_out.is_empty(),
        "with only the primary worktree present, nothing is reported"
    );
}

#[test]
fn test_branches_checked_out_in_worktrees_best_effort_on_command_failure() {
    let checked_out = worktree::branches_checked_out_in_worktrees(std::path::Path::new(
        "/definitely/not/a/git/repository",
    ));
    assert!(
        checked_out.is_empty(),
        "best-effort callers must degrade to an empty set, not panic, on inspection failure"
    );
}

#[test]
fn test_worktree_path_for_branch_excludes_callers_own_worktree() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    // "main" is checked out in the primary worktree, which IS the caller
    // (repo_path == dir). worktree_path_for_branch answers "checked out in
    // some OTHER worktree", so this must be None, not the primary's own path.
    let path = worktree::worktree_path_for_branch(dir, "main");
    assert!(
        path.is_none(),
        "the caller's own worktree must not be reported as an 'other' worktree"
    );
}

#[test]
fn test_worktree_path_for_branch_finds_linked_worktree_other_than_caller() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["branch", "feature/linked-path"]);
    run_git(
        dir,
        &[
            "worktree",
            "add",
            ".worktrees/feature-linked-path",
            "feature/linked-path",
        ],
    );

    let path = worktree::worktree_path_for_branch(dir, "feature/linked-path")
        .expect("linked worktree should be found");
    assert!(path.ends_with(".worktrees/feature-linked-path"));

    run_git(
        dir,
        &["worktree", "remove", ".worktrees/feature-linked-path"],
    );
}

#[test]
fn test_try_other_worktree_for_branch_none_vs_err() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Not checked out anywhere else: Ok(None), distinct from a lookup failure.
    let not_checked_out = worktree::try_other_worktree_for_branch(dir, "does-not-exist-anywhere");
    assert!(matches!(not_checked_out, Ok(None)));

    // An unreadable/non-existent repo path: Err(_), distinguishable from Ok(None).
    let inspection_failed = worktree::try_other_worktree_for_branch(
        std::path::Path::new("/definitely/not/a/git/repository"),
        "main",
    );
    assert!(
        inspection_failed.is_err(),
        "command failure must be observable as Err, not conflated with 'not checked out'"
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

    let rx = worktree::enrich_worktrees(worktrees.clone());
    let results: Vec<_> = rx.iter().collect();

    assert_eq!(results.len(), 1, "should receive one enrichment result");
    assert_eq!(results[0].path, worktrees[0].path);
    assert!(
        results[0].wt_status.is_clean(),
        "clean repo worktree should report clean status"
    );
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
