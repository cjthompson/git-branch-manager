use super::*;

fn squash_item(app: &App, branch: &BranchInfo) -> MenuItem {
    app.build_branch_menu_for(branch)
        .into_iter()
        .find(|item| item.action == BranchAction::SquashMerge)
        .expect("squash row present")
}

fn init_repo(dir: &std::path::Path) {
    run_git(dir, &["init", "-b", "main"]);
    run_git(dir, &["config", "user.email", "test@example.com"]);
    run_git(dir, &["config", "user.name", "Test User"]);
    std::fs::write(dir.join("README.md"), "base\n").unwrap();
    run_git(dir, &["add", "README.md"]);
    run_git(dir, &["commit", "-m", "initial"]);
    run_git(dir, &["branch", "feature"]);
}

#[test]
fn squash_row_stays_enabled_when_base_worktree_dirty() {
    let mut app = App::new("/repo".into(), "main".into(), Config::default());
    let mut base_wt = worktree("main");
    base_wt.is_base = true;
    base_wt.wt_status.has_modified = true;
    app.worktrees.set_items(vec![base_wt]);
    app.worktrees.loading = false;

    let item = squash_item(&app, &branch("feature", TrackingStatus::Local));

    assert!(item.enabled);
    assert_eq!(item.reason, None);
}

#[test]
fn squash_with_dirty_dispatch_worktree_opens_confirm_with_stash_risk() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let dir = tmpdir.path();
    init_repo(dir);
    std::fs::write(dir.join("README.md"), "modified\n").unwrap();
    std::fs::write(dir.join("scratch.txt"), "untracked\n").unwrap();

    let mut app = App::new(dir.to_path_buf(), "main".into(), Config::default());
    app.worktrees.loading = false;
    let item = squash_item(&app, &branch("feature", TrackingStatus::Local));

    app.execute_menu_action(item);

    let Some(Overlay::Confirm {
        preflight,
        choices,
        stage,
        ..
    }) = app.overlay.as_ref()
    else {
        panic!("expected confirm, got {:?}", app.overlay);
    };
    assert!(matches!(
        preflight.risks.as_slice(),
        [DeleteRisk::StashRequired {
            tracked: 1,
            untracked: 1,
            omitted: 0,
            ..
        }]
    ));
    assert_eq!(*stage, ConfirmStage::Initial);
    let [choice] = choices.as_slice() else {
        panic!("expected one choice: {choices:?}");
    };
    assert_eq!(choice.action, BranchAction::SquashMerge);
    assert_eq!(choice.targets, vec!["feature".to_string()]);
    assert!(choice.command.contains("--include-untracked"), "{}", choice.command);
}

#[test]
fn squash_with_clean_dispatch_worktree_skips_stash_prompt() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let dir = tmpdir.path();
    init_repo(dir);

    let mut app = App::new(dir.to_path_buf(), "main".into(), Config::default());
    app.worktrees.loading = false;
    let item = squash_item(&app, &branch("feature", TrackingStatus::Local));

    app.execute_menu_action(item);

    let Some(Overlay::Confirm { preflight, choices, .. }) = app.overlay.as_ref() else {
        panic!("expected confirm, got {:?}", app.overlay);
    };
    assert!(preflight.risks.is_empty(), "{:?}", preflight.risks);
    assert_eq!(choices.len(), 1);
    assert_eq!(choices[0].action, BranchAction::SquashMerge);
}

#[test]
fn squash_dispatch_path_unchanged_when_branch_has_worktree() {
    let mut app = App::new("/repo".into(), "main".into(), Config::default());
    let feature_wt = worktree("feature");
    let expected = feature_wt.path.clone();
    app.worktrees.set_items(vec![worktree("main"), feature_wt]);

    assert_eq!(
        app.dispatch_path_for(BranchAction::SquashMerge, &["feature".to_string()]),
        Some(expected)
    );
}
