use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::DefaultTerminal;

use git_branch_manager::git::{branch, cache, operations, squash_loader, status};
use git_branch_manager::types::{BranchAction, BranchInfo, MergeStatus, OperationResult, SquashResult, WorkingTreeStatus};
use crate::ui;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum View {
    BranchList,
    Confirm { action: BranchAction },
    Results,
    Help,
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
    pub working_tree_status: WorkingTreeStatus,
}

impl App {
    pub fn new(
        base_branch: String,
        repo_path: PathBuf,
        branches: Vec<BranchInfo>,
        squash_rx: Option<Receiver<SquashResult>>,
        working_tree_status: WorkingTreeStatus,
    ) -> Self {
        let len = branches.len();
        Self {
            base_branch,
            repo_path,
            branches,
            view: View::BranchList,
            cursor: 0,
            selected: vec![false; len],
            list_scroll_offset: 0,
            results: Vec::new(),
            should_exit: false,
            squash_rx,
            working_tree_status,
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
                if self.cursor + 1 < len {
                    self.cursor += 1;
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if self.cursor > 0 {
                    self.cursor -= 1;
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

        let selected_branches: Vec<String> = self
            .branches
            .iter()
            .zip(self.selected.iter())
            .filter(|&(_, &sel)| sel)
            .map(|(b, _)| b.name.clone())
            .collect();

        for branch_name in &selected_branches {
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

        let len = branches.len();
        self.branches = branches;
        self.selected = vec![false; len];
        self.cursor = self.cursor.min(len.saturating_sub(1));
        self.list_scroll_offset = 0;
        self.results.clear();

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
}
