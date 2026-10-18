use super::*;

#[test]
fn force_choice_opens_final_confirmation_before_enqueuing() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    let mut item = branch("feature/unmerged", TrackingStatus::Local);
    item.merge_status = MergeStatus::Unmerged;
    app.branches.set_items(vec![item]);

    app.delete_selected_branches(false);
    app.handle_overlay_key(KeyEvent::new(
        KeyCode::Char('r'),
        crossterm::event::KeyModifiers::NONE,
    ));
    assert_eq!(app.job_queue.current_action(), None);

    app.handle_overlay_key(KeyEvent::new(
        KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
    ));

    let Some(Overlay::Confirm {
        stage: ConfirmStage::FinalDestructive { choice, target },
        ..
    }) = app.overlay.as_ref()
    else {
        panic!("expected confirm overlay, got {:?}", app.overlay);
    };
    assert_eq!(choice.action, BranchAction::DeleteLocalForce);
    assert_eq!(choice.targets, vec!["feature/unmerged".to_string()]);
    assert_eq!(target, &ConfirmTarget::Branch("feature/unmerged".into()));
    assert_eq!(app.job_queue.current_action(), None);
}

#[test]
fn multi_selection_force_confirmation_enqueues_only_one_branch() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    let mut first = branch("feature/first", TrackingStatus::Local);
    first.merge_status = MergeStatus::Unmerged;
    let mut second = branch("feature/second", TrackingStatus::Local);
    second.merge_status = MergeStatus::Unmerged;
    app.branches.set_items(vec![first, second]);
    app.branches.selected_mut().fill(true);

    app.delete_selected_branches(false);
    for _ in 0..2 {
        app.handle_overlay_key(KeyEvent::new(
            KeyCode::Char('j'),
            crossterm::event::KeyModifiers::NONE,
        ));
    }
    app.handle_overlay_key(KeyEvent::new(
        KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
    ));

    let Some(Overlay::Confirm {
        stage: ConfirmStage::FinalDestructive { choice, target },
        ..
    }) = app.overlay.as_ref()
    else {
        panic!("expected final confirmation, got {:?}", app.overlay);
    };
    assert_eq!(choice.targets, vec!["feature/second".to_string()]);
    assert_eq!(target, &ConfirmTarget::Branch("feature/second".into()));
    assert_eq!(app.job_queue.current_action(), None);

    app.handle_overlay_key(KeyEvent::new(
        KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
    ));

    assert_eq!(
        app.job_queue.current_action(),
        Some(BranchAction::DeleteLocalForce)
    );
    assert_eq!(
        app.job_queue.current_targets_for_test(),
        Some(&["feature/second".to_string()][..])
    );
}

#[test]
fn duplicate_recovery_accelerator_keeps_navigated_target_selected() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    app.branches.set_items(vec![
        branch("feature/first", TrackingStatus::Local),
        branch("feature/second", TrackingStatus::Local),
    ]);
    app.branches.selected_mut().fill(true);

    app.delete_selected_branches(false);
    for _ in 0..2 {
        app.handle_overlay_key(KeyEvent::new(
            KeyCode::Char('j'),
            crossterm::event::KeyModifiers::NONE,
        ));
    }
    let Some(Overlay::Confirm {
        choices, selected, ..
    }) = app.overlay.as_ref()
    else {
        panic!("expected confirm overlay, got {:?}", app.overlay);
    };
    assert_eq!(*selected, 2);
    assert_eq!(choices[*selected].accelerator, 'r');
    assert_eq!(
        choices[*selected].destructive_target,
        Some(ConfirmTarget::Branch("feature/second".to_string()))
    );

    app.handle_overlay_key(KeyEvent::new(
        KeyCode::Char('r'),
        crossterm::event::KeyModifiers::NONE,
    ));

    assert!(matches!(
        app.overlay,
        Some(Overlay::Confirm { selected: 2, .. })
    ));

    app.handle_overlay_key(KeyEvent::new(
        KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
    ));

    assert!(matches!(
        app.overlay,
        Some(Overlay::Confirm {
            stage: ConfirmStage::FinalDestructive {
                target: ConfirmTarget::Branch(ref branch),
                ..
            },
            ..
        }) if branch == "feature/second"
    ));
    assert_eq!(app.job_queue.current_action(), None);
}

#[test]
fn force_delete_menu_opens_final_confirmation_without_enqueuing() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    app.branches
        .set_items(vec![branch("feature/menu-force", TrackingStatus::Local)]);
    let item = app
        .build_branch_menu()
        .into_iter()
        .find(|item| item.action == BranchAction::DeleteLocalForce)
        .expect("force-delete menu item");

    app.execute_menu_action(item);

    let Some(Overlay::Confirm {
        stage: ConfirmStage::FinalDestructive { choice, target },
        ..
    }) = app.overlay.as_ref()
    else {
        panic!("expected final confirmation, got {:?}", app.overlay);
    };
    assert_eq!(choice.action, BranchAction::DeleteLocalForce);
    assert_eq!(choice.targets, vec!["feature/menu-force".to_string()]);
    assert_eq!(target, &ConfirmTarget::Branch("feature/menu-force".into()));
    assert_eq!(app.job_queue.current_action(), None);

    app.handle_overlay_key(KeyEvent::new(
        KeyCode::Char('r'),
        crossterm::event::KeyModifiers::NONE,
    ));
    assert_eq!(app.job_queue.current_action(), None);
    assert!(matches!(
        app.overlay,
        Some(Overlay::Confirm {
            stage: ConfirmStage::FinalDestructive { .. },
            ..
        })
    ));
}

#[test]
fn force_worktree_remove_opens_final_confirmation_without_enqueuing() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    let mut wt = worktree("feature/worktree-force");
    wt.path = PathBuf::from("/repo/.worktrees/force");
    app.worktrees.set_items(vec![wt]);

    app.remove_selected_worktrees(true);

    let Some(Overlay::Confirm {
        stage: ConfirmStage::FinalDestructive { choice, target },
        ..
    }) = app.overlay.as_ref()
    else {
        panic!("expected final confirmation, got {:?}", app.overlay);
    };
    assert_eq!(choice.action, BranchAction::WorktreeForceRemove);
    assert_eq!(choice.targets, vec!["/repo/.worktrees/force".to_string()]);
    assert_eq!(
        target,
        &ConfirmTarget::Worktree(PathBuf::from("/repo/.worktrees/force"))
    );
    assert_eq!(app.job_queue.current_action(), None);
}

#[test]
fn linked_worktree_recovery_uses_a_distinct_choice_and_final_stage() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    let mut item = branch("feature/worktree", TrackingStatus::Local);
    item.merge_status = MergeStatus::Merged;
    app.branches.set_items(vec![item]);
    app.worktrees.set_items(vec![worktree("feature/worktree")]);

    app.delete_selected_branches(false);

    let Some(Overlay::Confirm { choices, .. }) = app.overlay.as_ref() else {
        panic!("expected confirm overlay, got {:?}", app.overlay);
    };
    let worktree_choice = choices
        .iter()
        .find(|choice| choice.accelerator == 'w')
        .expect("worktree recovery choice");
    assert_eq!(
        worktree_choice.action,
        BranchAction::DeleteBranchAndRemoveWorktree
    );
    assert_eq!(
        worktree_choice.targets,
        vec!["feature/worktree".to_string()]
    );
    assert_eq!(
        worktree_choice.destructive_target,
        Some(ConfirmTarget::BranchWorktree {
            branch: "feature/worktree".to_string(),
            worktree: PathBuf::from("/repo/.worktrees/feature-worktree"),
        })
    );
}

#[test]
fn linked_worktree_final_confirmation_repeats_exact_path() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    let mut item = branch("feature/worktree", TrackingStatus::Local);
    item.merge_status = MergeStatus::Unmerged;
    app.branches.set_items(vec![item]);
    let mut wt = worktree("feature/worktree");
    wt.path = PathBuf::from("/repo/.worktrees/exact-target");
    app.worktrees.set_items(vec![wt]);

    app.delete_selected_branches(false);
    app.handle_overlay_key(KeyEvent::new(
        KeyCode::Char('w'),
        crossterm::event::KeyModifiers::NONE,
    ));
    app.handle_overlay_key(KeyEvent::new(
        KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
    ));

    let rendered = render_app(&mut app, 100, 24);
    assert!(
        rendered.contains("Worktree: /repo/.worktrees/exact-target"),
        "final confirmation omitted the exact worktree path: {rendered}"
    );
    assert!(rendered.contains("Branch: feature/worktree"), "{rendered}");
    assert!(
        rendered.contains("Action: Force-remove worktree + delete branch"),
        "final confirmation omitted the exact forced recovery action: {rendered}"
    );
    assert!(
        rendered.contains("Command: git worktree remove --force"),
        "final confirmation did not repeat the exact destructive command: {rendered}"
    );
}

#[test]
fn primary_worktree_never_offers_a_recovery_choice() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    app.branches
        .set_items(vec![branch("feature/primary", TrackingStatus::Local)]);
    let mut wt = worktree("feature/primary");
    wt.is_main = true;
    wt.path = PathBuf::from("/repo");
    app.worktrees.set_items(vec![wt]);

    app.delete_selected_branches(false);

    let Some(Overlay::Confirm { choices, .. }) = app.overlay.as_ref() else {
        panic!("expected confirm overlay, got {:?}", app.overlay);
    };
    assert_eq!(
        choices
            .iter()
            .map(|choice| choice.action)
            .collect::<Vec<_>>(),
        vec![BranchAction::DeleteLocal],
        "the primary worktree may be checked safely, but must never get a force/remove route"
    );
}
