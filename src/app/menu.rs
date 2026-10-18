use super::*;

impl App {
    // ---- Context Menu Building ----

    pub(super) fn open_graph_commit_details(&mut self) {
        self.invalidate_commit_file_diff_request();
        self.invalidate_commit_details_request();
        self.return_view = self.active_view;
        let Some(commit) = self.graph.selected_commit().cloned() else {
            return;
        };
        self.commit_details_target = Some(commit.oid.clone());
        let loader = commit_details::spawn_commit_details_loader(
            self.repo_path.clone(),
            commit,
            self.base_branch.clone(),
        );
        let (tx, rx) = mpsc::channel();
        let oid = self.commit_details_target.clone().unwrap_or_default();
        std::thread::spawn(move || {
            if let Ok(result) = loader.recv() {
                let _ = tx.send((oid, result));
            }
        });
        self.commit_details_rx = Some(rx);
    }

    pub(super) fn commit_details_context_matches(&self, oid: &str) -> bool {
        self.active_view == ViewId::Graph
            && self.overlay.is_none()
            && self
                .graph
                .selected_commit()
                .is_some_and(|commit| commit.oid == oid)
    }

    pub(super) fn commit_details_context_is_current(&self) -> bool {
        self.commit_details_target
            .as_deref()
            .is_some_and(|oid| self.commit_details_context_matches(oid))
    }

    pub(super) fn invalidate_commit_details_request(&mut self) {
        self.commit_details_target = None;
        self.commit_details_rx = None;
    }

    pub(super) fn invalidate_commit_file_diff_request(&mut self) {
        self.commit_file_diff_target = None;
        self.commit_file_diff_rx = None;
    }

    pub(super) fn open_context_menu(&mut self) {
        let items = self.build_menu_items();
        let Some(row) = self.build_info_modal_row() else {
            return;
        };
        let first_enabled = first_enabled_index(&items);
        let focus = if first_enabled.is_none() {
            InfoModalFocus::Info
        } else {
            InfoModalFocus::Actions
        };
        self.return_view = self.active_view;
        // Clear any stale copy confirmation from a previous opening.
        self.info_copied_msg = None;
        self.info_modal_scroll_offset = 0;
        self.overlay = Some(Overlay::InfoModal {
            items,
            cursor: first_enabled.unwrap_or(0),
            info_cursor: 0,
            focus,
            row,
        });
    }

    pub(super) fn build_menu_items(&self) -> Vec<MenuItem> {
        match self.active_view {
            ViewId::Graph => self.build_graph_menu(),
            ViewId::Branches => self.build_branch_menu(),
            ViewId::Remotes => self.build_remote_menu(),
            ViewId::Tags => self.build_tag_menu(),
            ViewId::Worktrees => self.build_worktree_menu(),
        }
    }

    pub(super) fn build_info_modal_row(&self) -> Option<InfoModalRow> {
        match self.active_view {
            ViewId::Graph => self
                .graph
                .selected_commit()
                .cloned()
                .map(InfoModalRow::GraphCommit),
            ViewId::Branches => self
                .branches
                .cursor_item()
                .cloned()
                .map(InfoModalRow::Branch),
            ViewId::Remotes => self
                .remotes
                .cursor_item()
                .cloned()
                .map(InfoModalRow::Remote),
            ViewId::Tags => self.tags.cursor_item().cloned().map(InfoModalRow::Tag),
            ViewId::Worktrees => self
                .worktrees
                .cursor_item()
                .cloned()
                .map(InfoModalRow::Worktree),
        }
    }
}
