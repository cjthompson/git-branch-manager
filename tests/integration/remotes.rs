
use git_branch_manager::git::{
    branch,
    operations,
};
use git_branch_manager::types::MergeStatus;

use crate::common::*;

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

    // Create branches with explicit, distinct commit times. list_remote_branches_phase1
    // sorts by committer date, so set both author and committer dates.
    run_git(&work_dir, &["checkout", "-b", "older-branch"]);
    let f1 = work_dir.join("older.txt");
    std::fs::write(&f1, "older\n").unwrap();
    run_git(&work_dir, &["add", "older.txt"]);
    run_git_with_env(
        &work_dir,
        &["commit", "-m", "Older commit"],
        &[
            ("GIT_AUTHOR_DATE", "2001-01-01T00:00:00Z"),
            ("GIT_COMMITTER_DATE", "2001-01-01T00:00:00Z"),
        ],
    );
    run_git(&work_dir, &["push", "-u", "origin", "older-branch"]);

    run_git(&work_dir, &["checkout", "main"]);
    run_git(&work_dir, &["checkout", "-b", "newer-branch"]);
    let f2 = work_dir.join("newer.txt");
    std::fs::write(&f2, "newer\n").unwrap();
    run_git(&work_dir, &["add", "newer.txt"]);
    run_git_with_env(
        &work_dir,
        &["commit", "-m", "Newer commit"],
        &[
            ("GIT_AUTHOR_DATE", "2001-01-02T00:00:00Z"),
            ("GIT_COMMITTER_DATE", "2001-01-02T00:00:00Z"),
        ],
    );
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
