use super::*;

impl App {
    // ---- Event Dispatch ----

    pub(super) fn handle_event(&mut self, event: Event) {
        match event {
            Event::Key(key) => {
                if key.kind != KeyEventKind::Press {
                    return;
                }
                self.handle_key(key);
            }
            Event::Mouse(mouse) => self.handle_mouse(mouse),
            _ => {}
        }
    }

    pub(super) fn handle_key(&mut self, key: KeyEvent) {
        self.handle_key_inner(key);
        if self.commit_details_target.is_some() && !self.commit_details_context_is_current() {
            self.invalidate_commit_details_request();
        }
    }

    pub(super) fn handle_key_inner(&mut self, key: KeyEvent) {
        // If search is active, route to search handler first
        if self.is_search_active() {
            self.handle_search_key(key);
            return;
        }

        // Route to overlay handler if overlay is active
        if self.overlay.is_some() {
            self.handle_overlay_key(key);
            return;
        }

        // Global keys (work in every view, take priority)
        match key.code {
            KeyCode::Char('q') => {
                self.should_exit = true;
                return;
            }
            KeyCode::Char('?') => {
                self.overlay = Some(Overlay::Help { scroll: 0 });
                return;
            }
            KeyCode::Char(',') => {
                self.overlay = Some(Overlay::Settings { cursor: 0 });
                return;
            }
            KeyCode::Char('T') => {
                self.theme = self.theme.next();
                self.save_theme();
                return;
            }
            KeyCode::Char('Y') => {
                self.symbols = self.symbols.next();
                self.reload_graph_for_symbol_change();
                self.save_symbols();
                return;
            }
            KeyCode::Tab => {
                if key
                    .modifiers
                    .contains(crossterm::event::KeyModifiers::SHIFT)
                {
                    self.active_view = self.active_view.prev();
                } else {
                    self.active_view = self.active_view.next();
                }
                self.ensure_view_loaded();
                return;
            }
            KeyCode::BackTab => {
                self.active_view = self.active_view.prev();
                self.ensure_view_loaded();
                return;
            }
            KeyCode::Char('/') => {
                self.toggle_search();
                return;
            }
            KeyCode::Char('\\') => {
                self.overlay = Some(Overlay::FilterSelection { cursor: 0 });
                return;
            }
            KeyCode::F(2) => {
                self.return_view = self.active_view;
                self.overlay = Some(Overlay::Diagnostics { cursor: 0 });
                return;
            }
            KeyCode::Char('x') => {
                let is_worktree_removal = matches!(
                    self.job_queue.current_action(),
                    Some(
                        BranchAction::WorktreeRemove
                            | BranchAction::WorktreeForceRemove
                            | BranchAction::WorktreeRemoveAndDeleteBranch
                            | BranchAction::WorktreeRemoveAndDeleteBranchRemote
                    )
                );
                if is_worktree_removal && self.job_queue.current_partial_delete_risk() {
                    let current_item = self
                        .job_queue
                        .render_data()
                        .current_progress
                        .map(|p| p.current_item.clone())
                        .unwrap_or_default();
                    self.overlay = Some(Overlay::ConfirmCancelJob {
                        message: format!(
                            "Cancelling now may leave the worktree partially deleted:\n{current_item}\n\nCancel anyway?"
                        ),
                    });
                } else {
                    self.job_queue.cancel_current();
                }
                return;
            }
            KeyCode::Char('X') => {
                self.job_queue.clear_queued();
                return;
            }
            _ => {}
        }

        if self.active_view == ViewId::Graph {
            self.handle_graph_key(key);
            return;
        }

        // Jump from any ref list to the matching commit in Graph. Graph keeps
        // its existing `g`/`G` Home/End bindings because this branch is only
        // reached for the four list views.
        if key.code == KeyCode::Char('g') {
            self.jump_to_graph_from_active_list();
            return;
        }

        // Common navigation/selection keys (work in every view)
        if self.handle_common_list_key(key) {
            return;
        }

        // View-specific keys
        match self.active_view {
            ViewId::Graph => {
                self.handle_graph_key(key);
            }
            ViewId::Branches => self.handle_branches_key(key),
            ViewId::Remotes => self.handle_remotes_key(key),
            ViewId::Tags => self.handle_tags_key(key),
            ViewId::Worktrees => self.handle_worktrees_key(key),
        }
    }

    /// Keys shared by all 4 views: navigation, selection, sorting.
    /// Returns true if the key was handled.
    pub(super) fn handle_common_list_key(&mut self, key: KeyEvent) -> bool {
        macro_rules! with_state {
            ($op:expr) => {
                match self.active_view {
                    ViewId::Branches => $op(&mut self.branches),
                    ViewId::Remotes => $op(&mut self.remotes),
                    ViewId::Tags => $op(&mut self.tags),
                    ViewId::Worktrees => $op(&mut self.worktrees),
                    ViewId::Graph => {}
                }
            };
        }

        if self.config.horizontal_scrolling == Some(true) {
            match key.code {
                KeyCode::Char('h') | KeyCode::Left => {
                    match self.active_view {
                        ViewId::Branches => self.branches.scroll_left(),
                        ViewId::Remotes => self.remotes.scroll_left(),
                        ViewId::Tags => self.tags.scroll_left(),
                        ViewId::Worktrees => self.worktrees.scroll_left(),
                        ViewId::Graph => {}
                    }
                    return true;
                }
                KeyCode::Char('l') | KeyCode::Right => {
                    match self.active_view {
                        ViewId::Branches => self.branches.scroll_right(),
                        ViewId::Remotes => self.remotes.scroll_right(),
                        ViewId::Tags => self.tags.scroll_right(),
                        ViewId::Worktrees => self.worktrees.scroll_right(),
                        ViewId::Graph => {}
                    }
                    return true;
                }
                _ => {}
            }
        }

        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                with_state!(list_state::nav_down);
                true
            }
            KeyCode::Char('k') | KeyCode::Up => {
                with_state!(list_state::nav_up);
                true
            }
            KeyCode::PageDown => {
                with_state!(|s| list_state::nav_page_down(s, 20));
                true
            }
            KeyCode::PageUp => {
                with_state!(|s| list_state::nav_page_up(s, 20));
                true
            }
            KeyCode::Home => {
                with_state!(list_state::nav_home);
                true
            }
            KeyCode::End => {
                with_state!(list_state::nav_end);
                true
            }
            KeyCode::Char(' ') => {
                with_state!(list_state::select_toggle);
                true
            }
            KeyCode::Char('a') => {
                with_state!(list_state::select_all);
                true
            }
            KeyCode::Char('n') => {
                with_state!(list_state::deselect_all);
                true
            }
            KeyCode::Char('i') => {
                with_state!(list_state::invert_selection);
                true
            }
            KeyCode::Char('m') => {
                with_state!(list_state::select_merged);
                true
            }
            KeyCode::Char('s') => {
                self.cycle_sort();
                true
            }
            KeyCode::Char('S') => {
                self.toggle_sort_direction();
                true
            }
            KeyCode::Enter => {
                self.open_context_menu();
                true
            }
            _ => false,
        }
    }
}
