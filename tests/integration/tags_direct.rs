use git_branch_manager::git::tags;
use git_branch_manager::types::BranchAction;

use crate::common::*;

#[test]
fn test_list_tags_empty() {
    let (_tmpdir, repo) = setup_test_repo();
    let result = tags::list_tags(&repo);
    assert!(result.is_empty());
}

#[test]
fn test_list_tags_lightweight() {
    let (tmpdir, repo) = setup_test_repo();
    run_git(tmpdir.path(), &["tag", "v0.1"]);
    let result = tags::list_tags(&repo);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].name, "v0.1");
    assert!(!result[0].is_annotated);
    assert!(result[0].message.is_none());
}

#[test]
fn test_list_tags_with_annotated() {
    let (tmpdir, repo) = setup_test_repo();
    run_git(tmpdir.path(), &["tag", "-a", "v1.0", "-m", "Release 1.0"]);
    let result = tags::list_tags(&repo);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].name, "v1.0");
    assert!(result[0].is_annotated);
    assert_eq!(result[0].message.as_deref(), Some("Release 1.0"));
}

#[test]
fn test_delete_tag() {
    let (tmpdir, repo) = setup_test_repo();
    run_git(tmpdir.path(), &["tag", "v1.0"]);
    let result = tags::delete_tag(&repo, "v1.0");
    assert!(result.success);
    let remaining = tags::list_tags(&repo);
    assert!(remaining.is_empty());
}

#[test]
fn test_delete_tags_batch() {
    let (tmpdir, repo) = setup_test_repo();
    run_git(tmpdir.path(), &["tag", "v1.0"]);
    run_git(tmpdir.path(), &["tag", "v2.0"]);
    let names = vec!["v1.0".to_string(), "v2.0".to_string()];
    let results = tags::delete_tags_batch(&repo, &names);
    assert!(results.iter().all(|r| r.success));
    let remaining = tags::list_tags(&repo);
    assert!(remaining.is_empty());
}

#[test]
fn test_delete_tags_batch_preserves_order_for_mixed_results() {
    let (tmpdir, repo) = setup_test_repo();
    run_git(tmpdir.path(), &["tag", "v1.0"]);
    run_git(tmpdir.path(), &["tag", "v2.0"]);

    let names = vec![
        "v2.0".to_string(),
        "missing".to_string(),
        "v1.0".to_string(),
    ];
    let results = tags::delete_tags_batch(&repo, &names);

    assert_eq!(results.len(), names.len());
    assert_eq!(
        results
            .iter()
            .map(|result| result.branch_name.as_str())
            .collect::<Vec<_>>(),
        ["v2.0", "missing", "v1.0"]
    );
    assert_eq!(
        results
            .iter()
            .map(|result| result.success)
            .collect::<Vec<_>>(),
        [true, false, true]
    );
    assert!(results
        .iter()
        .all(|result| result.action == BranchAction::DeleteTag));
    assert_eq!(tags::list_tags(&repo).len(), 0);
}

#[test]
fn test_list_tags_skips_non_commit_targets() {
    let (_tmpdir, repo) = setup_test_repo();
    let blob_oid = repo.blob(b"not a commit").unwrap();
    repo.reference("refs/tags/not-a-commit", blob_oid, false, "test tag")
        .unwrap();

    assert!(tags::list_tags(&repo).is_empty());
    assert!(repo.find_reference("refs/tags/not-a-commit").is_ok());
}

#[test]
fn test_list_tags_sorts_newest_first() {
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();
    run_git_with_env(
        dir,
        &["commit", "--allow-empty", "-m", "old"],
        &[
            ("GIT_AUTHOR_DATE", "2020-01-01T00:00:00Z"),
            ("GIT_COMMITTER_DATE", "2020-01-01T00:00:00Z"),
        ],
    );
    run_git(dir, &["tag", "v1-old"]);
    run_git_with_env(
        dir,
        &["commit", "--allow-empty", "-m", "new"],
        &[
            ("GIT_AUTHOR_DATE", "2024-06-01T00:00:00Z"),
            ("GIT_COMMITTER_DATE", "2024-06-01T00:00:00Z"),
        ],
    );
    run_git(dir, &["tag", "v2-new"]);
    let result = tags::list_tags(&repo);
    let names: Vec<String> = result.iter().map(|t| t.name.clone()).collect();
    assert_eq!(names, vec!["v2-new", "v1-old"]);
    for tag in &result {
        assert_eq!(tag.commit_hash.len(), 7);
        assert!(!tag.is_annotated);
    }
}

#[test]
fn test_delete_tag_missing_reports_failure() {
    let (_tmpdir, repo) = setup_test_repo();
    let result = tags::delete_tag(&repo, "no-such-tag");
    assert!(!result.success);
    assert_eq!(result.branch_name, "no-such-tag");
    assert_eq!(result.action, BranchAction::DeleteTag);
    assert!(result.message.starts_with("Failed: "));
    assert!(result.failure.is_none());
}

#[test]
fn test_delete_remote_tags_batch_empty_input_returns_empty() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();
    let results = tags::delete_remote_tags_batch(&work_dir, &[]);
    assert!(results.is_empty());
}

#[test]
fn test_delete_remote_tags_batch_deletes_remote_tags() {
    let (tmpdir, work_dir, _repo) = setup_remote_test_repo();
    let remote_dir = tmpdir.path().join("remote.git");
    run_git(&work_dir, &["tag", "v1.0"]);
    run_git(&work_dir, &["tag", "v2.0"]);
    run_git(&work_dir, &["push", "origin", "v1.0", "v2.0"]);
    assert_eq!(git_output(&remote_dir, &["tag", "--list"]), "v1.0\nv2.0");

    let names = vec!["v1.0".to_string(), "v2.0".to_string()];
    let results = tags::delete_remote_tags_batch(&work_dir, &names);
    assert_eq!(results.len(), 2);
    for (result, name) in results.iter().zip(["v1.0", "v2.0"]) {
        assert!(result.success);
        assert_eq!(result.action, BranchAction::DeleteTagAndRemote);
        assert_eq!(result.message, format!("Deleted remote tag {name}"));
        assert_eq!(result.branch_name, name);
        assert!(result.failure.is_none());
    }
    assert_eq!(git_output(&remote_dir, &["tag", "--list"]), "");
    assert_eq!(git_output(&work_dir, &["tag", "--list"]), "v1.0\nv2.0");
}

#[test]
fn test_delete_remote_tags_batch_unreachable_repo_reports_failures() {
    let (tmpdir, _work_dir, _repo) = setup_remote_test_repo();
    let missing = tmpdir.path().join("no-such-dir");
    let names = vec!["v1.0".to_string(), "ghost".to_string()];
    let results = tags::delete_remote_tags_batch(&missing, &names);
    assert_eq!(results.len(), 2);
    for (result, name) in results.iter().zip(["v1.0", "ghost"]) {
        assert!(!result.success);
        assert_eq!(result.action, BranchAction::DeleteTagAndRemote);
        assert_eq!(
            result.message,
            format!("Failed to delete remote tag {name}")
        );
        assert_eq!(result.branch_name, name);
        assert!(result.failure.is_none());
    }
}

#[cfg(unix)]
#[test]
fn test_delete_remote_tags_batch_fallback_succeeds_per_tag() {
    use std::os::unix::fs::PermissionsExt;

    let (tmpdir, work_dir, _repo) = setup_remote_test_repo();
    let remote_dir = tmpdir.path().join("remote.git");
    run_git(&work_dir, &["tag", "v1.0"]);
    run_git(&work_dir, &["tag", "v2.0"]);
    run_git(&work_dir, &["push", "origin", "v1.0", "v2.0"]);

    // Reject only the first push (the batch delete); later pushes succeed.
    let marker = tmpdir.path().join("pre-receive-fired");
    let hook = remote_dir.join("hooks").join("pre-receive");
    std::fs::write(
        &hook,
        format!(
            "#!/bin/sh\ncat >/dev/null\nif [ -e '{m}' ]; then exit 0; fi\ntouch '{m}'\nexit 1\n",
            m = marker.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();

    let names = vec!["v1.0".to_string(), "v2.0".to_string()];
    let results = tags::delete_remote_tags_batch(&work_dir, &names);
    assert_eq!(results.len(), 2);
    for (result, name) in results.iter().zip(["v1.0", "v2.0"]) {
        assert!(result.success);
        assert_eq!(result.action, BranchAction::DeleteTagAndRemote);
        assert_eq!(result.message, format!("Deleted remote tag {name}"));
        assert_eq!(result.branch_name, name);
        assert!(result.failure.is_none());
    }
    assert!(marker.exists());
    assert_eq!(git_output(&remote_dir, &["tag", "--list"]), "");
}

#[test]
fn test_push_tag_pushes_to_remote() {
    let (tmpdir, work_dir, _repo) = setup_remote_test_repo();
    let remote_dir = tmpdir.path().join("remote.git");
    run_git(&work_dir, &["tag", "v3.0"]);
    let result = tags::push_tag(&work_dir, "v3.0");
    assert!(result.success);
    assert_eq!(result.branch_name, "v3.0");
    assert_eq!(result.action, BranchAction::PushTag);
    assert_eq!(result.message, "Pushed tag v3.0");
    assert!(result.failure.is_none());
    assert_eq!(git_output(&remote_dir, &["tag", "--list"]), "v3.0");
}

#[test]
fn test_push_tag_rejected_by_git_reports_stderr() {
    let (tmpdir, work_dir, _repo) = setup_remote_test_repo();
    let remote_dir = tmpdir.path().join("remote.git");
    let result = tags::push_tag(&work_dir, "no-such-tag");
    assert!(!result.success);
    assert_eq!(result.action, BranchAction::PushTag);
    assert_eq!(result.branch_name, "no-such-tag");
    assert!(!result.message.is_empty());
    assert!(result.failure.is_none());
    assert_eq!(git_output(&remote_dir, &["tag", "--list"]), "");
}

#[cfg(unix)]
#[test]
fn test_delete_remote_tags_batch_fallback_reports_partial_results() {
    use std::os::unix::fs::PermissionsExt;

    let (tmpdir, work_dir, _repo) = setup_remote_test_repo();
    let remote_dir = tmpdir.path().join("remote.git");
    run_git(&work_dir, &["tag", "v-delete"]);
    run_git(&work_dir, &["tag", "v-reject"]);
    run_git(&work_dir, &["push", "origin", "v-delete", "v-reject"]);

    let hook = remote_dir.join("hooks").join("pre-receive");
    std::fs::write(
        &hook,
        "#!/bin/sh\nwhile read old new ref; do\n  if [ \"$ref\" = \"refs/tags/v-reject\" ]; then\n    exit 1\n  fi\ndone\nexit 0\n",
    )
    .unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();

    let names = vec!["v-delete".to_string(), "v-reject".to_string()];
    let results = tags::delete_remote_tags_batch(&work_dir, &names);

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].branch_name, "v-delete");
    assert!(results[0].success);
    assert_eq!(results[0].action, BranchAction::DeleteTagAndRemote);
    assert_eq!(results[1].branch_name, "v-reject");
    assert!(!results[1].success);
    assert_eq!(results[1].action, BranchAction::DeleteTagAndRemote);
    assert_eq!(git_output(&remote_dir, &["tag", "--list"]), "v-reject");
    assert_eq!(
        git_output(&work_dir, &["tag", "--list"]),
        "v-delete\nv-reject"
    );
}

#[test]
fn test_push_tag_unreachable_repo_reports_spawn_error() {
    let (tmpdir, _work_dir, _repo) = setup_remote_test_repo();
    let missing = tmpdir.path().join("no-such-dir");
    let result = tags::push_tag(&missing, "v1.0");
    assert!(!result.success);
    assert_eq!(result.action, BranchAction::PushTag);
    assert!(!result.message.is_empty());
}
