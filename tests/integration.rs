use git2::Repository;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

use git_branch_manager::git::{branch, operations, squash_loader, worktree};
use git_branch_manager::types::MergeStatus;

fn run_git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git command failed");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn setup_test_repo() -> (TempDir, Repository) {
    let dir = TempDir::new().unwrap();
    let path = dir.path();
    run_git(path, &["init", "-b", "main"]);
    run_git(path, &["config", "user.email", "test@test.com"]);
    run_git(path, &["config", "user.name", "Test"]);
    std::fs::write(path.join("README.md"), "init").unwrap();
    run_git(path, &["add", "."]);
    run_git(path, &["commit", "-m", "initial"]);
    let repo = Repository::open(path).unwrap();
    (dir, repo)
}

fn setup_remote_test_repo() -> (TempDir, TempDir, Repository) {
    // Create bare remote with explicit main branch
    let remote_dir = TempDir::new().unwrap();
    run_git(remote_dir.path(), &["init", "--bare", "-b", "main"]);

    // Clone it
    let local_dir = TempDir::new().unwrap();
    let remote_url = remote_dir.path().to_str().unwrap();
    Command::new("git")
        .args(["clone", remote_url, local_dir.path().to_str().unwrap()])
        .output()
        .unwrap();

    let path = local_dir.path();
    run_git(path, &["config", "user.email", "test@test.com"]);
    run_git(path, &["config", "user.name", "Test"]);
    // Ensure we're on main (clone of empty repo may have no branch)
    run_git(path, &["checkout", "-B", "main"]);
    std::fs::write(path.join("README.md"), "init").unwrap();
    run_git(path, &["add", "."]);
    run_git(path, &["commit", "-m", "initial"]);
    run_git(path, &["push", "-u", "origin", "main"]);

    let repo = Repository::open(path).unwrap();
    (local_dir, remote_dir, repo)
}

// ===== Working Tree Status Tests =====

#[test]
fn test_wt_status_clean() {
    let (_dir, repo) = setup_test_repo();
    let status = git_branch_manager::git::status::detect_working_tree_status(&repo);
    assert!(status.is_clean());
}

#[test]
fn test_wt_status_staged_only() {
    let (dir, repo) = setup_test_repo();
    std::fs::write(dir.path().join("new.txt"), "content").unwrap();
    run_git(dir.path(), &["add", "new.txt"]);
    let status = git_branch_manager::git::status::detect_working_tree_status(&repo);
    assert!(status.has_staged);
    assert!(!status.has_unstaged);
    assert!(!status.has_untracked);
}

#[test]
fn test_wt_status_unstaged_only() {
    let (dir, repo) = setup_test_repo();
    std::fs::write(dir.path().join("README.md"), "modified").unwrap();
    let status = git_branch_manager::git::status::detect_working_tree_status(&repo);
    assert!(!status.has_staged);
    assert!(status.has_unstaged);
}

#[test]
fn test_wt_status_untracked_only() {
    let (dir, repo) = setup_test_repo();
    std::fs::write(dir.path().join("untracked.txt"), "x").unwrap();
    let status = git_branch_manager::git::status::detect_working_tree_status(&repo);
    assert!(!status.has_staged);
    assert!(!status.has_unstaged);
    assert!(status.has_untracked);
}

#[test]
fn test_wt_status_all_three() {
    let (dir, repo) = setup_test_repo();
    std::fs::write(dir.path().join("staged.txt"), "s").unwrap();
    run_git(dir.path(), &["add", "staged.txt"]);
    std::fs::write(dir.path().join("README.md"), "modified").unwrap();
    std::fs::write(dir.path().join("untracked.txt"), "u").unwrap();
    let status = git_branch_manager::git::status::detect_working_tree_status(&repo);
    assert!(status.has_staged);
    assert!(status.has_unstaged);
    assert!(status.has_untracked);
}

// ===== Merge Detection Tests =====

#[test]
fn test_merged_branch_detection() {
    let (dir, repo) = setup_test_repo();
    let path = dir.path();

    // Create and merge a branch
    run_git(path, &["checkout", "-b", "feature/merged"]);
    std::fs::write(path.join("feature.txt"), "feature content").unwrap();
    run_git(path, &["add", "."]);
    run_git(path, &["commit", "-m", "feature commit"]);
    run_git(path, &["checkout", "main"]);
    run_git(
        path,
        &["merge", "feature/merged", "--no-ff", "-m", "merge feature"],
    );

    let mut branches =
        git_branch_manager::git::branch::list_branches_phase1(&repo, "main").unwrap();
    // list_branches_phase1 already calls detect_merged_branches internally.
    // But merged branches get Merged status, not Pending.
    let feature = branches
        .iter_mut()
        .find(|b| b.name == "feature/merged")
        .unwrap();
    assert_eq!(
        feature.merge_status,
        git_branch_manager::types::MergeStatus::Merged
    );
}

#[test]
fn test_squash_merged_detection() {
    let (dir, _repo) = setup_test_repo();
    let path = dir.path();

    // Create a branch with content
    run_git(path, &["checkout", "-b", "feature/squashed"]);
    std::fs::write(path.join("squash.txt"), "squash content").unwrap();
    run_git(path, &["add", "."]);
    run_git(path, &["commit", "-m", "squash commit"]);
    run_git(path, &["checkout", "main"]);

    // Squash merge (--squash + commit)
    run_git(path, &["merge", "--squash", "feature/squashed"]);
    run_git(path, &["commit", "-m", "squashed feature"]);

    let is_squash = git_branch_manager::git::merge_detection::is_squash_merged(
        path,
        "main",
        "feature/squashed",
        None,
    );
    assert!(is_squash);
}

#[test]
fn test_unmerged_detection() {
    let (dir, _repo) = setup_test_repo();
    let path = dir.path();

    run_git(path, &["checkout", "-b", "feature/unmerged"]);
    std::fs::write(path.join("unmerged.txt"), "unmerged").unwrap();
    run_git(path, &["add", "."]);
    run_git(path, &["commit", "-m", "unmerged commit"]);
    run_git(path, &["checkout", "main"]);

    let is_squash = git_branch_manager::git::merge_detection::is_squash_merged(
        path,
        "main",
        "feature/unmerged",
        None,
    );
    assert!(!is_squash);
}

// ===== Branch Listing Tests =====

#[test]
fn test_detect_base_branch_main() {
    let (_dir, repo) = setup_test_repo();
    let base = git_branch_manager::git::branch::detect_base_branch(&repo, None).unwrap();
    assert_eq!(base, "main");
}

#[test]
fn test_detect_base_branch_override() {
    let (dir, repo) = setup_test_repo();
    run_git(dir.path(), &["checkout", "-b", "develop"]);
    run_git(dir.path(), &["checkout", "main"]);
    let base =
        git_branch_manager::git::branch::detect_base_branch(&repo, Some("develop")).unwrap();
    assert_eq!(base, "develop");
}

#[test]
fn test_list_branches_phase1() {
    let (dir, repo) = setup_test_repo();
    let path = dir.path();

    run_git(path, &["checkout", "-b", "feature/a"]);
    std::fs::write(path.join("a.txt"), "a").unwrap();
    run_git(path, &["add", "."]);
    run_git(path, &["commit", "-m", "feature a"]);
    run_git(path, &["checkout", "main"]);

    let branches = git_branch_manager::git::branch::list_branches_phase1(&repo, "main").unwrap();
    assert!(branches.len() >= 2);
    let main_branch = branches.iter().find(|b| b.name == "main").unwrap();
    assert!(main_branch.is_base);
    let feature = branches.iter().find(|b| b.name == "feature/a").unwrap();
    assert!(!feature.is_base);
}

// ===== Operations Tests =====

#[test]
fn test_delete_local_branch() {
    let (dir, repo) = setup_test_repo();
    let path = dir.path();
    run_git(path, &["checkout", "-b", "to-delete"]);
    run_git(path, &["checkout", "main"]);

    let result = git_branch_manager::git::operations::delete_local(&repo, "to-delete");
    assert!(result.success);

    let branches =
        git_branch_manager::git::branch::list_branches_phase1(&repo, "main").unwrap();
    assert!(branches.iter().all(|b| b.name != "to-delete"));
}

#[test]
fn test_checkout_branch() {
    let (dir, repo) = setup_test_repo();
    let path = dir.path();
    run_git(path, &["checkout", "-b", "feature/checkout-test"]);
    run_git(path, &["checkout", "main"]);

    let result = git_branch_manager::git::operations::checkout_branch(
        &repo,
        path,
        "feature/checkout-test",
        false,
    );
    assert!(result.success);
    // Reopen repo to see updated HEAD
    let repo = Repository::open(path).unwrap();
    let head = repo.head().unwrap();
    assert_eq!(head.shorthand().unwrap(), "feature/checkout-test");
}

#[test]
fn test_push_branch() {
    let (local_dir, _remote_dir, _repo) = setup_remote_test_repo();
    let path = local_dir.path();
    run_git(path, &["checkout", "-b", "feature/push-test"]);
    std::fs::write(path.join("push.txt"), "push").unwrap();
    run_git(path, &["add", "."]);
    run_git(path, &["commit", "-m", "push commit"]);

    let result = git_branch_manager::git::operations::push_branch(path, "feature/push-test");
    assert!(result.success);
}

#[test]
fn test_fetch_sync() {
    let (local_dir, _remote_dir, _repo) = setup_remote_test_repo();
    let result = git_branch_manager::git::operations::fetch_sync(local_dir.path());
    assert!(result);
}

// ===== Worktree Tests =====

#[test]
fn test_list_worktrees_main_only() {
    let (dir, _repo) = setup_test_repo();
    let worktrees = git_branch_manager::git::worktree::list_worktrees(dir.path());
    assert_eq!(worktrees.len(), 1);
    assert!(worktrees[0].is_main);
}

#[test]
fn test_create_and_list_worktree() {
    let (dir, _repo) = setup_test_repo();
    let path = dir.path();
    run_git(path, &["checkout", "-b", "feature/wt-test"]);
    run_git(path, &["checkout", "main"]);

    let result = git_branch_manager::git::operations::create_worktree(path, "feature/wt-test");
    assert!(result.success);

    let worktrees = git_branch_manager::git::worktree::list_worktrees(path);
    assert_eq!(worktrees.len(), 2);
    let wt = worktrees.iter().find(|w| !w.is_main).unwrap();
    assert_eq!(wt.branch.as_deref(), Some("feature/wt-test"));
}

// ===== Tag Tests =====

#[test]
fn test_list_tags_empty() {
    let (_dir, repo) = setup_test_repo();
    let tags = git_branch_manager::git::tags::list_tags(&repo);
    assert!(tags.is_empty());
}

#[test]
fn test_list_tags_with_annotated() {
    let (dir, repo) = setup_test_repo();
    run_git(dir.path(), &["tag", "-a", "v1.0", "-m", "Release 1.0"]);
    let tags = git_branch_manager::git::tags::list_tags(&repo);
    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0].name, "v1.0");
    assert!(tags[0].is_annotated);
    assert_eq!(tags[0].message.as_deref(), Some("Release 1.0"));
}

#[test]
fn test_list_tags_lightweight() {
    let (dir, repo) = setup_test_repo();
    run_git(dir.path(), &["tag", "v0.1"]);
    let tags = git_branch_manager::git::tags::list_tags(&repo);
    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0].name, "v0.1");
    assert!(!tags[0].is_annotated);
    assert!(tags[0].message.is_none());
}

#[test]
fn test_delete_tag() {
    let (dir, repo) = setup_test_repo();
    run_git(dir.path(), &["tag", "v1.0"]);
    let result = git_branch_manager::git::tags::delete_tag(&repo, "v1.0");
    assert!(result.success);
    let tags = git_branch_manager::git::tags::list_tags(&repo);
    assert!(tags.is_empty());
}

#[test]
fn test_delete_tags_batch() {
    let (dir, repo) = setup_test_repo();
    run_git(dir.path(), &["tag", "v1.0"]);
    run_git(dir.path(), &["tag", "v2.0"]);
    let names = vec!["v1.0".to_string(), "v2.0".to_string()];
    let results = git_branch_manager::git::tags::delete_tags_batch(&repo, &names);
    assert!(results.iter().all(|r| r.success));
    let tags = git_branch_manager::git::tags::list_tags(&repo);
    assert!(tags.is_empty());
}

// ===== Base Branch Detection Tests =====

#[test]
fn test_detect_base_branch_override_nonexistent() {
    let (_dir, repo) = setup_test_repo();
    let result = branch::detect_base_branch(&repo, Some("nonexistent"));
    assert!(
        result.is_err(),
        "expected error for nonexistent branch override"
    );
}

// ===== Branch Listing Tests =====

#[test]
fn test_list_branches() {
    let (dir, repo) = setup_test_repo();
    let path = dir.path();

    // Create two feature branches
    run_git(path, &["branch", "feature-a"]);
    run_git(path, &["branch", "feature-b"]);

    let branches = branch::list_branches_phase1(&repo, "main").unwrap();

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
fn test_ahead_behind_indicators() {
    let (local_dir, _remote_dir, _repo) = setup_remote_test_repo();
    let path = local_dir.path();

    // Create a feature branch and push it so it has a remote tracking branch
    run_git(path, &["checkout", "-b", "feature-ahead"]);
    std::fs::write(path.join("feature.txt"), "feature content\n").unwrap();
    run_git(path, &["add", "feature.txt"]);
    run_git(path, &["commit", "-m", "Feature commit 1"]);
    run_git(path, &["push", "-u", "origin", "feature-ahead"]);

    // Add another local commit (not pushed) -- this should make ahead=1
    std::fs::write(path.join("feature2.txt"), "more feature content\n").unwrap();
    run_git(path, &["add", "feature2.txt"]);
    run_git(path, &["commit", "-m", "Feature commit 2 (local only)"]);

    // Go back to main for listing
    run_git(path, &["checkout", "main"]);

    let repo = Repository::open(path).unwrap();
    let branches = branch::list_branches_phase1(&repo, "main").unwrap();

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
    let (dir, repo) = setup_test_repo();
    let path = dir.path();

    run_git(path, &["branch", "local-only"]);

    let branches = branch::list_branches_phase1(&repo, "main").unwrap();
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

// ===== Full Squash/Unmerged Detection Tests =====

#[test]
fn test_squash_merged_branch_detection() {
    // Tests squash detection via list_branches_phase1 + squash_loader (full pipeline)
    let (dir, _repo) = setup_test_repo();
    let path = dir.path();

    // Create feature-squashed branch with a commit
    run_git(path, &["checkout", "-b", "feature-squashed"]);
    std::fs::write(path.join("squash-feature.txt"), "squash feature content\n").unwrap();
    run_git(path, &["add", "squash-feature.txt"]);
    run_git(path, &["commit", "-m", "Add squash feature"]);

    // Switch back to main and squash merge
    run_git(path, &["checkout", "main"]);
    run_git(path, &["merge", "--squash", "feature-squashed"]);
    run_git(path, &["commit", "-m", "squash merge feature-squashed"]);

    // Re-open the repo so git2 sees the latest state
    let repo = Repository::open(path).unwrap();
    let mut branches = branch::list_branches_phase1(&repo, "main").unwrap();

    // Build candidates for squash check: (branch_name, commit_hash) for pending non-base branches
    let candidates: Vec<(String, String)> = branches
        .iter()
        .filter(|b| b.merge_status == MergeStatus::Pending)
        .filter_map(|b| {
            branch::get_commit_hash(&repo, &b.name).map(|h| (b.name.clone(), h))
        })
        .collect();

    let cache = git_branch_manager::git::cache::BranchCache::load(path);
    let rx = squash_loader::spawn_squash_checker(
        path.to_path_buf(),
        "main".to_string(),
        candidates,
        cache,
    );

    // Apply results
    let index_map: std::collections::HashMap<String, usize> = branches
        .iter()
        .enumerate()
        .map(|(i, b)| (b.name.clone(), i))
        .collect();

    for result in rx {
        if let Some(&idx) = index_map.get(&result.branch_name) {
            branches[idx].merge_status = if result.is_squash_merged {
                MergeStatus::SquashMerged
            } else {
                MergeStatus::Unmerged
            };
        }
    }

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
    // Tests that an unmerged branch remains Unmerged after full squash check
    let (dir, _repo) = setup_test_repo();
    let path = dir.path();

    run_git(path, &["checkout", "-b", "feature-wip"]);
    std::fs::write(path.join("wip.txt"), "work in progress\n").unwrap();
    run_git(path, &["add", "wip.txt"]);
    run_git(path, &["commit", "-m", "WIP commit"]);
    run_git(path, &["checkout", "main"]);

    let repo = Repository::open(path).unwrap();
    let mut branches = branch::list_branches_phase1(&repo, "main").unwrap();

    let candidates: Vec<(String, String)> = branches
        .iter()
        .filter(|b| b.merge_status == MergeStatus::Pending)
        .filter_map(|b| {
            branch::get_commit_hash(&repo, &b.name).map(|h| (b.name.clone(), h))
        })
        .collect();

    let cache = git_branch_manager::git::cache::BranchCache::load(path);
    let rx = squash_loader::spawn_squash_checker(
        path.to_path_buf(),
        "main".to_string(),
        candidates,
        cache,
    );

    let index_map: std::collections::HashMap<String, usize> = branches
        .iter()
        .enumerate()
        .map(|(i, b)| (b.name.clone(), i))
        .collect();

    for result in rx {
        if let Some(&idx) = index_map.get(&result.branch_name) {
            branches[idx].merge_status = if result.is_squash_merged {
                MergeStatus::SquashMerged
            } else {
                MergeStatus::Unmerged
            };
        }
    }

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

// ===== Operations Tests =====

#[test]
fn test_delete_local_nonexistent() {
    let (_dir, repo) = setup_test_repo();
    let result = operations::delete_local(&repo, "does-not-exist");
    assert!(
        !result.success,
        "delete_local on nonexistent branch should return success: false"
    );
}

#[test]
fn test_checkout_branch_with_stash() {
    let (dir, repo) = setup_test_repo();
    let path = dir.path();
    run_git(path, &["branch", "feature-stash-checkout"]);

    // Create a dirty working tree (unstaged change)
    std::fs::write(path.join("README.md"), "# Modified\n").unwrap();

    let result = operations::checkout_branch(&repo, path, "feature-stash-checkout", true);
    assert!(
        result.success,
        "checkout with stash should succeed: {}",
        result.message
    );

    let repo = Repository::open(path).unwrap();
    let head = repo.head().unwrap();
    assert_eq!(head.shorthand().unwrap(), "feature-stash-checkout");

    // The stash should have been popped, so the working tree change should be present
    let contents = std::fs::read_to_string(path.join("README.md")).unwrap();
    assert_eq!(
        contents, "# Modified\n",
        "stash pop should restore changes"
    );
}

// ===== Remote Branch Tests =====

#[test]
fn test_checkout_remote_branch() {
    let (local_dir, _remote_dir, _repo) = setup_remote_test_repo();
    let path = local_dir.path();

    // Create and push a branch, then delete local
    run_git(path, &["checkout", "-b", "remote-checkout-test"]);
    std::fs::write(path.join("checkout-test.txt"), "content\n").unwrap();
    run_git(path, &["add", "checkout-test.txt"]);
    run_git(path, &["commit", "-m", "Checkout test commit"]);
    run_git(path, &["push", "-u", "origin", "remote-checkout-test"]);
    run_git(path, &["checkout", "main"]);
    // Delete local branch and also remove the checked-out file
    run_git(path, &["branch", "-D", "remote-checkout-test"]);
    let _ = std::fs::remove_file(path.join("checkout-test.txt"));

    // Now checkout from remote
    let result = operations::checkout_remote_branch(path, "origin", "remote-checkout-test");
    assert!(
        result.success,
        "checkout_remote_branch should succeed: {}",
        result.message
    );

    // Verify we're on the new local branch
    let repo = Repository::open(path).unwrap();
    let head = repo.head().unwrap();
    assert_eq!(head.shorthand().unwrap(), "remote-checkout-test");

    // Verify the file from the remote branch exists
    assert!(
        path.join("checkout-test.txt").exists(),
        "checked-out file should exist"
    );
}

#[test]
fn test_checkout_remote_branch_already_exists() {
    let (local_dir, _remote_dir, _repo) = setup_remote_test_repo();
    let path = local_dir.path();

    // Create and push a branch, keep local copy
    run_git(path, &["checkout", "-b", "already-local"]);
    std::fs::write(path.join("already.txt"), "content\n").unwrap();
    run_git(path, &["add", "already.txt"]);
    run_git(path, &["commit", "-m", "Already local"]);
    run_git(path, &["push", "-u", "origin", "already-local"]);
    run_git(path, &["checkout", "main"]);

    // Trying to checkout remote when local already exists should fail
    let result = operations::checkout_remote_branch(path, "origin", "already-local");
    assert!(
        !result.success,
        "should fail when local branch already exists"
    );
}

#[test]
fn test_list_remote_branches() {
    let (local_dir, _remote_dir, _repo) = setup_remote_test_repo();
    let path = local_dir.path();

    // Create a feature branch and push it
    run_git(path, &["checkout", "-b", "feature-remote"]);
    std::fs::write(path.join("feature.txt"), "content\n").unwrap();
    run_git(path, &["add", "feature.txt"]);
    run_git(path, &["commit", "-m", "Feature commit"]);
    run_git(path, &["push", "-u", "origin", "feature-remote"]);
    run_git(path, &["checkout", "main"]);

    let repo = Repository::open(path).unwrap();
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
    let (local_dir, _remote_dir, _repo) = setup_remote_test_repo();
    let path = local_dir.path();

    // Push a branch that has a local counterpart
    run_git(path, &["checkout", "-b", "has-local"]);
    std::fs::write(path.join("local.txt"), "content\n").unwrap();
    run_git(path, &["add", "local.txt"]);
    run_git(path, &["commit", "-m", "Local branch commit"]);
    run_git(path, &["push", "-u", "origin", "has-local"]);

    // Push a branch then delete the local copy (remote-only)
    run_git(path, &["checkout", "-b", "remote-only"]);
    std::fs::write(path.join("remote-only.txt"), "content\n").unwrap();
    run_git(path, &["add", "remote-only.txt"]);
    run_git(path, &["commit", "-m", "Remote-only commit"]);
    run_git(path, &["push", "-u", "origin", "remote-only"]);
    run_git(path, &["checkout", "main"]);
    run_git(path, &["branch", "-D", "remote-only"]);

    // Re-open repo so git2 sees the deleted branch
    let repo = Repository::open(path).unwrap();
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
    let (local_dir, _remote_dir, _repo) = setup_remote_test_repo();
    let path = local_dir.path();

    // Create and push a feature branch
    run_git(path, &["checkout", "-b", "feature-to-merge"]);
    std::fs::write(path.join("merge-feature.txt"), "merge content\n").unwrap();
    run_git(path, &["add", "merge-feature.txt"]);
    run_git(path, &["commit", "-m", "Feature to merge"]);
    run_git(path, &["push", "-u", "origin", "feature-to-merge"]);

    // Add a commit on main so merge is not a fast-forward
    run_git(path, &["checkout", "main"]);
    std::fs::write(path.join("main-change.txt"), "main branch change\n").unwrap();
    run_git(path, &["add", "main-change.txt"]);
    run_git(path, &["commit", "-m", "Main branch commit"]);

    // Merge and push
    run_git(
        path,
        &["merge", "feature-to-merge", "-m", "Merge feature"],
    );
    run_git(path, &["push", "origin", "main"]);
    // Fetch to ensure tracking refs are up to date
    run_git(path, &["fetch", "origin"]);

    // Re-open to see updated refs
    let repo = Repository::open(path).unwrap();
    let mut remotes = branch::list_remote_branches_phase1(&repo, "main")
        .expect("list_remote_branches_phase1 failed");

    // Phase-1 returns Pending for non-base; enrichment resolves merge status.
    let rx = branch::spawn_remote_enricher(path.to_path_buf(), "main".to_string(), remotes.clone());
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
    let (local_dir, _remote_dir, _repo) = setup_remote_test_repo();
    let path = local_dir.path();

    // Create and push an unmerged feature branch
    run_git(path, &["checkout", "-b", "feature-unmerged"]);
    std::fs::write(path.join("unmerged.txt"), "unmerged content\n").unwrap();
    run_git(path, &["add", "unmerged.txt"]);
    run_git(path, &["commit", "-m", "Unmerged feature"]);
    run_git(path, &["push", "-u", "origin", "feature-unmerged"]);
    run_git(path, &["checkout", "main"]);

    let repo = Repository::open(path).unwrap();
    let remotes = branch::list_remote_branches_phase1(&repo, "main")
        .expect("list_remote_branches_phase1 failed");

    // Phase-1 sets non-base branches to Pending
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
    let (local_dir, _remote_dir, _repo) = setup_remote_test_repo();
    let path = local_dir.path();

    // Set up origin/HEAD (some repos have this)
    run_git(path, &["remote", "set-head", "origin", "main"]);

    let repo = Repository::open(path).unwrap();
    let remotes = branch::list_remote_branches_phase1(&repo, "main")
        .expect("list_remote_branches_phase1 failed");

    // Should not include any entry with short_name "HEAD"
    let head_entries: Vec<_> = remotes
        .iter()
        .filter(|r| r.short_name == "HEAD")
        .collect();
    assert!(
        head_entries.is_empty(),
        "origin/HEAD pseudo-ref should be filtered out"
    );
}

#[test]
fn test_remote_branches_sorted_by_date() {
    let (local_dir, _remote_dir, _repo) = setup_remote_test_repo();
    let path = local_dir.path();

    // Create branches with different commit times (sequential commits)
    run_git(path, &["checkout", "-b", "older-branch"]);
    std::fs::write(path.join("older.txt"), "older\n").unwrap();
    run_git(path, &["add", "older.txt"]);
    run_git(path, &["commit", "-m", "Older commit"]);
    run_git(path, &["push", "-u", "origin", "older-branch"]);

    run_git(path, &["checkout", "main"]);
    run_git(path, &["checkout", "-b", "newer-branch"]);
    std::fs::write(path.join("newer.txt"), "newer\n").unwrap();
    run_git(path, &["add", "newer.txt"]);
    run_git(path, &["commit", "-m", "Newer commit"]);
    run_git(path, &["push", "-u", "origin", "newer-branch"]);
    run_git(path, &["checkout", "main"]);

    let repo = Repository::open(path).unwrap();
    let remotes = branch::list_remote_branches_phase1(&repo, "main")
        .expect("list_remote_branches_phase1 failed");

    // Find positions -- newer should come before older (sorted newest-first)
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
    let (local_dir, _remote_dir, _repo) = setup_remote_test_repo();
    let path = local_dir.path();

    // Create a feature branch with unique content
    run_git(path, &["checkout", "-b", "squash-feature"]);
    std::fs::write(path.join("squash-feature.txt"), "squash feature content\n").unwrap();
    run_git(path, &["add", "squash-feature.txt"]);
    run_git(path, &["commit", "-m", "Squash feature commit"]);
    run_git(path, &["push", "-u", "origin", "squash-feature"]);

    // Squash-merge into main
    run_git(path, &["checkout", "main"]);
    run_git(path, &["merge", "--squash", "squash-feature"]);
    run_git(path, &["commit", "-m", "Squash merge squash-feature"]);
    run_git(path, &["push", "origin", "main"]);
    // Fetch to ensure tracking refs are up to date
    run_git(path, &["fetch", "origin"]);

    // Reload repo after push
    let repo = Repository::open(path).unwrap();
    let remotes = branch::list_remote_branches_phase1(&repo, "main")
        .expect("list_remote_branches_phase1 failed");

    // Build candidates for squash checker
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

    let cache = git_branch_manager::git::cache::BranchCache::load(path);
    let rx = squash_loader::spawn_squash_checker(
        path.to_path_buf(),
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

// ===== Merge Operation Tests =====

#[test]
fn test_merge_branch_success() {
    let (dir, _repo) = setup_test_repo();
    let path = dir.path();

    // Create a feature branch with a unique file
    run_git(path, &["checkout", "-b", "feature-to-merge"]);
    std::fs::write(path.join("feature.txt"), "feature content\n").unwrap();
    run_git(path, &["add", "feature.txt"]);
    run_git(path, &["commit", "-m", "Add feature"]);
    run_git(path, &["checkout", "main"]);

    let results = operations::merge_branch(path, "feature-to-merge", "main", false, false);
    assert_eq!(results.len(), 1);
    assert!(
        results[0].success,
        "merge should succeed: {}",
        results[0].message
    );

    // Verify the feature file is present on main
    assert!(
        path.join("feature.txt").exists(),
        "feature.txt should be on main after merge"
    );
}

#[test]
fn test_merge_branch_squash_success() {
    let (dir, _repo) = setup_test_repo();
    let path = dir.path();

    run_git(path, &["checkout", "-b", "feature-squash"]);
    std::fs::write(path.join("squash.txt"), "squash content\n").unwrap();
    run_git(path, &["add", "squash.txt"]);
    run_git(path, &["commit", "-m", "Squash candidate"]);
    run_git(path, &["checkout", "main"]);

    let results = operations::merge_branch(path, "feature-squash", "main", true, false);
    assert_eq!(results.len(), 1);
    assert!(
        results[0].success,
        "squash merge should succeed: {}",
        results[0].message
    );

    // The squash content should exist on main
    assert!(
        path.join("squash.txt").exists(),
        "squash.txt should be on main after squash merge"
    );

    // And main should have a single new commit (not a merge commit)
    let log = Command::new("git")
        .args(["log", "--oneline", "-3"])
        .current_dir(path)
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
    let (dir, _repo) = setup_test_repo();
    let path = dir.path();

    // Both branches create the same file with different content -> conflict
    run_git(path, &["checkout", "-b", "conflict-feature"]);
    std::fs::write(path.join("conflict.txt"), "feature version\n").unwrap();
    run_git(path, &["add", "conflict.txt"]);
    run_git(path, &["commit", "-m", "Feature adds conflict.txt"]);

    run_git(path, &["checkout", "main"]);
    std::fs::write(path.join("conflict.txt"), "main version\n").unwrap();
    run_git(path, &["add", "conflict.txt"]);
    run_git(path, &["commit", "-m", "Main adds conflict.txt"]);

    let results = operations::merge_branch(path, "conflict-feature", "main", false, false);
    assert_eq!(results.len(), 1);
    assert!(!results[0].success, "conflicting merge should fail");

    // Verify merge was aborted: repo must not be in mid-merge state
    let status = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(path)
        .output()
        .unwrap();
    let status_str = String::from_utf8_lossy(&status.stdout);
    assert!(
        !status_str.contains("UU"),
        "merge should have been aborted, no unresolved conflicts"
    );
}

// ===== Rebase Operation Tests =====

#[test]
fn test_rebase_branch_success() {
    let (dir, _repo) = setup_test_repo();
    let path = dir.path();

    // Feature branch: add a unique file
    run_git(path, &["checkout", "-b", "feature-rebase"]);
    std::fs::write(path.join("rebase-feature.txt"), "feature content\n").unwrap();
    run_git(path, &["add", "rebase-feature.txt"]);
    run_git(path, &["commit", "-m", "Feature commit"]);

    // Main gets a new commit (so rebase is non-trivial)
    run_git(path, &["checkout", "main"]);
    std::fs::write(path.join("main-update.txt"), "main update\n").unwrap();
    run_git(path, &["add", "main-update.txt"]);
    run_git(path, &["commit", "-m", "Main update"]);

    // Rebase feature onto main
    let results = operations::rebase_branch(path, "feature-rebase", "main", false);
    assert_eq!(results.len(), 1);
    assert!(
        results[0].success,
        "rebase should succeed: {}",
        results[0].message
    );

    // After rebase, feature should be 1 commit ahead of main
    let repo = Repository::open(path).unwrap();
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
    let (dir, _repo) = setup_test_repo();
    let path = dir.path();

    // Create a shared file on main
    std::fs::write(path.join("shared.txt"), "original\n").unwrap();
    run_git(path, &["add", "shared.txt"]);
    run_git(path, &["commit", "-m", "Add shared file"]);

    // Feature branch modifies shared.txt
    run_git(path, &["checkout", "-b", "rebase-conflict"]);
    std::fs::write(path.join("shared.txt"), "feature version\n").unwrap();
    run_git(path, &["add", "shared.txt"]);
    run_git(path, &["commit", "-m", "Feature modifies shared"]);

    // Main also modifies shared.txt (divergent history)
    run_git(path, &["checkout", "main"]);
    std::fs::write(path.join("shared.txt"), "main version\n").unwrap();
    run_git(path, &["add", "shared.txt"]);
    run_git(path, &["commit", "-m", "Main modifies shared"]);

    let results = operations::rebase_branch(path, "rebase-conflict", "main", false);
    assert_eq!(results.len(), 1);
    assert!(!results[0].success, "conflicting rebase should fail");

    // Rebase should be aborted: no ongoing rebase state
    let rebase_head = path.join(".git").join("REBASE_HEAD");
    assert!(
        !rebase_head.exists(),
        "REBASE_HEAD should not exist after abort"
    );
}

// ===== Remote Operations: push, pull, fast-forward, fetch-prune =====

#[test]
fn test_pull_branch_current() {
    let (local_dir, remote_dir, _repo) = setup_remote_test_repo();
    let path = local_dir.path();

    // Simulate another committer pushing to origin/main via a second clone
    let tmpdir2 = TempDir::new().unwrap();
    let clone2 = tmpdir2.path().join("clone2");
    Command::new("git")
        .args([
            "clone",
            remote_dir.path().to_str().unwrap(),
            clone2.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    run_git(&clone2, &["config", "user.name", "Other User"]);
    run_git(&clone2, &["config", "user.email", "other@example.com"]);
    std::fs::write(clone2.join("other.txt"), "other commit\n").unwrap();
    run_git(&clone2, &["add", "other.txt"]);
    run_git(&clone2, &["commit", "-m", "Other commit on main"]);
    run_git(&clone2, &["push", "origin", "main"]);

    // Pull in work_dir (main is current branch)
    let result = operations::pull_branch(path, "main", true);
    assert!(
        result.success,
        "pull_branch (current) should succeed: {}",
        result.message
    );

    // Verify the new file exists locally
    assert!(
        path.join("other.txt").exists(),
        "pulled file should exist after pull"
    );
}

#[test]
fn test_pull_branch_non_current() {
    let (local_dir, remote_dir, _repo) = setup_remote_test_repo();
    let path = local_dir.path();

    // Create and push a feature branch
    run_git(path, &["checkout", "-b", "pull-non-current"]);
    std::fs::write(path.join("pull-feature.txt"), "feature\n").unwrap();
    run_git(path, &["add", "pull-feature.txt"]);
    run_git(path, &["commit", "-m", "Feature commit"]);
    run_git(path, &["push", "-u", "origin", "pull-non-current"]);

    // Push another commit from a second clone
    let tmpdir2 = TempDir::new().unwrap();
    let clone2 = tmpdir2.path().join("clone2");
    Command::new("git")
        .args([
            "clone",
            remote_dir.path().to_str().unwrap(),
            clone2.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    run_git(&clone2, &["config", "user.name", "Other User"]);
    run_git(&clone2, &["config", "user.email", "other@example.com"]);
    run_git(&clone2, &["checkout", "pull-non-current"]);
    std::fs::write(clone2.join("extra.txt"), "extra\n").unwrap();
    run_git(&clone2, &["add", "extra.txt"]);
    run_git(&clone2, &["commit", "-m", "Extra commit"]);
    run_git(&clone2, &["push", "origin", "pull-non-current"]);

    // Fetch so work_dir knows about the remote update, then switch to main
    run_git(path, &["fetch", "origin"]);
    run_git(path, &["checkout", "main"]);

    let result = operations::pull_branch(path, "pull-non-current", false);
    assert!(
        result.success,
        "pull_branch (non-current) should succeed: {}",
        result.message
    );

    // The local branch tip should now be at "Extra commit"
    let repo = Repository::open(path).unwrap();
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
    let (local_dir, remote_dir, _repo) = setup_remote_test_repo();
    let path = local_dir.path();

    // Push a feature branch
    run_git(path, &["checkout", "-b", "ff-branch"]);
    std::fs::write(path.join("ff.txt"), "ff content\n").unwrap();
    run_git(path, &["add", "ff.txt"]);
    run_git(path, &["commit", "-m", "FF commit"]);
    run_git(path, &["push", "-u", "origin", "ff-branch"]);

    // Advance the remote via a second clone
    let tmpdir2 = TempDir::new().unwrap();
    let clone2 = tmpdir2.path().join("clone2");
    Command::new("git")
        .args([
            "clone",
            remote_dir.path().to_str().unwrap(),
            clone2.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    run_git(&clone2, &["config", "user.name", "Other User"]);
    run_git(&clone2, &["config", "user.email", "other@example.com"]);
    run_git(&clone2, &["checkout", "ff-branch"]);
    std::fs::write(clone2.join("ff2.txt"), "ff2 content\n").unwrap();
    run_git(&clone2, &["add", "ff2.txt"]);
    run_git(&clone2, &["commit", "-m", "FF commit 2"]);
    run_git(&clone2, &["push", "origin", "ff-branch"]);

    // Go back to main (ff-branch is not current)
    run_git(path, &["checkout", "main"]);

    // fast_forward fetches origin/ff-branch:ff-branch
    let result = operations::fast_forward(path, "ff-branch");
    assert!(
        result.success,
        "fast_forward should succeed: {}",
        result.message
    );

    // Verify the local branch was advanced
    let repo = Repository::open(path).unwrap();
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
    let (local_dir, remote_dir, _repo) = setup_remote_test_repo();
    let path = local_dir.path();

    // Push a branch so work_dir has a remote-tracking ref for it
    run_git(path, &["checkout", "-b", "prune-me"]);
    std::fs::write(path.join("prune.txt"), "prune\n").unwrap();
    run_git(path, &["add", "prune.txt"]);
    run_git(path, &["commit", "-m", "Prune commit"]);
    run_git(path, &["push", "-u", "origin", "prune-me"]);
    run_git(path, &["checkout", "main"]);

    // Delete the branch directly on the bare remote
    run_git(remote_dir.path(), &["branch", "-D", "prune-me"]);

    // Confirm origin/prune-me is still in local tracking refs before pruning
    let repo = Repository::open(path).unwrap();
    assert!(
        repo.find_branch("origin/prune-me", git2::BranchType::Remote)
            .is_ok(),
        "origin/prune-me should still exist in local refs before prune"
    );

    let result = operations::fetch_prune(path);
    assert!(
        result.success,
        "fetch_prune should succeed: {}",
        result.message
    );

    // After prune, stale tracking ref should be gone
    let repo2 = Repository::open(path).unwrap();
    assert!(
        repo2
            .find_branch("origin/prune-me", git2::BranchType::Remote)
            .is_err(),
        "origin/prune-me should be removed after fetch --prune"
    );
}

// ===== Remote Batch Delete Tests =====

#[test]
fn test_delete_remotes_batch_success() {
    let (local_dir, _remote_dir, _repo) = setup_remote_test_repo();
    let path = local_dir.path();

    // Create and push two branches
    for branch_name in &["batch-del-1", "batch-del-2"] {
        run_git(path, &["checkout", "-b", branch_name]);
        std::fs::write(path.join(format!("{}.txt", branch_name)), "content\n").unwrap();
        run_git(path, &["add", &format!("{}.txt", branch_name)]);
        run_git(path, &["commit", "-m", &format!("Add {}", branch_name)]);
        run_git(path, &["push", "-u", "origin", branch_name]);
        run_git(path, &["checkout", "main"]);
    }

    let names: Vec<String> = vec!["batch-del-1".to_string(), "batch-del-2".to_string()];
    let results = operations::delete_remotes_batch(path, &names);

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
    run_git(path, &["fetch", "--prune"]);
    let repo = Repository::open(path).unwrap();
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
    let (local_dir, _remote_dir, _repo) = setup_remote_test_repo();
    let path = local_dir.path();

    // Empty input should return empty results immediately
    let results = operations::delete_remotes_batch(path, &[]);
    assert!(
        results.is_empty(),
        "empty input should produce empty results"
    );
}

// ===== Worktree Operation Tests =====

#[test]
fn test_create_worktree_simple() {
    let (dir, _repo) = setup_test_repo();
    let path = dir.path();

    run_git(path, &["branch", "wt-feature"]);

    let result = operations::create_worktree(path, "wt-feature");
    assert!(
        result.success,
        "create_worktree should succeed: {}",
        result.message
    );

    let wt_path = path.join(".worktrees").join("wt-feature");
    assert!(wt_path.exists(), ".worktrees/wt-feature should be created");
    assert!(
        wt_path.join(".git").exists(),
        "worktree directory should be a valid git working tree"
    );
}

#[test]
fn test_create_worktree_sanitizes_slash() {
    let (dir, _repo) = setup_test_repo();
    let path = dir.path();

    // Branch names with slashes (e.g. "feature/foo") must be sanitized to "feature-foo"
    run_git(path, &["branch", "feature/slash-test"]);

    let result = operations::create_worktree(path, "feature/slash-test");
    assert!(
        result.success,
        "create_worktree with slash should succeed: {}",
        result.message
    );

    let wt_path = path.join(".worktrees").join("feature-slash-test");
    assert!(
        wt_path.exists(),
        ".worktrees/feature-slash-test should be created (slash -> dash)"
    );
}

#[test]
fn test_remove_worktree_clean() {
    let (dir, _repo) = setup_test_repo();
    let path = dir.path();

    run_git(path, &["branch", "wt-remove"]);
    run_git(
        path,
        &["worktree", "add", ".worktrees/wt-remove", "wt-remove"],
    );

    let wt_path = path.join(".worktrees").join("wt-remove");
    assert!(wt_path.exists(), "worktree should exist before removal");

    let result = operations::remove_worktree(path, &wt_path);
    assert!(
        result.success,
        "remove_worktree should succeed on clean worktree: {}",
        result.message
    );
    assert!(
        !wt_path.exists(),
        "worktree directory should be removed"
    );
}

#[test]
fn test_force_remove_worktree_dirty() {
    let (dir, _repo) = setup_test_repo();
    let path = dir.path();

    run_git(path, &["branch", "wt-dirty"]);
    run_git(
        path,
        &["worktree", "add", ".worktrees/wt-dirty", "wt-dirty"],
    );

    let wt_path = path.join(".worktrees").join("wt-dirty");

    // Create an uncommitted change in the worktree -- regular remove should fail
    std::fs::write(wt_path.join("dirty.txt"), "uncommitted change\n").unwrap();

    let result = operations::remove_worktree(path, &wt_path);
    assert!(
        !result.success,
        "remove_worktree should fail on dirty worktree"
    );

    // Force remove should succeed regardless
    let result = operations::force_remove_worktree(path, &wt_path);
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

// ===== Worktree Listing and Enrichment Tests =====

#[test]
fn test_list_worktrees_with_additional() {
    let (dir, _repo) = setup_test_repo();
    let path = dir.path();

    run_git(path, &["branch", "wt-list-test"]);
    run_git(
        path,
        &["worktree", "add", ".worktrees/wt-list-test", "wt-list-test"],
    );

    let worktrees = worktree::list_worktrees(path);
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
    run_git(path, &["worktree", "remove", ".worktrees/wt-list-test"]);
}

#[test]
fn test_enrich_worktrees_clean() {
    let (dir, _repo) = setup_test_repo();
    let path = dir.path();

    let worktrees = worktree::list_worktrees(path);
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
