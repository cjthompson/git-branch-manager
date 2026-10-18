use super::*;

#[test]
fn enrichment_after_invalidating_update_still_publishes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    run_git(path, &["init", "-b", "main"]);
    run_git(path, &["config", "user.name", "Test"]);
    run_git(path, &["config", "user.email", "test@example.com"]);
    run_git(path, &["commit", "--allow-empty", "-m", "root"]);
    run_git(path, &["checkout", "-b", "feature"]);
    std::fs::write(path.join("feature.txt"), "content\n").unwrap();
    run_git(path, &["add", "feature.txt"]);
    run_git(path, &["commit", "-m", "feature"]);
    run_git(path, &["checkout", "main"]);
    run_git(path, &["merge", "--squash", "feature"]);
    run_git(path, &["commit", "-m", "squash landing"]);
    let expected = git2::Repository::open(path)
        .unwrap()
        .head()
        .unwrap()
        .target()
        .unwrap()
        .to_string();
    let mut app = App::new(path.to_path_buf(), "main".into(), Config::default());
    let options = graph::GraphLoadOptions {
        base_branch: Some("main".into()),
        cache_root: app.cache_root.clone(),
        ..Default::default()
    };
    app.graph
        .apply_result(Ok(graph::load_graph(path, options).unwrap()));
    let before = graph::GraphRepositoryState::capture(path).unwrap();
    run_git(path, &["branch", "feature-copy", "feature"]);
    let after = graph::GraphRepositoryState::capture(path).unwrap();
    app.request_graph_update(Ok(graph::GraphRepositoryDelta::between(before, after)));
    assert_eq!(app.graph_generation, 1);
    assert_eq!(
        app.graph_generation_signal.load(Ordering::Acquire),
        app.graph_generation
    );
    wait_until(|| {
        app.drain_channels();
        app.graph
            .snapshot()
            .unwrap()
            .commits
            .iter()
            .any(|c| c.oid == expected && c.is_possible_squash_merge())
    });
    let marker = app
        .graph
        .snapshot()
        .unwrap()
        .commits
        .iter()
        .find(|c| c.oid == expected)
        .unwrap();
    assert!(
        marker.is_possible_squash_merge(),
        "new generation worker publishes real squash marker"
    );
    assert!(marker
        .possible_squash_merge_sources()
        .contains(&"feature-copy".into()));
    assert_eq!(
        app.graph_generation_signal.load(Ordering::Acquire),
        app.graph_generation
    );
}

#[test]
fn metadata_update_preserves_enrichment_completed_after_worker_snapshot() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let dir = tmpdir.path();
    run_git(dir, &["init", "-b", "main"]);
    run_git(dir, &["config", "user.name", "Test"]);
    run_git(dir, &["config", "user.email", "test@example.com"]);
    run_git(dir, &["commit", "--allow-empty", "-m", "root"]);
    let mut enriched = graph_snapshot(vec![]);
    enriched.commits[0].relationships = vec![
        graph::GraphRelationship {
            kind: graph::RelationshipKind::SquashMerge,
            matching: graph::RelationshipMatch::Exact,
            destination_oid: enriched.commits[0].oid.clone(),
            destination_refs: vec!["main".into()],
            source_oid: "1111111111111111111111111111111111111111".into(),
            source_refs: vec!["feature/source".into()],
        },
        graph::GraphRelationship {
            kind: graph::RelationshipKind::SquashMerge,
            matching: graph::RelationshipMatch::Fuzzy {
                similarity_percent: 87,
            },
            destination_oid: enriched.commits[0].oid.clone(),
            destination_refs: vec!["main".into()],
            source_oid: "3333333333333333333333333333333333333333".into(),
            source_refs: vec!["feature/near-source".into()],
        },
    ];
    let source_oid = enriched.commits[0].oid.clone();
    enriched.commits[0]
        .relationships
        .push(graph::GraphRelationship {
            kind: graph::RelationshipKind::CherryPick,
            matching: graph::RelationshipMatch::Exact,
            source_oid,
            source_refs: vec!["feature/picked".into()],
            destination_oid: "4444444444444444444444444444444444444444".into(),
            destination_refs: vec!["main".into()],
        });
    enriched.commits[0]
        .relationships
        .sort_by(|a, b| a.key().cmp(&b.key()));
    let expected_relationships = enriched.commits[0].relationships.clone();
    let mut stale_worker_snapshot = enriched.clone();
    stale_worker_snapshot.commits[0].relationships.clear();
    let worker_only = graph::GraphRelationship {
        kind: graph::RelationshipKind::SquashMerge,
        matching: graph::RelationshipMatch::Fuzzy {
            similarity_percent: 75,
        },
        destination_oid: stale_worker_snapshot.commits[0].oid.clone(),
        destination_refs: vec!["main".into()],
        source_oid: "5555555555555555555555555555555555555555".into(),
        source_refs: vec!["feature/worker-only".into()],
    };
    stale_worker_snapshot.commits[0]
        .relationships
        .push(worker_only.clone());
    let mut expected_relationships = expected_relationships;
    expected_relationships.push(worker_only);
    expected_relationships.sort_by(|a, b| a.key().cmp(&b.key()));
    let state = graph::GraphRepositoryState::capture(dir).unwrap();
    let delta = graph::GraphRepositoryDelta::between(state.clone(), state);
    let mut app = App::new(dir.to_path_buf(), "main".into(), Config::default());
    app.graph.apply_result(Ok(enriched));
    let options = graph::GraphLoadOptions {
        max_count: app.graph.max_count(),
        include_remotes: app.graph.includes_remotes(),
        line_style: graph::GraphLineStyle::from_symbol_name(app.symbols.name),
        base_branch: Some(app.base_branch.clone()),
        cache_root: app.cache_root.clone(),
    };
    let (tx, rx) = mpsc::channel();
    tx.send(graph::GraphUpdateMsg {
        revision: app.graph_revision,
        options,
        delta,
        result: Ok(stale_worker_snapshot),
    }).unwrap();
    app.graph_update_rx = Some(rx);
    app.drain_channels();

    let commit = &app.graph.snapshot().unwrap().commits[0];
    assert!(commit.is_possible_squash_merge());
    assert_eq!(commit.possible_squash_merge_sources(), ["feature/source"]);
    assert_eq!(commit.relationships, expected_relationships);
    assert_eq!(commit.fuzzy_squash_match(), None);
    assert!(commit.is_cherry_picked_commit());
}
