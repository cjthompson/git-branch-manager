use super::*;

impl App {
    // ---- View-specific key handlers ----

    pub(super) fn handle_branches_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('d') => self.delete_selected_branches(false),
            KeyCode::Char('D') => self.delete_selected_branches(true),
            KeyCode::Char('p') => self.push_selected_branches(),
            KeyCode::Char('R') => self.clear_cache_and_refresh(),
            KeyCode::Char('f') => self.start_fetch(false),
            KeyCode::Char('F') => self.start_fetch(true),
            KeyCode::Char('r') => {
                self.active_view = ViewId::Remotes;
                self.ensure_view_loaded();
            }
            KeyCode::Char('t') => {
                self.active_view = ViewId::Tags;
                self.ensure_view_loaded();
            }
            KeyCode::Char('w') => {
                self.active_view = ViewId::Worktrees;
                self.ensure_view_loaded();
            }
            _ => {}
        }
    }

    pub(super) fn handle_graph_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => self.open_graph_commit_details(),
            KeyCode::Char('j') | KeyCode::Down => self.graph.move_down(),
            KeyCode::Char('k') | KeyCode::Up => self.graph.move_up(),
            KeyCode::PageDown => self.graph.page_down(),
            KeyCode::PageUp => self.graph.page_up(),
            KeyCode::Char('h') | KeyCode::Left => self.graph.scroll_left(),
            KeyCode::Char('l') | KeyCode::Right => self.graph.scroll_right(),
            KeyCode::Home | KeyCode::Char('g') => self.graph.home(),
            KeyCode::End | KeyCode::Char('G') => self.graph.end(),
            KeyCode::Char('L') => self.load_older_graph(),
            KeyCode::Char('o') => self.open_graph_options(),
            KeyCode::Char('r') => self.reload_graph(),
            _ => {}
        }
    }

    pub(super) fn jump_to_graph_from_active_list(&mut self) {
        let Some((target, include_remotes)) = (match self.active_view {
            ViewId::Branches => self
                .branches
                .cursor_item()
                .map(|branch| (branch.name.clone(), false)),
            ViewId::Remotes => self
                .remotes
                .cursor_item()
                .map(|branch| (branch.full_ref.clone(), true)),
            ViewId::Tags => self.tags.cursor_item().map(|tag| (tag.name.clone(), false)),
            ViewId::Worktrees => self.worktrees.cursor_item().map(|worktree| {
                (
                    worktree
                        .branch
                        .clone()
                        .unwrap_or_else(|| worktree.commit_hash.clone()),
                    false,
                )
            }),
            ViewId::Graph => None,
        }) else {
            return;
        };

        self.jump_to_graph(target, include_remotes);
    }

    pub(super) fn jump_to_graph(&mut self, target: String, include_remotes: bool) {
        let graph_includes_remotes = self.graph.includes_remotes() || include_remotes;
        self.active_view = ViewId::Graph;
        self.graph.focus_target(target);

        if (include_remotes && !self.graph.includes_remotes())
            || (self.graph.snapshot().is_none() && !self.graph.is_loading())
        {
            self.spawn_graph_load(self.graph.max_count(), graph_includes_remotes);
        }
    }

    pub(super) fn handle_remotes_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('d') => self.delete_selected_remote_branches(),
            KeyCode::Char('f') => self.start_fetch(false),
            KeyCode::Char('F') => self.start_fetch(true),
            KeyCode::Char('b') | KeyCode::Char('r') | KeyCode::Esc => {
                self.active_view = ViewId::Branches;
            }
            KeyCode::Char('t') => {
                self.active_view = ViewId::Tags;
                self.ensure_view_loaded();
            }
            KeyCode::Char('w') => {
                self.active_view = ViewId::Worktrees;
                self.ensure_view_loaded();
            }
            _ => {}
        }
    }

    pub(super) fn handle_tags_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('d') => self.delete_selected_tags(false),
            KeyCode::Char('D') => self.delete_selected_tags(true),
            KeyCode::Char('p') => self.push_selected_tags(),
            KeyCode::Char('f') => self.start_fetch(false),
            KeyCode::Char('F') => self.start_fetch(true),
            KeyCode::Char('b') | KeyCode::Char('t') | KeyCode::Esc => {
                self.active_view = ViewId::Branches;
            }
            KeyCode::Char('r') => {
                self.active_view = ViewId::Remotes;
                self.ensure_view_loaded();
            }
            KeyCode::Char('w') => {
                self.active_view = ViewId::Worktrees;
                self.ensure_view_loaded();
            }
            _ => {}
        }
    }

    pub(super) fn handle_worktrees_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('d') => self.remove_selected_worktrees(false),
            KeyCode::Char('D') => self.remove_selected_worktrees(true),
            KeyCode::Char('f') => self.start_fetch(false),
            KeyCode::Char('F') => self.start_fetch(true),
            KeyCode::Char('b') | KeyCode::Char('w') | KeyCode::Esc => {
                self.active_view = ViewId::Branches;
            }
            KeyCode::Char('r') => {
                self.active_view = ViewId::Remotes;
                self.ensure_view_loaded();
            }
            KeyCode::Char('t') => {
                self.active_view = ViewId::Tags;
                self.ensure_view_loaded();
            }
            _ => {}
        }
    }
}
