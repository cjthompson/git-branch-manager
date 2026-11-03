use super::*;

/// Bare origin plus a `work` clone with one commit on `main`, pushed.
fn origin_and_work() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let tmp = tempfile::tempdir().expect("temp dir");
    let base = tmp.path();
    let remote_dir = base.join("remote.git");
    std::fs::create_dir_all(&remote_dir).unwrap();
    run_git(&remote_dir, &["init", "--bare", "-b", "main"]);
    run_git(base, &["clone", remote_dir.to_str().unwrap(), "work"]);
    let work_dir = base.join("work");
    run_git(&work_dir, &["config", "user.name", "Test User"]);
    run_git(&work_dir, &["config", "user.email", "test@example.com"]);
    std::fs::write(work_dir.join("README.md"), "# Test\n").unwrap();
    run_git(&work_dir, &["add", "."]);
    run_git(&work_dir, &["commit", "-m", "Initial commit"]);
    run_git(&work_dir, &["push", "-u", "origin", "main"]);
    (tmp, remote_dir, work_dir)
}

fn has_branch(dir: &Path, name: &str) -> bool {
    std::process::Command::new("git")
        .args(["show-ref", "--verify", "--quiet"])
        .arg(format!("refs/heads/{name}"))
        .current_dir(dir)
        .status()
        .expect("run git")
        .success()
}

fn run_delete_remote_and_local(
    work_dir: &Path,
    name: &str,
    cancelled: bool,
) -> Vec<OperationResult> {
    let (prog_tx, _prog_rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(cancelled));
    let partial_delete_risk = Arc::new(AtomicBool::new(false));
    execute_action_with_remote(
        BranchAction::DeleteRemoteAndLocal,
        &[name.to_string()],
        work_dir,
        "main",
        false,
        None,
        &prog_tx,
        &cancel,
        &partial_delete_risk,
    )
}

#[test]
fn delete_remote_and_local_removes_local_then_remote_for_merged_branch() {
    let (_tmp, remote_dir, work_dir) = origin_and_work();
    run_git(&work_dir, &["branch", "gone"]);
    run_git(&work_dir, &["push", "origin", "gone"]);

    let results = run_delete_remote_and_local(&work_dir, "gone", false);

    assert_eq!(results.len(), 2, "{results:?}");
    assert_eq!(results[0].action, BranchAction::DeleteRemoteAndLocal);
    assert!(results[0].success);
    assert_eq!(results[0].message, "Deleted gone");
    assert_eq!(results[1].action, BranchAction::DeleteRemoteBranch);
    assert!(results[1].success);
    assert!(!has_branch(&work_dir, "gone"));
    assert!(!has_branch(&remote_dir, "gone"));
}

#[test]
fn delete_remote_and_local_leaves_remote_untouched_when_local_delete_fails() {
    let (_tmp, remote_dir, work_dir) = origin_and_work();
    run_git(&work_dir, &["checkout", "-b", "wip"]);
    std::fs::write(work_dir.join("wip.txt"), "wip\n").unwrap();
    run_git(&work_dir, &["add", "wip.txt"]);
    run_git(&work_dir, &["commit", "-m", "wip commit"]);
    run_git(&work_dir, &["push", "origin", "wip"]);
    run_git(&work_dir, &["checkout", "main"]);

    let results = run_delete_remote_and_local(&work_dir, "wip", false);

    assert_eq!(results.len(), 1, "{results:?}");
    assert_eq!(results[0].action, BranchAction::DeleteRemoteAndLocal);
    assert!(!results[0].success);
    assert_eq!(results[0].failure, Some(FailureCause::NotMerged));
    assert!(has_branch(&work_dir, "wip"));
    assert!(has_branch(&remote_dir, "wip"));
}

#[test]
fn delete_remote_and_local_leaves_remote_untouched_for_current_branch() {
    let (_tmp, remote_dir, work_dir) = origin_and_work();

    let results = run_delete_remote_and_local(&work_dir, "main", false);

    assert_eq!(results.len(), 1, "{results:?}");
    assert_eq!(results[0].action, BranchAction::DeleteRemoteAndLocal);
    assert!(!results[0].success);
    assert!(has_branch(&remote_dir, "main"));
}

#[test]
fn delete_remote_and_local_cancelled_before_start_touches_nothing() {
    let (_tmp, remote_dir, work_dir) = origin_and_work();
    run_git(&work_dir, &["branch", "gone"]);
    run_git(&work_dir, &["push", "origin", "gone"]);

    let results = run_delete_remote_and_local(&work_dir, "gone", true);

    assert_eq!(results.len(), 1, "{results:?}");
    assert!(!results[0].success);
    assert_eq!(results[0].message, "Cancelled by user");
    assert!(has_branch(&work_dir, "gone"));
    assert!(has_branch(&remote_dir, "gone"));
}
