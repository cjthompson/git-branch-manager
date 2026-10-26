use super::*;

#[test]
fn cache_audit_closes_overlay_when_repo_cannot_be_opened() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut app = App::new(dir.path().to_path_buf(), "main".into(), Config::default());

    app.run_cache_audit();
    assert!(matches!(app.overlay, Some(Overlay::Executing { .. })));

    wait_until(|| {
        app.drain_channels();
        app.diag_rx.is_none()
    });

    assert!(app.overlay.is_none());
    assert_eq!(
        app.toast.as_ref().map(|t| t.message.as_str()),
        Some("Cache audit failed: repository could not be opened")
    );
}
