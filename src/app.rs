use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, MouseButton, MouseEventKind};
use ratatui::DefaultTerminal;
use ratatui::widgets::TableState;

use git_branch_manager::git::{branch, cache, operations, squash_loader, status, tags};
use git_branch_manager::git::tags::TagInfo;
use git_branch_manager::types::{BranchAction, BranchInfo, MergeStatus, OperationResult, SquashResult, TrackingStatus, WorkingTreeStatus};
use crate::ui;
use crate::ui::symbols::SymbolSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum View {
    BranchList,
    Confirm { action: BranchAction },
    Results,
    Help,
    Menu { cursor: usize },
    Tags,
}

pub struct App {
    pub base_branch: String,
    pub repo_path: PathBuf,
    pub branches: Vec<BranchInfo>,
    pub view: View,
    pub cursor: usize,
    pub selected: Vec<bool>,
    pub list_scroll_offset: usize,
    pub results: Vec<OperationResult>,
    pub should_exit: bool,
    pub squash_rx: Option<Receiver<SquashResult>>,
    pub squash_checked: usize,
    pub squash_total: usize,
    pub working_tree_status: WorkingTreeStatus,
    pub table_state: TableState,
    pub symbols: &'static SymbolSet,
    pub trim_strategy: String,
    pub sort_column: Option<usize>,  // 0=name, 1=age, 2=ahead, 3=status
    pub sort_ascending: bool,
    pub search_query: String,
    pub search_active: bool,
    pub tags: Vec<TagInfo>,
    pub tag_cursor: usize,
    pub tag_table_state: TableState,
    /// Which view to return to after the Results screen (BranchList or Tags).
    pub results_return_view: ResultsReturnView,
    /// Column header x-ranges for mouse click sorting: (x_start, sort_column_index).
    /// Populated during branch_list rendering. The last entry extends to the end of the row.
    pub header_columns: Vec<(u16, usize)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResultsReturnView {
    BranchList,
    Tags,
}

impl App {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        base_branch: String,
        repo_path: PathBuf,
        mut branches: Vec<BranchInfo>,
        squash_rx: Option<Receiver<SquashResult>>,
        squash_total: usize,
        working_tree_status: WorkingTreeStatus,
        symbols: &'static SymbolSet,
        trim_strategy: String,
    ) -> Self {
        // Sort: base first, then current, then the rest by date descending
        branches.sort_by(|a, b| {
            let pin_a = if a.is_base { 0 } else if a.is_current { 1 } else { 2 };
            let pin_b = if b.is_base { 0 } else if b.is_current { 1 } else { 2 };
            pin_a.cmp(&pin_b).then(b.last_commit_date.cmp(&a.last_commit_date))
        });

        let len = branches.len();
        let first_unpinned = branches.iter().position(|b| !b.is_pinned()).unwrap_or(0);

        Self {
            base_branch,
            repo_path,
            branches,
            view: View::BranchList,
            cursor: first_unpinned,
            selected: vec![false; len],
            list_scroll_offset: 0,
            results: Vec::new(),
            should_exit: false,
            squash_rx,
            squash_checked: 0,
            squash_total,
            working_tree_status,
            table_state: TableState::default().with_selected(Some(first_unpinned)),
            symbols,
            trim_strategy,
            sort_column: None,
            sort_ascending: true,
            search_query: String::new(),
            search_active: false,
            tags: Vec::new(),
            tag_cursor: 0,
            tag_table_state: TableState::default(),
            results_return_view: ResultsReturnView::BranchList,
            header_columns: Vec::new(),
        }
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        crossterm::execute!(std::io::stdout(), crossterm::event::EnableMouseCapture)?;
        while !self.should_exit {
            self.drain_squash_rx();
            terminal.draw(|frame| ui::render::draw(frame, self))?;

            if event::poll(Duration::from_millis(250))? {
                let ev = event::read()?;
                self.handle_event(ev);
            }
        }
        crossterm::execute!(std::io::stdout(), crossterm::event::DisableMouseCapture)?;
        Ok(())
    }

    fn handle_event(&mut self, event: Event) {
        match event {
            Event::Key(key) => {
                if key.kind != KeyEventKind::Press {
                    return;
                }

                // Search input takes priority over all other key handlers
                if self.search_active {
                    self.handle_search_key(key.code);
                    return;
                }

                match &self.view {
                    View::BranchList => self.handle_branch_list_key(key.code),
                    View::Confirm { .. } => self.handle_confirm_key(key.code),
                    View::Results => self.handle_results_key(key.code),
                    View::Help => self.handle_help_key(key.code),
                    View::Menu { .. } => self.handle_menu_key(key.code),
                    View::Tags => self.handle_tags_key(key.code),
                }
            }
            Event::Mouse(mouse) => {
                if let MouseEventKind::Down(MouseButton::Left) = mouse.kind {
                    self.handle_mouse_click(mouse.column, mouse.row);
                }
            }
            _ => {}
        }
    }

    fn handle_mouse_click(&mut self, x: u16, y: u16) {
        // Only handle clicks on the header row (row 1, inside the top border at row 0)
        // and only in BranchList view
        if y != 1 || self.view != View::BranchList || self.header_columns.is_empty() {
            return;
        }

        // Determine which sort column was clicked based on stored header x positions
        let mut clicked_col: Option<usize> = None;
        for (i, &(col_x, sort_idx)) in self.header_columns.iter().enumerate() {
            let next_x = if i + 1 < self.header_columns.len() {
                self.header_columns[i + 1].0
            } else {
                u16::MAX
            };
            if x >= col_x && x < next_x {
                clicked_col = Some(sort_idx);
                break;
            }
        }

        if let Some(col) = clicked_col {
            if self.sort_column == Some(col) {
                self.sort_ascending = !self.sort_ascending;
            } else {
                self.sort_column = Some(col);
                self.sort_ascending = true;
            }
            self.apply_sort();
        }
    }

    fn handle_branch_list_key(&mut self, code: KeyCode) {
        let len = self.branches.len();
        if len == 0 {
            if matches!(code, KeyCode::Char('q')) {
                self.should_exit = true;
            }
            return;
        }

        match code {
            KeyCode::Char('j') | KeyCode::Down => {
                let query = self.search_query.to_lowercase();
                let mut next = self.cursor + 1;
                while next < len && (self.branches[next].is_pinned() || !branch_matches_query(&self.branches[next], &query)) {
                    next += 1;
                }
                if next < len {
                    self.cursor = next;
                    self.table_state.select(Some(self.cursor));
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                let query = self.search_query.to_lowercase();
                if self.cursor > 0 {
                    let mut prev = self.cursor - 1;
                    while prev > 0 && (self.branches[prev].is_pinned() || !branch_matches_query(&self.branches[prev], &query)) {
                        prev -= 1;
                    }
                    if !self.branches[prev].is_pinned() && branch_matches_query(&self.branches[prev], &query) {
                        self.cursor = prev;
                        self.table_state.select(Some(self.cursor));
                    }
                }
            }
            KeyCode::Char(' ') => {
                let branch = &self.branches[self.cursor];
                if !branch.is_base && !branch.is_current {
                    self.selected[self.cursor] = !self.selected[self.cursor];
                }
            }
            KeyCode::Char('a') => {
                for (i, branch) in self.branches.iter().enumerate() {
                    self.selected[i] = !branch.is_base && !branch.is_current;
                }
            }
            KeyCode::Char('n') => {
                self.selected.fill(false);
            }
            KeyCode::Char('m') => {
                for (i, branch) in self.branches.iter().enumerate() {
                    self.selected[i] = !branch.is_base
                        && !branch.is_current
                        && matches!(
                            branch.merge_status,
                            MergeStatus::Merged | MergeStatus::SquashMerged
                        );
                }
            }
            KeyCode::Char('i') => {
                for (i, branch) in self.branches.iter().enumerate() {
                    if !branch.is_base && !branch.is_current {
                        self.selected[i] = !self.selected[i];
                    }
                }
            }
            KeyCode::Char('d') => {
                if self.has_selection() {
                    self.view = View::Confirm {
                        action: BranchAction::DeleteLocal,
                    };
                }
            }
            KeyCode::Char('D') => {
                if self.has_selection() {
                    self.view = View::Confirm {
                        action: BranchAction::DeleteLocalAndRemote,
                    };
                }
            }
            KeyCode::Char('R') => {
                let mut branch_cache = cache::BranchCache::load(&self.repo_path);
                branch_cache.clear();
                self.refresh_branches();
            }
            KeyCode::Char('x') => {
                let branch = &self.branches[self.cursor];
                if !branch.is_base && !branch.is_current {
                    self.view = View::Confirm {
                        action: BranchAction::DeleteLocal,
                    };
                }
            }
            KeyCode::Char('c') => {
                let branch = &self.branches[self.cursor];
                if !branch.is_current && !branch.is_base {
                    self.view = View::Confirm {
                        action: BranchAction::Checkout,
                    };
                }
            }
            KeyCode::Char('f') => {
                let result = operations::fetch(&self.repo_path);
                self.results.push(result);
                self.refresh_branches();
                self.view = View::Results;
            }
            KeyCode::Char('F') => {
                let result = operations::fetch_prune(&self.repo_path);
                self.results.push(result);
                self.refresh_branches();
                self.view = View::Results;
            }
            KeyCode::Enter => {
                if !self.branches[self.cursor].is_pinned() {
                    self.view = View::Menu { cursor: 0 };
                }
            }
            KeyCode::Char('s') => {
                self.sort_column = Some(match self.sort_column {
                    Some(c) => (c + 1) % 4,
                    None => 0,
                });
                self.sort_ascending = true;
                self.apply_sort();
            }
            KeyCode::Char('S') => {
                self.sort_ascending = !self.sort_ascending;
                self.apply_sort();
            }
            KeyCode::Char('t') => {
                let Ok(repo) = git2::Repository::open(&self.repo_path) else {
                    return;
                };
                self.tags = tags::list_tags(&repo);
                self.tag_cursor = 0;
                self.tag_table_state = TableState::default().with_selected(
                    if self.tags.is_empty() { None } else { Some(0) }
                );
                self.view = View::Tags;
            }
            KeyCode::Char('/') => {
                self.search_active = true;
            }
            KeyCode::PageDown => {
                let page_size = 20;
                let query = self.search_query.to_lowercase();
                let mut remaining = page_size;
                let mut next = self.cursor;
                while remaining > 0 && next + 1 < len {
                    let candidate = next + 1;
                    if !self.branches[candidate].is_pinned() && branch_matches_query(&self.branches[candidate], &query) {
                        next = candidate;
                        remaining -= 1;
                    } else {
                        next = candidate;
                    }
                }
                // Ensure we landed on a valid row
                if !self.branches[next].is_pinned() && branch_matches_query(&self.branches[next], &query) {
                    self.cursor = next;
                    self.table_state.select(Some(self.cursor));
                }
            }
            KeyCode::PageUp => {
                let page_size = 20;
                let query = self.search_query.to_lowercase();
                let first_unpinned = self.branches.iter().position(|b| !b.is_pinned()).unwrap_or(0);
                let mut remaining = page_size;
                let mut prev = self.cursor;
                while remaining > 0 && prev > first_unpinned {
                    let candidate = prev - 1;
                    if !self.branches[candidate].is_pinned() && branch_matches_query(&self.branches[candidate], &query) {
                        prev = candidate;
                        remaining -= 1;
                    } else if candidate > first_unpinned {
                        prev = candidate;
                    } else {
                        break;
                    }
                }
                if !self.branches[prev].is_pinned() && branch_matches_query(&self.branches[prev], &query) {
                    self.cursor = prev;
                    self.table_state.select(Some(self.cursor));
                }
            }
            KeyCode::Char('?') => {
                self.view = View::Help;
            }
            KeyCode::Char('q') => {
                self.should_exit = true;
            }
            _ => {}
        }
    }

    fn handle_confirm_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('y') => {
                let is_tag_action = matches!(
                    &self.view,
                    View::Confirm { action } if matches!(action, BranchAction::DeleteTag | BranchAction::PushTag)
                );
                if is_tag_action {
                    self.results_return_view = ResultsReturnView::Tags;
                }
                self.execute_action();
                self.view = View::Results;
            }
            KeyCode::Char('n') | KeyCode::Esc => {
                // Return to the appropriate view
                let is_tag_action = matches!(
                    &self.view,
                    View::Confirm { action } if matches!(action, BranchAction::DeleteTag | BranchAction::PushTag)
                );
                if is_tag_action {
                    self.view = View::Tags;
                } else {
                    self.view = View::BranchList;
                }
            }
            _ => {}
        }
    }

    fn handle_results_key(&mut self, _code: KeyCode) {
        match self.results_return_view {
            ResultsReturnView::Tags => {
                // Refresh the tag list and return to Tags view
                let Ok(repo) = git2::Repository::open(&self.repo_path) else {
                    self.view = View::BranchList;
                    return;
                };
                self.tags = tags::list_tags(&repo);
                if self.tag_cursor >= self.tags.len() {
                    self.tag_cursor = self.tags.len().saturating_sub(1);
                }
                self.tag_table_state.select(
                    if self.tags.is_empty() { None } else { Some(self.tag_cursor) }
                );
                self.results.clear();
                self.results_return_view = ResultsReturnView::BranchList;
                self.view = View::Tags;
            }
            ResultsReturnView::BranchList => {
                self.refresh_branches();
                self.view = View::BranchList;
            }
        }
    }

    fn handle_help_key(&mut self, _code: KeyCode) {
        self.view = View::BranchList;
    }

    fn handle_tags_key(&mut self, code: KeyCode) {
        let len = self.tags.len();
        match code {
            KeyCode::Char('j') | KeyCode::Down => {
                if len > 0 && self.tag_cursor + 1 < len {
                    self.tag_cursor += 1;
                    self.tag_table_state.select(Some(self.tag_cursor));
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if self.tag_cursor > 0 {
                    self.tag_cursor -= 1;
                    self.tag_table_state.select(Some(self.tag_cursor));
                }
            }
            KeyCode::PageDown => {
                if len > 0 {
                    let page_size = 20;
                    self.tag_cursor = (self.tag_cursor + page_size).min(len - 1);
                    self.tag_table_state.select(Some(self.tag_cursor));
                }
            }
            KeyCode::PageUp => {
                if len > 0 {
                    let page_size = 20;
                    self.tag_cursor = self.tag_cursor.saturating_sub(page_size);
                    self.tag_table_state.select(Some(self.tag_cursor));
                }
            }
            KeyCode::Char('d') => {
                if len > 0 {
                    self.view = View::Confirm {
                        action: BranchAction::DeleteTag,
                    };
                }
            }
            KeyCode::Char('p') => {
                if len > 0 {
                    let tag_name = self.tags[self.tag_cursor].name.clone();
                    let result = tags::push_tag(&self.repo_path, &tag_name);
                    self.results.push(result);
                    self.results_return_view = ResultsReturnView::Tags;
                    self.view = View::Results;
                }
            }
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('t') => {
                self.view = View::BranchList;
            }
            KeyCode::Char('?') => {
                self.view = View::Help;
            }
            _ => {}
        }
    }

    fn execute_action(&mut self) {
        let action = match &self.view {
            View::Confirm { action } => action.clone(),
            _ => return,
        };

        // Tag operations operate on the tag cursor, not branches
        if action == BranchAction::DeleteTag {
            if !self.tags.is_empty() {
                let tag_name = self.tags[self.tag_cursor].name.clone();
                let repo = match git2::Repository::open(&self.repo_path) {
                    Ok(r) => r,
                    Err(e) => {
                        self.results.push(OperationResult {
                            branch_name: tag_name,
                            action: BranchAction::DeleteTag,
                            success: false,
                            message: format!("Failed to open repo: {}", e),
                        });
                        return;
                    }
                };
                let result = tags::delete_tag(&repo, &tag_name);
                self.results.push(result);
            }
            return;
        }

        if action == BranchAction::PushTag {
            if !self.tags.is_empty() {
                let tag_name = self.tags[self.tag_cursor].name.clone();
                let result = tags::push_tag(&self.repo_path, &tag_name);
                self.results.push(result);
            }
            return;
        }

        // Checkout operates on the cursor branch, not the selection
        if action == BranchAction::Checkout {
            let branch_name = self.branches[self.cursor].name.clone();
            let needs_stash = !self.working_tree_status.is_clean();
            let result =
                operations::checkout_branch(&self.repo_path, &branch_name, needs_stash);
            self.results.push(result);
            return;
        }

        // Fast-forward operates on the cursor branch
        if action == BranchAction::FastForward {
            let branch_name = self.branches[self.cursor].name.clone();
            let result = operations::fast_forward(&self.repo_path, &branch_name);
            self.results.push(result);
            return;
        }

        // Merge / squash merge operates on the cursor branch into base
        if action == BranchAction::Merge || action == BranchAction::SquashMerge {
            let branch_name = self.branches[self.cursor].name.clone();
            let needs_stash = !self.working_tree_status.is_clean();
            let squash = action == BranchAction::SquashMerge;
            let results = operations::merge_branch(
                &self.repo_path,
                &branch_name,
                &self.base_branch,
                squash,
                needs_stash,
            );
            self.results.extend(results);
            return;
        }

        // Rebase operates on the cursor branch onto base
        if action == BranchAction::Rebase {
            let branch_name = self.branches[self.cursor].name.clone();
            let needs_stash = !self.working_tree_status.is_clean();
            let results = operations::rebase_branch(
                &self.repo_path,
                &branch_name,
                &self.base_branch,
                needs_stash,
            );
            self.results.extend(results);
            return;
        }

        // Create worktree operates on the cursor branch
        if action == BranchAction::Worktree {
            let branch_name = self.branches[self.cursor].name.clone();
            let result = operations::create_worktree(&self.repo_path, &branch_name);
            self.results.push(result);
            return;
        }

        let repo = match git2::Repository::open(&self.repo_path) {
            Ok(r) => r,
            Err(e) => {
                self.results.push(OperationResult {
                    branch_name: String::new(),
                    action: action.clone(),
                    success: false,
                    message: format!("Failed to open repo: {}", e),
                });
                return;
            }
        };

        let target_branches: Vec<String> = {
            let selected: Vec<String> = self
                .branches
                .iter()
                .zip(self.selected.iter())
                .filter(|&(_, &sel)| sel)
                .map(|(b, _)| b.name.clone())
                .collect();
            if selected.is_empty() {
                vec![self.branches[self.cursor].name.clone()]
            } else {
                selected
            }
        };

        for branch_name in &target_branches {
            match action {
                BranchAction::DeleteLocal => {
                    let result = operations::delete_local(&repo, branch_name);
                    self.results.push(result);
                }
                BranchAction::DeleteLocalAndRemote => {
                    let results =
                        operations::delete_local_and_remote(&repo, &self.repo_path, branch_name);
                    self.results.extend(results);
                }
                BranchAction::Checkout
                | BranchAction::Fetch
                | BranchAction::FetchPrune
                | BranchAction::FastForward
                | BranchAction::Merge
                | BranchAction::SquashMerge
                | BranchAction::Rebase
                | BranchAction::Worktree
                | BranchAction::DeleteTag
                | BranchAction::PushTag => unreachable!(),
            }
        }
    }

    fn refresh_branches(&mut self) {
        let Ok(repo) = git2::Repository::open(&self.repo_path) else { return };
        let Ok(branches) = branch::list_branches_phase1(&repo, &self.base_branch) else {
            return;
        };

        self.working_tree_status = status::detect_working_tree_status(&repo);

        let branch_cache = cache::BranchCache::load(&self.repo_path);

        let candidates: Vec<(String, String)> = branches
            .iter()
            .filter(|b| b.merge_status == MergeStatus::Unmerged && !b.is_base && !b.is_current)
            .filter_map(|b| {
                branch::get_commit_hash(&repo, &b.name)
                    .map(|hash| (b.name.clone(), hash))
            })
            .collect();

        let mut branches = branches;
        branches.sort_by(|a, b| {
            let pin_a = if a.is_base { 0 } else if a.is_current { 1 } else { 2 };
            let pin_b = if b.is_base { 0 } else if b.is_current { 1 } else { 2 };
            pin_a.cmp(&pin_b).then(b.last_commit_date.cmp(&a.last_commit_date))
        });

        let len = branches.len();
        let first_unpinned = branches.iter().position(|b| !b.is_pinned()).unwrap_or(0);

        self.branches = branches;
        self.selected = vec![false; len];
        self.cursor = first_unpinned;
        self.list_scroll_offset = 0;
        self.table_state.select(Some(self.cursor));
        self.results.clear();
        self.search_query.clear();
        self.search_active = false;

        self.squash_checked = 0;
        self.squash_total = candidates.len();

        self.squash_rx = if candidates.is_empty() {
            None
        } else {
            Some(squash_loader::spawn_squash_checker(
                self.repo_path.clone(),
                self.base_branch.clone(),
                candidates,
                branch_cache,
            ))
        };

        // Re-apply current sort so it persists across refreshes
        self.apply_sort();
    }

    fn apply_sort(&mut self) {
        let Some(col) = self.sort_column else { return };
        let asc = self.sort_ascending;

        // Find where pinned rows end — pinned rows are never sorted
        let pin_count = self.branches.iter().take_while(|b| b.is_pinned()).count();
        let sortable = &mut self.branches[pin_count..];

        sortable.sort_by(|a, b| {
            let ord = match col {
                0 => a.name.cmp(&b.name),
                1 => a.last_commit_date.cmp(&b.last_commit_date),
                2 => a.ahead.unwrap_or(0).cmp(&b.ahead.unwrap_or(0)),
                3 => {
                    let rank = |s: &MergeStatus| match s {
                        MergeStatus::Merged => 0,
                        MergeStatus::SquashMerged => 1,
                        MergeStatus::Unmerged => 2,
                    };
                    rank(&a.merge_status).cmp(&rank(&b.merge_status))
                }
                _ => std::cmp::Ordering::Equal,
            };
            if asc { ord } else { ord.reverse() }
        });

        // Reset selection and cursor after sort
        self.selected = vec![false; self.branches.len()];
        let first_unpinned = self.branches.iter().position(|b| !b.is_pinned()).unwrap_or(0);
        self.cursor = first_unpinned;
        self.table_state.select(Some(self.cursor));
    }

    fn drain_squash_rx(&mut self) {
        use std::sync::mpsc::TryRecvError;

        let Some(rx) = &self.squash_rx else { return };

        let done = loop {
            match rx.try_recv() {
                Ok(result) => {
                    self.squash_checked += 1;
                    if result.is_squash_merged
                        && let Some(branch) = self
                            .branches
                            .iter_mut()
                            .find(|b| b.name == result.branch_name)
                    {
                        branch.merge_status = MergeStatus::SquashMerged;
                    }
                }
                Err(TryRecvError::Empty) => break false,
                Err(TryRecvError::Disconnected) => break true,
            }
        };

        if done {
            self.squash_rx = None;
        }
    }

    pub fn build_menu_items(&self) -> Vec<ui::menu::MenuItem> {
        let branch = &self.branches[self.cursor];

        let mut items = Vec::new();

        // Checkout
        items.push(ui::menu::MenuItem {
            label: "Checkout".into(),
            enabled: !branch.is_current,
            reason: if branch.is_current {
                Some("current".into())
            } else {
                None
            },
        });

        // Delete local
        items.push(ui::menu::MenuItem {
            label: "Delete local".into(),
            enabled: !branch.is_base && !branch.is_current,
            reason: if branch.is_current {
                Some("current".into())
            } else if branch.is_base {
                Some("base".into())
            } else {
                None
            },
        });

        // Delete local + remote
        items.push(ui::menu::MenuItem {
            label: "Delete local + remote".into(),
            enabled: !branch.is_base && !branch.is_current,
            reason: if branch.is_current {
                Some("current".into())
            } else if branch.is_base {
                Some("base".into())
            } else {
                None
            },
        });

        // Fast-forward: only for non-current branches with a tracked (non-gone) remote
        let has_live_remote = matches!(
            &branch.tracking,
            TrackingStatus::Tracked { gone: false, .. }
        );
        items.push(ui::menu::MenuItem {
            label: "Fast-forward".into(),
            enabled: !branch.is_current && has_live_remote,
            reason: if branch.is_current {
                Some("current".into())
            } else if !has_live_remote {
                Some("no remote".into())
            } else {
                None
            },
        });

        // Merge into base
        items.push(ui::menu::MenuItem {
            label: "Merge into base".into(),
            enabled: !branch.is_base && !branch.is_current,
            reason: if branch.is_current {
                Some("current".into())
            } else if branch.is_base {
                Some("base".into())
            } else {
                None
            },
        });

        // Squash merge into base
        items.push(ui::menu::MenuItem {
            label: "Squash merge into base".into(),
            enabled: !branch.is_base && !branch.is_current,
            reason: if branch.is_current {
                Some("current".into())
            } else if branch.is_base {
                Some("base".into())
            } else {
                None
            },
        });

        // Rebase onto base
        items.push(ui::menu::MenuItem {
            label: "Rebase onto base".into(),
            enabled: !branch.is_base && !branch.is_current,
            reason: if branch.is_current {
                Some("current".into())
            } else if branch.is_base {
                Some("base".into())
            } else {
                None
            },
        });

        // Create worktree
        items.push(ui::menu::MenuItem {
            label: "Create worktree".into(),
            enabled: !branch.is_current,
            reason: if branch.is_current {
                Some("current".into())
            } else {
                None
            },
        });

        items
    }

    fn handle_menu_key(&mut self, code: KeyCode) {
        let items = self.build_menu_items();
        match code {
            KeyCode::Char('j') | KeyCode::Down => {
                if let View::Menu { ref mut cursor } = self.view
                    && *cursor + 1 < items.len()
                {
                    *cursor += 1;
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if let View::Menu { ref mut cursor } = self.view
                    && *cursor > 0
                {
                    *cursor -= 1;
                }
            }
            KeyCode::Enter => {
                let menu_cursor = if let View::Menu { cursor } = &self.view {
                    *cursor
                } else {
                    return;
                };
                let item = &items[menu_cursor];
                if item.enabled {
                    let action = match menu_cursor {
                        0 => BranchAction::Checkout,
                        1 => BranchAction::DeleteLocal,
                        2 => BranchAction::DeleteLocalAndRemote,
                        3 => BranchAction::FastForward,
                        4 => BranchAction::Merge,
                        5 => BranchAction::SquashMerge,
                        6 => BranchAction::Rebase,
                        7 => BranchAction::Worktree,
                        _ => return,
                    };
                    self.view = View::Confirm { action };
                }
            }
            KeyCode::Esc | KeyCode::Char('q') => {
                self.view = View::BranchList;
            }
            _ => {}
        }
    }

    fn handle_search_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => {
                self.search_query.clear();
                self.search_active = false;
                self.reset_cursor_to_first_match();
            }
            KeyCode::Enter => {
                self.search_active = false;
            }
            KeyCode::Backspace => {
                self.search_query.pop();
                self.reset_cursor_to_first_match();
            }
            KeyCode::Char(c) => {
                self.search_query.push(c);
                self.reset_cursor_to_first_match();
            }
            _ => {}
        }
    }

    /// Returns true if the branch matches the current search query.
    /// Always matches if the query is empty or the branch is pinned.
    pub fn matches_search(&self, branch: &BranchInfo) -> bool {
        if self.search_query.is_empty() {
            return true;
        }
        if branch.is_pinned() {
            return true;
        }
        branch.name.to_lowercase().contains(&self.search_query.to_lowercase())
    }

    /// Reset cursor to the first non-pinned branch that matches the search filter.
    fn reset_cursor_to_first_match(&mut self) {
        let first_match = self
            .branches
            .iter()
            .enumerate()
            .find(|(_, b)| !b.is_pinned() && self.matches_search(b))
            .map(|(i, _)| i);

        if let Some(idx) = first_match {
            self.cursor = idx;
            self.table_state.select(Some(idx));
        } else {
            // No match: keep cursor at first unpinned (or 0)
            let first_unpinned = self.branches.iter().position(|b| !b.is_pinned()).unwrap_or(0);
            self.cursor = first_unpinned;
            self.table_state.select(Some(self.cursor));
        }
    }

    pub fn has_selection(&self) -> bool {
        self.selected.iter().any(|&s| s)
    }

    pub fn selection_count(&self) -> usize {
        self.selected.iter().filter(|&&s| s).count()
    }

    pub fn selected_branch_names(&self) -> Vec<&str> {
        self.branches
            .iter()
            .zip(self.selected.iter())
            .filter(|&(_, &sel)| sel)
            .map(|(b, _)| b.name.as_str())
            .collect()
    }

    /// Returns the branches that will be targeted by the current action.
    /// If branches are selected, returns those; otherwise returns the cursor branch.
    pub fn target_branch_names(&self) -> Vec<&str> {
        let selected = self.selected_branch_names();
        if selected.is_empty() {
            vec![self.branches[self.cursor].name.as_str()]
        } else {
            selected
        }
    }
}

/// Check if a branch name matches the lowercased search query.
/// Returns true if query is empty (no filter active).
fn branch_matches_query(branch: &BranchInfo, query_lower: &str) -> bool {
    if query_lower.is_empty() {
        return true;
    }
    branch.name.to_lowercase().contains(query_lower)
}
