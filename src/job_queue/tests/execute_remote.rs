use super::*;
use std::path::PathBuf;

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
    assert!(
        bare_ref(&remote_dir, "refs/heads/remote-checkout").is_none(),
        "selected remote branch should be deleted: {deletion_results:?}"
    );
}

/// Creates a bare origin and a clone with `main` and `feat` (one commit) pushed.
/// Returns the base tempdir (keep alive), the clone path, and the bare path.
fn origin_and_work() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let base_tmp = tempfile::tempdir().expect("temp base dir");
    let base_dir = base_tmp.path();
    let bare = base_dir.join("remote.git");
    std::fs::create_dir_all(&bare).unwrap();
    run_git(&bare, &["init", "--bare", "-b", "main"]);

    run_git(base_dir, &["clone", bare.to_str().unwrap(), "work"]);
    let work = base_dir.join("work");
    run_git(&work, &["config", "user.name", "Test User"]);
    run_git(&work, &["config", "user.email", "test@example.com"]);
    std::fs::write(work.join("README.md"), "# Test\n").unwrap();
    run_git(&work, &["add", "README.md"]);
    run_git(&work, &["commit", "-m", "initial"]);
    run_git(&work, &["push", "-u", "origin", "main"]);

    run_git(&work, &["checkout", "-b", "feat"]);
    std::fs::write(work.join("feat.txt"), "feat\n").unwrap();
    run_git(&work, &["add", "feat.txt"]);
    run_git(&work, &["commit", "-m", "feat commit"]);
    run_git(&work, &["push", "-u", "origin", "feat"]);
    run_git(&work, &["checkout", "main"]);
    (base_tmp, work, bare)
}

fn run_remote_action(
    action: BranchAction,
    targets: &[&str],
    repo: &Path,
    remote: Option<&str>,
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
        "main",
        false,
        remote,
        &prog_tx,
        &cancel,
        &partial_delete_risk,
    );
    drop(prog_tx);
    (results, prog_rx.try_iter().collect())
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

fn bare_ref(bare: &Path, name: &str) -> Option<git2::Oid> {
    git2::Repository::open(bare)
        .unwrap()
        .refname_to_id(name)
        .ok()
}

fn local_ref_exists(dir: &Path, name: &str) -> bool {
    git2::Repository::open(dir)
        .unwrap()
        .find_reference(name)
        .is_ok()
}

fn repo_state(dir: &Path) -> git2::RepositoryState {
    git2::Repository::open(dir).unwrap().state()
}

/// Pushes one more commit to feat, then moves local feat back one commit so origin is ahead.
fn make_local_feat_behind_origin(work: &Path) {
    run_git(work, &["checkout", "feat"]);
    std::fs::write(work.join("feat2.txt"), "feat two\n").unwrap();
    run_git(work, &["add", "feat2.txt"]);
    run_git(work, &["commit", "-m", "feat two"]);
    run_git(work, &["push", "origin", "feat"]);
    run_git(work, &["checkout", "main"]);
    run_git(work, &["branch", "-f", "feat", "feat~1"]);
}

/// Makes feat edit README.md and pushes it, then edits README.md differently on main.
fn conflicting_feat_on_origin(work: &Path) {
    run_git(work, &["checkout", "feat"]);
    std::fs::write(work.join("README.md"), "feat edit\n").unwrap();
    run_git(work, &["add", "README.md"]);
    run_git(work, &["commit", "-m", "feat edit"]);
    run_git(work, &["push", "origin", "feat"]);
    run_git(work, &["checkout", "main"]);
    std::fs::write(work.join("README.md"), "main edit\n").unwrap();
    run_git(work, &["add", "README.md"]);
    run_git(work, &["commit", "-m", "main edit"]);
}

#[test]
fn execute_fast_forward_moves_local_branch_to_origin() {
    let (_base, work, bare) = origin_and_work();
    make_local_feat_behind_origin(&work);
    let origin_feat = bare_ref(&bare, "refs/heads/feat").unwrap();

    let (results, _) = run_remote_action(BranchAction::FastForward, &["feat"], &work, None, false);

    assert_eq!(results.len(), 1, "{results:?}");
    assert!(results[0].success, "{results:?}");
    assert_eq!(results[0].action, BranchAction::FastForward);
    assert_eq!(results[0].message, "Fast-forwarded feat");
    assert_eq!(
        git_stdout(&work, &["rev-parse", "feat"]),
        origin_feat.to_string()
    );
}

#[test]
fn execute_fast_forward_rejects_local_commits_ahead_of_origin() {
    let (_base, work, _bare) = origin_and_work();
    run_git(&work, &["checkout", "feat"]);
    std::fs::write(work.join("local.txt"), "local\n").unwrap();
    run_git(&work, &["add", "local.txt"]);
    run_git(&work, &["commit", "-m", "local only"]);
    run_git(&work, &["checkout", "main"]);
    let local_feat = git_stdout(&work, &["rev-parse", "feat"]);

    let (results, _) = run_remote_action(BranchAction::FastForward, &["feat"], &work, None, false);

    assert_eq!(results.len(), 1, "{results:?}");
    assert!(!results[0].success);
    assert!(!results[0].message.is_empty());
    assert_eq!(git_stdout(&work, &["rev-parse", "feat"]), local_feat);
}

#[test]
fn execute_pull_fast_forwards_local_branch_to_origin() {
    let (_base, work, bare) = origin_and_work();
    make_local_feat_behind_origin(&work);
    let origin_feat = bare_ref(&bare, "refs/heads/feat").unwrap();

    let (results, _) = run_remote_action(BranchAction::Pull, &["feat"], &work, None, false);

    assert_eq!(results.len(), 1, "{results:?}");
    assert!(results[0].success, "{results:?}");
    assert_eq!(results[0].message, "Fast-forwarded feat");
    assert_eq!(
        git_stdout(&work, &["rev-parse", "feat"]),
        origin_feat.to_string()
    );
}

#[test]
fn execute_push_uploads_unpushed_branch() {
    let (_base, work, bare) = origin_and_work();
    run_git(&work, &["push", "origin", "--delete", "feat"]);
    assert!(bare_ref(&bare, "refs/heads/feat").is_none());

    let (results, _) = run_remote_action(BranchAction::Push, &["feat"], &work, None, false);

    assert_eq!(results.len(), 1, "{results:?}");
    assert!(results[0].success, "{results:?}");
    assert_eq!(results[0].action, BranchAction::Push);
    assert_eq!(results[0].message, "Pushed feat");
    assert!(bare_ref(&bare, "refs/heads/feat").is_some());
}

#[test]
fn execute_push_multiple_branches_reports_progress_per_item() {
    let (_base, work, _bare) = origin_and_work();

    let (results, progress) =
        run_remote_action(BranchAction::Push, &["feat", "main"], &work, None, false);

    assert_eq!(results.len(), 2, "{results:?}");
    assert!(results.iter().all(|r| r.success), "{results:?}");
    let items: Vec<&str> = progress.iter().map(|p| p.current_item.as_str()).collect();
    assert!(items.contains(&"feat"), "{items:?}");
    assert!(items.contains(&"main"), "{items:?}");
    assert_eq!(items.last(), Some(&"Done"));
}

#[test]
fn execute_push_missing_branch_fails() {
    let (_base, work, _bare) = origin_and_work();

    let (results, _) = run_remote_action(BranchAction::Push, &["nope"], &work, None, false);

    assert_eq!(results.len(), 1, "{results:?}");
    assert!(!results[0].success);
}

#[test]
fn execute_force_push_overwrites_amended_remote_branch() {
    let (_base, work, bare) = origin_and_work();
    run_git(&work, &["checkout", "feat"]);
    run_git(&work, &["commit", "--amend", "-m", "feat amended"]);
    run_git(&work, &["checkout", "main"]);
    let local_feat = git_stdout(&work, &["rev-parse", "feat"]);

    let (results, _) = run_remote_action(BranchAction::ForcePush, &["feat"], &work, None, false);

    assert_eq!(results.len(), 1, "{results:?}");
    assert!(results[0].success, "{results:?}");
    assert_eq!(results[0].action, BranchAction::ForcePush);
    assert_eq!(results[0].message, "Force pushed feat");
    assert_eq!(
        bare_ref(&bare, "refs/heads/feat").unwrap().to_string(),
        local_feat
    );
}

#[test]
fn execute_push_tag_publishes_lightweight_tag() {
    let (_base, work, bare) = origin_and_work();
    run_git(&work, &["tag", "v1"]);

    let (results, _) = run_remote_action(BranchAction::PushTag, &["v1"], &work, None, false);

    assert_eq!(results.len(), 1, "{results:?}");
    assert!(results[0].success, "{results:?}");
    assert_eq!(results[0].action, BranchAction::PushTag);
    assert_eq!(results[0].message, "Pushed tag v1");
    assert!(bare_ref(&bare, "refs/tags/v1").is_some());
}

#[test]
fn execute_push_tag_missing_fails() {
    let (_base, work, _bare) = origin_and_work();

    let (results, _) = run_remote_action(BranchAction::PushTag, &["v-missing"], &work, None, false);

    assert_eq!(results.len(), 1, "{results:?}");
    assert!(!results[0].success);
}

#[test]
fn execute_delete_tag_and_remote_removes_both_tags() {
    let (_base, work, bare) = origin_and_work();
    run_git(&work, &["tag", "v1"]);
    run_git(&work, &["push", "origin", "v1"]);
    assert!(bare_ref(&bare, "refs/tags/v1").is_some());

    let (results, _) = run_remote_action(
        BranchAction::DeleteTagAndRemote,
        &["v1"],
        &work,
        None,
        false,
    );

    assert_eq!(results.len(), 2, "{results:?}");
    assert_eq!(results[0].action, BranchAction::DeleteTag);
    assert!(results[0].success, "{results:?}");
    assert_eq!(results[0].message, "Deleted tag v1");
    assert_eq!(results[1].action, BranchAction::DeleteTagAndRemote);
    assert!(results[1].success, "{results:?}");
    assert_eq!(results[1].message, "Deleted remote tag v1");
    assert!(!local_ref_exists(&work, "refs/tags/v1"));
    assert!(bare_ref(&bare, "refs/tags/v1").is_none());
}

#[test]
fn execute_delete_tag_and_remote_reports_remote_failure_for_local_only_tag() {
    let (_base, work, _bare) = origin_and_work();
    run_git(&work, &["tag", "v-local"]);

    let (results, _) = run_remote_action(
        BranchAction::DeleteTagAndRemote,
        &["v-local"],
        &work,
        None,
        false,
    );

    assert_eq!(results.len(), 2, "{results:?}");
    assert_eq!(results[0].action, BranchAction::DeleteTag);
    assert!(results[0].success, "{results:?}");
    assert_eq!(results[1].action, BranchAction::DeleteTagAndRemote);
    assert!(!results[1].success);
    assert_eq!(results[1].message, "Failed to delete remote tag v-local");
    assert!(!local_ref_exists(&work, "refs/tags/v-local"));
}

#[test]
fn execute_delete_remote_branch_removes_origin_branch() {
    let (_base, work, bare) = origin_and_work();

    let (results, _) = run_remote_action(
        BranchAction::DeleteRemoteBranch,
        &["feat"],
        &work,
        None,
        false,
    );

    assert_eq!(results.len(), 1, "{results:?}");
    assert!(results[0].success, "{results:?}");
    assert_eq!(results[0].action, BranchAction::DeleteRemoteBranch);
    assert_eq!(results[0].message, "Deleted remote feat");
    assert!(bare_ref(&bare, "refs/heads/feat").is_none());
}

#[test]
fn execute_delete_remote_branch_cancelled_keeps_origin_branch() {
    let (_base, work, bare) = origin_and_work();

    let (results, _) = run_remote_action(
        BranchAction::DeleteRemoteBranch,
        &["feat"],
        &work,
        None,
        true,
    );

    assert_eq!(results.len(), 1, "{results:?}");
    assert_eq!(results[0].branch_name, "feat");
    assert_eq!(results[0].action, BranchAction::DeleteRemoteBranch);
    assert_eq!(results[0].message, "Cancelled");
    assert!(!results[0].success);
    assert!(bare_ref(&bare, "refs/heads/feat").is_some());
}

#[test]
fn execute_delete_remote_branch_unknown_fails() {
    let (_base, work, _bare) = origin_and_work();

    let (results, _) = run_remote_action(
        BranchAction::DeleteRemoteBranch,
        &["nope"],
        &work,
        None,
        false,
    );

    assert_eq!(results.len(), 1, "{results:?}");
    assert!(!results[0].success);
}

#[test]
fn execute_fetch_remote_reports_named_remote() {
    let (_base, work, _bare) = origin_and_work();

    let (results, _) = run_remote_action(BranchAction::FetchRemote, &["x"], &work, None, false);

    assert_eq!(results.len(), 1, "{results:?}");
    assert!(results[0].success, "{results:?}");
    assert_eq!(results[0].action, BranchAction::FetchRemote);
    assert_eq!(results[0].branch_name, "origin");
    assert_eq!(results[0].message, "Fetched origin");
}

#[test]
fn execute_fetch_remote_unknown_remote_fails() {
    let (_base, work, _bare) = origin_and_work();

    let (results, _) = run_remote_action(
        BranchAction::FetchRemote,
        &["x"],
        &work,
        Some("nope"),
        false,
    );

    assert_eq!(results.len(), 1, "{results:?}");
    assert!(!results[0].success);
    assert_eq!(results[0].branch_name, "nope");
}

#[test]
fn execute_fetch_all_remotes_succeeds() {
    let (_base, work, _bare) = origin_and_work();

    let (results, _) = run_remote_action(BranchAction::Fetch, &["x"], &work, None, false);

    assert_eq!(results.len(), 1, "{results:?}");
    assert!(results[0].success, "{results:?}");
    assert_eq!(results[0].action, BranchAction::Fetch);
    assert_eq!(results[0].branch_name, "");
    assert_eq!(results[0].message, "Fetched all remotes");
}

#[test]
fn execute_fetch_prune_succeeds() {
    let (_base, work, _bare) = origin_and_work();

    let (results, _) = run_remote_action(BranchAction::FetchPrune, &["x"], &work, None, false);

    assert_eq!(results.len(), 1, "{results:?}");
    assert!(results[0].success, "{results:?}");
    assert_eq!(results[0].action, BranchAction::FetchPrune);
}

#[test]
fn execute_pull_remote_updates_local_branch_from_origin() {
    let (_base, work, bare) = origin_and_work();
    run_git(&work, &["branch", "-f", "feat", "feat~1"]);
    let origin_feat = bare_ref(&bare, "refs/heads/feat").unwrap();

    let (results, _) = run_remote_action(BranchAction::PullRemote, &["feat"], &work, None, false);

    assert_eq!(results.len(), 1, "{results:?}");
    assert!(results[0].success, "{results:?}");
    assert_eq!(results[0].action, BranchAction::PullRemote);
    assert_eq!(results[0].branch_name, "feat");
    assert_eq!(results[0].message, "Pulled origin/feat");
    assert_eq!(
        git_stdout(&work, &["rev-parse", "feat"]),
        origin_feat.to_string()
    );
}

#[test]
fn execute_pull_remote_rejects_local_commits_ahead_of_origin() {
    let (_base, work, _bare) = origin_and_work();
    run_git(&work, &["checkout", "feat"]);
    std::fs::write(work.join("local.txt"), "local\n").unwrap();
    run_git(&work, &["add", "local.txt"]);
    run_git(&work, &["commit", "-m", "local only"]);
    run_git(&work, &["checkout", "main"]);

    let (results, _) = run_remote_action(BranchAction::PullRemote, &["feat"], &work, None, false);

    assert_eq!(results.len(), 1, "{results:?}");
    assert!(!results[0].success);
}

#[test]
fn execute_merge_remote_into_current_merges_origin_branch() {
    let (_base, work, _bare) = origin_and_work();
    assert_eq!(
        git_stdout(&work, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "main"
    );

    let (results, _) = run_remote_action(
        BranchAction::MergeRemoteIntoCurrent,
        &["feat"],
        &work,
        None,
        false,
    );

    assert_eq!(results.len(), 1, "{results:?}");
    assert!(results[0].success, "{results:?}");
    assert_eq!(results[0].action, BranchAction::MergeRemoteIntoCurrent);
    assert_eq!(results[0].branch_name, "feat");
    assert_eq!(results[0].message, "Merged origin/feat into current");
    assert!(work.join("feat.txt").exists());
}

#[test]
fn execute_merge_remote_into_current_conflict_aborts() {
    let (_base, work, _bare) = origin_and_work();
    conflicting_feat_on_origin(&work);

    let (results, _) = run_remote_action(
        BranchAction::MergeRemoteIntoCurrent,
        &["feat"],
        &work,
        None,
        false,
    );

    assert!(
        results
            .iter()
            .any(|r| !r.success && r.message == "Merge conflict \u{2014} aborted"),
        "{results:?}"
    );
    assert_eq!(repo_state(&work), git2::RepositoryState::Clean);
}

#[test]
fn execute_cherry_pick_remote_applies_origin_commits() {
    let (_base, work, _bare) = origin_and_work();

    let (results, _) = run_remote_action(
        BranchAction::CherryPickRemote,
        &["feat"],
        &work,
        None,
        false,
    );

    assert_eq!(results.len(), 1, "{results:?}");
    assert!(results[0].success, "{results:?}");
    assert_eq!(results[0].action, BranchAction::CherryPickRemote);
    assert_eq!(results[0].message, "Cherry-picked origin/feat");
    assert!(work.join("feat.txt").exists());
}

#[test]
fn execute_cherry_pick_remote_conflict_aborts() {
    let (_base, work, _bare) = origin_and_work();
    conflicting_feat_on_origin(&work);

    let (results, _) = run_remote_action(
        BranchAction::CherryPickRemote,
        &["feat"],
        &work,
        None,
        false,
    );

    assert!(
        results
            .iter()
            .any(|r| !r.success && r.message == "Cherry-pick conflict \u{2014} aborted"),
        "{results:?}"
    );
    assert_eq!(repo_state(&work), git2::RepositoryState::Clean);
}
