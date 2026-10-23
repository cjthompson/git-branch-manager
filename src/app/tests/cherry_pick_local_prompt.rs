use super::*;

fn cherry_pick_local_item(target: &str) -> MenuItem {
    MenuItem {
        label: "Cherry-pick onto current".into(),
        shortcut: Some('C'),
        action: BranchAction::CherryPickLocal,
        target: target.into(),
        remote: None,
        enabled: true,
        reason: None,
    }
}

fn app_with_branches(branches: Vec<BranchInfo>) -> App {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    app.branches.set_items(branches);
    app
}

#[test]
fn cherry_pick_local_confirm_names_current_head_when_not_base() {
    let mut base = branch("main", TrackingStatus::Local);
    base.is_base = true;
    let mut head = branch("topic-a", TrackingStatus::Local);
    head.is_current = true;
    let target = branch("topic-b", TrackingStatus::Local);
    let mut app = app_with_branches(vec![base, head, target]);

    app.execute_menu_action(cherry_pick_local_item("topic-b"));

    let Some(Overlay::Confirm {
        preflight,
        choices,
        stage,
        ..
    }) = app.overlay.as_ref()
    else {
        panic!("expected confirm overlay, got {:?}", app.overlay);
    };
    assert_eq!(
        preflight.risks,
        vec![DeleteRisk::CherryPickTarget {
            branch: "topic-b".into(),
            head: Some("topic-a".into()),
            base: "main".into(),
        }]
    );
    assert_eq!(choices[0].action, BranchAction::CherryPickLocal);
    assert_eq!(choices[0].command, "git cherry-pick refs/heads/topic-b");
    assert_eq!(stage, &ConfirmStage::Initial);
}

#[test]
fn cherry_pick_local_confirm_has_no_head_warning_when_head_is_base() {
    let mut base = branch("main", TrackingStatus::Local);
    base.is_base = true;
    base.is_current = true;
    let target = branch("topic-b", TrackingStatus::Local);
    let mut app = app_with_branches(vec![base, target]);

    app.execute_menu_action(cherry_pick_local_item("topic-b"));

    let Some(Overlay::Confirm { preflight, .. }) = app.overlay.as_ref() else {
        panic!("expected confirm overlay, got {:?}", app.overlay);
    };
    assert!(preflight.risks.is_empty());
}

#[test]
fn cherry_pick_local_confirm_reports_detached_head() {
    let mut base = branch("main", TrackingStatus::Local);
    base.is_base = true;
    let target = branch("topic-b", TrackingStatus::Local);
    let mut app = app_with_branches(vec![base, target]);

    app.execute_menu_action(cherry_pick_local_item("topic-b"));

    let Some(Overlay::Confirm { preflight, .. }) = app.overlay.as_ref() else {
        panic!("expected confirm overlay, got {:?}", app.overlay);
    };
    assert_eq!(
        preflight.risks,
        vec![DeleteRisk::CherryPickTarget {
            branch: "topic-b".into(),
            head: None,
            base: "main".into(),
        }]
    );
}
