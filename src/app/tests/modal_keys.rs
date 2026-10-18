use super::*;

#[test]
fn settings_and_filter_action_lists_select_and_choose_with_keyboard() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );

    app.overlay = Some(Overlay::Settings { cursor: 0 });
    press_overlay_key(&mut app, KeyCode::Char('j'));
    press_overlay_key(&mut app, KeyCode::Up);
    press_overlay_key(&mut app, KeyCode::Down);
    assert!(matches!(app.overlay, Some(Overlay::Settings { cursor: 1 })));
    let original_theme = app.theme.name;
    press_overlay_key(&mut app, KeyCode::Enter);
    assert!(matches!(app.overlay, Some(Overlay::Settings { cursor: 1 })));
    assert_ne!(
        app.theme.name, original_theme,
        "Enter must choose the setting"
    );

    app.branches.set_horizontal_scroll_bounds(5);
    app.branches.scroll_right();
    for _ in 0..7 {
        press_overlay_key(&mut app, KeyCode::Down);
    }
    assert!(matches!(app.overlay, Some(Overlay::Settings { cursor: 8 })));
    press_overlay_key(&mut app, KeyCode::Enter);
    assert_eq!(app.config.horizontal_scrolling, Some(true));
    assert_eq!(app.branches.horizontal_offset(), 0);
    press_overlay_key(&mut app, KeyCode::Left);
    assert_eq!(app.config.horizontal_scrolling, Some(false));

    press_overlay_key(&mut app, KeyCode::Esc);
    assert!(
        app.overlay.is_none(),
        "Esc must close Settings before opening Filter"
    );
    app.active_view = ViewId::Branches;
    app.handle_key(KeyEvent::new(
        KeyCode::Char('\\'),
        crossterm::event::KeyModifiers::NONE,
    ));
    assert!(matches!(
        app.overlay,
        Some(Overlay::FilterSelection { cursor: 0 })
    ));
    press_overlay_key(&mut app, KeyCode::Down);
    press_overlay_key(&mut app, KeyCode::Up);
    press_overlay_key(&mut app, KeyCode::Char('j'));
    press_overlay_key(&mut app, KeyCode::Char('k'));
    press_overlay_key(&mut app, KeyCode::Char('j'));
    assert!(matches!(
        app.overlay,
        Some(Overlay::FilterSelection { cursor: 1 })
    ));
    press_overlay_key(&mut app, KeyCode::Enter);
    assert!(
        app.active_filter_query().contains("merge:squash"),
        "Enter must choose the selected filter"
    );
    press_overlay_key(&mut app, KeyCode::Char('m'));
    assert!(
        app.active_filter_query().contains("merge:merged"),
        "direct token keys must still toggle their filter"
    );
    press_overlay_key(&mut app, KeyCode::Esc);
    assert!(app.overlay.is_none(), "Esc must close the action list");
}

#[test]
fn page_keys_scroll_overflowing_confirm_and_results_bodies() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    let choices = (0..12)
        .map(|index| ConfirmChoice {
            accelerator: 'y',
            action: BranchAction::DeleteLocal,
            targets: vec![format!("feature/choice-{index}")],
            remote: None,
            command: format!("git branch -d -- feature/choice-{index}"),
            description: "Let Git reject unsafe deletion.".to_string(),
            destructive_target: None,
        })
        .collect();
    app.overlay = Some(Overlay::Confirm {
        preflight: DeletePreflight::default(),
        choices,
        selected: 0,
        body_scroll: ModalScroll::default(),
        stage: ConfirmStage::Initial,
    });

    render_app(&mut app, 48, 8);
    press_overlay_key(&mut app, KeyCode::PageDown);
    let rendered = render_app(&mut app, 48, 8);
    assert!(
        !rendered.contains("feature/choice-0"),
        "PageDown must visibly advance Confirm after redraw: {rendered}"
    );
    assert!(rendered.contains("feature/choice-1"), "{rendered}");
    press_overlay_key(&mut app, KeyCode::Char('j'));
    let rendered = render_app(&mut app, 48, 8);
    assert!(
        rendered.contains("feature/choice-1"),
        "moving focus after manual paging must keep the selected choice visible: {rendered}"
    );
    press_overlay_key(&mut app, KeyCode::PageUp);
    let rendered = render_app(&mut app, 48, 8);
    assert!(rendered.contains("feature/choice-0"), "{rendered}");

    let results = (0..12)
        .map(|index| {
            OperationResult::failure(
                format!("feature/result-{index}"),
                BranchAction::DeleteLocal,
                FailureCause::Other {
                    raw_message: format!("git failed for result {index}"),
                },
                "operation failed",
            )
        })
        .collect();
    app.overlay = Some(Overlay::Results {
        results,
        selected_index: 0,
        expanded_index: None,
        focus: ResultsFocus::Results,
        body_scroll: ModalScroll::default(),
    });

    let rendered = render_app(&mut app, 48, 8);
    assert!(rendered.contains("feature/result-0"), "{rendered}");
    press_overlay_key(&mut app, KeyCode::PageDown);
    let rendered = render_app(&mut app, 48, 8);
    assert!(
        !rendered.contains("feature/result-0"),
        "PageDown must visibly advance Results after redraw: {rendered}"
    );
    assert!(rendered.contains("feature/result-5"), "{rendered}");
    press_overlay_key(&mut app, KeyCode::Down);
    let rendered = render_app(&mut app, 48, 8);
    assert!(
        rendered.contains("feature/result-1"),
        "moving Results focus after manual paging must restore the selected row: {rendered}"
    );
    press_overlay_key(&mut app, KeyCode::PageUp);
    let rendered = render_app(&mut app, 48, 8);
    assert!(rendered.contains("feature/result-0"), "{rendered}");
}

#[test]
fn final_destructive_confirmation_pages_on_a_compact_terminal() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    let choice = ConfirmChoice {
        accelerator: 'r',
        action: BranchAction::DeleteLocalForce,
        targets: vec!["feature/unmerged".to_string()],
        remote: None,
        command: "git branch -D -- feature/unmerged".to_string(),
        description: "Permanently discard unique commits.".to_string(),
        destructive_target: Some(ConfirmTarget::Branch("feature/unmerged".to_string())),
    };
    app.overlay = Some(Overlay::Confirm {
        preflight: DeletePreflight::default(),
        choices: vec![choice.clone()],
        selected: 0,
        body_scroll: ModalScroll::default(),
        stage: ConfirmStage::FinalDestructive {
            choice,
            target: ConfirmTarget::Branch("feature/unmerged".to_string()),
        },
    });

    let initial = render_app(&mut app, 48, 7);
    assert!(
        initial.contains("FINAL DESTRUCTIVE CONFIRMATION"),
        "{initial}"
    );
    press_overlay_key(&mut app, KeyCode::PageDown);
    let paged = render_app(&mut app, 48, 7);
    assert!(
        !paged.contains("FINAL DESTRUCTIVE CONFIRMATION"),
        "PageDown must visibly advance the final confirmation: {paged}"
    );
    assert!(paged.contains("DATA LOSS"), "{paged}");
    press_overlay_key(&mut app, KeyCode::PageUp);
    let reset = render_app(&mut app, 48, 7);
    assert!(reset.contains("FINAL DESTRUCTIVE CONFIRMATION"), "{reset}");
}

#[test]
fn confirmation_body_scroll_keeps_selected_choice_visible() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    let branches = (0..8)
        .map(|index| branch(&format!("feature/risk-{index}"), TrackingStatus::Local))
        .collect();
    app.branches.set_items(branches);
    app.branches.selected_mut().fill(true);

    app.delete_selected_branches(false);
    app.handle_overlay_key(KeyEvent::new(
        KeyCode::Down,
        crossterm::event::KeyModifiers::NONE,
    ));
    let rendered = render_app(&mut app, 80, 8);

    let Some(Overlay::Confirm {
        selected,
        body_scroll,
        ..
    }) = app.overlay.as_ref()
    else {
        panic!("expected confirm overlay, got {:?}", app.overlay);
    };
    assert_eq!(*selected, 1, "the recovery choice should be selected");
    assert!(body_scroll.offset > 0, "risks should force body scrolling");
    assert!(
        rendered.contains("git branch -D -- feature/risk-0"),
        "selected choice is clipped: {rendered}"
    );
}
