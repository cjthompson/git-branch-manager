
use git_branch_manager::git::{
    branch, cache,
    operations, worktree,
};
use git_branch_manager::types::{
    ChangedFileKind, FailureCause,
};

use crate::common::*;

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
        result.success,
        "an already-gone branch should satisfy the requested delete"
    );
    assert!(matches!(result.failure, Some(FailureCause::BranchNotFound)));
}

#[test]
fn test_delete_local_requires_merge_but_force_delete_bypasses_it() {
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/unmerged-delete"]);
    std::fs::write(dir.join("wip.txt"), "wip\n").unwrap();
    run_git(dir, &["add", "wip.txt"]);
    run_git(dir, &["commit", "-m", "wip"]);
    run_git(dir, &["checkout", "main"]);

    let safe = operations::delete_local(&repo, "feature/unmerged-delete");
    assert!(
        !safe.success,
        "safe deletion must reject an unmerged branch"
    );
    assert!(matches!(safe.failure, Some(FailureCause::NotMerged)));

    let forced = operations::delete_local_force(&repo, "feature/unmerged-delete");
    assert!(
        forced.success,
        "force deletion should remove the branch: {forced:?}"
    );
    assert!(matches!(
        forced.action,
        git_branch_manager::types::BranchAction::DeleteLocalForce
    ));
    assert!(repo
        .find_branch("feature/unmerged-delete", git2::BranchType::Local)
        .is_err());
}

#[test]
fn test_delete_local_squash_merged_branch_requires_force() {
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/squashed-delete"]);
    std::fs::write(dir.join("squashed.txt"), "squashed\n").unwrap();
    run_git(dir, &["add", "squashed.txt"]);
    run_git(dir, &["commit", "-m", "squashed work"]);
    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/squashed-delete"]);
    run_git(dir, &["commit", "-m", "squash feature"]);

    let safe = operations::delete_local(&repo, "feature/squashed-delete");
    assert!(
        !safe.success,
        "safe deletion must reject a squash-merged branch"
    );
    assert!(matches!(safe.failure, Some(FailureCause::NotMerged)));

    let forced = operations::delete_local_force(&repo, "feature/squashed-delete");
    assert!(
        forced.success,
        "force deletion should remove the squash-merged branch: {forced:?}"
    );
}

#[test]
fn test_delete_local_classifies_primary_and_linked_worktree_failures() {
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();

    let primary = operations::delete_local(&repo, "main");
    assert!(
        !primary.success,
        "the checked-out primary branch is not removable"
    );
    // Intentional behavior change (P005 review): `main` is checked out in the
    // primary worktree, which IS the caller's own worktree (repo_path ==
    // dir here). classify_delete_command_error now uses
    // try_other_worktree_for_branch, which excludes the caller's own
    // worktree, so this case falls through to `Other` with the raw git
    // message preserved rather than being misreported as a recoverable
    // "checked out elsewhere". `is_main: true` was never actionable via the
    // Results overlay's `r` recovery key (see app.rs), so this is not a
    // user-visible regression.
    assert!(matches!(primary.failure, Some(FailureCause::Other { .. })));

    run_git(dir, &["branch", "feature/linked-delete"]);
    run_git(
        dir,
        &[
            "worktree",
            "add",
            ".worktrees/feature-linked-delete",
            "feature/linked-delete",
        ],
    );

    let linked = operations::delete_local(&repo, "feature/linked-delete");
    assert!(!linked.success, "a linked worktree branch is not removable");
    match linked.failure {
        Some(FailureCause::CheckedOutInWorktree {
            worktree_path,
            is_main: false,
        }) => assert!(worktree_path.ends_with(".worktrees/feature-linked-delete")),
        other => panic!("expected linked worktree cause, got {other:?}"),
    }
}

#[test]
fn test_dirty_linked_worktree_reports_all_changes_and_remains_recoverable() {
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();
    let linked_path = dir.join(".worktrees/dirty-linked");
    let linked_path_text = linked_path.to_string_lossy().into_owned();

    run_git(dir, &["branch", "feature/dirty-linked"]);
    run_git(
        dir,
        &[
            "worktree",
            "add",
            linked_path_text.as_str(),
            "feature/dirty-linked",
        ],
    );
    std::fs::write(linked_path.join("README.md"), "# Dirty linked worktree\n").unwrap();
    for index in 0..6 {
        std::fs::write(
            linked_path.join(format!("untracked-{index}.txt")),
            format!("untracked {index}\n"),
        )
        .unwrap();
    }

    let worktrees = worktree::list_worktrees(dir);
    let linked_index = worktrees
        .iter()
        .position(|worktree| worktree.branch.as_deref() == Some("feature/dirty-linked"))
        .expect("linked worktree should be listed");
    assert!(
        !worktrees[linked_index].is_main,
        "the dirty linked worktree must be non-primary"
    );

    let linked_path = worktrees[linked_index].path.clone();
    let statuses = worktree::enrich_worktrees(worktrees);
    let linked_status = statuses
        .iter()
        .find(|result| result.path == linked_path)
        .expect("linked worktree should be enriched")
        .wt_status;
    assert!(linked_status.has_modified, "README.md is modified");
    assert!(linked_status.has_untracked, "six files are untracked");
    assert!(!linked_status.has_staged, "no changes are staged");
    assert_eq!(
        linked_status.changed_files.len(),
        7,
        "the presentation layer may bound its list to five, but enrichment retains all facts"
    );
    assert!(linked_status
        .changed_files
        .iter()
        .any(|file| { file.path == "README.md" && file.kind == ChangedFileKind::Modified }));
    for index in 0..6 {
        assert!(
            linked_status.changed_files.iter().any(|file| {
                file.path == format!("untracked-{index}.txt")
                    && file.kind == ChangedFileKind::Untracked
            }),
            "untracked-{index}.txt was omitted from linked-worktree enrichment"
        );
    }

    let linked = operations::delete_local(&repo, "feature/dirty-linked");
    assert!(!linked.success, "a linked worktree branch is not removable");
    match linked.failure {
        Some(FailureCause::CheckedOutInWorktree {
            worktree_path,
            is_main: false,
        }) => assert!(
            worktree_path.ends_with(".worktrees/dirty-linked"),
            "Git must identify the linked worktree, got {}",
            worktree_path.display()
        ),
        other => panic!("expected a recoverable linked-worktree failure, got {other:?}"),
    }

    let primary = operations::delete_local(&repo, "main");
    assert!(!primary.success, "the primary branch is not removable");
    assert!(matches!(primary.failure, Some(FailureCause::Other { .. })));

    if std::env::var_os("GBM_KEEP_TEST_REPOS").is_none() {
        run_git(
            dir,
            &["worktree", "remove", "--force", linked_path_text.as_str()],
        );
    }
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
    let test_cache = tempfile::tempdir().expect("isolated branch cache");
    let branches = branch::list_branches_with_cache_root(
        &repo,
        "main",
        &cache::CacheRoot::at(test_cache.path()),
    )
    .expect("list_branches failed");

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

    let branches = test_branches(&tmpdir, &repo, "main").expect("list_branches failed");
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
