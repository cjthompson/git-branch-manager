
use git_branch_manager::git::graph;

use crate::common::*;

#[test]
fn test_load_graph_preserves_merge_lanes_and_local_refs() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/graph"]);
    std::fs::write(dir.join("feature.txt"), "feature\n").unwrap();
    run_git(dir, &["add", "feature.txt"]);
    run_git(dir, &["commit", "-m", "feature commit"]);
    run_git(dir, &["checkout", "main"]);
    std::fs::write(dir.join("main.txt"), "main\n").unwrap();
    run_git(dir, &["add", "main.txt"]);
    run_git(dir, &["commit", "-m", "main commit"]);
    run_git(
        dir,
        &[
            "merge",
            "--no-ff",
            "feature/graph",
            "-m",
            "merge feature graph",
        ],
    );

    let snapshot = graph::load_graph_with_squash_annotations(dir, tmpdir.graph_options())
        .expect("graph loader should handle an ordinary merged local branch");

    assert!(matches!(snapshot.source, graph::GraphSource::Gleisbau));
    assert!(snapshot
        .commits
        .iter()
        .any(|commit| commit.parents.len() == 2));
    assert!(snapshot.commits.iter().any(|commit| {
        commit
            .refs
            .iter()
            .any(|reference| reference.name == "feature/graph")
    }));
    assert!(snapshot.commits.iter().any(|commit| commit.lane.is_some()));
    assert_eq!(snapshot.ref_counts.local, 2);
    assert_eq!(snapshot.ref_counts.remote, 0);

    fn assert_send<T: Send>() {}
    assert_send::<graph::GraphSnapshot>();
}

#[test]
fn test_graph_base_branch_owns_first_parent_chain_with_retained_merged_ref() {
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();
    let initial_oid = repo.head().unwrap().target().unwrap();

    run_git(dir, &["checkout", "-b", "feat/retained"]);
    run_git(
        dir,
        &["commit", "--allow-empty", "-m", "retained feature tip"],
    );
    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--ff-only", "feat/retained"]);
    run_git(
        dir,
        &["commit", "--allow-empty", "-m", "main inferred commit"],
    );
    run_git(dir, &["commit", "--allow-empty", "-m", "main tip"]);

    fn assert_main_owns_first_parent_chain(snapshot: &graph::GraphSnapshot) {
        for summary in ["main inferred commit", "retained feature tip"] {
            let commit = snapshot
                .commits
                .iter()
                .find(|commit| commit.summary == summary)
                .expect("expected commit should be in graph");
            assert_eq!(
                commit.branch.as_ref().map(|branch| branch.name.as_str()),
                Some("main"),
                "{summary} should remain on the base branch's inferred track"
            );
            if summary == "main inferred commit" {
                assert!(
                    commit.refs.is_empty(),
                    "the regression row must exercise inferred Refs text"
                );
            }
        }
    }

    let options = graph::GraphLoadOptions {
        base_branch: Some("main".into()),
        ..tmpdir.graph_options()
    };
    let snapshot = graph::load_graph_with_squash_annotations(dir, options.clone())
        .expect("Gleisbau should preserve base-branch ownership");
    assert!(matches!(snapshot.source, graph::GraphSource::Gleisbau));
    assert_main_owns_first_parent_chain(&snapshot);

    std::fs::write(dir.join(".git/shallow"), format!("{initial_oid}\n")).unwrap();
    let fallback = graph::load_graph_with_squash_annotations(dir, options)
        .expect("Git CLI fallback should preserve base-branch ownership");
    assert!(matches!(
        fallback.source,
        graph::GraphSource::GitCliFallback { .. }
    ));
    assert_main_owns_first_parent_chain(&fallback);
}

#[test]
fn test_graph_both_loaders_agree_on_author_and_author_date() {
    use git_branch_manager::git::graph::load_graph_with_squash_annotations;

    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();
    let initial_oid = repo.head().unwrap().target().unwrap();
    run_git(dir, &["config", "user.email", "agree@example.com"]);
    run_git(dir, &["config", "user.name", "Agree Bot"]);
    run_git(dir, &["commit", "--allow-empty", "-m", "comparison tip"]);

    let gleisbau = load_graph_with_squash_annotations(dir, tmpdir.graph_options())
        .expect("gleisbau should succeed");
    assert!(matches!(gleisbau.source, graph::GraphSource::Gleisbau));

    std::fs::write(dir.join(".git/shallow"), format!("{initial_oid}\n")).unwrap();
    let fallback = load_graph_with_squash_annotations(dir, tmpdir.graph_options())
        .expect("fallback should succeed");
    assert!(matches!(
        fallback.source,
        graph::GraphSource::GitCliFallback { .. }
    ));

    assert_eq!(
        gleisbau.commits.len(),
        fallback.commits.len(),
        "commit count differs between loaders"
    );
    for (g, f) in gleisbau.commits.iter().zip(fallback.commits.iter()) {
        assert_eq!(g.oid, f.oid);
        assert_eq!(
            g.author_name, f.author_name,
            "author_name mismatch on {}",
            g.oid
        );
        assert_eq!(
            g.author_email, f.author_email,
            "author_email mismatch on {}",
            g.oid
        );
        assert_eq!(
            g.authored_at, f.authored_at,
            "authored_at mismatch on {}",
            g.oid
        );
    }
}

#[test]
fn test_graph_default_branch_renders_in_column_zero() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/long-branch"]);
    for i in 0..8 {
        run_git(
            dir,
            &[
                "commit",
                "--allow-empty",
                "-m",
                &format!("feature commit {i}"),
            ],
        );
    }

    run_git(dir, &["checkout", "main"]);
    for i in 0..3 {
        run_git(
            dir,
            &["commit", "--allow-empty", "-m", &format!("main commit {i}")],
        );
    }

    run_git(dir, &["checkout", "-b", "release/y"]);
    run_git(dir, &["commit", "--allow-empty", "-m", "release commit 0"]);
    run_git(dir, &["checkout", "main"]);

    let options = graph::GraphLoadOptions {
        base_branch: Some("main".into()),
        ..tmpdir.graph_options()
    };
    let snapshot = graph::load_graph_with_squash_annotations(dir, options)
        .expect("graph loader should place the default branch in column 0");

    let main_commits: Vec<_> = snapshot
        .commits
        .iter()
        .filter(|commit| commit.summary.starts_with("main commit "))
        .collect();
    assert_eq!(
        main_commits.len(),
        3,
        "all 3 main commits should be in the graph"
    );
    for commit in main_commits {
        assert_eq!(
            commit.lane,
            Some(0),
            "main commit {:?} should sit in column 0",
            commit.summary
        );
    }

    let feature_commit = snapshot
        .commits
        .iter()
        .find(|commit| commit.summary == "feature commit 0")
        .expect("feature commit 0 should be in graph");
    assert_ne!(
        feature_commit.lane,
        Some(0),
        "the longer feature branch should not trivially land in column 0"
    );
}

#[test]
fn test_graph_base_branch_with_special_chars_lands_in_column_zero() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "release/1.0"]);
    for i in 0..3 {
        run_git(
            dir,
            &[
                "commit",
                "--allow-empty",
                "-m",
                &format!("release1.0 commit {i}"),
            ],
        );
    }

    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["checkout", "-b", "release1X0"]);
    run_git(
        dir,
        &["commit", "--allow-empty", "-m", "release1X0 commit 0"],
    );
    run_git(dir, &["checkout", "main"]);

    let options = graph::GraphLoadOptions {
        base_branch: Some("release/1.0".into()),
        ..tmpdir.graph_options()
    };
    let snapshot = graph::load_graph_with_squash_annotations(dir, options)
        .expect("graph loader should place the special-char base branch in column 0");

    let base_commits: Vec<_> = snapshot
        .commits
        .iter()
        .filter(|commit| commit.summary.starts_with("release1.0 commit "))
        .collect();
    assert_eq!(
        base_commits.len(),
        3,
        "all 3 release/1.0 commits should be in the graph"
    );
    for commit in base_commits {
        assert_eq!(
            commit.lane,
            Some(0),
            "release/1.0 commit {:?} should sit in column 0",
            commit.summary
        );
    }

    let sibling_commit = snapshot
        .commits
        .iter()
        .find(|commit| commit.summary == "release1X0 commit 0")
        .expect("release1X0 commit should be in graph");
    assert_ne!(
        sibling_commit.lane,
        Some(0),
        "an unescaped regex must not let release1X0 match the release/1.0 base pattern"
    );
}

#[test]
fn test_graph_base_branch_lands_in_column_zero_with_diverged_remote() {
    let (tmpdir, work_dir, _repo) = setup_remote_test_repo();
    let base_dir = tmpdir.path();
    let remote_dir = base_dir.join("remote.git");

    // Advance origin/main independently via a second clone, so work_dir's
    // local main and origin/main diverge from a shared ancestor.
    let advance_dir = base_dir.join("advance");
    run_git(
        base_dir,
        &["clone", remote_dir.to_str().unwrap(), "advance"],
    );
    run_git(&advance_dir, &["config", "user.name", "Test User"]);
    run_git(&advance_dir, &["config", "user.email", "test@example.com"]);
    run_git(
        &advance_dir,
        &["commit", "--allow-empty", "-m", "remote ahead"],
    );
    run_git(&advance_dir, &["push", "origin", "main"]);

    // Pull origin/main's new tip into work_dir's remote-tracking ref without
    // touching local main.
    run_git(&work_dir, &["fetch", "origin"]);

    // Diverge local main from origin/main by adding local-only commits.
    run_git(
        &work_dir,
        &["commit", "--allow-empty", "-m", "main commit 0"],
    );
    run_git(
        &work_dir,
        &["commit", "--allow-empty", "-m", "main commit 1"],
    );

    let options = graph::GraphLoadOptions {
        include_remotes: true,
        base_branch: Some("main".into()),
        ..tmpdir.graph_options()
    };
    let snapshot = graph::load_graph_with_squash_annotations(&work_dir, options)
        .expect("graph loader should place local main in column 0 despite a diverged remote");

    let local_main_commits: Vec<_> = snapshot
        .commits
        .iter()
        .filter(|commit| commit.summary.starts_with("main commit "))
        .collect();
    assert_eq!(
        local_main_commits.len(),
        2,
        "both local main commits should be in the graph"
    );
    for commit in local_main_commits {
        assert_eq!(
            commit.lane,
            Some(0),
            "local main commit {:?} should sit in column 0",
            commit.summary
        );
    }

    let remote_ahead_commit = snapshot
        .commits
        .iter()
        .find(|commit| commit.summary == "remote ahead")
        .expect("origin/main's diverged commit should be in graph");
    assert_ne!(
        remote_ahead_commit.lane,
        Some(0),
        "the diverged origin/main track must not also claim column 0"
    );
}
