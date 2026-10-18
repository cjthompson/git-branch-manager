use super::*;

#[test]
fn primary_code_menu_action_runs_read_only_and_keeps_chosen_primary_evidence() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let dir = tmpdir.path();
    run_git(dir, &["init", "-b", "main"]);
    run_git(dir, &["config", "user.name", "Test"]);
    run_git(dir, &["config", "user.email", "test@example.com"]);
    run_git(dir, &["commit", "--allow-empty", "-m", "root"]);
    run_git(dir, &["checkout", "-b", "feature/check"]);
    std::fs::write(dir.join("feature.txt"), "feature\n").unwrap();
    run_git(dir, &["add", "feature.txt"]);
    run_git(dir, &["commit", "-m", "feature"]);
    let primary_oid = git2::Repository::open(dir)
        .unwrap()
        .find_branch("main", git2::BranchType::Local)
        .unwrap()
        .get()
        .target()
        .unwrap()
        .to_string();

    let mut app = App::new(dir.to_path_buf(), "main".into(), Config::default());
    app.active_view = ViewId::Tags;
    app.execute_menu_action(MenuItem {
        label: BranchAction::CheckPrimaryBranchCode.label().into(),
        enabled: true,
        reason: None,
        shortcut: None,
        action: BranchAction::CheckPrimaryBranchCode,
        target: "refs/heads/feature/check".into(),
        remote: None,
    });
    assert!(matches!(app.overlay, Some(Overlay::Executing { .. })));
    assert_eq!(app.job_queue.queued_len_for_test(), 0);
    assert_eq!(app.job_queue.current_action(), None);

    wait_until(|| {
        app.drain_channels();
        matches!(app.overlay, Some(Overlay::InfoModal { .. }))
    });
    assert!(matches!(
        app.overlay,
        Some(Overlay::InfoModal {
            row: InfoModalRow::PrimaryBranchCodeCheck {
                primary_reference: Some(reference),
                primary_oid: Some(oid),
                outcome: Ok(PrimaryBranchCodeMatch::NotFound),
                ..
            },
            ..
        }) if reference == "refs/heads/main" && oid == primary_oid
    ));
}

#[test]
fn late_primary_code_result_does_not_replace_another_executing_request() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    let (tx, rx) = mpsc::channel();
    let active_cancel = Arc::new(AtomicBool::new(false));
    let primary_cancel = Arc::new(AtomicBool::new(false));
    app.cancel_flag = Some(active_cancel);
    app.primary_code_check_request = Some(PrimaryCodeCheckRequest {
        receiver: rx,
        cancel: Arc::clone(&primary_cancel),
    });
    app.overlay = Some(Overlay::Executing {
        label: "Verifying cache accuracy...".into(),
        progress: None,
    });
    tx.send(PrimaryCodeCheckResult {
        display_ref: "feature/late".into(),
        base_branch: "main".into(),
        primary_reference: Some("refs/heads/main".into()),
        primary_oid: Some("a1b2c3d".into()),
        outcome: Ok(PrimaryBranchCodeMatch::Merged),
    })
    .unwrap();

    app.drain_channels();

    assert!(
        matches!(app.overlay, Some(Overlay::Executing { label, .. }) if label == "Verifying cache accuracy...")
    );
    assert!(primary_cancel.load(Ordering::Relaxed));
}

#[test]
fn late_canceled_fetch_completion_preserves_primary_check_ownership() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    let (op_tx, op_rx) = mpsc::channel();
    let (delta_tx, delta_rx) = mpsc::channel();
    app.op_rx = Some(op_rx);
    app.fetch_graph_delta_rx = Some(delta_rx);
    let fetch_cancel = Arc::new(AtomicBool::new(true));
    let primary_cancel = Arc::new(AtomicBool::new(false));
    app.cancel_flag = Some(Arc::clone(&fetch_cancel));
    app.fetch_overlay_dismissed = true;

    let (primary_tx, primary_rx) = mpsc::channel();
    app.primary_code_check_request = Some(PrimaryCodeCheckRequest {
        receiver: primary_rx,
        cancel: Arc::clone(&primary_cancel),
    });
    app.cancel_flag = Some(Arc::clone(&primary_cancel));
    app.overlay = Some(Overlay::Executing {
        label: "Checking code from feature/a in main...".into(),
        progress: None,
    });
    app.active_view = ViewId::Worktrees;
    app.return_view = ViewId::Worktrees;

    // Keep the scanner pending while the old fetch completion arrives.
    app.drain_channels();
    op_tx.send(Vec::new()).unwrap();
    delta_tx.send(Err("fetch observation unavailable".into())).unwrap();
    app.drain_channels();

    assert!(!primary_cancel.load(Ordering::Relaxed));
    assert!(Arc::ptr_eq(
        app.cancel_flag.as_ref().expect("scanner still owns cancel flag"),
        &primary_cancel
    ));
    assert!(app.primary_code_check_request.is_some());
    assert!(matches!(app.overlay, Some(Overlay::Executing { .. })));

    primary_tx
        .send(PrimaryCodeCheckResult {
            display_ref: "feature/a".into(),
            base_branch: "main".into(),
            primary_reference: Some("refs/heads/main".into()),
            primary_oid: Some("a1b2c3d".into()),
            outcome: Ok(PrimaryBranchCodeMatch::Merged),
        })
        .unwrap();
    app.drain_channels();
    assert!(matches!(
        app.overlay,
        Some(Overlay::InfoModal {
            row: InfoModalRow::PrimaryBranchCodeCheck {
                outcome: Ok(PrimaryBranchCodeMatch::Merged),
                ..
            },
            ..
        })
    ));
    assert!(app.cancel_flag.is_none());
}

#[test]
fn escape_cancels_and_drops_the_primary_code_check_request() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    let (_tx, rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    app.cancel_flag = Some(Arc::clone(&cancel));
    app.primary_code_check_request = Some(PrimaryCodeCheckRequest {
        receiver: rx,
        cancel: Arc::clone(&cancel),
    });
    app.overlay = Some(Overlay::Executing {
        label: "Checking code...".into(),
        progress: None,
    });

    app.handle_key(KeyEvent::new(
        KeyCode::Esc,
        crossterm::event::KeyModifiers::NONE,
    ));

    assert!(cancel.load(Ordering::Relaxed));
    assert!(app.primary_code_check_request.is_none());
    assert!(app.overlay.is_none());
}

#[test]
fn stale_empty_primary_code_request_is_canceled_without_touching_new_owner() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    let (_tx, rx) = mpsc::channel();
    let primary_cancel = Arc::new(AtomicBool::new(false));
    let newer_cancel = Arc::new(AtomicBool::new(false));
    app.cancel_flag = Some(Arc::clone(&newer_cancel));
    app.primary_code_check_request = Some(PrimaryCodeCheckRequest {
        receiver: rx,
        cancel: Arc::clone(&primary_cancel),
    });
    app.overlay = Some(Overlay::Executing {
        label: "Verifying cache accuracy...".into(),
        progress: None,
    });

    app.drain_channels();

    assert!(primary_cancel.load(Ordering::Relaxed));
    assert!(!newer_cancel.load(Ordering::Relaxed));
    assert!(Arc::ptr_eq(
        app.cancel_flag.as_ref().unwrap(),
        &newer_cancel
    ));
    assert!(app.primary_code_check_request.is_none());
    assert!(
        matches!(app.overlay, Some(Overlay::Executing { label, .. }) if label == "Verifying cache accuracy...")
    );
}

#[test]
fn primary_code_success_resets_info_modal_state_and_shows_result() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    let (tx, rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    app.cancel_flag = Some(Arc::clone(&cancel));
    app.primary_code_check_request = Some(PrimaryCodeCheckRequest {
        receiver: rx,
        cancel: Arc::clone(&cancel),
    });
    app.overlay = Some(Overlay::Executing {
        label: "Checking code...".into(),
        progress: None,
    });
    app.info_copied_msg = Some("old copy".into());
    app.info_modal_scroll_offset = 12;
    tx.send(PrimaryCodeCheckResult {
        display_ref: "feature/a".into(),
        base_branch: "main".into(),
        primary_reference: Some("refs/remotes/origin/main".into()),
        primary_oid: Some("1234567890abcdef".into()),
        outcome: Ok(PrimaryBranchCodeMatch::Merged),
    })
    .unwrap();

    app.drain_channels();

    assert!(matches!(
        app.overlay,
        Some(Overlay::InfoModal {
            row: InfoModalRow::PrimaryBranchCodeCheck {
                outcome: Ok(PrimaryBranchCodeMatch::Merged),
                ..
            },
            ..
        })
    ));
    assert!(app.primary_code_check_request.is_none());
    assert!(app.cancel_flag.is_none());
    assert!(app.info_copied_msg.is_none());
    assert_eq!(app.info_modal_scroll_offset, 0);
}

#[test]
fn primary_code_error_is_shown_in_the_result_modal() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    let (tx, rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    app.cancel_flag = Some(Arc::clone(&cancel));
    app.primary_code_check_request = Some(PrimaryCodeCheckRequest {
        receiver: rx,
        cancel: Arc::clone(&cancel),
    });
    app.overlay = Some(Overlay::Executing {
        label: "Checking code...".into(),
        progress: None,
    });
    tx.send(PrimaryCodeCheckResult {
        display_ref: "feature/a".into(),
        base_branch: "main".into(),
        primary_reference: None,
        primary_oid: None,
        outcome: Err("missing ref".into()),
    })
    .unwrap();

    app.drain_channels();

    assert!(
        matches!(app.overlay, Some(Overlay::InfoModal { row: InfoModalRow::PrimaryBranchCodeCheck { outcome: Err(error), .. }, .. }) if error == "missing ref")
    );
    assert!(app.primary_code_check_request.is_none());
    assert!(app.cancel_flag.is_none());
}

#[test]
fn disconnected_primary_code_request_clears_progress_and_shows_toast() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    let (tx, rx) = mpsc::channel();
    drop(tx);
    let cancel = Arc::new(AtomicBool::new(false));
    app.cancel_flag = Some(Arc::clone(&cancel));
    app.primary_code_check_request = Some(PrimaryCodeCheckRequest {
        receiver: rx,
        cancel: Arc::clone(&cancel),
    });
    app.overlay = Some(Overlay::Executing {
        label: "Checking code...".into(),
        progress: None,
    });

    app.drain_channels();

    assert!(app.overlay.is_none());
    assert!(app
        .toast
        .as_ref()
        .is_some_and(|toast| toast.message == "Primary branch check stopped unexpectedly"));
    assert!(app.primary_code_check_request.is_none());
    assert!(app.cancel_flag.is_none());
}
