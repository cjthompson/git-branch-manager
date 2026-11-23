use super::*;

#[test]
fn test_app_settings_saves_stay_in_isolated_config_root() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    assert_ne!(app.config_root, Config::default_root());

    app.handle_key(KeyEvent::new(
        KeyCode::Char('T'),
        crossterm::event::KeyModifiers::NONE,
    ));

    let saved = Config::load_from(&app.config_root);
    assert_eq!(saved.theme.as_deref(), Some(app.theme.name));
}

#[test]
fn graph_options_overlay_applies_remote_refs() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    app.active_view = ViewId::Graph;

    app.handle_key(KeyEvent::new(
        KeyCode::Char('o'),
        crossterm::event::KeyModifiers::NONE,
    ));
    assert!(matches!(
        app.overlay,
        Some(Overlay::GraphOptions {
            cursor: 0,
            include_remotes: false,
            show_squash_arrows: false,
        })
    ));

    app.handle_overlay_key(KeyEvent::new(
        KeyCode::Char(' '),
        crossterm::event::KeyModifiers::NONE,
    ));
    app.apply_graph_options(0, true);
    assert!(app.graph.includes_remotes());
    assert!(app.graph.is_loading());
    assert!(app.graph_rx.is_some());
}

fn key_press(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, crossterm::event::KeyModifiers::NONE)
}

#[test]
fn graph_options_overlay_toggles_squash_arrows_without_reload() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    app.active_view = ViewId::Graph;

    app.handle_key(key_press(KeyCode::Char('o')));
    app.handle_key(key_press(KeyCode::Char('j')));
    app.handle_key(key_press(KeyCode::Char('j')));
    assert!(matches!(
        app.overlay,
        Some(Overlay::GraphOptions {
            cursor: 2,
            include_remotes: false,
            show_squash_arrows: false,
        })
    ));

    app.handle_key(key_press(KeyCode::Char(' ')));
    assert!(matches!(
        app.overlay,
        Some(Overlay::GraphOptions {
            cursor: 2,
            include_remotes: false,
            show_squash_arrows: true,
        })
    ));

    app.handle_key(key_press(KeyCode::Enter));
    assert!(app.overlay.is_none());
    assert!(app.graph.shows_squash_arrows());
    assert!(!app.graph.includes_remotes());
    assert!(app.graph_rx.is_none(), "toggling arrows must not reload the graph");
    assert_eq!(
        Config::load_from(&app.config_root).show_squash_arrows,
        Some(true)
    );
}

#[test]
fn graph_options_arrows_row_reloads_only_when_remote_refs_changed() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    app.active_view = ViewId::Graph;

    app.handle_key(key_press(KeyCode::Char('o')));
    app.handle_key(key_press(KeyCode::Char(' ')));
    app.handle_key(key_press(KeyCode::Char('j')));
    app.handle_key(key_press(KeyCode::Char('j')));
    app.handle_key(key_press(KeyCode::Enter));

    assert!(app.graph.includes_remotes());
    assert!(app.graph.is_loading());
    assert!(app.graph_rx.is_some());
}

#[test]
fn graph_options_restores_squash_arrows_preference_from_config() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let config: Config = toml::from_str("show_squash_arrows = true\n").unwrap();

    let app = App::new(tmpdir.path().to_path_buf(), "main".into(), config);

    assert!(app.graph.shows_squash_arrows());
}

#[test]
fn app_starts_on_graph_tab() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );

    assert_eq!(app.active_view, ViewId::Graph);
}

#[test]
fn graph_load_older_control_reloads_with_next_page() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    app.active_view = ViewId::Graph;
    assert_eq!(app.graph.max_count(), 500);

    app.handle_graph_key(KeyEvent::new(
        KeyCode::Char('L'),
        crossterm::event::KeyModifiers::NONE,
    ));
    assert_eq!(app.graph.max_count(), 1000);
    assert!(app.graph.is_loading());
    assert!(app.graph_rx.is_some());
}

#[test]
fn app_restores_graph_remote_ref_preference_from_config() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let config: Config = toml::from_str("include_remotes = true\n").unwrap();

    let app = App::new(tmpdir.path().to_path_buf(), "main".into(), config);

    assert!(app.graph.includes_remotes());
}

#[test]
fn graph_result_clears_loading_toast() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    app.graph.begin_load(500, false);
    app.toast = Some(Toast::new("Loading graph...".into(), 300));

    let (tx, rx) = mpsc::channel();
    tx.send(Ok(graph::GraphSnapshot {
        source: graph::GraphSource::Gleisbau,
        commits: vec![],
        lines: vec![],
        ref_counts: Default::default(),
        max_count: 500,
        includes_remotes: false,
        generation: None,
    }))
    .expect("send graph result");
    app.graph_rx = Some(rx);

    app.drain_channels();

    assert!(!app.graph.is_loading());
    assert!(app.toast.is_none());
}
