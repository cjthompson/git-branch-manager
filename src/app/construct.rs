use super::*;

impl App {
    #[cfg(test)]
    pub fn new(repo_path: PathBuf, base_branch: String, config: Config) -> Self {
        let test_cache_root = tempfile::tempdir().expect("create isolated app test cache");
        let mut app = Self::with_cache_root(
            repo_path,
            base_branch,
            config,
            cache::CacheRoot::at(test_cache_root.path()),
        );
        app.config_root = test_cache_root.path().join("config");
        app._test_cache_root = Some(test_cache_root);
        app
    }

    pub fn with_cache_root(
        repo_path: PathBuf,
        base_branch: String,
        config: Config,
        cache_root: cache::CacheRoot,
    ) -> Self {
        let theme = Theme::from_name(config.theme.as_deref().unwrap_or("dark"));
        let symbols = SymbolSet::from_name(config.symbols.as_deref().unwrap_or("auto"));

        let branch_def = BranchesViewDef;
        let remote_def = RemotesViewDef;
        let tag_def = TagsViewDef;
        let worktree_def = WorktreesViewDef;

        let branch_cols = branch_def.columns();
        let remote_cols = remote_def.columns();
        let tag_cols = tag_def.columns();
        let worktree_cols = worktree_def.columns();

        let branch_sort_col = config
            .sort_column_branches
            .as_deref()
            .and_then(|k| sort_keys::index_for_key(&branch_cols, k));
        let branch_sort_asc = config.sort_asc_branches.unwrap_or(true);
        let remote_sort_col = config
            .sort_column_remotes
            .as_deref()
            .and_then(|k| sort_keys::index_for_key(&remote_cols, k));
        let remote_sort_asc = config.sort_asc_remotes.unwrap_or(true);
        let tag_sort_col = config
            .sort_column_tags
            .as_deref()
            .and_then(|k| sort_keys::index_for_key(&tag_cols, k));
        let tag_sort_asc = config.sort_asc_tags.unwrap_or(true);
        let worktree_sort_col = config
            .sort_column_worktrees
            .as_deref()
            .and_then(|k| sort_keys::index_for_key(&worktree_cols, k));
        let worktree_sort_asc = config.sort_asc_worktrees.unwrap_or(true);
        let include_remotes = config.include_remotes.unwrap_or(false);

        let mut branch_state = ListState::empty();
        branch_state.loading = true;
        branch_state.set_sort(branch_sort_col, branch_sort_asc);

        let mut remote_state = ListState::empty();
        remote_state.set_sort(remote_sort_col, remote_sort_asc);

        let mut tag_state = ListState::empty();
        tag_state.set_sort(tag_sort_col, tag_sort_asc);

        let mut worktree_state = ListState::empty();
        worktree_state.set_sort(worktree_sort_col, worktree_sort_asc);

        let remote_fetched = config.auto_fetch == Some(true);
        let cache = cache::BranchCache::load_for_base(&repo_path, &base_branch, &cache_root);
        let has_configured_remote = git2::Repository::open(&repo_path)
            .and_then(|r| r.remotes())
            .map(|r| !r.is_empty())
            .unwrap_or(false);
        let job_queue = ActionJobQueue::new(repo_path.clone(), base_branch.clone());

        Self {
            repo_path,
            base_branch,
            cache_root,
            #[cfg(test)]
            _test_cache_root: None,
            config_root: Config::default_root(),
            config,
            theme,
            symbols,
            should_exit: false,
            active_view: ViewId::Graph,
            graph: {
                let mut graph = GraphState::new();
                graph.set_include_remotes(include_remotes);
                graph
            },
            branches: branch_state,
            remotes: remote_state,
            tags: tag_state,
            worktrees: worktree_state,
            branch_columns: branch_def.columns(),
            remote_columns: remote_def.columns(),
            tag_columns: tag_def.columns(),
            worktree_columns: worktree_def.columns(),
            branch_filter_tokens: branch_def.filter_tokens(),
            remote_filter_tokens: remote_def.filter_tokens(),
            tag_filter_tokens: tag_def.filter_tokens(),
            worktree_filter_tokens: worktree_def.filter_tokens(),
            overlay: None,
            return_view: ViewId::Branches,
            squash_rx: None,
            squash_checked: 0,
            squash_total: 0,
            remote_squash_rx: None,
            cherry_rx: None,
            cherry_checked: 0,
            cherry_total: 0,
            remote_enrich_rx: None,
            worktree_enrich_rx: None,
            pr_rx: None,
            pr_map: PrMap::new(),
            op_rx: None,
            fetch_graph_delta_rx: None,
            fetch_overlay_dismissed: false,
            progress_rx: None,
            progress: None,
            diag_rx: None,
            cache_verify_rx: None,
            primary_code_check_request: None,
            verified_branches: HashSet::new(),
            remote_fetch_rx: None,
            tag_load_rx: None,
            worktree_load_rx: None,
            remote_load_rx: None,
            phase1_rx: None,
            graph_rx: None,
            graph_update_rx: None,
            graph_enrich_rx: None,
            commit_details_rx: None,
            commit_details_target: None,
            commit_file_diff_rx: None,
            commit_file_diff_target: None,
            graph_generation: 0,
            graph_generation_signal: Arc::new(AtomicU64::new(0)),
            graph_revision: 0,
            graph_repository_state: None,
            pending_graph_deltas: Vec::new(),
            graph_update_invalidates_enrichment: false,
            staged_graph_snapshot: None,
            reconciling_initial_graph: false,
            cache,
            toast: None,
            cancel_flag: None,
            job_queue,
            terminal_rows: 0,
            info_hit_regions: Vec::new(),
            info_copied_msg: None,
            info_modal_scroll_offset: 0,
            remote_fetched,
            has_configured_remote,
            last_branch_fingerprint: None,
        }
    }
}
