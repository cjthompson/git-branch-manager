use super::*;

impl App {
    // ---- View Loading ----

    pub(super) fn ensure_view_loaded(&mut self) {
        match self.active_view {
            ViewId::Graph if self.graph.snapshot().is_none() && !self.graph.is_loading() => {
                self.spawn_graph_load(self.graph.max_count(), self.graph.includes_remotes());
            }
            ViewId::Branches
                if self.worktrees.items().is_empty()
                    && !self.worktrees.loading
                    && self.config.load_worktrees_on_launch != Some(false) =>
            {
                // Branch-delete pre-flight needs worktree ownership and
                // working-tree status. Load it lazily when Branches becomes
                // visible, while preserving the opt-out for large repos.
                self.spawn_worktree_load();
            }
            ViewId::Tags if self.tags.items().is_empty() && !self.tags.loading => {
                self.spawn_tag_load();
            }
            ViewId::Remotes if self.remotes.items().is_empty() && !self.remotes.loading => {
                self.spawn_remote_load();
                // Trigger fetch if not yet done this session
                if !self.remote_fetched && self.config.auto_fetch == Some(true) {
                    self.start_remote_fetch();
                }
            }
            ViewId::Worktrees if self.worktrees.items().is_empty() && !self.worktrees.loading => {
                self.spawn_worktree_load();
            }
            _ => {}
        }
    }

    /// Preload the authoritative rows needed to adapt Graph refs into the
    /// existing Remote and Tag action menus. Branch rows already stream at
    /// launch through `phase1_rx`.
    pub fn preload_graph_action_metadata(&mut self) {
        if self.remotes.items().is_empty() && !self.remotes.loading {
            self.spawn_remote_load();
        }
        if self.tags.items().is_empty() && !self.tags.loading {
            self.spawn_tag_load();
        }
    }

    pub(super) fn bump_graph_generation(&mut self) {
        self.graph_generation = self.graph_generation.saturating_add(1);
        self.graph_generation_signal
            .store(self.graph_generation, Ordering::Release);
    }

    pub fn spawn_graph_load(&mut self, max_count: usize, include_remotes: bool) {
        self.bump_graph_generation();
        self.graph_revision = self.graph_revision.saturating_add(1);
        self.graph_update_invalidates_enrichment = false;
        self.staged_graph_snapshot = None;
        self.reconciling_initial_graph = false;
        // Signal cooperative cancellation and drop the previous receiver;
        // the drain generation gate also rejects racing publications.
        self.graph_enrich_rx = None;
        self.graph.begin_load(max_count, include_remotes);
        self.graph_rx = Some(graph::spawn_graph_loader(
            self.repo_path.clone(),
            graph::GraphLoadOptions {
                max_count,
                include_remotes,
                line_style: graph::GraphLineStyle::from_symbol_name(self.symbols.name),
                base_branch: Some(self.base_branch.clone()),
                cache_root: self.cache_root.clone(),
            },
        ));
        self.toast = Some(Toast::new("Loading graph...".into(), 300));
    }

    pub(super) fn reload_graph_for_symbol_change(&mut self) {
        if self.graph.snapshot().is_some() || self.graph.is_loading() {
            self.spawn_graph_load(self.graph.max_count(), self.graph.includes_remotes());
        }
    }

    pub(super) fn reload_graph(&mut self) {
        self.spawn_graph_load(self.graph.max_count(), self.graph.includes_remotes());
    }

    pub(super) fn reload_graph_with_remotes(&mut self, include_remotes: bool) {
        self.spawn_graph_load(self.graph.max_count(), include_remotes);
    }

    pub(super) fn apply_graph_options(&mut self, cursor: usize, include_remotes: bool) {
        if cursor == 0 {
            self.reload_graph_with_remotes(include_remotes);
        } else {
            self.graph.set_include_remotes(include_remotes);
            self.load_older_graph();
        }
    }

    pub(super) fn load_older_graph(&mut self) {
        let max_count = self.graph.load_older_history();
        self.spawn_graph_load(max_count, self.graph.includes_remotes());
    }

    pub(super) fn open_graph_options(&mut self) {
        self.return_view = ViewId::Graph;
        self.overlay = Some(Overlay::GraphOptions {
            cursor: 0,
            include_remotes: self.graph.includes_remotes(),
        });
    }

    pub(super) fn spawn_tag_load(&mut self) {
        self.tags.loading = true;
        let repo_path = self.repo_path.clone();
        let (tx, rx) = mpsc::channel();
        self.tag_load_rx = Some(rx);
        self.toast = Some(Toast::new("Loading tags...".into(), 300));
        std::thread::spawn(move || {
            if let Ok(repo) = git2::Repository::open(&repo_path) {
                let tag_list = tags::list_tags(&repo);
                let _ = tx.send(tag_list);
            }
        });
    }

    pub(super) fn spawn_remote_load(&mut self) {
        self.remotes.loading = true;
        let repo_path = self.repo_path.clone();
        let base_branch = self.base_branch.clone();
        let cache_root = self.cache_root.clone();
        let (tx, rx) = mpsc::channel();
        self.remote_load_rx = Some(rx);
        self.toast = Some(Toast::new("Loading remote branches...".into(), 300));
        std::thread::spawn(move || {
            let Ok(repo) = git2::Repository::open(&repo_path) else {
                return;
            };
            let Ok(remote_branches) = branch::list_remote_branches_phase1(&repo, &base_branch)
            else {
                return;
            };

            let branch_cache =
                cache::BranchCache::load_for_base(&repo_path, &base_branch, &cache_root);
            // Remote branches don't precompute a merge base, so the merge-base slot is
            // None and is_squash_merged falls back to `git merge-base` for them.
            let candidates: Vec<(String, String, Option<String>)> = remote_branches
                .iter()
                .filter(|b| b.merge_status == MergeStatus::Pending && !b.is_base)
                .filter_map(|b| {
                    let refname = format!("refs/remotes/{}", b.full_ref);
                    repo.find_reference(&refname)
                        .ok()
                        .and_then(|r| r.peel_to_commit().ok())
                        .map(|c| (b.full_ref.clone(), c.id().to_string(), None))
                })
                .collect();

            let _ = tx.send((remote_branches, candidates, branch_cache));
        });
    }

    /// Re-correlate each loaded worktree's merge status from the current branch
    /// list. Cheap linear scan, no I/O. Called whenever either the worktree set
    /// or branch merge statuses change, since the two load on separate channels
    /// and can arrive in any order.
    pub(super) fn refresh_worktree_merge_status(&mut self) {
        worktree::apply_branch_merge_status(self.worktrees.items_mut(), self.branches.items());
        list_state::apply_sort(&mut self.worktrees, &self.worktree_columns);
    }

    pub(super) fn spawn_worktree_load(&mut self) {
        self.worktrees.loading = true;
        let repo_path = self.repo_path.clone();
        let (tx, rx) = mpsc::channel();
        self.worktree_load_rx = Some(rx);
        self.toast = Some(Toast::new("Loading worktrees...".into(), 300));
        std::thread::spawn(move || {
            let wts = worktree::list_worktrees(&repo_path);
            let _ = tx.send(wts);
        });
    }

    pub(super) fn start_remote_fetch(&mut self) {
        self.start_observed_remote_fetch(false);
    }

    pub fn start_auto_fetch(&mut self) {
        self.start_observed_remote_fetch(true);
    }

    pub(super) fn start_observed_remote_fetch(&mut self, startup: bool) {
        // Startup fetch already covers any immediate request from the
        // Remotes view. Keep its completion receiver until its delta is routed.
        if self.remote_fetch_rx.is_some() {
            return;
        }
        let repo_path = self.repo_path.clone();
        let (tx, rx) = mpsc::channel();
        self.remote_fetch_rx = Some(rx);
        if !startup {
            self.toast = Some(Toast::new("Fetching remote branches...".into(), 300));
        }
        std::thread::spawn(move || {
            let before = graph::GraphRepositoryState::capture(&repo_path);
            let success = operations::fetch_sync(&repo_path);
            let after = graph::GraphRepositoryState::capture(&repo_path);
            let graph_delta = match (before, after) {
                (Ok(before), Ok(after)) => Ok(graph::GraphRepositoryDelta::between(before, after)),
                (Err(error), _) | (_, Err(error)) => Err(error),
            };
            let _ = tx.send(RemoteFetchCompletion { success, graph_delta });
        });
    }
}
