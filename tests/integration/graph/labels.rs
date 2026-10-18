
use git_branch_manager::git::graph;

use crate::common::*;

/// Build one graph with live, nested, deleted, squashed, and tag-only topics.
///
/// The branch refs for the deleted topics are removed deliberately; tags keep
/// the squash and detached histories reachable for graph inspection.
pub(crate) fn setup_graph_label_fixture() -> TestDir {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path().to_path_buf();

    run_git(&dir, &["checkout", "-b", "release/0.3"]);
    run_git(&dir, &["commit", "--allow-empty", "-m", "release commit 1"]);

    run_git(&dir, &["checkout", "-b", "feature/nested"]);
    run_git(
        &dir,
        &["commit", "--allow-empty", "-m", "nested branch commit"],
    );
    run_git(&dir, &["checkout", "release/0.3"]);
    run_git(
        &dir,
        &[
            "merge",
            "--no-ff",
            "feature/nested",
            "-m",
            "Merge branch 'feature/nested' into release/0.3",
        ],
    );
    run_git(&dir, &["branch", "-D", "feature/nested"]);

    run_git(&dir, &["checkout", "main"]);
    run_git(
        &dir,
        &[
            "commit",
            "--allow-empty",
            "-m",
            "main first-parent fallback",
        ],
    );
    run_git(
        &dir,
        &[
            "merge",
            "--no-ff",
            "release/0.3",
            "-m",
            "Merge branch 'release/0.3' into main",
        ],
    );

    run_git(&dir, &["checkout", "-b", "worktree-agent-deleted"]);
    run_git(
        &dir,
        &[
            "commit",
            "--allow-empty",
            "-m",
            "deleted merge branch commit",
        ],
    );
    run_git(&dir, &["checkout", "main"]);
    run_git(
        &dir,
        &["commit", "--allow-empty", "-m", "main before deleted merge"],
    );
    run_git(
        &dir,
        &[
            "merge",
            "--no-ff",
            "worktree-agent-deleted",
            "-m",
            "Merge branch 'worktree-agent-deleted' into main",
        ],
    );
    run_git(&dir, &["branch", "-D", "worktree-agent-deleted"]);

    run_git(&dir, &["checkout", "-b", "feature/squash"]);
    std::fs::write(dir.join("squash-one.txt"), "one\n").unwrap();
    run_git(&dir, &["add", "squash-one.txt"]);
    run_git(&dir, &["commit", "-m", "squash source commit 1"]);
    std::fs::write(dir.join("squash-two.txt"), "two\n").unwrap();
    run_git(&dir, &["add", "squash-two.txt"]);
    run_git(&dir, &["commit", "-m", "squash source commit 2"]);
    run_git(&dir, &["tag", "squash-source"]);
    run_git(&dir, &["checkout", "main"]);
    run_git(&dir, &["merge", "--squash", "feature/squash"]);
    run_git(&dir, &["commit", "-m", "squash merge feature/squash"]);
    run_git(&dir, &["branch", "-D", "feature/squash"]);

    run_git(&dir, &["checkout", "--detach", "main"]);
    run_git(
        &dir,
        &["commit", "--allow-empty", "-m", "detached tagged commit"],
    );
    run_git(&dir, &["tag", "detached-topic"]);
    run_git(&dir, &["checkout", "main"]);

    tmpdir
}

#[test]
fn test_graph_branch_labels_follow_visual_branch_tracks() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "release/0.3"]);
    run_git(dir, &["commit", "--allow-empty", "-m", "release commit 1"]);
    run_git(dir, &["commit", "--allow-empty", "-m", "release commit 2"]);
    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["commit", "--allow-empty", "-m", "main commit"]);
    run_git(
        dir,
        &["merge", "--no-ff", "release/0.3", "-m", "merge release/0.3"],
    );

    let snapshot = graph::load_graph_with_squash_annotations(dir, tmpdir.graph_options())
        .expect("graph loader should preserve live branch tracks");
    let release_commit = snapshot
        .commits
        .iter()
        .find(|commit| commit.summary == "release commit 1")
        .expect("release commit should be in graph");

    assert_eq!(
        release_commit
            .branch
            .as_ref()
            .map(|branch| branch.name.as_str()),
        Some("release/0.3")
    );

    let main_commit = snapshot
        .commits
        .iter()
        .find(|commit| commit.summary == "main commit")
        .expect("main commit should be in graph");
    assert_eq!(
        main_commit
            .branch
            .as_ref()
            .map(|branch| branch.name.as_str()),
        Some("main")
    );
}

#[test]
fn test_graph_local_branch_owns_track_before_matching_remote() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    run_git(&work_dir, &["checkout", "-b", "release/0.3"]);
    run_git(
        &work_dir,
        &["commit", "--allow-empty", "-m", "release inferred commit"],
    );
    run_git(
        &work_dir,
        &["commit", "--allow-empty", "-m", "release remote tip"],
    );
    run_git(&work_dir, &["push", "-u", "origin", "release/0.3"]);
    run_git(
        &work_dir,
        &["commit", "--allow-empty", "-m", "release local tip"],
    );

    let snapshot = graph::load_graph_with_squash_annotations(
        &work_dir,
        graph::GraphLoadOptions {
            include_remotes: true,
            base_branch: Some("main".into()),
            .._tmpdir.graph_options()
        },
    )
    .expect("graph loader should preserve local ownership with remotes enabled");
    let inferred = snapshot
        .commits
        .iter()
        .find(|commit| commit.summary == "release inferred commit")
        .expect("inferred release commit should be in graph");

    assert!(inferred.refs.is_empty());
    assert_eq!(
        inferred.branch.as_ref().map(|branch| branch.name.as_str()),
        Some("release/0.3")
    );
}

#[test]
fn test_graph_does_not_expose_a_deleted_merge_branch_as_a_live_ref() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "worktree-agent-deleted"]);
    run_git(
        dir,
        &["commit", "--allow-empty", "-m", "deleted branch commit"],
    );
    run_git(dir, &["checkout", "main"]);
    run_git(
        dir,
        &[
            "merge",
            "--no-ff",
            "worktree-agent-deleted",
            "-m",
            "merge worktree-agent-deleted",
        ],
    );
    run_git(dir, &["branch", "-D", "worktree-agent-deleted"]);

    let snapshot = graph::load_graph_with_squash_annotations(dir, tmpdir.graph_options())
        .expect("graph loader should handle deleted merge branches");
    let deleted_commit = snapshot
        .commits
        .iter()
        .find(|commit| commit.summary == "deleted branch commit")
        .expect("deleted branch commit should remain in history");

    assert!(deleted_commit.branch.is_none());
    assert!(!snapshot.commits.iter().any(|commit| {
        commit
            .refs
            .iter()
            .any(|reference| reference.name == "worktree-agent-deleted")
    }));
}

#[test]
fn test_graph_labels_deleted_merge_branch_from_conventional_subject() {
    let tmpdir = setup_graph_label_fixture();
    let snapshot = graph::load_graph_with_squash_annotations(tmpdir.path(), tmpdir.graph_options())
        .expect("graph loader should preserve the composed fixture");
    let deleted_commit = snapshot
        .commits
        .iter()
        .find(|commit| commit.summary == "deleted merge branch commit")
        .expect("deleted merge branch commit should be in graph");

    assert_eq!(
        deleted_commit
            .branch
            .as_ref()
            .map(|branch| branch.name.as_str()),
        Some("worktree-agent-deleted")
    );
}

#[test]
fn test_graph_label_fixture_labels_nested_and_first_parent_tracks() {
    let tmpdir = setup_graph_label_fixture();
    let snapshot = graph::load_graph_with_squash_annotations(tmpdir.path(), tmpdir.graph_options())
        .expect("graph loader should preserve the composed fixture");

    let nested_commit = snapshot
        .commits
        .iter()
        .find(|commit| commit.summary == "nested branch commit")
        .expect("nested branch commit should be in graph");
    assert_eq!(
        nested_commit
            .branch
            .as_ref()
            .map(|branch| branch.name.as_str()),
        Some("feature/nested")
    );

    let main_commit = snapshot
        .commits
        .iter()
        .find(|commit| commit.summary == "main first-parent fallback")
        .expect("main first-parent commit should be in graph");
    assert_eq!(
        main_commit
            .branch
            .as_ref()
            .map(|branch| branch.name.as_str()),
        Some("main")
    );
}

#[test]
fn test_graph_label_fixture_keeps_tag_only_histories_reachable() {
    let tmpdir = setup_graph_label_fixture();
    let snapshot = graph::load_graph_with_squash_annotations(
        tmpdir.path(),
        graph::GraphLoadOptions {
            include_remotes: true,
            ..tmpdir.graph_options()
        },
    )
    .expect("graph loader should include tag-only fixture histories");

    let squash_source = snapshot
        .commits
        .iter()
        .find(|commit| commit.summary == "squash source commit 2")
        .expect("tagged squash source should be in graph");
    assert!(squash_source
        .refs
        .iter()
        .any(|reference| reference.name == "squash-source"));
    assert!(squash_source.branch.is_none());

    let detached = snapshot
        .commits
        .iter()
        .find(|commit| commit.summary == "detached tagged commit")
        .expect("detached tagged commit should be in graph");
    assert!(detached
        .refs
        .iter()
        .any(|reference| reference.name == "detached-topic"));
    assert!(detached.branch.is_none());
}
