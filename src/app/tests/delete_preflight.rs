use super::*;

#[test]
fn branches_view_lazily_loads_worktrees_for_delete_preflight() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    app.active_view = ViewId::Branches;
    app.worktrees.loading = false;

    app.ensure_view_loaded();

    assert!(app.worktrees.loading);
}

#[test]
fn branches_view_respects_worktree_load_opt_out() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let config = Config {
        load_worktrees_on_launch: Some(false),
        ..Config::default()
    };
    let mut app = App::new(tmpdir.path().to_path_buf(), "main".into(), config);
    app.active_view = ViewId::Branches;
    app.worktrees.loading = false;

    app.ensure_view_loaded();

    assert!(!app.worktrees.loading);
}

#[test]
fn dirty_linked_worktree_git_facts_flow_into_delete_preflight_and_confirm_rendering() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let dir = tmpdir.path();
    run_git(dir, &["init"]);
    run_git(dir, &["config", "user.email", "test@example.com"]);
    run_git(dir, &["config", "user.name", "Test User"]);
    std::fs::write(dir.join("README.md"), "base\n").expect("write initial file");
    run_git(dir, &["add", "README.md"]);
    run_git(dir, &["commit", "-m", "initial"]);
    run_git(dir, &["branch", "-M", "main"]);
    run_git(dir, &["branch", "feature/dirty-linked"]);

    let linked_dir = dir.join("linked-dirty");
    let linked_dir_arg = linked_dir.to_str().expect("UTF-8 temporary path");
    run_git(
        dir,
        &["worktree", "add", linked_dir_arg, "feature/dirty-linked"],
    );
    std::fs::write(linked_dir.join("README.md"), "dirty linked worktree\n")
        .expect("modify tracked file in linked worktree");
    for index in 0..6 {
        std::fs::write(
            linked_dir.join(format!("untracked-{index}.txt")),
            "untracked\n",
        )
        .expect("write untracked file in linked worktree");
    }

    let repo = git2::Repository::open(dir).expect("open temporary repository");
    let cache_dir = tempfile::tempdir().expect("isolated branch test cache");
    let branches = branch::list_branches_with_cache_root(
        &repo,
        "main",
        &cache::CacheRoot::at(cache_dir.path()),
    )
    .expect("load real branch facts");
    let mut worktrees = worktree::list_worktrees(dir);
    let enrich_rx = worktree::enrich_worktrees(worktrees.clone());
    for update in enrich_rx {
        let worktree = worktrees
            .iter_mut()
            .find(|worktree| worktree.path == update.path)
            .unwrap();
        worktree.wt_status = update.wt_status;
        worktree.age_date = update.age_date;
    }
    let linked_status = worktrees
        .iter()
        .find(|worktree| worktree.path.canonicalize().ok() == linked_dir.canonicalize().ok())
        .unwrap_or_else(|| panic!("discover linked worktree in {worktrees:?}"))
        .wt_status
        .clone();
    assert!(linked_status.has_modified);
    assert!(linked_status.has_untracked);
    assert_eq!(linked_status.changed_files.len(), 7);

    let mut app = App::new(dir.to_path_buf(), "main".into(), Config::default());
    app.branches.set_items(branches);
    let selected_index = app
        .branches
        .items()
        .iter()
        .position(|branch| branch.name == "feature/dirty-linked")
        .expect("discover linked branch");
    app.branches.selected_mut()[selected_index] = true;
    app.worktrees.set_items(worktrees);

    app.delete_selected_branches(false);
    let Some(Overlay::Confirm { preflight, .. }) = app.overlay.as_ref() else {
        panic!("expected delete preflight, got {:?}", app.overlay);
    };
    let Some(DeleteRisk::DirtyWorktree { files, omitted, .. }) = preflight
        .risks
        .iter()
        .find(|risk| matches!(risk, DeleteRisk::DirtyWorktree { .. }))
    else {
        panic!("expected real dirty linked-worktree risk: {preflight:?}");
    };
    assert_eq!(
        files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        [
            "README.md",
            "untracked-0.txt",
            "untracked-1.txt",
            "untracked-2.txt",
            "untracked-3.txt",
        ]
    );
    assert_eq!(*omitted, 2);

    let rendered = render_app(&mut app, 100, 30);
    for file in [
        "modified: README.md",
        "untracked: untracked-0.txt",
        "untracked: untracked-1.txt",
        "untracked: untracked-2.txt",
        "untracked: untracked-3.txt",
    ] {
        assert!(rendered.contains(file), "missing {file}: {rendered}");
    }
    assert!(rendered.contains("+2 more"), "{rendered}");
    assert!(!rendered.contains("untracked-4.txt"), "{rendered}");
    assert!(!rendered.contains("untracked-5.txt"), "{rendered}");
}

#[test]
fn likely_squash_merged_preflight_requires_force_review() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    let mut item = branch("feature/possible", TrackingStatus::Local);
    item.merge_status = MergeStatus::LikelySquashMerged;
    item.merge_base_commit = Some("abc12345".to_string());
    app.branches.set_items(vec![item]);

    app.delete_selected_branches(false);

    let Some(Overlay::Confirm {
        preflight, choices, ..
    }) = app.overlay.as_ref()
    else {
        panic!("expected confirm overlay, got {:?}", app.overlay);
    };
    assert!(matches!(
        preflight.risks.as_slice(),
        [DeleteRisk::UniqueCommits {
            branch,
            base,
            status: MergeStatus::LikelySquashMerged,
            merge_base_commit: Some(hash),
        }] if branch == "feature/possible" && base == "main" && hash == "abc12345"
    ));
    assert!(choices.iter().any(|choice| {
        choice.accelerator == 'r'
            && choice.action == BranchAction::DeleteLocalForce
            && choice.targets == ["feature/possible"]
            && choice.destructive_target
                == Some(ConfirmTarget::Branch("feature/possible".to_string()))
    }));

    let rendered = render_app(&mut app, 100, 24);
    assert!(
        rendered.contains("Status: Possible Squash Merge"),
        "{rendered}"
    );
    assert!(rendered.contains("Merge base: abc12345"), "{rendered}");
    assert!(
        !rendered.contains("Unique commits"),
        "a likely squash merge must not be presented as a confirmed unique-commit result: {rendered}"
    );
}

#[test]
fn dirty_worktree_warning_lists_at_most_five_modified_or_untracked_files() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    let mut item = branch("feature/dirty", TrackingStatus::Local);
    item.merge_status = MergeStatus::Merged;
    app.branches.set_items(vec![item]);

    let mut wt = worktree("feature/dirty");
    wt.path = PathBuf::from("/repo/.worktrees/dirty");
    wt.wt_status = WorkingTreeStatus {
        has_staged: true,
        has_modified: true,
        has_untracked: true,
        changed_files: vec![
            ChangedFile {
                path: "staged-only.rs".into(),
                kind: ChangedFileKind::Staged,
            },
            ChangedFile {
                path: "modified-1.rs".into(),
                kind: ChangedFileKind::Modified,
            },
            ChangedFile {
                path: "modified-2.rs".into(),
                kind: ChangedFileKind::Modified,
            },
            ChangedFile {
                path: "modified-3.rs".into(),
                kind: ChangedFileKind::Modified,
            },
            ChangedFile {
                path: "modified-4.rs".into(),
                kind: ChangedFileKind::Modified,
            },
            ChangedFile {
                path: "untracked-1.rs".into(),
                kind: ChangedFileKind::Untracked,
            },
            ChangedFile {
                path: "untracked-2.rs".into(),
                kind: ChangedFileKind::Untracked,
            },
            ChangedFile {
                path: "untracked-3.rs".into(),
                kind: ChangedFileKind::Untracked,
            },
        ],
    };
    app.worktrees.set_items(vec![wt]);

    app.delete_selected_branches(false);
    let rendered = render_app(&mut app, 100, 30);

    assert!(!rendered.contains("staged-only.rs"), "{rendered}");
    for expected in [
        "modified-1.rs",
        "modified-2.rs",
        "modified-3.rs",
        "modified-4.rs",
        "untracked-1.rs",
    ] {
        assert!(
            rendered.contains(expected),
            "missing {expected}: {rendered}"
        );
    }
    assert!(!rendered.contains("untracked-2.rs"), "{rendered}");
    assert!(!rendered.contains("untracked-3.rs"), "{rendered}");
    assert!(rendered.contains("+2 more"), "{rendered}");
}
