use super::*;

#[test]
fn execute_cherry_pick_commit_uses_selected_historical_oid() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let dir = tmpdir.path();
    run_git(dir, &["init", "-b", "main"]);
    run_git(dir, &["config", "user.email", "test@example.com"]);
    run_git(dir, &["config", "user.name", "Test User"]);
    std::fs::write(dir.join("base.txt"), "base\n").unwrap();
    run_git(dir, &["add", "base.txt"]);
    run_git(dir, &["commit", "-m", "base"]);
    run_git(dir, &["checkout", "-b", "feature/history"]);
    std::fs::write(dir.join("first.txt"), "first\n").unwrap();
    run_git(dir, &["add", "first.txt"]);
    run_git(dir, &["commit", "-m", "first selected commit"]);
    let selected_oid = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir)
        .output()
        .unwrap();
    let selected_oid = String::from_utf8(selected_oid.stdout)
        .unwrap()
        .trim()
        .to_string();
    std::fs::write(dir.join("second.txt"), "second\n").unwrap();
    run_git(dir, &["add", "second.txt"]);
    run_git(dir, &["commit", "-m", "newer branch tip"]);
    run_git(dir, &["checkout", "main"]);

    let (prog_tx, _prog_rx) = mpsc::channel();
    let results = execute_action_with_remote(
        BranchAction::CherryPickCommit,
        std::slice::from_ref(&selected_oid),
        dir,
        "main",
        false,
        None,
        &prog_tx,
        &Arc::new(AtomicBool::new(false)),
        &Arc::new(AtomicBool::new(false)),
    );

    assert_eq!(results.len(), 1);
    assert!(results[0].success, "{results:?}");
    assert_eq!(results[0].branch_name, selected_oid);
    assert!(dir.join("first.txt").exists());
    assert!(!dir.join("second.txt").exists());
    let subject = std::process::Command::new("git")
        .args(["log", "-1", "--format=%s"])
        .current_dir(dir)
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8(subject.stdout).unwrap().trim(),
        "first selected commit"
    );
}

#[test]
fn execute_action_worktree_remove_and_delete_branch_local_only() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let dir = tmpdir.path();
    run_git(dir, &["init", "-b", "main"]);
    run_git(dir, &["config", "user.email", "test@example.com"]);
    run_git(dir, &["config", "user.name", "Test User"]);
    std::fs::write(dir.join("README.md"), "base\n").unwrap();
    run_git(dir, &["add", "README.md"]);
    run_git(dir, &["commit", "-m", "initial"]);

    run_git(dir, &["branch", "wt-branch-delete"]);
    run_git(
        dir,
        &[
            "worktree",
            "add",
            ".worktrees/wt-branch-delete",
            "wt-branch-delete",
        ],
    );
    let wt_path = dir.join(".worktrees").join("wt-branch-delete");
    assert!(wt_path.exists());

    let (prog_tx, _prog_rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let partial_delete_risk = Arc::new(AtomicBool::new(false));
    let results = execute_action_with_remote(
        BranchAction::WorktreeRemoveAndDeleteBranch,
        &[wt_path.to_string_lossy().to_string()],
        dir,
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
    assert!(!wt_path.exists());

    let repo = git2::Repository::open(dir).unwrap();
    assert!(repo
        .find_branch("wt-branch-delete", git2::BranchType::Local)
        .is_err());
}

#[test]
fn execute_action_delete_local_force_removes_unmerged_branch() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let dir = tmpdir.path();
    run_git(dir, &["init", "-b", "main"]);
    run_git(dir, &["config", "user.email", "test@example.com"]);
    run_git(dir, &["config", "user.name", "Test User"]);
    std::fs::write(dir.join("README.md"), "base\n").unwrap();
    run_git(dir, &["add", "README.md"]);
    run_git(dir, &["commit", "-m", "initial"]);
    run_git(dir, &["checkout", "-b", "force-delete"]);
    std::fs::write(dir.join("wip.txt"), "wip\n").unwrap();
    run_git(dir, &["add", "wip.txt"]);
    run_git(dir, &["commit", "-m", "wip"]);
    run_git(dir, &["checkout", "main"]);

    let (prog_tx, _prog_rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let partial_delete_risk = Arc::new(AtomicBool::new(false));
    let results = execute_action_with_remote(
        BranchAction::DeleteLocalForce,
        &["force-delete".into()],
        dir,
        "main",
        false,
        None,
        &prog_tx,
        &cancel,
        &partial_delete_risk,
    );

    assert_eq!(results.len(), 1, "force delete should produce one result");
    assert!(results[0].success, "{results:?}");
    assert_eq!(results[0].action, BranchAction::DeleteLocalForce);
    assert!(git2::Repository::open(dir)
        .unwrap()
        .find_branch("force-delete", git2::BranchType::Local)
        .is_err());
}

#[test]
fn execute_action_branch_cascade_resolves_worktree_by_branch_name() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let dir = tmpdir.path();
    run_git(dir, &["init", "-b", "main"]);
    run_git(dir, &["config", "user.email", "test@example.com"]);
    run_git(dir, &["config", "user.name", "Test User"]);
    std::fs::write(dir.join("README.md"), "base\n").unwrap();
    run_git(dir, &["add", "README.md"]);
    run_git(dir, &["commit", "-m", "initial"]);
    run_git(dir, &["branch", "cascade-clean"]);
    run_git(
        dir,
        &[
            "worktree",
            "add",
            ".worktrees/cascade-clean",
            "cascade-clean",
        ],
    );
    let wt_path = dir.join(".worktrees/cascade-clean");

    let (prog_tx, _prog_rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let partial_delete_risk = Arc::new(AtomicBool::new(false));
    let results = execute_action_with_remote(
        BranchAction::DeleteBranchAndRemoveWorktree,
        &["cascade-clean".into()],
        dir,
        "main",
        false,
        None,
        &prog_tx,
        &cancel,
        &partial_delete_risk,
    );

    assert!(
        results
            .iter()
            .any(|r| r.action == BranchAction::WorktreeRemove && r.success),
        "{results:?}"
    );
    assert!(
        results
            .iter()
            .any(|r| r.action == BranchAction::DeleteLocal && r.success),
        "{results:?}"
    );
    assert!(!wt_path.exists());
    assert!(git2::Repository::open(dir)
        .unwrap()
        .find_branch("cascade-clean", git2::BranchType::Local)
        .is_err());
}

#[test]
fn execute_action_force_cascade_removes_dirty_worktree_and_branch() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let dir = tmpdir.path();
    run_git(dir, &["init", "-b", "main"]);
    run_git(dir, &["config", "user.email", "test@example.com"]);
    run_git(dir, &["config", "user.name", "Test User"]);
    std::fs::write(dir.join("README.md"), "base\n").unwrap();
    run_git(dir, &["add", "README.md"]);
    run_git(dir, &["commit", "-m", "initial"]);
    run_git(dir, &["branch", "cascade-dirty"]);
    run_git(
        dir,
        &[
            "worktree",
            "add",
            ".worktrees/cascade-dirty",
            "cascade-dirty",
        ],
    );
    let wt_path = dir.join(".worktrees/cascade-dirty");
    std::fs::write(wt_path.join("untracked.txt"), "discard me\n").unwrap();

    let (prog_tx, _prog_rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let partial_delete_risk = Arc::new(AtomicBool::new(false));
    let results = execute_action_with_remote(
        BranchAction::DeleteBranchAndRemoveWorktreeForce,
        &["cascade-dirty".into()],
        dir,
        "main",
        false,
        None,
        &prog_tx,
        &cancel,
        &partial_delete_risk,
    );

    assert!(
        results
            .iter()
            .any(|r| r.action == BranchAction::WorktreeForceRemove && r.success),
        "{results:?}"
    );
    assert!(
        results
            .iter()
            .any(|r| r.action == BranchAction::DeleteLocalForce && r.success),
        "{results:?}"
    );
    assert!(!wt_path.exists());
    assert!(git2::Repository::open(dir)
        .unwrap()
        .find_branch("cascade-dirty", git2::BranchType::Local)
        .is_err());
}

#[test]
fn execute_action_cascade_rejects_primary_worktree() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let dir = tmpdir.path();
    run_git(dir, &["init", "-b", "main"]);
    run_git(dir, &["config", "user.email", "test@example.com"]);
    run_git(dir, &["config", "user.name", "Test User"]);
    std::fs::write(dir.join("README.md"), "base\n").unwrap();
    run_git(dir, &["add", "README.md"]);
    run_git(dir, &["commit", "-m", "initial"]);

    let (prog_tx, _prog_rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let partial_delete_risk = Arc::new(AtomicBool::new(false));
    let results = execute_action_with_remote(
        BranchAction::DeleteBranchAndRemoveWorktreeForce,
        &["main".into()],
        dir,
        "main",
        false,
        None,
        &prog_tx,
        &cancel,
        &partial_delete_risk,
    );

    assert!(results.iter().any(|result| matches!(
        &result.failure,
        Some(FailureCause::CheckedOutInWorktree { is_main: true, .. })
    )));
}
