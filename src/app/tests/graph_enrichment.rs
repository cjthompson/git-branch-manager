use super::*;

#[test]
fn graph_enrichment_with_stale_generation_is_dropped_by_drain() {
    // Drives the App's drain_channels directly: a stale enrichment
    // message (generation 1) must NOT mark a snapshot whose generation
    // is 2. This is the App-level half of the "stale enrichment cannot
    // overwrite a newer snapshot" requirement.
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    app.active_view = ViewId::Graph;
    // Force a known starting generation.
    app.spawn_graph_load(500, false);
    let first_generation = app.graph_generation;

    // Inject the structural snapshot for the first load directly.
    let (tx, rx) = mpsc::channel();
    tx.send(Ok(graph::GraphSnapshot {
        source: graph::GraphSource::Gleisbau,
        commits: vec![graph::GraphCommit {
            oid: "1111111111111111111111111111111111111111".into(),
            summary: "first".into(),
            parents: vec![],
            lane: Some(0),
            branch: None,
            refs: vec![graph::GraphRef {
                name: "main".into(),
                kind: graph::GraphRefKind::LocalBranch,
                has_linked_worktree: false,
                is_current: false,
                tracking: None,
            }],
            relationships: vec![],
            ..graph::GraphCommit::default()
        }],
        lines: vec![graph::GraphLine {
            graph: "*".into(),
            commit_index: Some(0),
        }],
        ref_counts: Default::default(),
        max_count: 500,
        includes_remotes: false,
        generation: None,
    }))
    .expect("send first snapshot");
    app.graph_rx = Some(rx);
    app.drain_channels();
    assert!(!app.graph.is_loading());

    // Trigger a second load — bumps graph_generation.
    app.spawn_graph_load(500, false);
    let second_generation = app.graph_generation;
    assert!(second_generation > first_generation);

    // Inject a structural snapshot for the second load.
    let (tx2, rx2) = mpsc::channel();
    tx2.send(Ok(graph::GraphSnapshot {
        source: graph::GraphSource::Gleisbau,
        commits: vec![graph::GraphCommit {
            oid: "2222222222222222222222222222222222222222".into(),
            summary: "second".into(),
            parents: vec![],
            lane: Some(0),
            branch: None,
            refs: vec![],
            relationships: vec![],
            ..graph::GraphCommit::default()
        }],
        lines: vec![graph::GraphLine {
            graph: "*".into(),
            commit_index: Some(0),
        }],
        ref_counts: Default::default(),
        max_count: 500,
        includes_remotes: false,
        generation: None,
    }))
    .expect("send second snapshot");
    app.graph_rx = Some(rx2);
    app.drain_channels();

    // Now a STALE enrichment arrives (tagged with the first generation).
    // The App's drain must drop it because msg.generation != current.
    let (etx, erx) = mpsc::channel();
    etx.send(graph::GraphEnrichmentMsg {
        generation: first_generation,
        updates: vec![graph::GraphRelationship {
            kind: graph::RelationshipKind::SquashMerge,
            matching: graph::RelationshipMatch::Exact,
            destination_oid: "2222222222222222222222222222222222222222".into(),
            destination_refs: vec!["main".into()],
            source_oid: "1111111111111111111111111111111111111111".into(),
            source_refs: vec![],
        }],
    })
    .expect("send stale enrichment");
    app.graph_enrich_rx = Some(erx);
    app.drain_channels();

    let current = app
        .graph
        .snapshot()
        .expect("snapshot present after second load");
    assert!(
        !current.commits[0].is_possible_squash_merge(),
        "stale enrichment must not overwrite the newer snapshot's marker"
    );
}

#[test]
fn bump_graph_generation_keeps_signal_in_step() {
    let dir = tempfile::tempdir().unwrap();
    run_git(dir.path(), &["init", "-b", "main"]);
    run_git(dir.path(), &["config", "user.name", "Test"]);
    run_git(dir.path(), &["config", "user.email", "test@example.com"]);
    run_git(dir.path(), &["commit", "--allow-empty", "-m", "root"]);
    let mut app = App::new(dir.path().to_path_buf(), "main".into(), Config::default());
    assert_eq!(app.graph_generation_signal.load(Ordering::Acquire), 0);
    app.request_graph_update(Err("no snapshot".into()));
    assert_eq!(app.graph_generation, 0);
    assert_eq!(app.graph_generation_signal.load(Ordering::Acquire), 0);
    app.bump_graph_generation();
    assert_eq!(app.graph_generation, 1);
    assert_eq!(app.graph_generation_signal.load(Ordering::Acquire), 1);
    app.spawn_graph_load(500, false);
    assert_eq!(
        app.graph_generation_signal.load(Ordering::Acquire),
        app.graph_generation
    );
    wait_until(|| {
        app.drain_channels();
        !app.graph.is_loading() && app.graph_rx.is_none()
    });
    assert!(app.graph.snapshot().is_some());
    let before = graph::GraphRepositoryState::capture(dir.path()).unwrap();
    let generation = app.graph_generation;
    let (_tx, rx) = mpsc::channel();
    app.graph_update_rx = Some(rx);
    app.request_graph_update(Ok(graph::GraphRepositoryDelta::between(
        before.clone(),
        before.clone(),
    )));
    assert_eq!(
        app.graph_generation, generation,
        "metadata-only update keeps generation"
    );
    assert_eq!(
        app.graph_generation_signal.load(Ordering::Acquire),
        generation
    );
    run_git(dir.path(), &["branch", "feature"]);
    let after = graph::GraphRepositoryState::capture(dir.path()).unwrap();
    app.request_graph_update(Ok(graph::GraphRepositoryDelta::between(before, after)));
    assert_eq!(
        app.graph_generation,
        generation + 1,
        "queued invalidation cancels promptly"
    );
    assert_eq!(
        app.graph_generation_signal.load(Ordering::Acquire),
        app.graph_generation
    );
    assert!(app.graph_enrich_rx.is_none());
    app.graph_update_rx = None;
    let _ = app.pending_graph_deltas.remove(0);
    let invalidating = app.pending_graph_deltas.remove(0);
    app.request_graph_update(invalidating);
    assert_eq!(
        app.graph_generation,
        generation + 1,
        "replay does not bump twice"
    );
    assert_eq!(
        app.graph_generation_signal.load(Ordering::Acquire),
        app.graph_generation
    );
}

#[test]
fn queued_failed_graph_update_cancels_enrichment_until_reload() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = action_refresh_app(dir.path());
    let snapshot = graph::load_graph(
        dir.path(),
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            cache_root: app.cache_root.clone(),
            ..Default::default()
        },
    )
    .unwrap();
    app.graph.apply_result(Ok(snapshot));
    let cancel = graph::EnrichmentCancel::new(Arc::clone(&app.graph_generation_signal), 0);
    let (_update_tx, update_rx) = mpsc::channel();
    app.graph_update_rx = Some(update_rx);
    let (_enrich_tx, enrich_rx) = mpsc::channel();
    app.graph_enrich_rx = Some(enrich_rx);
    app.request_graph_update(Err("queued capture failure".into()));
    assert!(
        cancel.is_cancelled(),
        "queued failure cancels before updater finishes"
    );
    assert_eq!(app.graph_generation, 1);
    assert_eq!(app.graph_generation_signal.load(Ordering::Acquire), 1);
    assert!(app.graph_enrich_rx.is_none());
    app.request_graph_update(Err("another queued failure".into()));
    assert_eq!(
        app.graph_generation, 1,
        "pending invalidation does not bump twice"
    );
    app.graph_update_rx = None;
    let queued = app.pending_graph_deltas.remove(0);
    app.request_graph_update(queued);
    assert!(app.graph.is_loading());
    assert_eq!(
        app.graph_generation, 2,
        "replay fallback starts a new structural load"
    );
    assert_eq!(
        app.graph_generation_signal.load(Ordering::Acquire),
        app.graph_generation
    );
}

#[test]
fn graph_enrichment_with_matching_generation_is_applied_by_drain() {
    // Companion to the stale-enrichment test: a matching-generation
    // enrichment message must apply, confirming the App's gate is the
    // only thing filtering enrichment.
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    app.active_view = ViewId::Graph;
    app.spawn_graph_load(500, false);
    let generation = app.graph_generation;

    let (tx, rx) = mpsc::channel();
    tx.send(Ok(graph::GraphSnapshot {
        source: graph::GraphSource::Gleisbau,
        commits: vec![graph::GraphCommit {
            oid: "3333333333333333333333333333333333333333".into(),
            summary: "third".into(),
            parents: vec![],
            lane: Some(0),
            branch: None,
            refs: vec![],
            relationships: vec![],
            ..graph::GraphCommit::default()
        }],
        lines: vec![graph::GraphLine {
            graph: "*".into(),
            commit_index: Some(0),
        }],
        ref_counts: Default::default(),
        max_count: 500,
        includes_remotes: false,
        generation: None,
    }))
    .expect("send snapshot");
    app.graph_rx = Some(rx);
    app.drain_channels();

    // Enrichment tagged with the same generation as the snapshot.
    let (etx, erx) = mpsc::channel();
    etx.send(graph::GraphEnrichmentMsg {
        generation,
        updates: vec![graph::GraphRelationship {
            kind: graph::RelationshipKind::SquashMerge,
            matching: graph::RelationshipMatch::Exact,
            destination_oid: "3333333333333333333333333333333333333333".into(),
            destination_refs: vec!["main".into()],
            source_oid: "1111111111111111111111111111111111111111".into(),
            source_refs: vec![],
        }],
    })
    .expect("send matching enrichment");
    app.graph_enrich_rx = Some(erx);
    app.drain_channels();

    let current = app.graph.snapshot().expect("snapshot present");
    assert!(
        current.commits[0].is_possible_squash_merge(),
        "matching-generation enrichment must be applied by the drain"
    );
}
