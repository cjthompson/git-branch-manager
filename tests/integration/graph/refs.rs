
use git_branch_manager::git::graph;

use crate::common::*;

#[test]
fn test_load_graph_includes_remote_refs_only_when_requested() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    run_git(&work_dir, &["checkout", "-b", "remote-only"]);
    std::fs::write(work_dir.join("remote-only.txt"), "remote\n").unwrap();
    run_git(&work_dir, &["add", "remote-only.txt"]);
    run_git(&work_dir, &["commit", "-m", "remote-only commit"]);
    run_git(&work_dir, &["push", "-u", "origin", "remote-only"]);
    run_git(&work_dir, &["checkout", "main"]);
    run_git(&work_dir, &["branch", "-D", "remote-only"]);

    let local_only = graph::load_graph_with_squash_annotations(&work_dir, _tmpdir.graph_options())
        .expect("local graph load should succeed");
    assert!(!local_only
        .commits
        .iter()
        .flat_map(|commit| commit.refs.iter())
        .any(|reference| reference.name == "origin/remote-only"));
    assert_eq!(local_only.ref_counts.remote, 0);

    let with_remotes = graph::load_graph_with_squash_annotations(
        &work_dir,
        graph::GraphLoadOptions {
            include_remotes: true,
            .._tmpdir.graph_options()
        },
    )
    .expect("remote graph load should succeed");
    assert!(with_remotes.commits.iter().any(|commit| {
        commit
            .refs
            .iter()
            .any(|reference| reference.name == "origin/remote-only")
    }));
    assert!(with_remotes.ref_counts.remote > 0);
}

#[test]
fn test_graph_refs_include_remote_tracking_state() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    run_git(&work_dir, &["checkout", "-b", "ahead"]);
    run_git(&work_dir, &["commit", "--allow-empty", "-m", "ahead base"]);
    run_git(&work_dir, &["push", "-u", "origin", "ahead"]);
    run_git(
        &work_dir,
        &["commit", "--allow-empty", "-m", "ahead local commit"],
    );
    run_git(&work_dir, &["checkout", "main"]);

    let snapshot = graph::load_graph_with_squash_annotations(
        &work_dir,
        graph::GraphLoadOptions {
            include_remotes: true,
            .._tmpdir.graph_options()
        },
    )
    .expect("graph load with remote tracking refs should succeed");

    let ahead_ref = snapshot
        .commits
        .iter()
        .find(|commit| commit.summary == "ahead local commit")
        .and_then(|commit| {
            commit
                .refs
                .iter()
                .find(|reference| reference.name == "ahead")
        })
        .expect("local ahead ref should be attached to its tip");
    let tracking = ahead_ref
        .tracking
        .as_ref()
        .expect("local branch should expose matching remote tracking");
    assert_eq!((tracking.ahead, tracking.behind), (1, 0));

    let main_ref = snapshot
        .commits
        .iter()
        .flat_map(|commit| commit.refs.iter())
        .find(|reference| reference.name == "main")
        .expect("main ref should be present");
    let main_tracking = main_ref
        .tracking
        .as_ref()
        .expect("main should expose its matching remote");
    assert_eq!((main_tracking.ahead, main_tracking.behind), (0, 0));

    let remote_ahead = snapshot
        .commits
        .iter()
        .find(|commit| commit.summary == "ahead base")
        .expect("remote tracking ref should remain on its own tip");
    assert!(remote_ahead
        .refs
        .iter()
        .any(|reference| reference.name == "origin/ahead"));
    assert!(!remote_ahead
        .refs
        .iter()
        .any(|reference| reference.name == "ahead"));
}

#[test]
fn test_graph_refs_mark_only_linked_worktrees() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();
    run_git(dir, &["checkout", "-b", "feature/linked"]);
    run_git(dir, &["checkout", "main"]);
    let worktree_parent = tempfile::tempdir().expect("worktree parent");
    let linked_path = worktree_parent.path().join("linked");
    let linked_path_string = linked_path.to_string_lossy().into_owned();
    run_git(
        dir,
        &["worktree", "add", &linked_path_string, "feature/linked"],
    );

    let snapshot = graph::load_graph_with_squash_annotations(dir, tmpdir.graph_options())
        .expect("graph load should include linked worktree metadata");
    let linked = snapshot
        .commits
        .iter()
        .flat_map(|commit| commit.refs.iter())
        .find(|reference| reference.name == "feature/linked")
        .expect("linked branch ref");
    assert!(linked.has_linked_worktree);
    let main = snapshot
        .commits
        .iter()
        .flat_map(|commit| commit.refs.iter())
        .find(|reference| reference.name == "main")
        .expect("main branch ref");
    assert!(!main.has_linked_worktree);
}

#[test]
fn test_load_graph_caps_history_at_five_hundred_commits() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    for index in 0..501 {
        let message = format!("history commit {index}");
        run_git(dir, &["commit", "--allow-empty", "-m", &message]);
    }

    let snapshot = graph::load_graph_with_squash_annotations(dir, tmpdir.graph_options())
        .expect("bounded graph load should succeed");
    assert_eq!(snapshot.commits.len(), 500);
    assert_eq!(snapshot.max_count, 500);
}

#[test]
fn test_load_graph_uses_cli_fallback_for_shallow_repository() {
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();
    let head = repo.head().unwrap().target().unwrap();
    std::fs::write(dir.join(".git/shallow"), format!("{head}\n")).unwrap();

    let snapshot = graph::load_graph_with_squash_annotations(dir, tmpdir.graph_options())
        .expect("git CLI fallback should handle a shallow repository");

    assert!(matches!(
        snapshot.source,
        graph::GraphSource::GitCliFallback { ref cause } if cause.contains("shallow")
    ));
    assert_eq!(snapshot.commits.len(), 1);
    assert!(snapshot
        .lines
        .iter()
        .any(|line| line.commit_index.is_some()));
}

/// `main` and `origin/main` pointing at the same commit must expose BOTH refs
/// (Local and Remote) on that commit, even when `include_remotes=false`. The
/// Remote ref drives the cloud icon under the LRT column; suppressing it makes
/// the commit look like it has no upstream tracking, which the user reported
/// on `claude-code-config` (`64931a9d` where `main` had `[origin/main]`
/// tracking but the cloud was missing).
#[test]
fn test_graph_remote_ref_visible_at_default_include_remotes_for_tracked_base() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    let options = graph::GraphLoadOptions {
        max_count: 50,
        include_remotes: false, // <-- explicit: default behavior under test
        line_style: graph::GraphLineStyle::Thin,
        base_branch: Some("main".to_string()),
        cache_root: _tmpdir.graph_options().cache_root,
    };
    let snapshot =
        graph::load_graph_with_squash_annotations(&work_dir, options).expect("graph load failed");

    // Find the local HEAD commit (the one `main` and `origin/main` both point to).
    let main_tip = git_output(&work_dir, &["rev-parse", "main"]);

    let commit = snapshot
        .commits
        .iter()
        .find(|commit| commit.oid == main_tip)
        .expect("main tip should appear in snapshot");

    let has_local_main = commit
        .refs
        .iter()
        .any(|r| r.kind == graph::GraphRefKind::LocalBranch && r.name == "main");
    let has_remote_main = commit
        .refs
        .iter()
        .any(|r| r.kind == graph::GraphRefKind::RemoteBranch && r.name == "origin/main");

    assert!(
        has_local_main,
        "commit {} (main) should carry a LocalBranch ref named 'main'",
        main_tip
    );
    assert!(
        has_remote_main,
        "commit {} (main) should carry a RemoteBranch ref named 'origin/main' \
         even when include_remotes=false, so the LRT 'R' column shows the cloud",
        main_tip
    );
}
