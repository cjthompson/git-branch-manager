
use git_branch_manager::git::graph;

use crate::common::*;

#[test]
fn test_graph_cli_fallback_receives_exact_squash_annotation() {
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();
    let root_oid = repo.head().unwrap().target().unwrap();

    run_git(dir, &["checkout", "-b", "feature/fallback-squash"]);
    std::fs::write(dir.join("fallback.txt"), "fallback patch\n").unwrap();
    run_git(dir, &["add", "fallback.txt"]);
    run_git(dir, &["commit", "-m", "fallback source"]);
    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/fallback-squash"]);
    run_git(dir, &["commit", "-m", "fallback landing"]);
    let squash_oid = git_output(dir, &["rev-parse", "HEAD"]);

    std::fs::write(dir.join(".git/shallow"), format!("{root_oid}\n")).unwrap();
    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..tmpdir.graph_options()
        },
    )
    .expect("CLI fallback should receive the shared squash annotation");

    assert!(matches!(
        snapshot.source,
        graph::GraphSource::GitCliFallback { .. }
    ));
    assert!(snapshot
        .commits
        .iter()
        .find(|commit| commit.oid == squash_oid)
        .expect("fallback squash commit should be displayed")
        .is_possible_squash_merge());
}

#[test]
fn test_graph_squash_matching_stays_within_displayed_history_bound() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/outside-window"]);
    std::fs::write(dir.join("bounded.txt"), "bounded patch\n").unwrap();
    run_git(dir, &["add", "bounded.txt"]);
    run_git(dir, &["commit", "-m", "bounded source"]);
    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/outside-window"]);
    run_git(dir, &["commit", "-m", "bounded landing"]);
    run_git(
        dir,
        &["commit", "--allow-empty", "-m", "newest displayed commit"],
    );

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            max_count: 1,
            base_branch: Some("main".into()),
            ..tmpdir.graph_options()
        },
    )
    .expect("bounded graph load should not inspect undisplayed matching history");

    assert_eq!(snapshot.commits.len(), 1);
    assert!(snapshot
        .commits
        .iter()
        .all(|commit| !commit.is_possible_squash_merge()));
}

#[test]
fn test_load_graph_fallback_with_remotes_includes_tag_only_history() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    run_git(&work_dir, &["checkout", "-b", "remote-only"]);
    std::fs::write(work_dir.join("remote-only.txt"), "remote\n").unwrap();
    run_git(&work_dir, &["add", "remote-only.txt"]);
    run_git(&work_dir, &["commit", "-m", "remote-only commit"]);
    run_git(&work_dir, &["push", "-u", "origin", "remote-only"]);
    run_git(&work_dir, &["checkout", "main"]);
    run_git(&work_dir, &["branch", "-D", "remote-only"]);

    run_git(&work_dir, &["checkout", "--orphan", "tag-only"]);
    run_git(&work_dir, &["rm", "-rf", "."]);
    std::fs::write(work_dir.join("tag-only.txt"), "tag\n").unwrap();
    run_git(&work_dir, &["add", "tag-only.txt"]);
    run_git(&work_dir, &["commit", "-m", "tag-only commit"]);
    run_git(&work_dir, &["tag", "tag-only"]);
    run_git(&work_dir, &["checkout", "main"]);
    run_git(&work_dir, &["branch", "-D", "tag-only"]);

    let repo = git2::Repository::open(&work_dir).unwrap();
    let head = repo.head().unwrap().target().unwrap();
    std::fs::write(work_dir.join(".git/shallow"), format!("{head}\n")).unwrap();

    let snapshot = graph::load_graph_with_squash_annotations(
        &work_dir,
        graph::GraphLoadOptions {
            include_remotes: true,
            .._tmpdir.graph_options()
        },
    )
    .expect("git CLI fallback should retain the remote and tag-only ref range");

    assert!(matches!(
        snapshot.source,
        graph::GraphSource::GitCliFallback { ref cause } if cause.contains("shallow")
    ));
    assert!(snapshot.commits.iter().any(|commit| {
        commit.refs.iter().any(|reference| {
            reference.name == "origin/remote-only"
                && reference.kind == graph::GraphRefKind::RemoteBranch
        })
    }));
    assert!(snapshot.commits.iter().any(|commit| {
        commit.refs.iter().any(|reference| {
            reference.name == "tag-only" && reference.kind == graph::GraphRefKind::Tag
        })
    }));
}
