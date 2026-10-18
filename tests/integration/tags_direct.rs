
use git_branch_manager::git::tags;

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
