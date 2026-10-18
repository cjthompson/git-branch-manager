use super::*;

#[test]
fn execute_action_worktree_remove_and_delete_branch_remote() {
    let base_tmp = tempfile::tempdir().expect("temp base dir");
    let base_dir = base_tmp.path();

    let remote_dir = base_dir.join("remote.git");
    std::fs::create_dir_all(&remote_dir).unwrap();
    run_git(&remote_dir, &["init", "--bare", "-b", "main"]);

    run_git(base_dir, &["clone", remote_dir.to_str().unwrap(), "work"]);
    let work_dir = base_dir.join("work");
    run_git(&work_dir, &["config", "user.name", "Test User"]);
    run_git(&work_dir, &["config", "user.email", "test@example.com"]);

    std::fs::write(work_dir.join("README.md"), "# Test\n").unwrap();
    run_git(&work_dir, &["add", "."]);
    run_git(&work_dir, &["commit", "-m", "Initial commit"]);
    run_git(&work_dir, &["push", "-u", "origin", "main"]);

    run_git(&work_dir, &["checkout", "-b", "wt-remote-branch"]);
    std::fs::write(work_dir.join("feature.txt"), "content\n").unwrap();
    run_git(&work_dir, &["add", "feature.txt"]);
    run_git(&work_dir, &["commit", "-m", "feature commit"]);
    run_git(&work_dir, &["push", "-u", "origin", "wt-remote-branch"]);
    run_git(&work_dir, &["checkout", "main"]);

    run_git(
        &work_dir,
        &[
            "worktree",
            "add",
            ".worktrees/wt-remote-branch",
            "wt-remote-branch",
        ],
    );
    let wt_path = work_dir.join(".worktrees").join("wt-remote-branch");
    assert!(wt_path.exists());

    let (prog_tx, _prog_rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let partial_delete_risk = Arc::new(AtomicBool::new(false));
    let results = execute_action_with_remote(
        BranchAction::WorktreeRemoveAndDeleteBranchRemote,
        &[wt_path.to_string_lossy().to_string()],
        &work_dir,
        "main",
        false,
        None,
        &prog_tx,
        &cancel,
        &partial_delete_risk,
    );

    assert!(results
        .iter()
        .any(|r| r.action == BranchAction::WorktreeRemove && r.success));
    assert!(results
        .iter()
        .any(|r| r.action == BranchAction::DeleteLocal && r.success));
    assert!(results
        .iter()
        .any(|r| r.action == BranchAction::DeleteRemoteBranch && r.success));
    assert!(!wt_path.exists());

    let repo = git2::Repository::open(&work_dir).unwrap();
    assert!(repo
        .find_branch("wt-remote-branch", git2::BranchType::Local)
        .is_err());

    run_git(&work_dir, &["fetch", "--prune"]);
    assert!(repo
        .find_branch("origin/wt-remote-branch", git2::BranchType::Remote)
        .is_err());
}

#[test]
fn execute_action_branch_name_cascade_remote_deletes_tracking_branch() {
    let base_tmp = tempfile::tempdir().expect("temp base dir");
    let base_dir = base_tmp.path();

    let remote_dir = base_dir.join("remote.git");
    std::fs::create_dir_all(&remote_dir).unwrap();
    run_git(&remote_dir, &["init", "--bare", "-b", "main"]);

    run_git(base_dir, &["clone", remote_dir.to_str().unwrap(), "work"]);
    let work_dir = base_dir.join("work");
    run_git(&work_dir, &["config", "user.name", "Test User"]);
    run_git(&work_dir, &["config", "user.email", "test@example.com"]);
    std::fs::write(work_dir.join("README.md"), "# Test\n").unwrap();
    run_git(&work_dir, &["add", "."]);
    run_git(&work_dir, &["commit", "-m", "Initial commit"]);
    run_git(&work_dir, &["push", "-u", "origin", "main"]);

    run_git(&work_dir, &["checkout", "-b", "branch-name-remote"]);
    std::fs::write(work_dir.join("feature.txt"), "content\n").unwrap();
    run_git(&work_dir, &["add", "feature.txt"]);
    run_git(&work_dir, &["commit", "-m", "feature commit"]);
    run_git(&work_dir, &["push", "-u", "origin", "branch-name-remote"]);
    run_git(&work_dir, &["checkout", "main"]);
    run_git(
        &work_dir,
        &[
            "worktree",
            "add",
            ".worktrees/branch-name-remote",
            "branch-name-remote",
        ],
    );
    let wt_path = work_dir.join(".worktrees/branch-name-remote");

    let (prog_tx, _prog_rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let partial_delete_risk = Arc::new(AtomicBool::new(false));
    let results = execute_action_with_remote(
        BranchAction::DeleteBranchAndRemoveWorktreeRemote,
        &["branch-name-remote".into()],
        &work_dir,
        "main",
        false,
        Some("origin"),
        &prog_tx,
        &cancel,
        &partial_delete_risk,
    );

    assert!(results
        .iter()
        .any(|r| r.action == BranchAction::WorktreeRemove && r.success));
    assert!(results
        .iter()
        .any(|r| r.action == BranchAction::DeleteLocal && r.success));
    assert!(results
        .iter()
        .any(|r| r.action == BranchAction::DeleteRemoteBranch && r.success));
    assert!(!wt_path.exists());

    let repo = git2::Repository::open(&work_dir).unwrap();
    assert!(repo
        .find_branch("branch-name-remote", git2::BranchType::Local)
        .is_err());
    run_git(&work_dir, &["fetch", "--prune"]);
    assert!(repo
        .find_branch("origin/branch-name-remote", git2::BranchType::Remote)
        .is_err());
}

#[test]
fn execute_action_remote_checkout_uses_the_selected_remote() {
    let base_tmp = tempfile::tempdir().expect("temp base dir");
    let base_dir = base_tmp.path();
    let remote_dir = base_dir.join("remote.git");
    std::fs::create_dir_all(&remote_dir).unwrap();
    run_git(&remote_dir, &["init", "--bare", "-b", "main"]);

    run_git(base_dir, &["clone", remote_dir.to_str().unwrap(), "work"]);
    let work_dir = base_dir.join("work");
    run_git(&work_dir, &["config", "user.name", "Test User"]);
    run_git(&work_dir, &["config", "user.email", "test@example.com"]);
    std::fs::write(work_dir.join("README.md"), "# Test\n").unwrap();
    run_git(&work_dir, &["add", "README.md"]);
    run_git(&work_dir, &["commit", "-m", "Initial commit"]);
    run_git(&work_dir, &["branch", "-M", "main"]);
    run_git(&work_dir, &["push", "origin", "main"]);
    run_git(&work_dir, &["remote", "rename", "origin", "upstream"]);

    run_git(&work_dir, &["checkout", "-b", "remote-checkout"]);
    std::fs::write(work_dir.join("feature.txt"), "content\n").unwrap();
    run_git(&work_dir, &["add", "feature.txt"]);
    run_git(&work_dir, &["commit", "-m", "feature commit"]);
    run_git(&work_dir, &["push", "-u", "upstream", "remote-checkout"]);
    run_git(&work_dir, &["checkout", "main"]);
    run_git(&work_dir, &["branch", "-D", "remote-checkout"]);
    run_git(&work_dir, &["fetch", "upstream"]);

    let (prog_tx, _prog_rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let partial_delete_risk = Arc::new(AtomicBool::new(false));
    let results = execute_action_with_remote(
        BranchAction::CheckoutRemote,
        &["remote-checkout".into()],
        &work_dir,
        "main",
        false,
        Some("upstream"),
        &prog_tx,
        &cancel,
        &partial_delete_risk,
    );

    assert!(results.iter().any(|result| result.success), "{results:?}");
    let repo = git2::Repository::open(&work_dir).unwrap();
    assert!(repo
        .find_branch("remote-checkout", git2::BranchType::Local)
        .is_ok());

    run_git(&work_dir, &["checkout", "main"]);
    let deletion_results = execute_action_with_remote(
        BranchAction::DeleteLocalAndRemote,
        &["remote-checkout".into()],
        &work_dir,
        "main",
        false,
        Some("upstream"),
        &prog_tx,
        &cancel,
        &partial_delete_risk,
    );
    assert!(
        deletion_results
            .iter()
            .any(|result| result.action == BranchAction::DeleteRemoteBranch && result.success),
        "{deletion_results:?}"
    );
    assert!(deletion_results
        .iter()
        .any(|result| result.action == BranchAction::DeleteLocal && result.success));
}
