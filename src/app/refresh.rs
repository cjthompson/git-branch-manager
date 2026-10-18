use super::*;

impl App {
    pub fn refresh_branches(&mut self, trigger: &str) {
        let repo_path = self.repo_path.clone();
        let base_branch = self.base_branch.clone();

        let Ok(repo) = git2::Repository::open(&repo_path) else {
            return;
        };

        self.has_configured_remote = repo.remotes().map(|r| !r.is_empty()).unwrap_or(false);

        let fingerprint = branch_input_fingerprint(&repo, &base_branch);
        let inputs_changed = self.last_branch_fingerprint != Some(fingerprint);
        self.last_branch_fingerprint = Some(fingerprint);
        let _span = tracing::info_span!(
            "branch_load",
            trigger = %trigger,
            path = "sync_full",
            inputs_changed = inputs_changed,
        )
        .entered();

        let Ok(branches) = branch::list_branches_phase1(&repo, &base_branch) else {
            return;
        };

        let new_cache =
            cache::BranchCache::load_for_base(&repo_path, &base_branch, &self.cache_root);
        let cache_for_cherry =
            cache::BranchCache::load_for_base(&repo_path, &base_branch, &self.cache_root);

        // list_branches_phase1 already filled merge bases; skip disjoint branches
        // (no merge base) and carry the precomputed merge base into the squash check.
        let candidates: Vec<(String, String, Option<String>)> = branches
            .iter()
            .filter(|b| {
                b.merge_status == MergeStatus::Pending
                    && !b.is_base
                    && !b.is_current
                    && b.merge_base_commit.is_some()
            })
            .filter_map(|b| {
                branch::get_commit_hash(&repo, &b.name)
                    .map(|hash| (b.name.clone(), hash, b.merge_base_commit.clone()))
            })
            .collect();

        self.branches.set_items(branches);

        // Restore sort
        list_state::apply_sort(&mut self.branches, &self.branch_columns);

        // Spawn squash checker
        self.squash_total = candidates.len();
        self.squash_checked = 0;
        let cherry_candidates = candidates.clone();
        if !candidates.is_empty() {
            self.squash_rx = Some(squash_loader::spawn_squash_checker(
                repo_path.clone(),
                base_branch.clone(),
                candidates,
                new_cache,
            ));
        }

        // Spawn cherry-pick checker
        self.cherry_total = cherry_candidates.len();
        self.cherry_checked = 0;
        if !cherry_candidates.is_empty() {
            self.cherry_rx = Some(cherry_loader::spawn_cherry_checker(
                repo_path.clone(),
                base_branch.clone(),
                cherry_candidates,
                cache_for_cherry,
            ));
        }

        // Re-spawn PR loader
        self.pr_rx = Some(pr_loader::spawn_pr_loader(repo_path));
    }

    pub(super) fn refresh_after_operation(&mut self) {
        match self.return_view {
            ViewId::Graph => {
                self.reload_graph();
                self.active_view = ViewId::Graph;
            }
            ViewId::Branches => {
                self.refresh_branches("post_operation");
                self.active_view = ViewId::Branches;
            }
            ViewId::Remotes => {
                // Reload remote branches (spawn_remote_load sets loading=true;
                // do not reset ListState here or sort_column/sort_ascending is lost)
                self.spawn_remote_load();
                self.active_view = ViewId::Remotes;
            }
            ViewId::Tags => {
                self.spawn_tag_load();
                self.active_view = ViewId::Tags;
            }
            ViewId::Worktrees => {
                self.spawn_worktree_load();
                self.active_view = ViewId::Worktrees;
            }
        }
    }

    /// Like `refresh_after_operation`, but does not snap `active_view` back to
    /// the completed job's originating view. Since job-queue execution is
    /// non-modal, the user may have navigated elsewhere while it ran, and
    /// forcing them back to `view` when it finishes would be a jarring
    /// regression (this is safe for `refresh_after_operation`'s other callers
    /// only because those flows are still fully modal).
    pub(super) fn refresh_view_data(&mut self, view: ViewId) {
        match view {
            ViewId::Graph => self.reload_graph(),
            ViewId::Branches => self.refresh_branches("post_operation"),
            ViewId::Remotes => self.spawn_remote_load(),
            ViewId::Tags => self.spawn_tag_load(),
            ViewId::Worktrees => self.spawn_worktree_load(),
        }
    }

    pub(super) fn refresh_after_job(&mut self, action: BranchAction, origin: ViewId, _targets: &[String], _remote: Option<&str>, _results: &[OperationResult], graph_delta: Result<graph::GraphRepositoryDelta, String>) {
        let graph_affecting = graph_affected_views(action).contains(&ViewId::Graph);

        // Refresh the other (non-Graph) views affected by this action. Graph
        // gets its own two-tier treatment below.
        for view in graph_affected_views(action) {
            if *view == ViewId::Graph {
                continue;
            }
            self.refresh_view_data(*view);
        }
        if origin != ViewId::Graph && !graph_affected_views(action).contains(&origin) {
            self.refresh_view_data(origin);
        }

        if graph_affecting {
            self.request_graph_update(graph_delta);
        }
    }

    pub(super) fn request_graph_update(&mut self, delta: Result<graph::GraphRepositoryDelta, String>) {
        self.graph_revision = self.graph_revision.saturating_add(1);
        let invalidates_enrichment = match delta.as_ref() {
            Err(_) => true,
            Ok(delta) => {
            let topology_ref = |id: &graph::GraphRefId| id.kind == graph::GraphRefKind::LocalBranch || self.graph.includes_remotes();
                delta.added_refs.iter().any(|(id, _)| topology_ref(id))
                || delta.removed_refs.iter().any(|(id, _)| topology_ref(id))
                || delta.moved_refs.iter().any(|(id, _, _)| topology_ref(id))
                || (self.graph.includes_remotes() && delta.additional_roots_changed)
            }
        };
        let update_in_flight = ((self.graph.is_loading() || self.graph_rx.is_some())
            && !self.reconciling_initial_graph)
            || self.graph_update_rx.is_some();
        let was_invalidating = self.graph_update_invalidates_enrichment;
        self.graph_update_invalidates_enrichment |= invalidates_enrichment;
        // An immediate failed delta reloads below and signals cancellation
        // there. Queued failures must cancel before that reload can start.
        if invalidates_enrichment
            && (delta.is_ok() || update_in_flight)
            && !was_invalidating
            && (self.graph.snapshot().is_some() || self.staged_graph_snapshot.is_some())
        {
            self.bump_graph_generation();
            self.graph_enrich_rx = None;
        }
        if update_in_flight {
            self.pending_graph_deltas.push(delta);
            return;
        }
        let Some(snapshot) = self.staged_graph_snapshot.clone().or_else(|| self.graph.snapshot().cloned()) else {
            // Graph has never loaded; its first open reads current state.
            if !self.reconciling_initial_graph {
                return;
            }
            self.pending_graph_deltas.push(delta);
            self.reload_graph();
            return;
        };
        let delta = match delta {
            Ok(delta) => delta,
            Err(_) => {
                self.graph_enrich_rx = None;
                self.reload_graph();
                return;
            }
        };
        let options = graph::GraphLoadOptions {
                max_count: self.graph.max_count(),
                include_remotes: self.graph.includes_remotes(),
                line_style: graph::GraphLineStyle::from_symbol_name(self.symbols.name),
                base_branch: Some(self.base_branch.clone()),
                cache_root: self.cache_root.clone(),
            };
        self.graph_repository_state = Some(delta.after.clone());
        self.graph_update_rx = Some(graph::spawn_graph_updater(
            self.repo_path.clone(), snapshot, delta, options, self.graph_revision,
        ));
    }

    pub(super) fn start_fetch(&mut self, prune: bool) {
        if self.op_rx.is_some() {
            return;
        }
        let repo_path = self.repo_path.clone();
        let (tx, rx) = mpsc::channel();
        let (graph_tx, graph_rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_clone = Arc::clone(&cancel);
        self.op_rx = Some(rx);
        self.fetch_graph_delta_rx = Some(graph_rx);
        self.fetch_overlay_dismissed = false;
        self.cancel_flag = Some(cancel);
        self.return_view = self.active_view;
        let label = if prune {
            "Fetching with prune..."
        } else {
            "Fetching..."
        };
        self.overlay = Some(Overlay::Executing {
            label: label.into(),
            progress: None,
        });
        std::thread::spawn(move || {
            let before = graph::GraphRepositoryState::capture(&repo_path);
            let result = if prune {
                operations::fetch_prune(&repo_path, &cancel_clone)
            } else {
                operations::fetch(&repo_path, &cancel_clone)
            };
            let after = graph::GraphRepositoryState::capture(&repo_path);
            let observed = match (before, after) {
                (Ok(before), Ok(after)) => Ok(graph::GraphRepositoryDelta::between(before, after)),
                (Err(error), _) | (_, Err(error)) => Err(error),
            };
            let _ = graph_tx.send(observed);
            let _ = tx.send(vec![result]);
        });
    }

    pub(super) fn clear_cache_and_refresh(&mut self) {
        let mut bc =
            cache::BranchCache::load_for_base(&self.repo_path, &self.base_branch, &self.cache_root);
        bc.clear();
        self.refresh_branches("manual_refresh_R");
        self.toast = Some(Toast::new("Cache cleared".into(), 3));
    }
}
