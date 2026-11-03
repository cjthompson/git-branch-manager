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

fn init_repo() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().expect("temp repo");
    let dir = tmp.path();
    run_git(dir, &["init", "-b", "main"]);
    run_git(dir, &["config", "user.email", "test@example.com"]);
    run_git(dir, &["config", "user.name", "Test User"]);
    std::fs::write(dir.join("README.md"), "base\n").unwrap();
    run_git(dir, &["add", "README.md"]);
    run_git(dir, &["commit", "-m", "initial"]);
    tmp
}

fn git_stdout(dir: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap_or_else(|e| panic!("failed to run git {:?}: {}", args, e));
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

fn commit_file(dir: &Path, file: &str, content: &str, message: &str) {
    std::fs::write(dir.join(file), content).unwrap();
    run_git(dir, &["add", file]);
    run_git(dir, &["commit", "-m", message]);
}

/// Runs one action without a remote and returns its results and progress updates.
fn run_action(
    action: BranchAction,
    targets: &[&str],
    repo: &Path,
    base: &str,
    needs_stash: bool,
    cancelled: bool,
) -> (Vec<OperationResult>, Vec<ProgressUpdate>) {
    let (prog_tx, prog_rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(cancelled));
    let partial_delete_risk = Arc::new(AtomicBool::new(false));
    let items: Vec<String> = targets.iter().map(|s| s.to_string()).collect();
    let results = execute_action_with_remote(
        action,
        &items,
        repo,
        base,
        needs_stash,
        None,
        &prog_tx,
        &cancel,
        &partial_delete_risk,
    );
    drop(prog_tx);
    (results, prog_rx.try_iter().collect())
}

fn repo_state(dir: &Path) -> git2::RepositoryState {
    git2::Repository::open(dir).unwrap().state()
}

fn local_branch_exists(dir: &Path, name: &str) -> bool {
    git2::Repository::open(dir)
        .unwrap()
        .find_branch(name, git2::BranchType::Local)
        .is_ok()
}

#[test]
fn execute_checkout_switches_to_local_branch() {
    let tmp = init_repo();
    let dir = tmp.path();
    run_git(dir, &["branch", "feat"]);

    let (results, _) = run_action(BranchAction::Checkout, &["feat"], dir, "main", false, false);

    assert_eq!(results.len(), 1);
    assert!(results[0].success, "{results:?}");
    assert_eq!(results[0].branch_name, "feat");
    assert_eq!(results[0].message, "Checked out feat");
    assert_eq!(
        git_stdout(dir, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "feat"
    );
}

#[test]
fn execute_checkout_reports_open_failure() {
    let tmp = tempfile::tempdir().expect("temp base dir");
    let missing = tmp.path().join("missing");

    let (results, _) = run_action(
        BranchAction::Checkout,
        &["feat"],
        &missing,
        "main",
        false,
        false,
    );

    assert_eq!(results.len(), 1);
    assert!(!results[0].success);
    assert_eq!(results[0].branch_name, "feat");
    assert!(results[0].message.starts_with("Failed to open repo"));
    assert!(results[0].failure.is_none());
}

#[test]
fn execute_checkout_unknown_branch_fails() {
    let tmp = init_repo();
    let dir = tmp.path();

    let (results, _) = run_action(BranchAction::Checkout, &["nope"], dir, "main", false, false);

    assert_eq!(results.len(), 1);
    assert!(!results[0].success);
    assert!(results[0].message.contains("did not match"), "{results:?}");
}

#[test]
fn execute_checkout_without_targets_is_a_noop() {
    let tmp = init_repo();
    let dir = tmp.path();

    let (results, progress) = run_action(BranchAction::Checkout, &[], dir, "main", false, false);

    assert!(results.is_empty());
    assert_eq!(progress.len(), 1);
    assert_eq!(progress[0].completed, 0);
    assert_eq!(progress[0].total, 0);
    assert_eq!(progress[0].current_item, "Done");
    assert_eq!(
        git_stdout(dir, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "main"
    );
}

#[test]
fn execute_checkout_with_stash_keeps_dirty_change() {
    let tmp = init_repo();
    let dir = tmp.path();
    run_git(dir, &["branch", "feat"]);
    std::fs::write(dir.join("README.md"), "dirty\n").unwrap();

    let (results, _) = run_action(BranchAction::Checkout, &["feat"], dir, "main", true, false);

    assert_eq!(results.len(), 1);
    assert!(results[0].success, "{results:?}");
    assert_eq!(
        std::fs::read_to_string(dir.join("README.md")).unwrap(),
        "dirty\n"
    );
}

#[test]
fn execute_delete_local_cancelled_keeps_branch() {
    let tmp = init_repo();
    let dir = tmp.path();
    run_git(dir, &["branch", "gone"]);

    let (results, _) = run_action(
        BranchAction::DeleteLocal,
        &["gone"],
        dir,
        "main",
        false,
        true,
    );

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].branch_name, "");
    assert_eq!(results[0].message, "Cancelled by user");
    assert!(!results[0].success);
    assert!(local_branch_exists(dir, "gone"));
}

#[test]
fn execute_delete_local_removes_merged_branch_and_reports_progress() {
    let tmp = init_repo();
    let dir = tmp.path();
    run_git(dir, &["branch", "merged"]);

    let (results, progress) = run_action(
        BranchAction::DeleteLocal,
        &["merged"],
        dir,
        "main",
        false,
        false,
    );

    assert_eq!(results.len(), 1, "{results:?}");
    assert!(results[0].success, "{results:?}");
    assert_eq!(results[0].action, BranchAction::DeleteLocal);
    assert_eq!(results[0].branch_name, "merged");
    assert!(!local_branch_exists(dir, "merged"));
    assert_eq!(progress.len(), 2, "{progress:?}");
    assert_eq!(progress[0].completed, 0);
    assert_eq!(progress[0].total, 1);
    assert_eq!(progress[0].current_item, "merged");
    assert_eq!(progress[1].completed, 1);
    assert_eq!(progress[1].total, 1);
    assert_eq!(progress[1].current_item, "Done");
}

#[test]
fn execute_push_cancelled_before_start_sends_only_done() {
    let tmp = init_repo();
    let dir = tmp.path();

    let (results, progress) = run_action(BranchAction::Push, &["a", "b"], dir, "main", false, true);

    assert!(results.is_empty());
    assert_eq!(progress.len(), 1);
    assert_eq!(progress[0].completed, 0);
    assert_eq!(progress[0].total, 2);
    assert_eq!(progress[0].current_item, "Done");
}

#[test]
fn execute_merge_fast_forwards_feature_into_base() {
    let tmp = init_repo();
    let dir = tmp.path();
    run_git(dir, &["checkout", "-b", "feat"]);
    commit_file(dir, "feat.txt", "feat\n", "feat commit");
    run_git(dir, &["checkout", "main"]);

    let (results, _) = run_action(BranchAction::Merge, &["feat"], dir, "main", false, false);

    assert_eq!(results.len(), 1);
    assert!(results[0].success, "{results:?}");
    assert_eq!(results[0].action, BranchAction::Merge);
    assert_eq!(results[0].message, "Merged feat into main");
    assert!(dir.join("feat.txt").exists());
    assert_eq!(repo_state(dir), git2::RepositoryState::Clean);
}

#[test]
fn execute_merge_conflict_aborts_and_leaves_clean_state() {
    let tmp = init_repo();
    let dir = tmp.path();
    run_git(dir, &["checkout", "-b", "feat"]);
    commit_file(dir, "README.md", "feat\n", "feat edit");
    run_git(dir, &["checkout", "main"]);
    commit_file(dir, "README.md", "main\n", "main edit");

    let (results, _) = run_action(BranchAction::Merge, &["feat"], dir, "main", false, false);

    assert!(
        results
            .iter()
            .any(|r| !r.success && r.message == "Merge conflict \u{2014} aborted"),
        "{results:?}"
    );
    assert_eq!(repo_state(dir), git2::RepositoryState::Clean);
}

#[test]
fn execute_squash_merge_creates_single_commit_on_base() {
    let tmp = init_repo();
    let dir = tmp.path();
    run_git(dir, &["checkout", "-b", "feat"]);
    commit_file(dir, "feat1.txt", "one\n", "feat one");
    commit_file(dir, "feat2.txt", "two\n", "feat two");
    run_git(dir, &["checkout", "main"]);
    let before = git_stdout(dir, &["rev-list", "--count", "main"]);

    let (results, _) = run_action(
        BranchAction::SquashMerge,
        &["feat"],
        dir,
        "main",
        false,
        false,
    );

    assert_eq!(results.len(), 1, "{results:?}");
    assert!(results[0].success, "{results:?}");
    assert_eq!(results[0].action, BranchAction::SquashMerge);
    assert_eq!(results[0].message, "Merged feat into main");
    let after = git_stdout(dir, &["rev-list", "--count", "main"]);
    assert_eq!(before, "1");
    assert_eq!(after, "2");
    let parents = git_stdout(dir, &["rev-list", "--parents", "-n", "1", "main"]);
    assert_eq!(parents.split_whitespace().count(), 2);
    assert!(dir.join("feat1.txt").exists());
    assert!(dir.join("feat2.txt").exists());
}

#[test]
fn execute_squash_merge_conflict_reports_merge_conflict() {
    let tmp = init_repo();
    let dir = tmp.path();
    run_git(dir, &["checkout", "-b", "feat"]);
    commit_file(dir, "README.md", "feat\n", "feat edit");
    run_git(dir, &["checkout", "main"]);
    commit_file(dir, "README.md", "main\n", "main edit");

    let (results, _) = run_action(
        BranchAction::SquashMerge,
        &["feat"],
        dir,
        "main",
        false,
        false,
    );

    assert!(
        results
            .iter()
            .any(|r| !r.success && r.message.contains("Merge conflict")),
        "{results:?}"
    );
}

#[test]
fn execute_rebase_replays_feature_onto_base() {
    let tmp = init_repo();
    let dir = tmp.path();
    run_git(dir, &["checkout", "-b", "feat"]);
    commit_file(dir, "feat.txt", "feat\n", "feat commit");
    run_git(dir, &["checkout", "main"]);
    commit_file(dir, "main.txt", "main\n", "main commit");
    let main_tip = git_stdout(dir, &["rev-parse", "main"]);

    let (results, _) = run_action(BranchAction::Rebase, &["feat"], dir, "main", false, false);

    assert_eq!(results.len(), 1, "{results:?}");
    assert!(results[0].success, "{results:?}");
    assert_eq!(results[0].message, "Rebased feat onto main");
    assert_eq!(
        git_stdout(dir, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "feat"
    );
    assert_eq!(git_stdout(dir, &["rev-parse", "feat^"]), main_tip);
}

#[test]
fn execute_rebase_conflict_aborts_and_keeps_feature_tip() {
    let tmp = init_repo();
    let dir = tmp.path();
    run_git(dir, &["checkout", "-b", "feat"]);
    commit_file(dir, "README.md", "feat\n", "feat edit");
    let feat_tip = git_stdout(dir, &["rev-parse", "feat"]);
    run_git(dir, &["checkout", "main"]);
    commit_file(dir, "README.md", "main\n", "main edit");

    let (results, _) = run_action(BranchAction::Rebase, &["feat"], dir, "main", false, false);

    assert!(
        results
            .iter()
            .any(|r| !r.success && r.message == "Rebase conflict \u{2014} aborted"),
        "{results:?}"
    );
    assert_eq!(git_stdout(dir, &["rev-parse", "feat"]), feat_tip);
    assert_eq!(repo_state(dir), git2::RepositoryState::Clean);
}

#[test]
fn execute_worktree_creates_checkout_under_dot_worktrees() {
    let tmp = init_repo();
    let dir = tmp.path();
    run_git(dir, &["branch", "feature/wt"]);

    let (results, _) = run_action(
        BranchAction::Worktree,
        &["feature/wt"],
        dir,
        "main",
        false,
        false,
    );

    assert_eq!(results.len(), 1, "{results:?}");
    assert!(results[0].success, "{results:?}");
    assert_eq!(results[0].action, BranchAction::Worktree);
    assert!(results[0].message.starts_with("Created worktree at"));
    assert!(dir.join(".worktrees").join("feature-wt").exists());
}

#[test]
fn execute_worktree_unknown_branch_fails() {
    let tmp = init_repo();
    let dir = tmp.path();

    let (results, _) = run_action(BranchAction::Worktree, &["nope"], dir, "main", false, false);

    assert_eq!(results.len(), 1);
    assert!(!results[0].success);
}

#[test]
fn execute_cherry_pick_local_applies_feature_commits() {
    let tmp = init_repo();
    let dir = tmp.path();
    run_git(dir, &["checkout", "-b", "feat"]);
    commit_file(dir, "feat.txt", "feat\n", "feat commit");
    run_git(dir, &["checkout", "main"]);

    let (results, _) = run_action(
        BranchAction::CherryPickLocal,
        &["feat"],
        dir,
        "main",
        false,
        false,
    );

    assert_eq!(results.len(), 1, "{results:?}");
    assert!(results[0].success, "{results:?}");
    assert_eq!(results[0].branch_name, "feat");
    assert_eq!(results[0].message, "Cherry-picked refs/heads/feat");
    assert!(dir.join("feat.txt").exists());
}

#[test]
fn execute_cherry_pick_local_conflict_aborts_and_leaves_clean_state() {
    let tmp = init_repo();
    let dir = tmp.path();
    run_git(dir, &["checkout", "-b", "feat"]);
    commit_file(dir, "README.md", "feat\n", "feat edit");
    run_git(dir, &["checkout", "main"]);
    commit_file(dir, "README.md", "main\n", "main edit");

    let (results, _) = run_action(
        BranchAction::CherryPickLocal,
        &["feat"],
        dir,
        "main",
        false,
        false,
    );

    assert!(
        results
            .iter()
            .any(|r| !r.success && r.message == "Cherry-pick conflict \u{2014} aborted"),
        "{results:?}"
    );
    assert_eq!(repo_state(dir), git2::RepositoryState::Clean);
}

/// Adds a linked worktree on a new branch, returning its path as a string.
fn add_worktree(dir: &Path, name: &str) -> String {
    let rel = format!(".worktrees/{name}");
    run_git(dir, &["worktree", "add", &rel, "-b", name]);
    dir.join(&rel).to_string_lossy().to_string()
}

#[test]
fn execute_worktree_remove_clean_worktree() {
    let tmp = init_repo();
    let dir = tmp.path();
    let wt = add_worktree(dir, "wt-clean");
    assert!(Path::new(&wt).exists());

    let (results, _) = run_action(
        BranchAction::WorktreeRemove,
        &[wt.as_str()],
        dir,
        "main",
        false,
        false,
    );

    assert_eq!(results.len(), 1, "{results:?}");
    assert!(results[0].success, "{results:?}");
    assert_eq!(results[0].action, BranchAction::WorktreeRemove);
    assert_eq!(results[0].branch_name, wt);
    assert!(!Path::new(&wt).exists());
}

#[test]
fn execute_worktree_remove_refuses_dirty_worktree() {
    let tmp = init_repo();
    let dir = tmp.path();
    let wt = add_worktree(dir, "wt-dirty");
    std::fs::write(Path::new(&wt).join("README.md"), "dirty\n").unwrap();

    let (results, _) = run_action(
        BranchAction::WorktreeRemove,
        &[wt.as_str()],
        dir,
        "main",
        false,
        false,
    );

    assert_eq!(results.len(), 1, "{results:?}");
    assert!(!results[0].success);
    assert!(
        results[0].message.contains("uncommitted changes"),
        "{results:?}"
    );
    assert!(Path::new(&wt).exists());
}

#[test]
fn execute_worktree_force_remove_removes_dirty_worktree() {
    let tmp = init_repo();
    let dir = tmp.path();
    let wt = add_worktree(dir, "wt-force");
    std::fs::write(Path::new(&wt).join("README.md"), "dirty\n").unwrap();

    let (results, _) = run_action(
        BranchAction::WorktreeForceRemove,
        &[wt.as_str()],
        dir,
        "main",
        false,
        false,
    );

    assert_eq!(results.len(), 1, "{results:?}");
    assert!(results[0].success, "{results:?}");
    assert_eq!(results[0].action, BranchAction::WorktreeForceRemove);
    assert!(!Path::new(&wt).exists());
}

#[test]
fn execute_worktree_remove_refuses_main_worktree() {
    let tmp = init_repo();
    let dir = tmp.path();
    let main_path = dir.to_string_lossy().to_string();

    let (results, _) = run_action(
        BranchAction::WorktreeRemove,
        &[main_path.as_str()],
        dir,
        "main",
        false,
        false,
    );

    assert_eq!(results.len(), 1, "{results:?}");
    assert!(!results[0].success);
    assert_eq!(results[0].message, "Refusing to remove the main worktree");
    assert!(dir.exists());
}

#[test]
fn execute_worktree_remove_cancelled_keeps_worktree() {
    let tmp = init_repo();
    let dir = tmp.path();
    let wt = add_worktree(dir, "wt-cancel");

    let (results, _) = run_action(
        BranchAction::WorktreeRemove,
        &[wt.as_str()],
        dir,
        "main",
        false,
        true,
    );

    assert!(results.is_empty(), "{results:?}");
    assert!(Path::new(&wt).exists());
}

#[test]
fn execute_worktree_remove_and_delete_branch_reports_open_failure() {
    let tmp = tempfile::tempdir().expect("temp base dir");
    let missing = tmp.path().join("missing");

    let (results, _) = run_action(
        BranchAction::WorktreeRemoveAndDeleteBranch,
        &["/x"],
        &missing,
        "main",
        false,
        false,
    );

    assert_eq!(results.len(), 1);
    assert!(!results[0].success);
    assert_eq!(results[0].branch_name, "");
    assert_eq!(
        results[0].action,
        BranchAction::WorktreeRemoveAndDeleteBranch
    );
    assert!(results[0].message.starts_with("Failed to open repo"));
}

#[test]
fn execute_delete_tag_removes_local_tag() {
    let tmp = init_repo();
    let dir = tmp.path();
    run_git(dir, &["tag", "v1"]);

    let (results, _) = run_action(BranchAction::DeleteTag, &["v1"], dir, "main", false, false);

    assert_eq!(results.len(), 1, "{results:?}");
    assert!(results[0].success, "{results:?}");
    assert_eq!(results[0].action, BranchAction::DeleteTag);
    assert_eq!(results[0].message, "Deleted tag v1");
    assert!(git2::Repository::open(dir)
        .unwrap()
        .find_reference("refs/tags/v1")
        .is_err());
}

#[test]
fn execute_delete_tag_missing_reports_failure() {
    let tmp = init_repo();
    let dir = tmp.path();

    let (results, _) = run_action(
        BranchAction::DeleteTag,
        &["v-missing"],
        dir,
        "main",
        false,
        false,
    );

    assert_eq!(results.len(), 1, "{results:?}");
    assert!(!results[0].success);
    assert_eq!(results[0].branch_name, "v-missing");
    assert!(results[0].message.starts_with("Failed: "), "{results:?}");
}

#[test]
fn execute_delete_tag_reports_open_failure() {
    let tmp = tempfile::tempdir().expect("temp base dir");
    let missing = tmp.path().join("missing");

    let (results, _) = run_action(
        BranchAction::DeleteTag,
        &["v1"],
        &missing,
        "main",
        false,
        false,
    );

    assert_eq!(results.len(), 1);
    assert!(!results[0].success);
    assert_eq!(results[0].branch_name, "");
    assert!(results[0].message.starts_with("Failed to open repo"));
}

#[test]
fn execute_no_op_actions_send_only_done_progress() {
    let tmp = init_repo();
    let dir = tmp.path();

    for action in [
        BranchAction::ViewRemotePR,
        BranchAction::JumpToGraph,
        BranchAction::CheckPrimaryBranchCode,
    ] {
        let (results, progress) = run_action(action, &["main"], dir, "main", false, false);

        assert!(results.is_empty(), "{action:?}: {results:?}");
        assert_eq!(progress.len(), 1, "{action:?}");
        assert_eq!(progress[0].completed, 0);
        assert_eq!(progress[0].total, 1);
        assert_eq!(progress[0].current_item, "Done");
    }
}
