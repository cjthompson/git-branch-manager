
use git_branch_manager::git::{
    graph,
    operations,
};

use crate::common::*;

#[test]
fn test_graph_surgical_update_after_delete_local_action() {
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Create a feature branch with one commit on top of main.
    run_git(dir, &["checkout", "-b", "feature/surgical"]);
    run_git(
        dir,
        &["commit", "--allow-empty", "-m", "feature commit"],
    );
    let feature_oid = git_output(dir, &["rev-parse", "HEAD"]);
    let feature_oid = feature_oid.trim();

    // Load the graph and confirm feature/surgical appears as a ref.
    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions::default(),
    )
    .expect("graph load should succeed");
    assert!(
        snapshot.commits.iter().any(|c| {
            c.oid == feature_oid
                && c.refs
                    .iter()
                    .any(|r| r.name == "feature/surgical")
        }),
        "feature/surgical ref should be present before delete"
    );

    // A branch cannot be deleted while it is checked out in the main
    // worktree. Keep the loaded pre-action snapshot, then switch to main so
    // the delete exercises the view patch rather than a reload.
    run_git(dir, &["checkout", "main"]);

    // Observe the actual ref change and run the same incremental updater used
    // by App after queued Git actions.
    let before_state = graph::GraphRepositoryState::capture(dir).unwrap();
    let op_result = operations::delete_local_force(&repo, "feature/surgical");
    assert!(op_result.success, "delete_local must succeed");
    let after_state = graph::GraphRepositoryState::capture(dir).unwrap();

    let patched = graph::update_graph_incrementally(
        dir, snapshot, &graph::GraphRepositoryDelta::between(before_state, after_state),
        graph::GraphLoadOptions::default(),
    ).expect("incremental update should succeed");
    assert!(
        !patched.commits.iter().any(|c| {
            c.refs
                .iter()
                .any(|r| r.name == "feature/surgical")
        }),
        "feature/surgical ref must be dropped after incremental update"
    );
    assert!(
        !patched.commits.iter().any(|c| c.oid == feature_oid),
        "unreachable feature commit must leave the resulting Graph window"
    );
}

#[test]
fn incremental_updates_preserve_captured_squash_relationships() {
    let (tmpdir, destination, exact, _high, _low) = paired_squash_fixture();
    let dir = tmpdir.path();
    let options = graph::GraphLoadOptions {
        base_branch: Some("main".into()),
        ..tmpdir.graph_options()
    };
    let snapshot = graph::load_graph_with_squash_annotations(dir, options.clone()).unwrap();
    let expected = snapshot
        .commits
        .iter()
        .find(|c| c.oid == destination)
        .unwrap()
        .relationships
        .clone();
    assert_eq!(expected.len(), 4);
    assert_eq!(
        expected
            .iter()
            .filter(|relationship| relationship.kind == graph::RelationshipKind::SquashMerge)
            .count(),
        3
    );
    assert_eq!(
        expected
            .iter()
            .filter(|relationship| relationship.kind == graph::RelationshipKind::CherryPick)
            .cloned()
            .collect::<Vec<_>>(),
        vec![graph::GraphRelationship {
            kind: graph::RelationshipKind::CherryPick,
            matching: graph::RelationshipMatch::Exact,
            destination_oid: destination.clone(),
            destination_refs: vec!["main".into()],
            source_oid: exact,
            source_refs: vec!["feature/exact".into(), "feature/exact-alias".into()],
        }]
    );
    let before = graph::GraphRepositoryState::capture(dir).unwrap();
    run_git(dir, &["branch", "-m", "feature/high", "feature/renamed"]);
    let renamed = graph::GraphRepositoryState::capture(dir).unwrap();
    let metadata = graph::update_graph_incrementally(
        dir,
        snapshot,
        &graph::GraphRepositoryDelta::between(before, renamed.clone()),
        options.clone(),
    )
    .unwrap();
    assert_eq!(
        metadata
            .commits
            .iter()
            .find(|c| c.oid == destination)
            .unwrap()
            .relationships,
        expected,
        "metadata updates retain captured names and pairs"
    );
    run_git(dir, &["commit", "--allow-empty", "-m", "advance topology"]);
    let advanced = graph::GraphRepositoryState::capture(dir).unwrap();
    let topology = graph::update_graph_incrementally(
        dir,
        metadata,
        &graph::GraphRepositoryDelta::between(renamed, advanced),
        options,
    )
    .unwrap();
    assert_eq!(
        topology
            .commits
            .iter()
            .find(|c| c.oid == destination)
            .unwrap()
            .relationships,
        expected,
        "topology rebuilds retain the complete relationship vector"
    );
}

#[test]
fn test_graph_incremental_update_after_commit_affecting_merge() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Create a feature branch with one commit on top of main.
    run_git(dir, &["checkout", "-b", "feature/full-reload"]);
    run_git(
        dir,
        &["commit", "--allow-empty", "-m", "feature commit"],
    );
    let feature_oid = git_output(dir, &["rev-parse", "HEAD"]);
    let feature_oid = feature_oid.trim();

    // Switch back to main and load the initial graph snapshot.
    run_git(dir, &["checkout", "main"]);
    let options = graph::GraphLoadOptions::default();
    let before_state = graph::GraphRepositoryState::capture(dir).unwrap();
    let before = graph::load_graph_with_squash_annotations(dir, options.clone()).expect("initial graph load should succeed");
    assert!(
        before.commits.iter().any(|c| c.oid == feature_oid),
        "feature commit should be reachable from main before merge"
    );

    // Perform the commit-affecting action (a merge).
    run_git(dir, &["merge", "--no-ff", "feature/full-reload", "-m", "merge"]);
    let after_state = graph::GraphRepositoryState::capture(dir).unwrap();
    let after = graph::update_graph_incrementally(
        dir,
        before.clone(),
        &graph::GraphRepositoryDelta::between(before_state, after_state),
        options,
    ).expect("incremental merge update should succeed");

    // The merge commit is a brand-new OID that did not exist in `before`.
    let merge_present = after
        .commits
        .iter()
        .any(|c| c.parents.len() == 2 && c.refs.iter().any(|r| r.is_current));
    assert!(
        merge_present,
        "incrementally updated snapshot must include the new merge commit"
    );

    // The previous snapshot is now stale: the surgical patch route is
    // inapplicable because OID topology changed. Confirm the snapshot's
    // commit set differs from the original.
    let before_oids: std::collections::HashSet<&str> =
        before.commits.iter().map(|c| c.oid.as_str()).collect();
    let after_oids: std::collections::HashSet<&str> =
        after.commits.iter().map(|c| c.oid.as_str()).collect();
    assert_ne!(
        before_oids, after_oids,
        "commit set must include the merge result while retaining prior records"
    );
}

#[test]
fn incremental_update_keeps_history_until_its_last_eligible_ref_is_deleted() {
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();
    run_git(dir, &["checkout", "-b", "feature/reachable"]);
    run_git(dir, &["commit", "--allow-empty", "-m", "reachable commit"]);
    let tip = git_output(dir, &["rev-parse", "HEAD"]).trim().to_string();
    run_git(dir, &["branch", "feature/retainer"]);
    run_git(dir, &["checkout", "main"]);
    let options = graph::GraphLoadOptions::default();
    let before_state = graph::GraphRepositoryState::capture(dir).unwrap();
    let before = graph::load_graph(dir, options.clone()).unwrap();

    assert!(operations::delete_local_force(&repo, "feature/reachable").success);
    let retained_state = graph::GraphRepositoryState::capture(dir).unwrap();
    let retained = graph::update_graph_incrementally(
        dir,
        before,
        &graph::GraphRepositoryDelta::between(before_state, retained_state.clone()),
        options.clone(),
    ).unwrap();
    assert!(retained.commits.iter().any(|commit| commit.oid == tip), "the surviving ref must retain its history");

    assert!(operations::delete_local_force(&repo, "feature/retainer").success);
    let deleted_state = graph::GraphRepositoryState::capture(dir).unwrap();
    let deleted = graph::update_graph_incrementally(
        dir,
        retained,
        &graph::GraphRepositoryDelta::between(retained_state, deleted_state),
        options,
    ).unwrap();
    assert!(!deleted.commits.iter().any(|commit| commit.oid == tip), "the final ref deletion must remove unreachable history");
}
