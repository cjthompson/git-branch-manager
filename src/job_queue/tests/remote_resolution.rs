use super::super::execute::execute_action_recording;
use super::super::RemoteRefs;
use super::*;
use std::path::PathBuf;

/// Bare remotes plus a `work` clone with `main` committed and a local `feature` branch.
fn fixture(remotes: &[&str]) -> (tempfile::TempDir, PathBuf) {
    let base_tmp = tempfile::tempdir().expect("temp base dir");
    let base = base_tmp.path().to_path_buf();
    for remote in remotes {
        let bare = base.join(format!("{remote}.git"));
        std::fs::create_dir_all(&bare).unwrap();
        run_git(&bare, &["init", "--bare", "-b", "main"]);
    }
    let work = base.join("work");
    std::fs::create_dir_all(&work).unwrap();
    run_git(&work, &["init", "-b", "main"]);
    run_git(&work, &["config", "user.name", "Test User"]);
    run_git(&work, &["config", "user.email", "test@example.com"]);
    std::fs::write(work.join("README.md"), "# Test\n").unwrap();
    run_git(&work, &["add", "README.md"]);
    run_git(&work, &["commit", "-m", "Initial commit"]);
    for remote in remotes {
        let bare = base.join(format!("{remote}.git"));
        run_git(&work, &["remote", "add", remote, bare.to_str().unwrap()]);
    }
    run_git(&work, &["checkout", "-b", "feature"]);
    std::fs::write(work.join("feature.txt"), "content\n").unwrap();
    run_git(&work, &["add", "feature.txt"]);
    run_git(&work, &["commit", "-m", "feature commit"]);
    run_git(&work, &["checkout", "main"]);
    (base_tmp, work)
}

fn remote_has_ref(base: &Path, remote: &str, full_ref: &str) -> bool {
    let git_dir = base
        .join(format!("{remote}.git"))
        .to_string_lossy()
        .into_owned();
    std::process::Command::new("git")
        .args([
            "--git-dir",
            &git_dir,
            "rev-parse",
            "--verify",
            "-q",
            full_ref,
        ])
        .output()
        .unwrap()
        .status
        .success()
}

fn config_value(work: &Path, key: &str) -> Option<String> {
    git2::Repository::open(work)
        .unwrap()
        .config()
        .unwrap()
        .get_string(key)
        .ok()
}

/// Runs one job through the recording path; returns results and recorded `(remote, branch)` pairs.
fn run_job(
    action: BranchAction,
    targets: &[&str],
    work: &Path,
    remote: Option<&str>,
) -> (Vec<OperationResult>, Vec<(String, String)>) {
    let (prog_tx, _prog_rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let partial = Arc::new(AtomicBool::new(false));
    let refs: RemoteRefs = Arc::default();
    let items: Vec<String> = targets.iter().map(|t| t.to_string()).collect();
    let results = execute_action_recording(
        action, &items, work, "main", false, remote, &prog_tx, &cancel, &partial, &refs,
    );
    let deleted = refs.lock().unwrap().clone();
    (results, deleted)
}

#[test]
fn push_single_remote_without_upstream_sets_upstream_config() {
    let (_tmp, work) = fixture(&["origin"]);
    assert!(config_value(&work, "branch.feature.remote").is_none());

    let (results, _) = run_job(BranchAction::Push, &["feature"], &work, None);

    assert!(results.iter().any(|r| r.success), "{results:?}");
    assert_eq!(
        config_value(&work, "branch.feature.remote").as_deref(),
        Some("origin")
    );
    assert_eq!(
        config_value(&work, "branch.feature.merge").as_deref(),
        Some("refs/heads/feature")
    );
    assert!(remote_has_ref(
        work.parent().unwrap(),
        "origin",
        "refs/heads/feature"
    ));
}

#[test]
fn push_uses_push_default_and_sets_upstream_to_it() {
    let (_tmp, work) = fixture(&["origin", "upstream"]);
    run_git(&work, &["config", "remote.pushDefault", "upstream"]);

    let (results, _) = run_job(BranchAction::Push, &["feature"], &work, None);

    assert!(results.iter().any(|r| r.success), "{results:?}");
    assert_eq!(
        config_value(&work, "branch.feature.remote").as_deref(),
        Some("upstream")
    );
    assert!(remote_has_ref(
        work.parent().unwrap(),
        "upstream",
        "refs/heads/feature"
    ));
    assert!(!remote_has_ref(
        work.parent().unwrap(),
        "origin",
        "refs/heads/feature"
    ));
}

#[test]
fn push_two_remotes_without_upstream_or_push_default_pushes_nothing() {
    let (_tmp, work) = fixture(&["origin", "upstream"]);

    let (results, _) = run_job(BranchAction::Push, &["feature"], &work, None);

    assert!(results.iter().all(|r| !r.success), "{results:?}");
    assert!(results[0].message.contains("origin") && results[0].message.contains("upstream"));
    let base = work.parent().unwrap();
    assert!(!remote_has_ref(base, "origin", "refs/heads/feature"));
    assert!(!remote_has_ref(base, "upstream", "refs/heads/feature"));
}

#[test]
fn delete_remote_without_upstream_with_two_remotes_fails_and_keeps_remote_branch() {
    let (_tmp, work) = fixture(&["origin", "upstream"]);
    run_git(&work, &["push", "origin", "feature"]);
    assert!(config_value(&work, "branch.feature.remote").is_none());

    let (results, deleted) = run_job(BranchAction::DeleteRemoteBranch, &["feature"], &work, None);

    assert!(results.iter().all(|r| !r.success), "{results:?}");
    assert!(deleted.is_empty());
    assert!(remote_has_ref(
        work.parent().unwrap(),
        "origin",
        "refs/heads/feature"
    ));
}

#[test]
fn delete_remote_uses_upstream_and_records_the_pair() {
    let (_tmp, work) = fixture(&["origin", "upstream"]);
    run_git(&work, &["push", "-u", "upstream", "feature"]);

    let (results, deleted) = run_job(BranchAction::DeleteRemoteBranch, &["feature"], &work, None);

    assert!(results.iter().any(|r| r.success), "{results:?}");
    assert_eq!(
        deleted,
        vec![("upstream".to_string(), "feature".to_string())]
    );
    assert!(!remote_has_ref(
        work.parent().unwrap(),
        "upstream",
        "refs/heads/feature"
    ));
}

#[test]
fn push_tag_two_remotes_without_push_default_fails_and_pushes_nothing() {
    let (_tmp, work) = fixture(&["origin", "upstream"]);
    run_git(&work, &["tag", "v1"]);

    let (results, _) = run_job(BranchAction::PushTag, &["v1"], &work, None);

    assert!(results.iter().all(|r| !r.success), "{results:?}");
    let base = work.parent().unwrap();
    assert!(!remote_has_ref(base, "origin", "refs/tags/v1"));
    assert!(!remote_has_ref(base, "upstream", "refs/tags/v1"));
}
