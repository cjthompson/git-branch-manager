use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::DefaultTerminal;

use git_branch_manager::git::operations;
use git_branch_manager::types::{BranchAction, BranchInfo, MergeStatus, OperationResult};
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
    pub branches: Vec<BranchInfo>,
    pub view: View,
    pub cursor: usize,
    pub selected: Vec<bool>,
    pub list_scroll_offset: usize,
    pub results: Vec<OperationResult>,
    pub should_exit: bool,
}

impl App {
    pub fn new(base_branch: String, branches: Vec<BranchInfo>) -> Self {
        let len = branches.len();
        Self {
            base_branch,
            branches,
            view: View::BranchList,
            cursor: 0,
            selected: vec![false; len],
            list_scroll_offset: 0,
            results: Vec::new(),
            should_exit: false,
        }
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        while !self.should_exit {
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
        self.should_exit = true;
    }

    fn handle_help_key(&mut self, _code: KeyCode) {
        self.view = View::BranchList;
    }

    fn execute_action(&mut self) {
        let action = match &self.view {
            View::Confirm { action } => action.clone(),
            _ => return,
        };

        let repo = match git2::Repository::discover(".") {
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
                    let results = operations::delete_local_and_remote(&repo, branch_name);
                    self.results.extend(results);
                }
            }
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
