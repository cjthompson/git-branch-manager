use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::DefaultTerminal;
use ratatui::widgets::TableState;

use git_branch_manager::git::{branch, cache, operations, squash_loader, status};
use git_branch_manager::types::{BranchAction, BranchInfo, MergeStatus, OperationResult, SquashResult, TrackingStatus, WorkingTreeStatus};
use crate::ui;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum View {
    BranchList,
    Confirm { action: BranchAction },
    Results,
    Help,
    Menu { cursor: usize },
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
}

impl App {
    pub fn new(
        base_branch: String,
        repo_path: PathBuf,
        mut branches: Vec<BranchInfo>,
        squash_rx: Option<Receiver<SquashResult>>,
        squash_total: usize,
        working_tree_status: WorkingTreeStatus,
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
        }
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        while !self.should_exit {
            self.drain_squash_rx();
            terminal.draw(|frame| ui::render::draw(frame, self))?;

            if event::poll(Duration::from_millis(250))? {
                let ev = event::read()?;
                self.handle_event(ev);
            }
        }
        Ok(())
    }

    fn handle_event(&mut self, event: Event) {
        let Event::Key(key) = event else { return };
        if key.kind != KeyEventKind::Press {
            return;
        }

        match &self.view {
            View::BranchList => self.handle_branch_list_key(key.code),
            View::Confirm { .. } => self.handle_confirm_key(key.code),
            View::Results => self.handle_results_key(key.code),
            View::Help => self.handle_help_key(key.code),
            View::Menu { .. } => self.handle_menu_key(key.code),
        }
    }

    fn handle_branch_list_key(&mut self, code: KeyCode) {
        let len = self.branches.len();
        if len == 0 {
            if matches!(code, KeyCode::Char('q') | KeyCode::Esc) {
                self.should_exit = true;
            }
            return;
        }

        match code {
            KeyCode::Char('j') | KeyCode::Down => {
                let mut next = self.cursor + 1;
                while next < len && self.branches[next].is_pinned() {
                    next += 1;
                }
                if next < len {
                    self.cursor = next;
                    self.table_state.select(Some(self.cursor));
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if self.cursor > 0 {
                    let mut prev = self.cursor - 1;
                    while prev > 0 && self.branches[prev].is_pinned() {
                        prev -= 1;
                    }
                    if !self.branches[prev].is_pinned() {
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
            KeyCode::Char('?') => {
                self.view = View::Help;
            }
            KeyCode::Char('q') | KeyCode::Esc => {
                self.should_exit = true;
            }
            _ => {}
        }
    }

    fn handle_confirm_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('y') => {
                self.execute_action();
                self.view = View::Results;
            }
            KeyCode::Char('n') | KeyCode::Esc => {
                self.view = View::BranchList;
            }
            _ => {}
        }
    }

    fn handle_results_key(&mut self, _code: KeyCode) {
        self.refresh_branches();
        self.view = View::BranchList;
    }

    fn handle_help_key(&mut self, _code: KeyCode) {
        self.view = View::BranchList;
    }

    fn execute_action(&mut self) {
        let action = match &self.view {
            View::Confirm { action } => action.clone(),
            _ => return,
        };

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
                | BranchAction::SquashMerge => unreachable!(),
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
