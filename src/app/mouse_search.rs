use super::*;

impl App {
    // ---- Search ----

    pub(super) fn is_search_active(&self) -> bool {
        match self.active_view {
            ViewId::Graph => false,
            ViewId::Branches => self.branches.search_active(),
            ViewId::Remotes => self.remotes.search_active(),
            ViewId::Tags => self.tags.search_active(),
            ViewId::Worktrees => self.worktrees.search_active(),
        }
    }

    pub(super) fn toggle_search(&mut self) {
        match self.active_view {
            ViewId::Graph => {}
            ViewId::Branches => self.branches.set_search_active(true),
            ViewId::Remotes => self.remotes.set_search_active(true),
            ViewId::Tags => self.tags.set_search_active(true),
            ViewId::Worktrees => self.worktrees.set_search_active(true),
        }
    }

    pub(super) fn handle_search_key(&mut self, key: KeyEvent) {
        macro_rules! with_state {
            ($state:expr, $code:expr) => {
                match $code {
                    KeyCode::Esc => {
                        $state.set_search_query(String::new());
                        $state.set_search_active(false);
                    }
                    KeyCode::Enter => {
                        $state.set_search_active(false);
                    }
                    KeyCode::Backspace => {
                        let mut q = $state.search_query().to_string();
                        q.pop();
                        $state.set_search_query(q);
                    }
                    KeyCode::Char(c) => {
                        let mut q = $state.search_query().to_string();
                        q.push(c);
                        $state.set_search_query(q);
                    }
                    _ => {}
                }
            };
        }
        match self.active_view {
            ViewId::Graph => {}
            ViewId::Branches => with_state!(self.branches, key.code),
            ViewId::Remotes => with_state!(self.remotes, key.code),
            ViewId::Tags => with_state!(self.tags, key.code),
            ViewId::Worktrees => with_state!(self.worktrees, key.code),
        }
    }

    // ---- Mouse handling ----

    pub(super) fn handle_mouse(&mut self, mouse: crossterm::event::MouseEvent) {
        // The info modal handles left-clicks (click a value to copy it) and
        // mouse-wheel scrolling of its combined info+actions pane.
        if matches!(self.overlay, Some(Overlay::InfoModal { .. })) {
            match mouse.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    self.handle_info_modal_click(mouse.column, mouse.row);
                }
                MouseEventKind::ScrollDown => {
                    self.info_modal_scroll_offset = self.info_modal_scroll_offset.saturating_add(3);
                }
                MouseEventKind::ScrollUp => {
                    self.info_modal_scroll_offset = self.info_modal_scroll_offset.saturating_sub(3);
                }
                _ => {}
            }
            return;
        }

        // Don't handle mouse in other overlays
        if self.overlay.is_some() {
            return;
        }

        match mouse.kind {
            MouseEventKind::ScrollDown => {
                macro_rules! with_state {
                    ($op:expr) => {
                        match self.active_view {
                            ViewId::Graph => self.graph.move_down(),
                            ViewId::Branches => $op(&mut self.branches),
                            ViewId::Remotes => $op(&mut self.remotes),
                            ViewId::Tags => $op(&mut self.tags),
                            ViewId::Worktrees => $op(&mut self.worktrees),
                        }
                    };
                }
                with_state!(list_state::nav_down);
            }
            MouseEventKind::ScrollUp => {
                macro_rules! with_state {
                    ($op:expr) => {
                        match self.active_view {
                            ViewId::Graph => self.graph.move_up(),
                            ViewId::Branches => $op(&mut self.branches),
                            ViewId::Remotes => $op(&mut self.remotes),
                            ViewId::Tags => $op(&mut self.tags),
                            ViewId::Worktrees => $op(&mut self.worktrees),
                        }
                    };
                }
                with_state!(list_state::nav_up);
            }
            MouseEventKind::Down(MouseButton::Left) => {
                self.handle_left_click(mouse.column, mouse.row);
            }
            MouseEventKind::Down(MouseButton::Right) => {
                self.handle_right_click(mouse.column, mouse.row);
            }
            _ => {}
        }
    }

    /// A left-click inside the info modal: if it landed on a value, copy that
    /// value to the host clipboard and show a confirmation in the modal.
    pub(super) fn handle_info_modal_click(&mut self, x: u16, y: u16) {
        let hit = self
            .info_hit_regions
            .iter()
            .find(|r| {
                x >= r.rect.x
                    && x < r.rect.x + r.rect.width
                    && y >= r.rect.y
                    && y < r.rect.y + r.rect.height
            })
            .map(|r| (r.label.clone(), r.value.clone()));

        if let Some((label, value)) = hit {
            self.copy_info_value(label, value);
        }
    }

    pub(super) fn copy_info_value(&mut self, label: String, value: String) {
        self.info_copied_msg = Some(match copy_to_clipboard(&value) {
            Ok(()) => format!("{label} copied to clipboard"),
            Err(e) => format!("Clipboard error: {e}"),
        });
    }

    pub(super) fn handle_left_click(&mut self, x: u16, y: u16) {
        // Header row click (y == 1): sort by column
        if y == 1 {
            let clicked_col = self.find_header_click(x);
            if let Some(col) = clicked_col {
                match self.active_view {
                    ViewId::Graph => {}
                    ViewId::Branches => list_state::sort_by_column_click(
                        &mut self.branches,
                        &self.branch_columns,
                        col,
                    ),
                    ViewId::Remotes => list_state::sort_by_column_click(
                        &mut self.remotes,
                        &self.remote_columns,
                        col,
                    ),
                    ViewId::Tags => {
                        list_state::sort_by_column_click(&mut self.tags, &self.tag_columns, col)
                    }
                    ViewId::Worktrees => list_state::sort_by_column_click(
                        &mut self.worktrees,
                        &self.worktree_columns,
                        col,
                    ),
                }
            }
        } else if self.terminal_rows > 0 && y == self.terminal_rows - 1 {
            // Status bar click
            let items = match self.active_view {
                ViewId::Graph => Vec::new(),
                ViewId::Branches => self.branches.status_bar_items.clone(),
                ViewId::Remotes => self.remotes.status_bar_items.clone(),
                ViewId::Tags => self.tags.status_bar_items.clone(),
                ViewId::Worktrees => self.worktrees.status_bar_items.clone(),
            };
            for &(x_start, x_end, key) in &items {
                if x >= x_start && x < x_end {
                    self.handle_key(KeyEvent::new(key, crossterm::event::KeyModifiers::NONE));
                    break;
                }
            }
        } else if y >= 2 {
            // Click on a data row
            macro_rules! click_row {
                ($state:expr) => {{
                    let scroll_offset = $state.table_state().offset();
                    let clicked_display_row = (y - 2) as usize + scroll_offset;
                    if let Some(&raw_idx) = $state.display_indices().get(clicked_display_row) {
                        $state.set_cursor(raw_idx);
                        $state.table_state_mut().select(Some(clicked_display_row));
                    }
                }};
            }
            match self.active_view {
                ViewId::Graph => {}
                ViewId::Branches => click_row!(self.branches),
                ViewId::Remotes => click_row!(self.remotes),
                ViewId::Tags => click_row!(self.tags),
                ViewId::Worktrees => click_row!(self.worktrees),
            }
        }
    }

    pub(super) fn find_header_click(&self, x: u16) -> Option<usize> {
        let header_columns = match self.active_view {
            ViewId::Graph => &[] as &[(u16, u16, usize)],
            ViewId::Branches => &self.branches.header_columns,
            ViewId::Remotes => &self.remotes.header_columns,
            ViewId::Tags => &self.tags.header_columns,
            ViewId::Worktrees => &self.worktrees.header_columns,
        };
        if header_columns.is_empty() {
            return None;
        }
        header_columns
            .iter()
            .find(|&&(start, end, _)| x >= start && x < end)
            .map(|&(_, _, sort_idx)| sort_idx)
    }

    pub(super) fn handle_right_click(&mut self, _x: u16, y: u16) {
        if y < 2 {
            return;
        }

        macro_rules! move_cursor {
            ($state:expr) => {{
                let scroll_offset = $state.table_state().offset();
                let clicked_display_row = (y - 2) as usize + scroll_offset;
                if let Some(&raw_idx) = $state.display_indices().get(clicked_display_row) {
                    $state.set_cursor(raw_idx);
                    $state.table_state_mut().select(Some(clicked_display_row));
                    true
                } else {
                    false
                }
            }};
        }

        let moved = match self.active_view {
            ViewId::Graph => false,
            ViewId::Branches => move_cursor!(self.branches),
            ViewId::Remotes => move_cursor!(self.remotes),
            ViewId::Tags => move_cursor!(self.tags),
            ViewId::Worktrees => move_cursor!(self.worktrees),
        };

        if moved {
            self.open_context_menu();
        }
    }
}
