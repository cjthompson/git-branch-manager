use super::*;

#[test]
fn phase1_enrichment_reapplies_saved_sort_and_preserves_row_identity() {
    for ascending in [true, false] {
        let config_root = tempfile::tempdir().unwrap();
        Config::update_at(config_root.path(), |config| {
            config.sort_column_branches = Some("merge".into());
            config.sort_asc_branches = Some(ascending);
            config.symbols = Some("ascii".into());
        });
        let saved_config = Config::load_from(config_root.path());
        let mut app = App::new(PathBuf::from("/missing/repo"), "main".into(), saved_config);
        app.config_root = config_root.path().to_path_buf();
        app.active_view = ViewId::Branches;

        let mut base = branch("main", TrackingStatus::Local);
        base.is_base = true;
        let mut current = branch("current", TrackingStatus::Local);
        current.is_current = true;
        let alpha = branch("alpha", TrackingStatus::Local);
        let mut beta = branch("beta", TrackingStatus::Local);
        beta.merge_status = MergeStatus::Merged;

        let (tx, rx) = std::sync::mpsc::channel();
        app.phase1_rx = Some(rx);
        tx.send(Phase1Msg::Fast(
            vec![beta, current, alpha, base],
            Box::new(cache::BranchCache::load_for_base(
                &app.repo_path,
                &app.base_branch,
                &app.cache_root,
            )),
            Box::new(cache::BranchCache::load_for_base(
                &app.repo_path,
                &app.base_branch,
                &app.cache_root,
            )),
        ))
        .unwrap();
        app.drain_channels();

        let ascending_order = ["main", "current", "beta", "alpha"];
        let descending_order = ["main", "current", "alpha", "beta"];
        let expected = if ascending {
            ascending_order
        } else {
            descending_order
        };
        let displayed_names = |app: &App| {
            app.branches
                .display_indices()
                .iter()
                .map(|&index| app.branches.items()[index].name.clone())
                .collect::<Vec<_>>()
        };

        assert!(!app.branches.loading);
        assert_eq!(app.branches.sort_column(), Some(5));
        assert_eq!(app.branches.sort_ascending(), ascending);
        assert_eq!(displayed_names(&app), expected);
        let indicator = if ascending { "Merge▲" } else { "Merge▼" };
        assert!(render_app(&mut app, 120, 20).contains(indicator));

        let alpha_index = app
            .branches
            .items()
            .iter()
            .position(|branch| branch.name == "alpha")
            .unwrap();
        app.branches.set_cursor(alpha_index);
        app.branches.selected_mut()[alpha_index] = true;
        assert_eq!(app.branches.cursor_item().unwrap().name, "alpha");

        tx.send(Phase1Msg::MergeStatuses(vec![
            ("alpha".into(), MergeStatus::Merged),
            ("beta".into(), MergeStatus::Unmerged),
        ]))
        .unwrap();
        app.drain_channels();

        let enriched_expected = if ascending {
            ["main", "current", "alpha", "beta"]
        } else {
            ["main", "current", "beta", "alpha"]
        };
        assert_eq!(displayed_names(&app), enriched_expected);
        assert_eq!(app.branches.sort_column(), Some(5));
        assert_eq!(app.branches.sort_ascending(), ascending);
        assert!(render_app(&mut app, 120, 20).contains(indicator));
        assert_eq!(app.branches.cursor_item().unwrap().name, "alpha");
        let selected_alpha_index = app
            .branches
            .items()
            .iter()
            .position(|branch| branch.name == "alpha")
            .unwrap();
        assert!(app.branches.selected()[selected_alpha_index]);
        assert_eq!(app.branches.items()[0].name, "main");
        assert_eq!(app.branches.items()[1].name, "current");
    }
}

#[test]
fn worktree_enrichment_matches_paths_across_sorting_between_batches() {
    let mut app = App::new(PathBuf::from("/repo"), "main".into(), Config::default());
    app.active_view = ViewId::Worktrees;
    let zulu = worktree("zulu");
    let alpha = worktree("alpha");
    let zulu_path = zulu.path.clone();
    let alpha_path = alpha.path.clone();
    app.worktrees.set_items(vec![zulu, alpha]);
    app.worktrees.set_sort(Some(0), true);
    let (tx, rx) = std::sync::mpsc::channel();
    app.worktree_enrich_rx = Some(rx);

    let mut dirty = WorkingTreeStatus::clean();
    dirty.has_modified = true;
    tx.send(WorktreeEnrichResult {
        path: zulu_path.clone(),
        wt_status: dirty,
        age_date: Utc::now(),
    })
    .unwrap();
    app.drain_channels();
    assert_eq!(
        app.worktrees
            .items()
            .iter()
            .map(|wt| wt.path.clone())
            .collect::<Vec<_>>(),
        vec![alpha_path.clone(), zulu_path.clone()]
    );

    tx.send(WorktreeEnrichResult {
        path: alpha_path.clone(),
        wt_status: WorkingTreeStatus::clean(),
        age_date: Utc::now(),
    })
    .unwrap();
    app.drain_channels();

    assert_eq!(app.worktrees.sort_column(), Some(0));
    assert!(app.worktrees.items()[1].wt_status.has_modified);
    assert!(!app.worktrees.items()[0].wt_status.has_modified);
}
