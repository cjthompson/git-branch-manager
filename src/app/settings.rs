use super::*;

impl App {
    // ---- Diagnostics ----

    /// Run the cache-accuracy audit on a background thread, recomputing every
    /// cached value fresh and diffing it against the on-disk cache. Progress is
    /// streamed to the Executing overlay; the result lands in `diag_rx`.
    pub(super) fn run_cache_audit(&mut self) {
        let repo_path = self.repo_path.clone();
        let base_branch = self.base_branch.clone();
        let cache_root = self.cache_root.clone();

        let (diag_tx, diag_rx) = mpsc::channel();
        let (prog_tx, prog_rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_clone = Arc::clone(&cancel);

        self.overlay = Some(Overlay::Executing {
            label: "Verifying cache accuracy...".into(),
            progress: None,
        });
        self.diag_rx = Some(diag_rx);
        self.progress_rx = Some(prog_rx);
        self.cancel_flag = Some(cancel);

        std::thread::spawn(move || {
            let Ok(repo) = git2::Repository::open(&repo_path) else {
                return;
            };
            let branch_cache =
                cache::BranchCache::load_for_base(&repo_path, &base_branch, &cache_root);
            let audit = diagnostics::audit_cache(
                &repo,
                &repo_path,
                &base_branch,
                &branch_cache,
                &cancel_clone,
                |completed, total, item| {
                    let _ = prog_tx.send(ProgressUpdate {
                        completed,
                        total,
                        current_item: item.to_string(),
                    });
                },
            );
            let _ = diag_tx.send(audit);
        });
    }

    /// Write the audit's freshly-computed corrections back to the cache and
    /// reload the view so the screen reflects the corrected data.
    pub(super) fn apply_cache_fix(&mut self, audit: CacheAudit) {
        let mut branch_cache =
            cache::BranchCache::load_for_base(&self.repo_path, &self.base_branch, &self.cache_root);
        diagnostics::apply_fix(&mut branch_cache, &audit);
        self.toast = Some(Toast::new("Cache corrected".into(), 3));
        self.refresh_after_operation();
    }

    // ---- Config ----
    //
    // Every save below goes through `Config::update`, which reloads the
    // on-disk file, applies only the field(s) this action changed, and
    // writes the result back. config.toml is a single global path shared by
    // every running instance (not per-repo), and users can also hand-edit it
    // while the app is running — so a save must never write back a full
    // in-memory snapshot that may be stale outside the field(s) actually
    // being changed. See `Config::update`'s doc comment.

    pub(super) fn save_theme(&mut self) {
        let theme = self.theme.name.to_string();
        self.config = Config::update_at(&self.config_root, |c| c.theme = Some(theme.clone()));
    }

    pub(super) fn save_symbols(&mut self) {
        let symbols = self.symbols.name.to_string();
        self.config = Config::update_at(&self.config_root, |c| c.symbols = Some(symbols.clone()));
    }

    pub(super) fn save_include_remotes(&mut self) {
        let include_remotes = self.graph.includes_remotes();
        self.config = Config::update_at(&self.config_root, |c| {
            c.include_remotes = Some(include_remotes)
        });
    }

    pub(super) fn save_auto_fetch(&mut self) {
        let auto_fetch = self.config.auto_fetch;
        self.config = Config::update_at(&self.config_root, |c| c.auto_fetch = auto_fetch);
    }

    pub(super) fn save_load_worktrees_on_launch(&mut self) {
        let load_worktrees_on_launch = self.config.load_worktrees_on_launch;
        self.config = Config::update_at(&self.config_root, |c| {
            c.load_worktrees_on_launch = load_worktrees_on_launch
        });
    }

    pub(super) fn toggle_horizontal_scrolling(&mut self) {
        self.config.horizontal_scrolling = Some(self.config.horizontal_scrolling != Some(true));
        self.branches.reset_horizontal_scroll();
        self.remotes.reset_horizontal_scroll();
        self.tags.reset_horizontal_scroll();
        self.worktrees.reset_horizontal_scroll();
    }

    pub(super) fn save_horizontal_scrolling(&mut self) {
        let horizontal_scrolling = self.config.horizontal_scrolling;
        self.config = Config::update(|c| c.horizontal_scrolling = horizontal_scrolling);
    }

    /// Persist only the given view's sort column/direction.
    pub(super) fn save_sort_field(&mut self, view: ViewId) {
        match view {
            ViewId::Graph => {}
            ViewId::Branches => {
                let col = self
                    .branches
                    .sort_column()
                    .and_then(|i| sort_keys::key_for_index(&self.branch_columns, i))
                    .map(str::to_string);
                let asc = self.branches.sort_ascending();
                self.config = Config::update_at(&self.config_root, |c| {
                    c.sort_column_branches = col.clone();
                    c.sort_asc_branches = Some(asc);
                });
            }
            ViewId::Remotes => {
                let col = self
                    .remotes
                    .sort_column()
                    .and_then(|i| sort_keys::key_for_index(&self.remote_columns, i))
                    .map(str::to_string);
                let asc = self.remotes.sort_ascending();
                self.config = Config::update_at(&self.config_root, |c| {
                    c.sort_column_remotes = col.clone();
                    c.sort_asc_remotes = Some(asc);
                });
            }
            ViewId::Tags => {
                let col = self
                    .tags
                    .sort_column()
                    .and_then(|i| sort_keys::key_for_index(&self.tag_columns, i))
                    .map(str::to_string);
                let asc = self.tags.sort_ascending();
                self.config = Config::update_at(&self.config_root, |c| {
                    c.sort_column_tags = col.clone();
                    c.sort_asc_tags = Some(asc);
                });
            }
            ViewId::Worktrees => {
                let col = self
                    .worktrees
                    .sort_column()
                    .and_then(|i| sort_keys::key_for_index(&self.worktree_columns, i))
                    .map(str::to_string);
                let asc = self.worktrees.sort_ascending();
                self.config = Config::update_at(&self.config_root, |c| {
                    c.sort_column_worktrees = col.clone();
                    c.sort_asc_worktrees = Some(asc);
                });
            }
        }
    }

    /// Persist only the settings-overlay field the given cursor row controls.
    /// Row order matches the overlay: 0=symbols, 1=theme, 2-5=per-view sort,
    /// 6=auto_fetch, 7=load_worktrees_on_launch, 8=horizontal_scrolling.
    pub(super) fn save_settings_field(&mut self, cursor: usize) {
        match cursor {
            0 => self.save_symbols(),
            1 => self.save_theme(),
            2 => self.save_sort_field(ViewId::Branches),
            3 => self.save_sort_field(ViewId::Remotes),
            4 => self.save_sort_field(ViewId::Tags),
            5 => self.save_sort_field(ViewId::Worktrees),
            6 => self.save_auto_fetch(),
            7 => self.save_load_worktrees_on_launch(),
            8 => self.save_horizontal_scrolling(),
            _ => {}
        }
    }

    /// Cycle the given view's sort through its cycle of states. `forward` selects
    /// direction of the cycle (Right/Left in settings).
    pub(super) fn cycle_view_sort(&mut self, view: ViewId, forward: bool) {
        match view {
            ViewId::Graph => {}
            ViewId::Branches => {
                let pairs = sort_keys::sort_state_cycle(&self.branch_columns);
                let current = (self.branches.sort_column(), self.branches.sort_ascending());
                let cur_pos = pairs.iter().position(|&p| p == current).unwrap_or(0);
                let len = pairs.len();
                let next_pos = if forward {
                    (cur_pos + 1) % len
                } else {
                    (cur_pos + len - 1) % len
                };
                let (col, asc) = pairs[next_pos];
                self.branches.set_sort(col, asc);
                list_state::apply_sort(&mut self.branches, &self.branch_columns);
            }
            ViewId::Remotes => {
                let pairs = sort_keys::sort_state_cycle(&self.remote_columns);
                let current = (self.remotes.sort_column(), self.remotes.sort_ascending());
                let cur_pos = pairs.iter().position(|&p| p == current).unwrap_or(0);
                let len = pairs.len();
                let next_pos = if forward {
                    (cur_pos + 1) % len
                } else {
                    (cur_pos + len - 1) % len
                };
                let (col, asc) = pairs[next_pos];
                self.remotes.set_sort(col, asc);
                list_state::apply_sort(&mut self.remotes, &self.remote_columns);
            }
            ViewId::Tags => {
                let pairs = sort_keys::sort_state_cycle(&self.tag_columns);
                let current = (self.tags.sort_column(), self.tags.sort_ascending());
                let cur_pos = pairs.iter().position(|&p| p == current).unwrap_or(0);
                let len = pairs.len();
                let next_pos = if forward {
                    (cur_pos + 1) % len
                } else {
                    (cur_pos + len - 1) % len
                };
                let (col, asc) = pairs[next_pos];
                self.tags.set_sort(col, asc);
                list_state::apply_sort(&mut self.tags, &self.tag_columns);
            }
            ViewId::Worktrees => {
                let pairs = sort_keys::sort_state_cycle(&self.worktree_columns);
                let current = (
                    self.worktrees.sort_column(),
                    self.worktrees.sort_ascending(),
                );
                let cur_pos = pairs.iter().position(|&p| p == current).unwrap_or(0);
                let len = pairs.len();
                let next_pos = if forward {
                    (cur_pos + 1) % len
                } else {
                    (cur_pos + len - 1) % len
                };
                let (col, asc) = pairs[next_pos];
                self.worktrees.set_sort(col, asc);
                list_state::apply_sort(&mut self.worktrees, &self.worktree_columns);
            }
        }
    }

    // ---- Filter helpers ----

    pub(super) fn active_filter_query(&self) -> String {
        match self.active_view {
            ViewId::Graph => String::new(),
            ViewId::Branches => self.branches.filter_query().to_string(),
            ViewId::Remotes => self.remotes.filter_query().to_string(),
            ViewId::Tags => self.tags.filter_query().to_string(),
            ViewId::Worktrees => self.worktrees.filter_query().to_string(),
        }
    }

    pub(super) fn set_active_filter(&mut self, query: String) {
        match self.active_view {
            ViewId::Graph => {}
            ViewId::Branches => self.branches.set_filter_query(query),
            ViewId::Remotes => self.remotes.set_filter_query(query),
            ViewId::Tags => self.tags.set_filter_query(query),
            ViewId::Worktrees => self.worktrees.set_filter_query(query),
        }
    }

    pub(super) fn clear_toast(&mut self) {
        self.toast = None;
    }
}
