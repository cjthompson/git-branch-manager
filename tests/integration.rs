use git2::Repository;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

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

#[allow(dead_code)]
fn setup_remote_test_repo() -> (TempDir, TempDir, Repository) {
    // Create bare remote
    let remote_dir = TempDir::new().unwrap();
    run_git(remote_dir.path(), &["init", "--bare"]);

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
