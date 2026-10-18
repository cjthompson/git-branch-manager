use super::*;

#[test]
fn queued_delete_observation_reaches_graph_publication_without_full_reload() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let dir = tmpdir.path();
    run_git(dir, &["init", "-b", "main"]);
    run_git(dir, &["config", "user.name", "Test"]);
    run_git(dir, &["config", "user.email", "test@example.com"]);
    run_git(dir, &["commit", "--allow-empty", "-m", "root"]);
    run_git(dir, &["branch", "feature/queued-delete"]);
    let before = graph::load_graph(dir, graph::GraphLoadOptions { base_branch: Some("main".into()), ..Default::default() }).unwrap();
    assert!(before.commits.iter().flat_map(|commit| &commit.refs).any(|reference| reference.name == "feature/queued-delete"));

    let mut app = App::new(dir.to_path_buf(), "main".into(), Config::default());
    app.graph.apply_result(Ok(before));
    app.active_view = ViewId::Worktrees;
    app.job_queue.enqueue_or_start(BranchAction::DeleteLocal, vec!["feature/queued-delete".into()], ViewId::Graph);
    wait_until(|| {
        app.drain_channels();
        app.job_queue.current_action().is_none() && app.graph_update_rx.is_none()
    });

    let updated = app.graph.snapshot().expect("Graph snapshot remains published");
    assert!(!updated.commits.iter().flat_map(|commit| &commit.refs).any(|reference| reference.name == "feature/queued-delete"));
    assert!(app.graph_rx.is_none(), "queued Git mutation must use the incremental updater");
    assert_eq!(app.active_view, ViewId::Worktrees);
}

#[test]
fn graph_state_patch_leaves_revision_ownership_to_app() {
    // P110: a ref-only action (DeleteLocal) with a loaded Graph snapshot
    // must use the surgical patch path — apply_ref_delta mutates the
    // snapshot in place while App owns generation revisions.
    let mut app = graph_app(vec![graph_ref(
        "feature/surgical",
        graph::GraphRefKind::LocalBranch,
    )]);
    let before_gen = app
        .graph
        .snapshot()
        .expect("graph_app loads a snapshot")
        .generation;

    // Ref identities are applied from observed repository state by the
    // same GraphState patch path used for queued completions.
    let _ = app.graph.apply_ref_delta(vec!["feature/surgical".into()]);
    let after_gen = app
        .graph
        .snapshot()
        .expect("snapshot still present")
        .generation;
    assert_eq!(after_gen, before_gen, "GraphState must not bump App revisions");
}

#[test]
fn observed_commit_change_updates_loaded_graph_without_full_reload_or_navigation() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let dir = tmpdir.path();
    run_git(dir, &["init", "-b", "main"]);
    run_git(dir, &["config", "user.name", "Test"]);
    run_git(dir, &["config", "user.email", "test@example.com"]);
    run_git(dir, &["commit", "--allow-empty", "-m", "root"]);
    let before = graph::GraphRepositoryState::capture(dir).unwrap();
    let old_snapshot = graph::load_graph(dir, graph::GraphLoadOptions { base_branch: Some("main".into()), ..Default::default() }).unwrap();
    run_git(dir, &["checkout", "-b", "feature/incremental"]);
    run_git(dir, &["commit", "--allow-empty", "-m", "feature"]);
    let feature_oid = String::from_utf8(Command::new("git").args(["rev-parse", "HEAD"]).current_dir(dir).output().unwrap().stdout).unwrap();
    let after = graph::GraphRepositoryState::capture(dir).unwrap();

    let mut app = App::new(dir.to_path_buf(), "main".into(), Config::default());
    app.graph.apply_result(Ok(old_snapshot));
    app.active_view = ViewId::Worktrees;
    app.request_graph_update(Ok(graph::GraphRepositoryDelta::between(before, after)));
    wait_until(|| {
        app.drain_channels();
        app.graph_update_rx.is_none()
    });

    let updated = app.graph.snapshot().expect("incremental snapshot remains loaded");
    assert!(updated.commits.iter().any(|commit| commit.oid == feature_oid.trim()));
    assert!(updated.commits.iter().any(|commit| commit.summary == "root"));
    assert!(app.graph_rx.is_none(), "normal loaded-snapshot update must not start a full Graph load");
    assert_eq!(app.active_view, ViewId::Worktrees);
}

#[test]
fn automatic_fetch_completion_uses_graph_update_scheduler_without_full_reload() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let dir = tmpdir.path();
    run_git(dir, &["init", "-b", "main"]);
    run_git(dir, &["config", "user.name", "Test"]);
    run_git(dir, &["config", "user.email", "test@example.com"]);
    run_git(dir, &["commit", "--allow-empty", "-m", "root"]);
    let snapshot = graph::load_graph(dir, graph::GraphLoadOptions { base_branch: Some("main".into()), ..Default::default() }).unwrap();
    let mut app = App::new(dir.to_path_buf(), "main".into(), Config::default());
    app.graph.apply_result(Ok(snapshot));
    app.active_view = ViewId::Tags;
    app.start_auto_fetch();
    wait_until(|| {
        app.drain_channels();
        app.remote_fetch_rx.is_none() && app.graph_update_rx.is_none()
    });
    assert!(app.graph.snapshot().is_some());
    assert!(app.graph_rx.is_none(), "automatic fetch must not force a full Graph load");
    assert_eq!(app.active_view, ViewId::Tags);
}

#[test]
fn startup_fetch_completion_is_retained_while_another_auto_fetch_is_requested() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let dir = tmpdir.path();
    run_git(dir, &["init", "-b", "main"]);
    run_git(dir, &["config", "user.name", "Test"]);
    run_git(dir, &["config", "user.email", "test@example.com"]);
    run_git(dir, &["commit", "--allow-empty", "-m", "root"]);
    let before = graph::GraphRepositoryState::capture(dir).unwrap();
    let options = graph::GraphLoadOptions { include_remotes: true, base_branch: Some("main".into()), ..Default::default() };
    let snapshot = graph::load_graph(dir, options).unwrap();
    let repo = git2::Repository::open(dir).unwrap();
    let parent = repo.head().unwrap().peel_to_commit().unwrap();
    let sig = git2::Signature::now("Test", "test@example.com").unwrap();
    let remote_tip = repo.commit(None, &sig, &sig, "fetched topic", &parent.tree().unwrap(), &[&parent]).unwrap();
    repo.reference("refs/remotes/origin/topic", remote_tip, true, "test fetch").unwrap();
    let after = graph::GraphRepositoryState::capture(dir).unwrap();
    let (tx, rx) = mpsc::channel();
    tx.send(RemoteFetchCompletion {
        success: true,
        graph_delta: Ok(graph::GraphRepositoryDelta::between(before, after)),
    }).unwrap();

    let mut app = App::new(dir.to_path_buf(), "main".into(), Config::default());
    app.graph.apply_result(Ok(snapshot));
    app.active_view = ViewId::Tags;
    app.remote_fetch_rx = Some(rx);
    app.start_auto_fetch();
    assert!(app.remote_fetch_rx.is_some(), "active completion receiver must not be replaced");
    wait_until(|| {
        app.drain_channels();
        app.remote_fetch_rx.is_none() && app.graph_update_rx.is_none()
    });

    let updated = app.graph.snapshot().unwrap();
    assert!(updated.commits.iter().any(|commit| commit.oid == remote_tip.to_string()));
    assert!(updated.commits.iter().flat_map(|commit| &commit.refs).any(|reference| reference.name == "origin/topic"));
    assert!(app.graph_rx.is_none());
    assert_eq!(app.active_view, ViewId::Tags);
}

#[test]
fn modal_fetch_completion_uses_graph_update_scheduler_without_double_reload() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let dir = tmpdir.path();
    run_git(dir, &["init", "-b", "main"]);
    run_git(dir, &["config", "user.name", "Test"]);
    run_git(dir, &["config", "user.email", "test@example.com"]);
    run_git(dir, &["commit", "--allow-empty", "-m", "root"]);
    let snapshot = graph::load_graph(dir, graph::GraphLoadOptions { base_branch: Some("main".into()), ..Default::default() }).unwrap();
    let mut app = App::new(dir.to_path_buf(), "main".into(), Config::default());
    app.graph.apply_result(Ok(snapshot));
    app.active_view = ViewId::Graph;
    app.start_fetch(false);
    wait_until(|| {
        app.drain_channels();
        app.op_rx.is_none() && app.graph_update_rx.is_none()
    });
    assert!(app.graph.snapshot().is_some());
    assert!(app.graph_rx.is_none(), "modal fetch must not trigger a second full Graph load");
    assert_eq!(app.active_view, ViewId::Graph);
}

#[test]
fn cancelled_modal_fetch_keeps_and_applies_partial_observation_without_reopening_overlay() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let dir = tmpdir.path();
    run_git(dir, &["init", "-b", "main"]);
    run_git(dir, &["config", "user.name", "Test"]);
    run_git(dir, &["config", "user.email", "test@example.com"]);
    run_git(dir, &["commit", "--allow-empty", "-m", "root"]);
    let before = graph::GraphRepositoryState::capture(dir).unwrap();
    let options = graph::GraphLoadOptions { include_remotes: true, base_branch: Some("main".into()), ..Default::default() };
    let snapshot = graph::load_graph(dir, options.clone()).unwrap();
    let repo = git2::Repository::open(dir).unwrap();
    let parent = repo.head().unwrap().peel_to_commit().unwrap();
    let tree = parent.tree().unwrap();
    let sig = git2::Signature::now("Test", "test@example.com").unwrap();
    let topic = repo.commit(None, &sig, &sig, "partial fetched commit", &tree, &[&parent]).unwrap();
    repo.reference("refs/remotes/origin/topic", topic, true, "test fetch").unwrap();
    let after = graph::GraphRepositoryState::capture(dir).unwrap();

    let mut app = App::new(dir.to_path_buf(), "main".into(), Config::default());
    app.graph.apply_result(Ok(snapshot));
    app.active_view = ViewId::Worktrees;
    app.return_view = ViewId::Worktrees;
    let (op_tx, op_rx) = mpsc::channel();
    let (delta_tx, delta_rx) = mpsc::channel();
    app.op_rx = Some(op_rx);
    app.fetch_graph_delta_rx = Some(delta_rx);
    app.cancel_flag = Some(Arc::new(AtomicBool::new(false)));
    app.overlay = Some(Overlay::Executing { label: "Fetching...".into(), progress: None });
    app.handle_overlay_key(KeyEvent::new(KeyCode::Esc, crossterm::event::KeyModifiers::NONE));
    assert!(app.op_rx.is_some(), "cancellation must retain the completion receiver");

    delta_tx.send(Ok(graph::GraphRepositoryDelta::between(before, after))).unwrap();
    op_tx.send(Vec::new()).unwrap();
    // The fetch thread exits after sending; `op_rx` clears only on disconnect.
    drop(op_tx);
    wait_until(|| {
        app.drain_channels();
        app.op_rx.is_none() && app.graph_update_rx.is_none()
    });
    assert!(app.graph.snapshot().unwrap().commits.iter().any(|commit| commit.oid == topic.to_string()));
    assert!(app.overlay.is_none(), "a dismissed modal must stay dismissed after completion");
    assert_eq!(app.active_view, ViewId::Worktrees);
}

#[test]
fn initial_graph_load_is_not_published_until_pending_mutation_is_reconciled() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let dir = tmpdir.path();
    run_git(dir, &["init", "-b", "main"]);
    run_git(dir, &["config", "user.name", "Test"]);
    run_git(dir, &["config", "user.email", "test@example.com"]);
    run_git(dir, &["commit", "--allow-empty", "-m", "root"]);
    let before_state = graph::GraphRepositoryState::capture(dir).unwrap();
    let old_snapshot = graph::load_graph(dir, graph::GraphLoadOptions { base_branch: Some("main".into()), ..Default::default() }).unwrap();
    run_git(dir, &["checkout", "-b", "feature/racing-load"]);
    run_git(dir, &["commit", "--allow-empty", "-m", "racing feature"]);
    let new_oid = String::from_utf8(Command::new("git").args(["rev-parse", "HEAD"]).current_dir(dir).output().unwrap().stdout).unwrap();
    let after_state = graph::GraphRepositoryState::capture(dir).unwrap();

    let mut app = App::new(dir.to_path_buf(), "main".into(), Config::default());
    app.graph.begin_load(500, false);
    let (tx, rx) = mpsc::channel();
    tx.send(Ok(old_snapshot)).unwrap();
    app.graph_rx = Some(rx);
    app.request_graph_update(Ok(graph::GraphRepositoryDelta::between(before_state, after_state)));
    app.drain_channels();
    assert!(app.graph.snapshot().is_none(), "stale initial snapshot stays staged until reconciliation");
    wait_until(|| {
        app.drain_channels();
        !app.graph.is_loading() && app.graph_update_rx.is_none()
    });
    assert!(app.graph.snapshot().is_some(), "snapshot missing: error={:?}, loading={}, update_rx={}, graph_rx={}", app.graph.error(), app.graph.is_loading(), app.graph_update_rx.is_some(), app.graph_rx.is_some());
    assert!(app.graph.snapshot().unwrap().commits.iter().any(|commit| commit.oid == new_oid.trim()));
}

#[test]
fn pending_mutation_reconciles_from_staged_snapshot_with_new_graph_options() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let dir = tmpdir.path();
    run_git(dir, &["init", "-b", "main"]);
    run_git(dir, &["config", "user.name", "Test"]);
    run_git(dir, &["config", "user.email", "test@example.com"]);
    run_git(dir, &["commit", "--allow-empty", "-m", "root"]);
    let before = graph::GraphRepositoryState::capture(dir).unwrap();
    let mut app = App::new(dir.to_path_buf(), "main".into(), Config::default());
    app.graph.apply_result(Ok(graph::load_graph(dir, graph::GraphLoadOptions { base_branch: Some("main".into()), ..Default::default() }).unwrap()));
    app.active_view = ViewId::Worktrees;
    app.spawn_graph_load(2, true);
    let repo = git2::Repository::open(dir).unwrap();
    let parent = repo.head().unwrap().peel_to_commit().unwrap();
    let sig = git2::Signature::now("Test", "test@example.com").unwrap();
    let tip = repo.commit(None, &sig, &sig, "during load", &parent.tree().unwrap(), &[&parent]).unwrap();
    repo.reference("refs/remotes/origin/during-load", tip, true, "test mutation").unwrap();
    let after = graph::GraphRepositoryState::capture(dir).unwrap();
    app.request_graph_update(Ok(graph::GraphRepositoryDelta::between(before, after)));

    wait_until(|| {
        app.drain_channels();
        !app.graph.is_loading() && app.graph_rx.is_none() && app.graph_update_rx.is_none()
    });
    let updated = app.graph.snapshot().expect("reconciled snapshot");
    assert_eq!(updated.max_count, 2);
    assert!(updated.includes_remotes);
    assert!(updated.commits.iter().any(|commit| commit.oid == tip.to_string()));
    assert!(updated.commits.iter().flat_map(|commit| &commit.refs).any(|reference| reference.name == "origin/during-load"));
    assert!(app.graph_rx.is_none(), "reconciliation must not strand a newer load");
    assert_eq!(app.active_view, ViewId::Worktrees);
}
