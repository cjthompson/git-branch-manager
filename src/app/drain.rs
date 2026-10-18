use super::*;

impl App {
    // ---- Channel Draining ----

    pub(super) fn drain_channels(&mut self) -> bool {
        let mut dirty = false;

        for result in drain_channel(&mut self.graph_rx, 1, &mut dirty) {
            // Stamp the snapshot with the current reload generation so the
            // enrichment that follows can be matched back to it. The
            // generation is also the gate we check below in the enrich
            // drain — stale enrichment from a previous load is dropped
            // rather than overwriting the newer snapshot's markers.
            let result = match result {
                Ok(mut snapshot) => {
                    snapshot.generation = Some(self.graph_generation);
                    Ok(snapshot)
                }
                err => err,
            };
            self.clear_toast();
            self.graph_repository_state = graph::GraphRepositoryState::capture(&self.repo_path).ok();
            let mut pending = std::mem::take(&mut self.pending_graph_deltas);
            if let (Ok(snapshot), Some(first)) = (result.as_ref().map(Clone::clone), pending.first().cloned()) {
                self.staged_graph_snapshot = Some(snapshot);
                self.reconciling_initial_graph = true;
                let _ = pending.remove(0);
                self.pending_graph_deltas = pending;
                self.request_graph_update(first);
                continue;
            }
            self.graph.apply_result(result);

            if let Some(snapshot) = self.graph.snapshot().cloned() {
                self.graph_enrich_rx = Some(graph::spawn_possible_squash_enrichment(
                    snapshot,
                    self.repo_path.clone(),
                    Some(self.base_branch.clone()),
                    self.graph_generation,
                    self.cache_root.clone(),
                    Arc::clone(&self.graph_generation_signal),
                ));
            }
        }

        let mut update_failed = false;
        let update_result = if let Some(rx) = self.graph_update_rx.take() {
            match rx.try_recv() {
                Ok(message) => Some(message),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => { update_failed = true; None },
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    self.graph_update_rx = Some(rx);
                    None
                }
            }
        } else { None };
        if let Some(message) = update_result {
            let options_are_current = message.options.max_count == self.graph.max_count()
                && message.options.include_remotes == self.graph.includes_remotes()
                && message.options.base_branch.as_deref() == Some(self.base_branch.as_str())
                && message.options.line_style == graph::GraphLineStyle::from_symbol_name(self.symbols.name);
            if message.revision == self.graph_revision && options_are_current {
                match message.result {
                    Ok(mut snapshot) => {
                        if snapshot.max_count != message.options.max_count
                            || snapshot.includes_remotes != message.options.include_remotes
                        {
                            self.pending_graph_deltas.insert(0, Ok(message.delta));
                        } else {
                            snapshot.generation = Some(self.graph_generation);
                            // A metadata-only worker may have captured an
                            // unenriched snapshot before the async enrichment
                            // completed. Carry the latest values forward by OID.
                            if !self.graph_update_invalidates_enrichment {
                                if let Some(current) = self.graph.snapshot() {
                                    let relationships = current
                                        .commits
                                        .iter()
                                        .flat_map(|commit| commit.relationships.iter().cloned())
                                        .collect::<Vec<_>>();
                                    graph::apply_squash_enrichment(&mut snapshot, &relationships);
                                }
                            }
                            if self.reconciling_initial_graph {
                                self.staged_graph_snapshot = Some(snapshot);
                                if self.pending_graph_deltas.is_empty() {
                                    if let Some(snapshot) = self.staged_graph_snapshot.take() {
                                        self.graph.apply_result(Ok(snapshot));
                                    }
                                    self.reconciling_initial_graph = false;
                                }
                            } else {
                                self.graph.apply_incremental_result(Ok(snapshot));
                            }
                            if self.graph_update_invalidates_enrichment {
                                self.graph_update_invalidates_enrichment = false;
                                self.graph_enrich_rx = None;
                            }
                            if self.graph_enrich_rx.is_none() && !self.reconciling_initial_graph {
                                if let Some(snapshot) = self.graph.snapshot().cloned() {
                                    self.graph_enrich_rx = Some(graph::spawn_possible_squash_enrichment(
                                        snapshot, self.repo_path.clone(), Some(self.base_branch.clone()), self.graph_generation, self.cache_root.clone(), Arc::clone(&self.graph_generation_signal),
                                    ));
                                }
                            }
                        }
                    }
                    Err(error) => {
                        self.toast = Some(Toast::new(format!("Graph update failed: {error}"), 4));
                        self.reconciling_initial_graph = false;
                        self.staged_graph_snapshot = None;
                        self.reload_graph();
                    }
                }
            } else {
                self.pending_graph_deltas.insert(0, Ok(message.delta));
            }
            if !self.pending_graph_deltas.is_empty() && self.graph_update_rx.is_none() {
                let next = self.pending_graph_deltas.remove(0);
                self.request_graph_update(next);
            }
            dirty = true;
        }
        if update_failed {
            self.reconciling_initial_graph = false;
            self.staged_graph_snapshot = None;
            self.reload_graph();
            dirty = true;
        }

        for msg in drain_channel(&mut self.graph_enrich_rx, 1, &mut dirty) {
            // Cooperative cancellation stops superseded workers at work
            // boundaries. Keep the generation gate for a result that raced
            // with invalidation immediately before publication.
            if msg.generation == self.graph_generation {
                self.graph.apply_squash_enrichment(&msg.updates);
            }
        }

        if self.commit_details_target.is_some() && !self.commit_details_context_is_current() {
            self.invalidate_commit_details_request();
        }
        for (oid, result) in drain_channel(&mut self.commit_details_rx, 1, &mut dirty) {
            if self.commit_details_target.as_deref() != Some(oid.as_str()) {
                continue;
            }
            self.commit_details_target = None;
            if !self.commit_details_context_matches(&oid) {
                continue;
            }
            match result {
                Ok(details) => {
                    let commit = self
                        .graph
                        .snapshot()
                        .and_then(|snapshot| snapshot.commits.iter().find(|c| c.oid == oid))
                        .cloned();
                    if let Some(commit) = commit {
                        let items = self.build_graph_menu_for(&commit);
                        let cursor = first_enabled_index(&items).unwrap_or(0);
                        let focus = if items.iter().any(|item| item.enabled) {
                            CommitDetailsFocus::Actions
                        } else {
                            CommitDetailsFocus::Files
                        };
                        self.overlay = Some(Overlay::CommitDetails {
                            commit,
                            details,
                            items,
                            cursor,
                            file_cursor: 0,
                            focus,
                            scroll: ModalScroll::default(),
                        });
                    }
                }
                Err(error) => self.toast = Some(Toast::new(error, 5)),
            }
        }

        for ((oid, path), result) in drain_channel(&mut self.commit_file_diff_rx, 1, &mut dirty) {
            if self.commit_file_diff_target.as_ref() != Some(&(oid.clone(), path.clone())) {
                continue;
            }
            self.commit_file_diff_target = None;
            let valid_context = matches!(
                self.overlay.as_ref(),
                Some(Overlay::CommitDetails { commit, details, file_cursor, .. })
                    if commit.oid == oid
                        && details.oid == oid
                        && details.files.get(*file_cursor).is_some_and(|file| file.path == path)
            );
            if !valid_context {
                continue;
            }
            match result {
                Ok(diff) => {
                    if let Some(Overlay::CommitDetails {
                        commit,
                        details,
                        items,
                        cursor,
                        file_cursor,
                        focus,
                        scroll: details_scroll,
                    }) = self.overlay.take()
                    {
                        self.overlay = Some(Overlay::CommitDiff {
                            commit,
                            details,
                            items,
                            cursor,
                            file_cursor,
                            focus,
                            details_scroll,
                            diff,
                            scroll: ModalScroll::default(),
                        });
                    }
                }
                Err(error) => self.toast = Some(Toast::new(error, 5)),
            }
        }

        // Phase-1 messages (fast metadata first, merge statuses second)
        for msg in drain_channel(&mut self.phase1_rx, 2, &mut dirty) {
            match msg {
                Phase1Msg::Fast(branches, cache_for_app, _cache_for_squash) => {
                    self.cache = *cache_for_app;
                    self.branches.set_items(branches);
                    self.branches.loading = false;
                    list_state::apply_sort(&mut self.branches, &self.branch_columns);
                    // Squash checker and PR loader are spawned after merge statuses arrive.
                }
                Phase1Msg::MergeStatuses(updates) => {
                    let update_map: std::collections::HashMap<String, MergeStatus> =
                        updates.into_iter().collect();
                    for b in self.branches.items_mut() {
                        // A branch the launch-time cache verifier already corrected
                        // wins regardless of arrival order — this restore path can
                        // otherwise reintroduce the exact stale status the verifier
                        // just fixed (it restores from cache, which is why the
                        // verifier exists in the first place).
                        if self.verified_branches.contains(&b.name) {
                            continue;
                        }
                        if let Some(&new_status) = update_map.get(&b.name) {
                            b.merge_status = new_status;
                        }
                    }
                    list_state::apply_sort(&mut self.branches, &self.branch_columns);

                    // Now spawn squash checker on the updated (merged-filtered) set.
                    let repo_path = self.repo_path.clone();
                    let base_branch = self.base_branch.clone();
                    let cache_for_squash = cache::BranchCache::load_for_base(
                        &repo_path,
                        &base_branch,
                        &self.cache_root,
                    );
                    let cache_for_cherry = cache::BranchCache::load_for_base(
                        &repo_path,
                        &base_branch,
                        &self.cache_root,
                    );
                    if let Ok(repo) = git2::Repository::open(&repo_path) {
                        // MergeBaseCommits is sent before MergeStatuses, so merge_base_commit
                        // is populated here. A Pending branch with no merge base is disjoint
                        // from base and can't be squash-merged. list_branches_fast marked every
                        // non-pinned branch Pending without knowing merge bases, so resolve the
                        // disjoint ones to Unmerged now — otherwise they'd sit at Pending forever
                        // (they're excluded from the squash candidate set below).
                        for b in self.branches.items_mut() {
                            if b.merge_status == MergeStatus::Pending
                                && !b.is_base
                                && !b.is_current
                                && b.merge_base_commit.is_none()
                            {
                                b.merge_status = MergeStatus::Unmerged;
                            }
                        }
                        list_state::apply_sort(&mut self.branches, &self.branch_columns);

                        // Connected branches carry their precomputed merge base into the squash
                        // check so it never re-derives it via the unbounded `git merge-base` walk.
                        let candidates: Vec<(String, String, Option<String>)> = self
                            .branches
                            .items()
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

                        self.squash_total = candidates.len();
                        self.squash_checked = 0;
                        let cherry_candidates = candidates.clone();
                        if !candidates.is_empty() {
                            self.squash_rx = Some(squash_loader::spawn_squash_checker(
                                repo_path.clone(),
                                base_branch.clone(),
                                candidates,
                                cache_for_squash,
                            ));
                        }

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
                    }
                    self.pr_rx = Some(pr_loader::spawn_pr_loader(repo_path));
                    // Branch merge statuses just changed; re-correlate worktrees
                    // if they're already loaded.
                    self.refresh_worktree_merge_status();
                }
                Phase1Msg::AheadBehind(updates) => {
                    for (name, ahead, behind) in updates {
                        if let Some(b) = self
                            .branches
                            .items_mut()
                            .iter_mut()
                            .find(|b| b.name == name)
                        {
                            b.ahead = ahead;
                            b.behind = behind;
                        }
                    }
                    list_state::apply_sort(&mut self.branches, &self.branch_columns);
                }
                Phase1Msg::MergeBaseCommits(updates) => {
                    for (name, hash) in updates {
                        if let Some(b) = self
                            .branches
                            .items_mut()
                            .iter_mut()
                            .find(|b| b.name == name)
                        {
                            b.merge_base_commit = Some(hash);
                        }
                    }
                    list_state::apply_sort(&mut self.branches, &self.branch_columns);
                }
            }
        }

        // Squash-merge results (cap 32 per tick)
        let squash_results = drain_channel(&mut self.squash_rx, 32, &mut dirty);
        let had_squash_results = !squash_results.is_empty();
        for result in squash_results {
            self.squash_checked += 1;
            if let Some(b) = self
                .branches
                .items_mut()
                .iter_mut()
                .find(|b| b.name == result.branch_name)
            {
                if apply_merge_status_if_confident(&mut b.merge_status, result.status) {
                    b.squash_confidence = result.confidence;
                }
            }
            // Propagate squash-merge status to the matching remote branch. The remote
            // enricher marks ahead>0 branches as Unmerged; squash detection on the local
            // branch is authoritative for squash-merged remotes.
            if !matches!(result.status, MergeStatus::Unmerged | MergeStatus::Pending) {
                if let Some(r) = self
                    .remotes
                    .items_mut()
                    .iter_mut()
                    .find(|r| r.short_name == result.branch_name)
                {
                    if apply_merge_status_if_confident(&mut r.merge_status, result.status) {
                        r.squash_confidence = result.confidence;
                    }
                }
            }
        }
        if had_squash_results {
            // Squash detection just resolved branch statuses; re-correlate worktrees.
            list_state::apply_sort(&mut self.branches, &self.branch_columns);
            self.refresh_worktree_merge_status();
            list_state::apply_sort(&mut self.remotes, &self.remote_columns);
        }

        // Cherry-pick results (cap 32 per tick, mirrors squash cadence)
        let cherry_results = drain_channel(&mut self.cherry_rx, 32, &mut dirty);
        let had_cherry_results = !cherry_results.is_empty();
        for result in cherry_results {
            self.cherry_checked += 1;
            if let Some(b) = self
                .branches
                .items_mut()
                .iter_mut()
                .find(|b| b.name == result.branch_name)
            {
                if apply_merge_status_if_confident(&mut b.merge_status, result.status) {
                    b.squash_confidence = None;
                }
            }
            if !matches!(result.status, MergeStatus::Unmerged | MergeStatus::Pending) {
                if let Some(r) = self
                    .remotes
                    .items_mut()
                    .iter_mut()
                    .find(|r| r.short_name == result.branch_name)
                {
                    if apply_merge_status_if_confident(&mut r.merge_status, result.status) {
                        r.squash_confidence = None;
                    }
                }
            }
        }
        if had_cherry_results {
            list_state::apply_sort(&mut self.branches, &self.branch_columns);
            self.refresh_worktree_merge_status();
            list_state::apply_sort(&mut self.remotes, &self.remote_columns);
        }

        // Remote squash-merge results
        let remote_squash_results = drain_channel(&mut self.remote_squash_rx, 32, &mut dirty);
        let had_remote_squash_results = !remote_squash_results.is_empty();
        for result in remote_squash_results {
            if let Some(b) = self
                .remotes
                .items_mut()
                .iter_mut()
                .find(|b| b.full_ref == result.branch_name)
            {
                if apply_merge_status_if_confident(&mut b.merge_status, result.status) {
                    b.squash_confidence = result.confidence;
                }
            }
        }
        if had_remote_squash_results {
            list_state::apply_sort(&mut self.remotes, &self.remote_columns);
        }

        // Remote enrichment
        let remote_enrich_results = drain_channel(&mut self.remote_enrich_rx, 32, &mut dirty);
        let had_remote_enrich_results = !remote_enrich_results.is_empty();
        for result in remote_enrich_results {
            if let Some(b) = self
                .remotes
                .items_mut()
                .iter_mut()
                .find(|b| b.full_ref == result.full_ref)
            {
                if apply_merge_status_if_confident(&mut b.merge_status, result.merge_status) {
                    b.squash_confidence = None;
                }
                b.ahead = result.ahead;
                b.behind = result.behind;
                b.disjoint = result.disjoint;
            }
        }
        if had_remote_enrich_results {
            list_state::apply_sort(&mut self.remotes, &self.remote_columns);
        }

        // Worktree enrichment
        let worktree_enrich_results = drain_channel(&mut self.worktree_enrich_rx, 32, &mut dirty);
        let had_worktree_enrich_results = !worktree_enrich_results.is_empty();
        for result in worktree_enrich_results {
            if let Some(wt) = self
                .worktrees
                .items_mut()
                .iter_mut()
                .find(|wt| wt.path == result.path)
            {
                wt.wt_status = result.wt_status;
                wt.age_date = result.age_date;
            }
        }
        if had_worktree_enrich_results {
            list_state::apply_sort(&mut self.worktrees, &self.worktree_columns);
        }

        // PR map (one-shot)
        for map in drain_channel(&mut self.pr_rx, 1, &mut dirty) {
            self.pr_map = map;
            // Push PR data into branch items
            for branch in self.branches.items_mut() {
                branch.pr = self.pr_map.get(&branch.name).cloned();
            }
            // Push PR data into remote items (keyed by short_name)
            for remote in self.remotes.items_mut() {
                remote.pr = self.pr_map.get(&remote.short_name).cloned();
            }
            list_state::apply_sort(&mut self.branches, &self.branch_columns);
            list_state::apply_sort(&mut self.remotes, &self.remote_columns);
        }

        // Tag loading (one-shot)
        for items in drain_channel(&mut self.tag_load_rx, 1, &mut dirty) {
            self.tags.set_items(items);
            self.tags.loading = false;
            list_state::apply_sort(&mut self.tags, &self.tag_columns);
            self.clear_toast();
        }

        // Remote loading (one-shot)
        for (remotes, candidates, remote_cache) in
            drain_channel(&mut self.remote_load_rx, 1, &mut dirty)
        {
            self.remotes.set_items(remotes);
            self.remotes.loading = false;
            list_state::apply_sort(&mut self.remotes, &self.remote_columns);
            self.clear_toast();

            // Spawn remote enrichment
            let unmerged: Vec<RemoteBranchInfo> = self
                .remotes
                .items()
                .iter()
                .filter(|b| !b.is_base)
                .cloned()
                .collect();
            if !unmerged.is_empty() {
                self.remote_enrich_rx = Some(branch::spawn_remote_enricher(
                    self.repo_path.clone(),
                    self.base_branch.clone(),
                    unmerged,
                ));
            }

            // Spawn remote squash checker
            if !candidates.is_empty() {
                self.remote_squash_rx = Some(squash_loader::spawn_squash_checker(
                    self.repo_path.clone(),
                    self.base_branch.clone(),
                    candidates,
                    remote_cache,
                ));
            }
        }

        // Worktree loading (one-shot)
        for items in drain_channel(&mut self.worktree_load_rx, 1, &mut dirty) {
            self.worktrees.set_items(items);
            self.worktrees.loading = false;
            list_state::apply_sort(&mut self.worktrees, &self.worktree_columns);
            self.clear_toast();

            // Branches may already be loaded; correlate merge status now.
            self.refresh_worktree_merge_status();

            // Spawn worktree enrichment
            let rx = worktree::enrich_worktrees(self.worktrees.items().to_vec());
            self.worktree_enrich_rx = Some(rx);
        }

        if let Some(request) = self.primary_code_check_request.as_ref() {
            match request.receiver.try_recv() {
                Ok(result) => {
                    let request = self.primary_code_check_request.take().unwrap();
                    let owns_execution = self
                        .cancel_flag
                        .as_ref()
                        .is_some_and(|cancel| Arc::ptr_eq(cancel, &request.cancel));
                    if owns_execution && matches!(self.overlay, Some(Overlay::Executing { .. })) {
                        self.cancel_flag = None;
                        self.toast = None;
                        self.info_copied_msg = None;
                        self.info_modal_scroll_offset = 0;
                        self.overlay = Some(Overlay::InfoModal {
                            items: Vec::new(),
                            cursor: 0,
                            info_cursor: 0,
                            focus: InfoModalFocus::Info,
                            row: InfoModalRow::PrimaryBranchCodeCheck {
                                display_ref: result.display_ref,
                                base_branch: result.base_branch,
                                primary_reference: result.primary_reference,
                                primary_oid: result.primary_oid,
                                outcome: result.outcome,
                            },
                        });
                    } else {
                        request.cancel.store(true, Ordering::Relaxed);
                        if owns_execution {
                            self.cancel_flag = None;
                        }
                    }
                    dirty = true;
                }
                Err(TryRecvError::Disconnected) => {
                    let request = self.primary_code_check_request.take().unwrap();
                    let owns_execution = self
                        .cancel_flag
                        .as_ref()
                        .is_some_and(|cancel| Arc::ptr_eq(cancel, &request.cancel));
                    if owns_execution && matches!(self.overlay, Some(Overlay::Executing { .. })) {
                        self.cancel_flag = None;
                        self.overlay = None;
                        self.toast = Some(Toast::new(
                            "Primary branch check stopped unexpectedly".into(),
                            60,
                        ));
                    } else {
                        request.cancel.store(true, Ordering::Relaxed);
                        if owns_execution {
                            self.cancel_flag = None;
                        }
                    }
                    dirty = true;
                }
                Err(TryRecvError::Empty) => {
                    let owns_execution = self
                        .cancel_flag
                        .as_ref()
                        .is_some_and(|cancel| Arc::ptr_eq(cancel, &request.cancel));
                    let request_is_active =
                        owns_execution && matches!(self.overlay, Some(Overlay::Executing { .. }));
                    if !request_is_active {
                        let request = self.primary_code_check_request.take().unwrap();
                        request.cancel.store(true, Ordering::Relaxed);
                        if owns_execution {
                            self.cancel_flag = None;
                        }
                        dirty = true;
                    }
                }
            }
        }

        // Operation results (one-shot) -- fetch only; confirmed-action jobs
        // are handled by self.job_queue below, non-modally.
        for results in drain_channel(&mut self.op_rx, 1, &mut dirty) {
            self.clear_cancel_flag_unless_primary_code_check_owns_it();
            self.progress_rx = None;
            self.progress = None;
            let graph_delta = self
                .fetch_graph_delta_rx
                .take()
                .and_then(|rx| rx.try_recv().ok())
                .unwrap_or_else(|| Err("fetch repository state observation was unavailable".into()));
            match self.return_view {
                ViewId::Branches => self.refresh_branches("post_fetch"),
                ViewId::Remotes => self.spawn_remote_load(),
                ViewId::Tags => self.spawn_tag_load(),
                ViewId::Worktrees | ViewId::Graph => {}
            }
            self.request_graph_update(graph_delta);
            if !self.fetch_overlay_dismissed && !self.primary_code_check_is_executing() {
                self.overlay = Some(Overlay::results(results));
            }
            self.fetch_overlay_dismissed = false;
        }

        // Confirmed-action job queue (delete, push, merge, worktree ops, ...)
        let job_poll = self.job_queue.poll();
        if job_poll.dirty {
            dirty = true;
        }
        if let Some(JobEvent {
            action,
            targets,
            remote,
            results,
            graph_delta,
            failures,
            return_view,
        }) = job_poll.event
        {
            self.refresh_after_job(action, return_view, &targets, remote.as_deref(), &results, graph_delta);

            // Plan P005 §5: when the job produced any typed failures,
            // auto-open the Results overlay so the user sees the cause
            // and can review a per-result recovery without scrolling past
            // successes. Success-only completion stays non-modal — the
            // transient `CompletionSummary` in the status area is the
            // only feedback. `failures` already excludes `BranchNotFound`
            // (treated as "already gone" success — see §9).
            if !failures.is_empty() {
                self.overlay = Some(Overlay::results(failures));
            }

            // When DeleteLocalAndRemote completes, immediately filter confirmed remote
            // deletions from the in-memory remotes list so they don't appear until a fetch.
            if action == BranchAction::DeleteLocalAndRemote {
                let selected_remote = remote.as_deref().unwrap_or("origin");
                let successfully_deleted: Vec<String> = results
                    .iter()
                    .filter(|r| r.success && r.action == BranchAction::DeleteRemoteBranch)
                    .map(|r| r.branch_name.clone())
                    .collect();

                if !successfully_deleted.is_empty() {
                    let deleted_set: std::collections::HashSet<_> =
                        successfully_deleted.into_iter().collect();
                    self.remotes.set_items(
                        self.remotes
                            .items()
                            .iter()
                            .filter(|remote| {
                                remote.remote != selected_remote
                                    || !deleted_set.contains(&remote.short_name)
                            })
                            .cloned()
                            .collect(),
                    );
                    list_state::apply_sort(&mut self.remotes, &self.remote_columns);
                }
            }
        }

        // Cache-audit result (one-shot)
        for audit in drain_channel(&mut self.diag_rx, 1, &mut dirty) {
            self.clear_cancel_flag_unless_primary_code_check_owns_it();
            self.progress_rx = None;
            self.progress = None;
            if !self.primary_code_check_is_executing() {
                self.overlay = Some(Overlay::DiagnosticsReport { audit, scroll: 0 });
            }
        }

        // Silent, automatic launch-time cache verification (one-shot). The
        // audit has already been applied to disk by `spawn_cache_verifier`;
        // this only patches live state. No overlay, no toast — fully silent.
        for audit in drain_channel(&mut self.cache_verify_rx, 1, &mut dirty) {
            if audit.is_clean() {
                continue;
            }
            for d in &audit.discrepancies {
                if let Some(b) = self
                    .branches
                    .items_mut()
                    .iter_mut()
                    .find(|b| b.name == d.branch)
                {
                    match &d.fix {
                        CacheFix::Status { status, .. } => {
                            b.merge_status = *status;
                            self.verified_branches.insert(d.branch.clone());
                        }
                        CacheFix::AheadBehind { ahead, behind, .. } => {
                            b.ahead = Some(*ahead);
                            b.behind = Some(*behind);
                        }
                        CacheFix::MergeBase { merge_base, .. } => {
                            b.merge_base_commit = merge_base.clone();
                        }
                    }
                }
            }
            list_state::apply_sort(&mut self.branches, &self.branch_columns);
            self.refresh_worktree_merge_status();
        }

        // Progress updates
        for update in drain_channel(&mut self.progress_rx, 32, &mut dirty) {
            self.progress = Some(update.clone());
            if let Some(Overlay::Executing { progress, .. }) = &mut self.overlay {
                *progress = Some(update);
            }
        }

        // Remote fetch is a one-shot receiver: consume it explicitly as soon
        // as its completion arrives so a later Remotes request can start.
        let fetch_completion = match self.remote_fetch_rx.as_ref().map(Receiver::try_recv) {
            Some(Ok(completion)) => { self.remote_fetch_rx = None; Some(completion) }
            Some(Err(std::sync::mpsc::TryRecvError::Disconnected)) => { self.remote_fetch_rx = None; None }
            _ => None,
        };
        if let Some(completion) = fetch_completion {
            dirty = true;
            if completion.success {
                self.remote_fetched = true;
                // Reload remote branches if we're on that view
                if self.active_view == ViewId::Remotes {
                    self.spawn_remote_load();
                }
            }
            self.request_graph_update(completion.graph_delta);
            self.clear_toast();
        }

        // Expire toast
        if let Some(ref toast) = self.toast {
            if toast.is_expired() {
                self.toast = None;
                dirty = true;
            }
        }

        if self.graph_update_rx.is_none()
            && !self.pending_graph_deltas.is_empty()
            && (!self.graph.is_loading() || self.reconciling_initial_graph)
        {
            let next = self.pending_graph_deltas.remove(0);
            self.request_graph_update(next);
            dirty = true;
        }

        if dirty {
            self.refresh_open_graph_menu();
        }

        dirty
    }
}
