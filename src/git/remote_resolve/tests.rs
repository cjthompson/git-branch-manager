use super::*;
use git2::Repository;

fn repo_with_remotes(names: &[&str]) -> (tempfile::TempDir, Repository) {
    let dir = tempfile::tempdir().expect("temp dir");
    let repo = Repository::init(dir.path()).expect("init repo");
    for name in names {
        repo.remote(name, "file:///nonexistent-remote")
            .expect("add remote");
    }
    assert!(
        repo.config()
            .unwrap()
            .get_string("remote.pushDefault")
            .is_err(),
        "fixture precondition: remote.pushDefault must be unset"
    );
    (dir, repo)
}

fn set_config(repo: &Repository, key: &str, value: &str) {
    repo.config()
        .unwrap()
        .set_str(key, value)
        .expect("set config");
}

#[test]
fn resolve_push_without_remotes_is_no_remote() {
    let (_dir, repo) = repo_with_remotes(&[]);
    assert_eq!(resolve_push(&repo, "feature"), PushTarget::NoRemote);
}

#[test]
fn resolve_delete_without_remotes_is_error() {
    let (_dir, repo) = repo_with_remotes(&[]);
    assert!(resolve_delete(&repo, "feature").is_err());
}

#[test]
fn resolve_push_single_remote_without_upstream_sets_upstream() {
    let (_dir, repo) = repo_with_remotes(&["origin"]);
    assert_eq!(
        resolve_push(&repo, "feature"),
        PushTarget::Resolved {
            remote: "origin".into(),
            set_upstream: true,
        }
    );
}

#[test]
fn resolve_delete_single_remote_without_upstream_is_error() {
    let (_dir, repo) = repo_with_remotes(&["origin"]);
    let error = resolve_delete(&repo, "feature").unwrap_err();
    assert!(error.contains("no upstream"), "{error}");
}

#[test]
fn resolve_delete_single_remote_with_upstream_uses_it() {
    let (_dir, repo) = repo_with_remotes(&["origin"]);
    set_config(&repo, "branch.feature.remote", "origin");
    assert_eq!(resolve_delete(&repo, "feature").unwrap(), "origin");
}

#[test]
fn resolve_push_single_remote_with_upstream_does_not_set_upstream() {
    let (_dir, repo) = repo_with_remotes(&["origin"]);
    set_config(&repo, "branch.feature.remote", "origin");
    assert_eq!(
        resolve_push(&repo, "feature"),
        PushTarget::Resolved {
            remote: "origin".into(),
            set_upstream: false,
        }
    );
}

#[test]
fn resolve_push_two_remotes_without_upstream_needs_choice() {
    let (_dir, repo) = repo_with_remotes(&["origin", "upstream"]);
    let PushTarget::NeedsChoice(mut names) = resolve_push(&repo, "feature") else {
        panic!("expected NeedsChoice");
    };
    names.sort();
    assert_eq!(names, vec!["origin".to_string(), "upstream".to_string()]);
}

#[test]
fn resolve_push_push_default_wins_over_multiple_remotes() {
    let (_dir, repo) = repo_with_remotes(&["origin", "upstream"]);
    set_config(&repo, "remote.pushDefault", "upstream");
    assert_eq!(
        resolve_push(&repo, "feature"),
        PushTarget::Resolved {
            remote: "upstream".into(),
            set_upstream: true,
        }
    );
}

#[test]
fn resolve_tag_two_remotes_without_push_default_needs_choice() {
    let (_dir, repo) = repo_with_remotes(&["origin", "upstream"]);
    assert!(matches!(
        resolve_tag(&repo),
        PushTarget::NeedsChoice(names) if names.len() == 2
    ));
}

#[test]
fn resolve_delete_ignores_upstream_value_that_is_not_a_remote() {
    let (_dir, repo) = repo_with_remotes(&["origin"]);
    set_config(&repo, "branch.feature.remote", ".");
    assert!(resolve_delete(&repo, "feature").is_err());
}
