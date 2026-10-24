use super::*;

#[test]
fn graph_commit_details_uses_local_branch_metadata_for_actions() {
    let mut app = graph_app(vec![graph_ref(
        "feature/local",
        graph::GraphRefKind::LocalBranch,
    )]);
    let mut local = branch(
        "feature/local",
        TrackingStatus::Tracked {
            remote_ref: "origin/feature/local".into(),
            gone: false,
        },
    );
    local.ahead = Some(0);
    app.branches.set_items(vec![local]);
    app.has_configured_remote = true;
    app.pr_map.insert(
        "feature/local".into(),
        PrInfo {
            number: 42,
            status: PrStatus::Open,
        },
    );

    app.handle_key(KeyEvent::new(
        KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
    ));
    complete_graph_commit_details(&mut app);

    let items = commit_details_items(&app);
    let push = items
        .iter()
        .find(|item| item.action == BranchAction::Push)
        .expect("local Push action");
    assert!(!push.enabled);
    assert_eq!(push.reason.as_deref(), Some("not ahead"));
    assert!(items
        .iter()
        .any(|item| { item.action == BranchAction::ViewRemotePR && item.enabled }));
}

#[test]
fn graph_commit_details_uses_remote_branch_metadata_for_actions() {
    let mut app = graph_app(vec![graph_ref(
        "upstream/feature/remote",
        graph::GraphRefKind::RemoteBranch,
    )]);
    app.remotes
        .set_items(vec![remote("upstream/feature/remote", "feature/remote")]);

    app.handle_key(KeyEvent::new(
        KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
    ));
    complete_graph_commit_details(&mut app);

    let items = commit_details_items(&app);
    assert!(items
        .iter()
        .any(|item| { item.action == BranchAction::CheckoutRemote && item.enabled }));
    let delete = items
        .iter()
        .find(|item| item.action == BranchAction::DeleteRemoteBranch)
        .expect("remote Delete action");
    assert!(delete.enabled);
    assert_eq!(delete.target, "feature/remote");
    assert_eq!(delete.remote.as_deref(), Some("upstream"));
}

#[test]
fn commit_details_open_selects_first_enabled_action_after_disabled_checkout() {
    let mut app = graph_app(vec![
        graph_ref("feature/local", graph::GraphRefKind::LocalBranch),
        graph_ref("origin/feature/remote", graph::GraphRefKind::RemoteBranch),
    ]);
    let mut local = branch("feature/local", TrackingStatus::Local);
    local.is_current = true;
    app.branches.set_items(vec![local]);
    app.remotes
        .set_items(vec![remote("origin/feature/remote", "feature/remote")]);

    app.handle_key(KeyEvent::new(
        KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
    ));
    complete_graph_commit_details(&mut app);

    let items = commit_details_items(&app);
    let first_enabled = items
        .iter()
        .position(|item| item.enabled)
        .expect("an enabled read-only or remote action should be available");
    let checkout = items
        .iter()
        .find(|item| item.action == BranchAction::Checkout)
        .expect("local Checkout action");
    assert!(!checkout.enabled, "local Checkout must be disabled");
    assert!(matches!(
        app.overlay,
        Some(Overlay::CommitDetails {
            cursor,
            focus: CommitDetailsFocus::Actions,
            ..
        }) if cursor == first_enabled
    ));
}

#[test]
fn graph_commit_actions_use_the_selected_oid_and_disable_merges() {
    let oid = "1111111111111111111111111111111111111111";
    let mut app = graph_app(vec![]);
    let commit = app.graph.selected_commit().unwrap().clone();
    let items = app.build_graph_menu_for(&commit);
    let check = items
        .iter()
        .find(|item| item.action == BranchAction::CheckPrimaryBranchCode)
        .unwrap();
    assert_eq!(check.target, oid);
    let pick = items
        .iter()
        .find(|item| item.action == BranchAction::CherryPickCommit)
        .unwrap();
    assert!(pick.enabled);
    assert_eq!(pick.target, oid);
    app.execute_menu_action(pick.clone());
    assert!(matches!(
        app.overlay,
        Some(Overlay::Confirm { ref choices, .. })
            if matches!(choices.as_slice(), [ConfirmChoice { action: BranchAction::CherryPickCommit, targets, .. }] if targets == &[oid])
    ));

    let mut merge = commit;
    merge.parents = vec!["p1".into(), "p2".into()];
    let merge_items = app.build_graph_menu_for(&merge);
    let check = merge_items
        .iter()
        .find(|item| item.action == BranchAction::CheckPrimaryBranchCode)
        .unwrap();
    assert!(check.enabled);
    assert_eq!(check.target, oid);
    let pick = merge_items
        .iter()
        .find(|item| item.action == BranchAction::CherryPickCommit)
        .unwrap();
    assert!(!pick.enabled);
    assert_eq!(pick.reason.as_deref(), Some("requires mainline selection"));
}

#[test]
fn graph_commit_details_keeps_tag_metadata_and_offers_ref_free_commit_actions() {
    let mut tag_app = graph_app(vec![graph_ref("v1.2.3", graph::GraphRefKind::Tag)]);
    tag_app.tags.set_items(vec![tag("v1.2.3")]);

    tag_app.handle_key(KeyEvent::new(
        KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
    ));
    complete_graph_commit_details(&mut tag_app);
    assert!(commit_details_items(&tag_app)
        .iter()
        .any(|item| item.action == BranchAction::PushTag));

    let mut ref_free_app = graph_app(vec![]);
    ref_free_app.handle_key(KeyEvent::new(
        KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
    ));
    complete_graph_commit_details(&mut ref_free_app);
    let items = commit_details_items(&ref_free_app);
    for action in [
        BranchAction::CheckPrimaryBranchCode,
        BranchAction::CherryPickCommit,
    ] {
        let item = items.iter().find(|item| item.action == action).unwrap();
        assert!(item.enabled);
        assert_eq!(item.target, "1111111111111111111111111111111111111111");
    }
    assert!(matches!(
        ref_free_app.overlay,
        Some(Overlay::CommitDetails {
            focus: CommitDetailsFocus::Actions,
            ..
        })
    ));
}

#[test]
fn graph_commit_details_picks_up_authoritative_metadata_loaded_in_background() {
    let mut app = graph_app(vec![graph_ref("v1.2.3", graph::GraphRefKind::Tag)]);
    app.handle_key(KeyEvent::new(
        KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
    ));
    complete_graph_commit_details(&mut app);
    assert!(commit_details_items(&app)
        .iter()
        .any(|item| item.action == BranchAction::CherryPickCommit));
    assert!(!commit_details_items(&app)
        .iter()
        .any(|item| item.action == BranchAction::PushTag));

    let (tx, rx) = mpsc::channel();
    tx.send(vec![tag("v1.2.3")]).unwrap();
    app.tag_load_rx = Some(rx);
    app.drain_channels();

    app.refresh_open_graph_menu();
    assert!(commit_details_items(&app)
        .iter()
        .any(|item| item.action == BranchAction::PushTag));
}

#[test]
fn graph_commit_details_refreshes_selected_commit_when_graph_reloads() {
    let mut app = graph_app(vec![graph_ref(
        "feature/local",
        graph::GraphRefKind::LocalBranch,
    )]);
    app.handle_key(KeyEvent::new(
        KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
    ));
    complete_graph_commit_details(&mut app);

    let mut snapshot = graph_snapshot(vec![graph_ref(
        "feature/local",
        graph::GraphRefKind::LocalBranch,
    )]);
    snapshot.commits[0].summary = "reloaded commit".into();
    app.graph.apply_result(Ok(snapshot));
    app.refresh_open_graph_menu();

    assert!(matches!(
        app.overlay,
        Some(Overlay::CommitDetails {
            ref commit,
            ref details,
            ..
        }) if commit.summary == "reloaded commit" && details.summary == "selected commit"
    ));
}
