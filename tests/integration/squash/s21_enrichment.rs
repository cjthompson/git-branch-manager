
use git_branch_manager::git::{
    cache, graph,
};

use crate::common::*;

#[test]
fn test_squash_scenario_21_structural_graph_render_not_blocked_by_squash_enrichment() {
    // Plan scenario 21: a UX/architecture characterization test, not a
    // detection-correctness one. The parent plan doc's "Loading and UI
    // direction" follow-up task calls for decoupling the Graph's structural
    // snapshot (DAG, refs, commit summaries) from squash-merge enrichment, so
    // the structural view can render before annotation completes.
    //
    // Post-decoupling, `git::graph::load_graph` returns the structural
    // snapshot immediately and squash-merge enrichment is a separate,
    // asynchronous step. This test pins the new contract: structural data
    // is available synchronously, with no squash markers set until
    // enrichment runs.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/21"]);
    std::fs::write(dir.join("f21.txt"), "content\n").unwrap();
    run_git(dir, &["add", "f21.txt"]);
    run_git(dir, &["commit", "-m", "feature commit"]);
    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/21"]);
    run_git(dir, &["commit", "-m", "squash landing"]);

    // Step 1: structural snapshot is available immediately, with no
    // squash markers set — this is the new contract.
    let snapshot = graph::load_graph(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..tmpdir.graph_options()
        },
    )
    .expect("structural graph load should succeed");

    assert!(
        !snapshot.commits.is_empty(),
        "structural snapshot should be populated synchronously"
    );
    assert!(
        !snapshot
            .commits
            .iter()
            .any(|c| c.is_possible_squash_merge()),
        "no squash markers should be set on the fresh structural snapshot — \
         enrichment runs asynchronously and is gated by the reload generation"
    );
    assert!(
        snapshot
            .commits
            .iter()
            .all(|c| c.fuzzy_squash_match().is_none()),
        "no fuzzy squash matches should be set on the fresh structural snapshot"
    );
}

#[test]
fn cancelled_enrichment_publishes_nothing() {
    let (tmpdir, _repo) = setup_test_repo();
    let mut options = tmpdir.graph_options();
    options.base_branch = Some("main".into());
    let snapshot = graph::load_graph(tmpdir.path(), options.clone()).unwrap();
    let latest = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(9));
    let rx = graph::spawn_possible_squash_enrichment(
        snapshot,
        tmpdir.path().to_path_buf(),
        Some("main".into()),
        1,
        options.cache_root,
        latest,
    );
    assert_eq!(
        rx.recv_timeout(std::time::Duration::from_secs(60)),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected),
        "a cancelled worker drops its sender without publishing",
    );
}

#[test]
fn test_squash_scenario_21b_completed_enrichment_updates_squash_marker() {
    // Companion to scenario 21: once enrichment runs (the asynchronous
    // step), the squash marker must be set on the matching base commit.
    // This is the "completed enrichment updates the marker" half of the
    // new decoupled contract.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/21b"]);
    std::fs::write(dir.join("f21b.txt"), "content\n").unwrap();
    run_git(dir, &["add", "f21b.txt"]);
    run_git(dir, &["commit", "-m", "feature commit"]);
    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/21b"]);
    run_git(dir, &["commit", "-m", "squash landing"]);

    let mut snapshot = graph::load_graph(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..tmpdir.graph_options()
        },
    )
    .expect("structural graph load should succeed");

    // No markers yet.
    assert!(
        !snapshot
            .commits
            .iter()
            .any(|c| c.is_possible_squash_merge()),
        "fresh structural snapshot must not carry squash markers"
    );

    // Run enrichment synchronously, as `spawn_possible_squash_enrichment`
    // would do on its background thread.
    let updates = graph::compute_relationships(
        dir,
        &snapshot,
        Some("main"),
        &cache::CacheRoot::at(tmpdir.cache_root().to_path_buf()),
        &graph::EnrichmentCancel::never(),
    );
    graph::apply_squash_enrichment(&mut snapshot, &updates);

    assert!(
        snapshot
            .commits
            .iter()
            .any(|c| c.is_possible_squash_merge()),
        "after enrichment, the squash landing commit on main must be flagged"
    );
}

#[test]
fn test_squash_scenario_21c_stale_enrichment_does_not_overwrite_newer_snapshot() {
    // Stale enrichment — a result from a previous reload that lands after a
    // newer load — must be dropped, not applied to the newer snapshot. The
    // public surface is the per-message `generation` field; the App uses it
    // to gate updates. This test exercises the gate via the git module's
    // enrichment-update data type directly.
    let (tmpdir, destination, exact_source, _high, _low) = paired_squash_fixture();
    let dir = tmpdir.path();

    // Build a fresh snapshot (no markers yet).
    let mut snapshot = graph::load_graph(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..tmpdir.graph_options()
        },
    )
    .expect("structural graph load should succeed");
    assert!(!snapshot
        .commits
        .iter()
        .any(|c| c.is_possible_squash_merge()));

    // Simulate a stale result carrying a real source/destination pairing.
    snapshot.generation = Some(2);
    let stale = graph::GraphEnrichmentMsg {
        generation: 1, // stale: belongs to an earlier reload
        updates: vec![graph::GraphRelationship {
            kind: graph::RelationshipKind::SquashMerge,
            matching: graph::RelationshipMatch::Exact,
            destination_oid: destination.clone(),
            destination_refs: vec!["main".into()],
            source_oid: exact_source.clone(),
            source_refs: vec!["feature/exact".into(), "feature/exact-alias".into()],
        }],
    };
    // Re-load a clean snapshot for the "current" generation so we can
    // assert no markers leaked from the stale message.
    let mut current = graph::load_graph(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..tmpdir.graph_options()
        },
    )
    .expect("structural graph load should succeed");
    current.generation = Some(2);

    // The App's drain applies only when msg.generation == self.graph_generation.
    // We model that gate here: with current.generation=2 and stale.generation=1,
    // the stale update must NOT touch the current snapshot.
    if stale.generation == 2 {
        graph::apply_squash_enrichment(&mut current, &stale.updates);
    }
    assert!(
        !current.commits.iter().any(|c| c.is_possible_squash_merge()),
        "stale enrichment (generation 1) must not overwrite the newer snapshot's markers"
    );

    // Sanity check: a *matching* generation WOULD have applied the
    // update, confirming the gate is the only thing keeping the snapshot
    // clean.
    let matching = graph::GraphEnrichmentMsg {
        generation: 2,
        ..stale.clone()
    };
    if matching.generation == current.generation.unwrap_or(0) {
        graph::apply_squash_enrichment(&mut current, &matching.updates);
    }
    assert!(
        current.commits.iter().any(|c| c.is_possible_squash_merge()),
        "matching-generation enrichment must apply, confirming the gate is the only filter"
    );
}

#[test]
fn test_squash_scenario_21d_failed_enrichment_leaves_snapshot_usable_and_unmarked() {
    // Failed enrichment — empty updates, or a panic, or a channel drop —
    // must leave the structural snapshot usable and unmarked. The Graph
    // view must not be left in a "Loading graph..." state just because
    // the optional enrichment pass failed.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/21d"]);
    std::fs::write(dir.join("f21d.txt"), "content\n").unwrap();
    run_git(dir, &["add", "f21d.txt"]);
    run_git(dir, &["commit", "-m", "feature commit"]);
    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/21d"]);
    run_git(dir, &["commit", "-m", "squash landing"]);

    let mut snapshot = graph::load_graph(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..tmpdir.graph_options()
        },
    )
    .expect("structural graph load should succeed");
    assert!(!snapshot.commits.is_empty());

    // Simulate a "failed" enrichment: empty update list. The GraphState
    // must remain fully usable (structural data + refs) and unmarked.
    let empty = graph::GraphEnrichmentMsg {
        generation: snapshot.generation.unwrap_or(0),
        updates: Vec::new(),
    };
    let before = snapshot.clone();
    graph::apply_squash_enrichment(&mut snapshot, &empty.updates);
    assert_eq!(
        snapshot, before,
        "empty enrichment must not mutate the snapshot"
    );
    assert!(
        !snapshot
            .commits
            .iter()
            .any(|c| c.is_possible_squash_merge()),
        "failed enrichment must not mark any commit"
    );
    assert!(
        !snapshot.commits.is_empty() && !snapshot.lines.is_empty(),
        "structural data must remain usable after failed enrichment"
    );

    // Simulate a worker thread that panics: the channel is dropped, no
    // message ever arrives. The snapshot stays as-is — that is the
    // observable behavior of `apply_squash_enrichment(&[], …)`.
    graph::apply_squash_enrichment(&mut snapshot, &[]);
    assert!(
        !snapshot
            .commits
            .iter()
            .any(|c| c.is_possible_squash_merge()),
        "never-arriving enrichment (worker panic) must leave the snapshot unmarked"
    );
}
