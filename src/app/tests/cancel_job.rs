use super::*;

#[test]
fn x_key_on_risky_worktree_removal_prompts_for_confirmation() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );

    let (_op_tx, op_rx) = mpsc::channel();
    let (prog_tx, prog_rx) = mpsc::channel();
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let partial_delete_risk = Arc::new(AtomicBool::new(true));
    app.job_queue.inject_running_for_test(
        git_branch_manager::job_queue::ActionJob {
            action: BranchAction::WorktreeRemove,
            targets: vec!["/repo/.worktrees/feature".to_string()],
            remote: None,
            return_view: ViewId::Worktrees,
            dispatch_path: None,
        },
        op_rx,
        prog_rx,
        Arc::clone(&cancel_flag),
        Arc::clone(&partial_delete_risk),
    );
    prog_tx
        .send(ProgressUpdate {
            completed: 0,
            total: 1,
            current_item: "/repo/.worktrees/feature — 3/10: src/main.rs".into(),
        })
        .unwrap();
    app.drain_channels();

    app.handle_key(KeyEvent::new(
        KeyCode::Char('x'),
        crossterm::event::KeyModifiers::NONE,
    ));

    match &app.overlay {
        Some(Overlay::ConfirmCancelJob { message }) => {
            assert!(message.contains("/repo/.worktrees/feature"));
        }
        other => panic!("expected ConfirmCancelJob overlay, got {other:?}"),
    }
    assert!(
        app.job_queue.is_running_for_test(),
        "job must keep running while the confirmation is pending"
    );
    assert!(!cancel_flag.load(Ordering::Relaxed));

    // 'y' actually cancels.
    app.handle_key(KeyEvent::new(
        KeyCode::Char('y'),
        crossterm::event::KeyModifiers::NONE,
    ));
    assert!(app.overlay.is_none());
    assert!(cancel_flag.load(Ordering::Relaxed));
    assert!(!app.job_queue.is_running_for_test());
    assert!(app.job_queue.is_draining_for_test());
}

#[test]
fn x_key_confirm_cancel_job_dismiss_keeps_job_running() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );

    let (_op_tx, op_rx) = mpsc::channel();
    let (_prog_tx, prog_rx) = mpsc::channel();
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let partial_delete_risk = Arc::new(AtomicBool::new(true));
    app.job_queue.inject_running_for_test(
        git_branch_manager::job_queue::ActionJob {
            action: BranchAction::WorktreeForceRemove,
            targets: vec!["/repo/.worktrees/feature".to_string()],
            remote: None,
            return_view: ViewId::Worktrees,
            dispatch_path: None,
        },
        op_rx,
        prog_rx,
        Arc::clone(&cancel_flag),
        Arc::clone(&partial_delete_risk),
    );

    app.handle_key(KeyEvent::new(
        KeyCode::Char('x'),
        crossterm::event::KeyModifiers::NONE,
    ));
    assert!(matches!(
        app.overlay,
        Some(Overlay::ConfirmCancelJob { .. })
    ));

    // 'n' dismisses without cancelling.
    app.handle_key(KeyEvent::new(
        KeyCode::Char('n'),
        crossterm::event::KeyModifiers::NONE,
    ));
    assert!(app.overlay.is_none());
    assert!(!cancel_flag.load(Ordering::Relaxed));
    assert!(app.job_queue.is_running_for_test());

    // Esc behaves the same as 'n'.
    app.handle_key(KeyEvent::new(
        KeyCode::Char('x'),
        crossterm::event::KeyModifiers::NONE,
    ));
    assert!(matches!(
        app.overlay,
        Some(Overlay::ConfirmCancelJob { .. })
    ));
    app.handle_key(KeyEvent::new(
        KeyCode::Esc,
        crossterm::event::KeyModifiers::NONE,
    ));
    assert!(app.overlay.is_none());
    assert!(!cancel_flag.load(Ordering::Relaxed));
    assert!(app.job_queue.is_running_for_test());
}

#[test]
fn x_key_cancels_immediately_when_no_partial_delete_risk() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );

    let (_op_tx, op_rx) = mpsc::channel();
    let (_prog_tx, prog_rx) = mpsc::channel();
    let cancel_flag = Arc::new(AtomicBool::new(false));
    app.job_queue.inject_running_for_test(
        git_branch_manager::job_queue::ActionJob {
            action: BranchAction::WorktreeRemove,
            targets: vec!["/repo/.worktrees/feature".to_string()],
            remote: None,
            return_view: ViewId::Worktrees,
            dispatch_path: None,
        },
        op_rx,
        prog_rx,
        Arc::clone(&cancel_flag),
        Arc::new(AtomicBool::new(false)), // no partial-delete risk
    );

    app.handle_key(KeyEvent::new(
        KeyCode::Char('x'),
        crossterm::event::KeyModifiers::NONE,
    ));

    assert!(
        app.overlay.is_none(),
        "no confirmation needed when there's no partial-delete risk"
    );
    assert!(cancel_flag.load(Ordering::Relaxed));
    assert!(!app.job_queue.is_running_for_test());
    assert!(app.job_queue.is_draining_for_test());
}

#[test]
fn x_key_cancels_immediately_for_non_worktree_action_even_with_risk_flag_set() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );

    let (_op_tx, op_rx) = mpsc::channel();
    let (_prog_tx, prog_rx) = mpsc::channel();
    let cancel_flag = Arc::new(AtomicBool::new(false));
    // partial_delete_risk is only ever set true by worktree-removal code
    // paths in practice, but this pins the branch to `current_action()`,
    // not the flag alone.
    let partial_delete_risk = Arc::new(AtomicBool::new(true));
    app.job_queue.inject_running_for_test(
        git_branch_manager::job_queue::ActionJob {
            action: BranchAction::DeleteLocal,
            targets: vec!["feature/x".to_string()],
            remote: None,
            return_view: ViewId::Branches,
            dispatch_path: None,
        },
        op_rx,
        prog_rx,
        Arc::clone(&cancel_flag),
        partial_delete_risk,
    );

    app.handle_key(KeyEvent::new(
        KeyCode::Char('x'),
        crossterm::event::KeyModifiers::NONE,
    ));

    assert!(app.overlay.is_none());
    assert!(cancel_flag.load(Ordering::Relaxed));
    assert!(app.job_queue.is_draining_for_test());
}
