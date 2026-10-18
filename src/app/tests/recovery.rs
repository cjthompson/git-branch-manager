use super::*;

#[test]
fn modal_keyboard_transition_preserves_single_target_recovery_across_resize() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let dir = tmpdir.path();
    run_git(dir, &["init"]);
    run_git(dir, &["config", "user.email", "test@example.com"]);
    run_git(dir, &["config", "user.name", "Test User"]);
    std::fs::write(dir.join("README.md"), "base\n").expect("write initial file");
    run_git(dir, &["add", "README.md"]);
    run_git(dir, &["commit", "-m", "initial"]);
    run_git(dir, &["branch", "-M", "main"]);
    run_git(dir, &["branch", "feature/first"]);
    let mut app = App::new(dir.to_path_buf(), "main".into(), Config::default());
    app.branches.set_items(vec![
        branch("feature/first", TrackingStatus::Local),
        branch("feature/second", TrackingStatus::Local),
    ]);
    app.branches.selected_mut().fill(true);

    app.delete_selected_branches(false);
    let Some(Overlay::Confirm {
        preflight, choices, ..
    }) = app.overlay.as_ref()
    else {
        panic!("expected delete preflight, got {:?}", app.overlay);
    };
    assert_eq!(preflight.risks.len(), 2);
    assert_eq!(choices.len(), 3, "safe plus one recovery per branch");

    press_overlay_key(&mut app, KeyCode::Esc);
    assert!(app.overlay.is_none(), "Esc must cancel preflight");

    app.delete_selected_branches(false);
    press_overlay_key(&mut app, KeyCode::Down);
    press_overlay_key(&mut app, KeyCode::Up);
    press_overlay_key(&mut app, KeyCode::Char('j'));
    assert!(matches!(
        app.overlay,
        Some(Overlay::Confirm { selected: 1, .. })
    ));
    press_overlay_key(&mut app, KeyCode::Enter);
    assert!(matches!(
        app.overlay,
        Some(Overlay::Confirm {
            stage: ConfirmStage::FinalDestructive {
                target: ConfirmTarget::Branch(ref branch),
                ..
            },
            ..
        }) if branch == "feature/first"
    ));
    assert_eq!(app.job_queue.current_action(), None);

    press_overlay_key(&mut app, KeyCode::Esc);
    assert!(
        app.overlay.is_none(),
        "Esc must cancel the final review too"
    );

    app.delete_selected_branches(false);
    press_overlay_key(&mut app, KeyCode::Char('j'));
    press_overlay_key(&mut app, KeyCode::Enter);
    press_overlay_key(&mut app, KeyCode::Enter);
    assert_eq!(
        app.job_queue.current_action(),
        Some(BranchAction::DeleteLocalForce)
    );
    assert_eq!(
        app.job_queue.current_targets_for_test(),
        Some(&["feature/first".to_string()][..]),
        "the initial final confirmation must enqueue exactly one selected branch"
    );

    open_results_fixture(&mut app);
    press_overlay_key(&mut app, KeyCode::Down);
    press_overlay_key(&mut app, KeyCode::Char('k'));
    press_overlay_key(&mut app, KeyCode::Char('j'));
    press_overlay_key(&mut app, KeyCode::Enter);
    assert!(matches!(
        app.overlay,
        Some(Overlay::Results {
            selected_index: 1,
            expanded_index: Some(1),
            focus: ResultsFocus::Results,
            ..
        })
    ));

    let roomy = render_app(&mut app, 120, 30);
    let compact = render_app(&mut app, 48, 8);
    assert!(roomy.contains("feature/unmerged"), "{roomy}");
    assert!(compact.contains("feature/unmerged"), "{compact}");
    assert!(matches!(
        app.overlay,
        Some(Overlay::Results {
            selected_index: 1,
            expanded_index: Some(1),
            focus: ResultsFocus::Results,
            ..
        })
    ));

    press_overlay_key(&mut app, KeyCode::Tab);
    press_overlay_key(&mut app, KeyCode::Down);
    press_overlay_key(&mut app, KeyCode::Up);
    press_overlay_key(&mut app, KeyCode::Enter);

    let Some(Overlay::Confirm {
        stage: ConfirmStage::FinalDestructive { choice, target },
        ..
    }) = app.overlay.as_ref()
    else {
        panic!("expected selected result recovery, got {:?}", app.overlay);
    };
    assert_eq!(choice.action, BranchAction::DeleteLocalForce);
    assert_eq!(choice.targets, ["feature/unmerged"]);
    assert_eq!(target, &ConfirmTarget::Branch("feature/unmerged".into()));
    press_overlay_key(&mut app, KeyCode::Enter);
    assert!(
        app.overlay.is_none(),
        "Enter must commit the final recovery"
    );
    assert_eq!(
        app.job_queue.current_targets_for_test(),
        Some(&["feature/first".to_string()][..]),
        "the earlier force-delete stays current while recovery queues"
    );
    assert_eq!(
        app.job_queue.queued_len_for_test(),
        1,
        "the final recovery must enqueue exactly the expanded feature/unmerged branch"
    );
    wait_until(|| {
        app.job_queue.poll();
        app.job_queue.current_targets_for_test() == Some(&["feature/unmerged".to_string()])
    });
    assert_eq!(
        app.job_queue.current_targets_for_test(),
        Some(&["feature/unmerged".to_string()][..]),
        "after the first queued force-delete completes, the actual pending recovery targets only the expanded branch"
    );
    assert_eq!(app.job_queue.queued_len_for_test(), 0);
}

#[test]
fn results_expands_only_one_selected_branch_at_a_time() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    open_results_fixture(&mut app);

    press_overlay_key(&mut app, KeyCode::Char('j'));
    press_overlay_key(&mut app, KeyCode::Enter);
    let Some(Overlay::Results { expanded_index, .. }) = app.overlay.as_ref() else {
        panic!("expected Results overlay, got {:?}", app.overlay);
    };
    assert_eq!(*expanded_index, Some(1));

    press_overlay_key(&mut app, KeyCode::Char('j'));
    press_overlay_key(&mut app, KeyCode::Enter);
    let Some(Overlay::Results { expanded_index, .. }) = app.overlay.as_ref() else {
        panic!("expected Results overlay, got {:?}", app.overlay);
    };
    assert_eq!(*expanded_index, Some(2));
}

#[test]
fn results_recovery_targets_only_the_expanded_result() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    open_results_fixture(&mut app);

    press_overlay_key(&mut app, KeyCode::Char('j'));
    press_overlay_key(&mut app, KeyCode::Enter);
    press_overlay_key(&mut app, KeyCode::Tab);
    press_overlay_key(&mut app, KeyCode::Enter);

    let Some(Overlay::Confirm {
        stage: ConfirmStage::FinalDestructive { choice, target },
        ..
    }) = app.overlay.as_ref()
    else {
        panic!("expected final confirmation, got {:?}", app.overlay);
    };
    assert_eq!(choice.action, BranchAction::DeleteLocalForce);
    assert_eq!(choice.targets, vec!["feature/unmerged".to_string()]);
    assert_eq!(target, &ConfirmTarget::Branch("feature/unmerged".into()));
    assert_eq!(app.job_queue.current_action(), None);
    assert_eq!(app.job_queue.queued_len_for_test(), 0);
}

#[test]
fn worktree_recovery_opens_final_confirmation_before_job_start() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    open_results_fixture(&mut app);

    press_overlay_key(&mut app, KeyCode::Char('j'));
    press_overlay_key(&mut app, KeyCode::Char('j'));
    press_overlay_key(&mut app, KeyCode::Enter);
    press_overlay_key(&mut app, KeyCode::Tab);
    press_overlay_key(&mut app, KeyCode::Enter);

    let Some(Overlay::Confirm {
        stage: ConfirmStage::FinalDestructive { choice, target },
        ..
    }) = app.overlay.as_ref()
    else {
        panic!("expected final confirmation, got {:?}", app.overlay);
    };
    assert_eq!(choice.action, BranchAction::DeleteBranchAndRemoveWorktree);
    assert_eq!(choice.targets, vec!["feature/worktree".to_string()]);
    assert_eq!(
        target,
        &ConfirmTarget::BranchWorktree {
            branch: "feature/worktree".into(),
            worktree: PathBuf::from("/repo/.worktrees/feature-worktree"),
        }
    );
    assert_eq!(app.job_queue.current_action(), None);
    assert_eq!(app.job_queue.queued_len_for_test(), 0);
}

#[test]
fn safe_delete_choice_starts_only_the_selected_branch_action() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    let mut item = branch("feature/safe", TrackingStatus::Local);
    item.merge_status = MergeStatus::Merged;
    app.branches.set_items(vec![item]);

    app.delete_selected_branches(false);
    app.handle_overlay_key(KeyEvent::new(
        KeyCode::Char('y'),
        crossterm::event::KeyModifiers::NONE,
    ));

    assert!(
        matches!(app.overlay, Some(Overlay::Confirm { .. })),
        "the direct row key selects the safe choice; it must not dispatch it"
    );
    assert_eq!(app.job_queue.current_action(), None);

    app.handle_overlay_key(KeyEvent::new(
        KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
    ));

    assert!(app.overlay.is_none());
    assert_eq!(
        app.job_queue.current_action(),
        Some(BranchAction::DeleteLocal)
    );
    assert_eq!(
        app.job_queue.current_targets_for_test(),
        Some(&["feature/safe".to_string()][..])
    );
}
