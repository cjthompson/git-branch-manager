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
