use super::*;

impl App {
    pub(super) fn handle_results_key(
        &mut self,
        key: KeyCode,
        results: Vec<OperationResult>,
        mut selected_index: usize,
        mut expanded_index: Option<usize>,
        mut focus: ResultsFocus,
        mut body_scroll: ModalScroll,
    ) {
        if key == KeyCode::Esc {
            return;
        }
        if results.is_empty() {
            self.overlay = Some(Overlay::results(results));
            return;
        }

        selected_index = selected_index.min(results.len() - 1);
        expanded_index = expanded_index.filter(|index| *index < results.len());
        if matches!(key, KeyCode::PageDown | KeyCode::PageUp) {
            match key {
                KeyCode::PageDown => body_scroll.page_down(5),
                KeyCode::PageUp => body_scroll.page_up(5),
                _ => unreachable!("page-key match is exhaustive"),
            }
            self.overlay = Some(Overlay::Results {
                results,
                selected_index,
                expanded_index,
                focus,
                body_scroll,
            });
            return;
        }
        match focus {
            ResultsFocus::Results => match key {
                KeyCode::Char('j') | KeyCode::Down => {
                    selected_index = selected_index.saturating_add(1).min(results.len() - 1);
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    selected_index = selected_index.saturating_sub(1);
                }
                KeyCode::Enter => {
                    expanded_index = if expanded_index == Some(selected_index) {
                        None
                    } else {
                        Some(selected_index)
                    };
                    body_scroll = ModalScroll::default();
                }
                KeyCode::Tab if expanded_index == Some(selected_index) => {
                    focus = ResultsFocus::Actions {
                        selected_index: 0,
                        raw_details_visible: false,
                    };
                }
                _ => {}
            },
            ResultsFocus::Actions {
                selected_index: action_index,
                raw_details_visible,
            } => {
                let Some(result_index) = expanded_index else {
                    focus = ResultsFocus::Results;
                    self.overlay = Some(Overlay::Results {
                        results,
                        selected_index,
                        expanded_index,
                        focus,
                        body_scroll,
                    });
                    return;
                };
                let actions = result_actions(&results[result_index]);
                let action_index = action_index.min(actions.len().saturating_sub(1));
                match key {
                    KeyCode::Char('j') | KeyCode::Down => {
                        focus = ResultsFocus::Actions {
                            selected_index: action_index
                                .saturating_add(1)
                                .min(actions.len().saturating_sub(1)),
                            raw_details_visible,
                        };
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        focus = ResultsFocus::Actions {
                            selected_index: action_index.saturating_sub(1),
                            raw_details_visible,
                        };
                    }
                    KeyCode::Tab => focus = ResultsFocus::Results,
                    KeyCode::Enter => match actions.get(action_index).copied() {
                        Some(ResultsAction::ViewRawGitDetails) => {
                            focus = ResultsFocus::Actions {
                                selected_index: action_index,
                                raw_details_visible: !raw_details_visible,
                            };
                        }
                        Some(action) => {
                            let result = results[result_index].clone();
                            self.open_result_recovery(&result, action);
                            return;
                        }
                        None => focus = ResultsFocus::Results,
                    },
                    _ => {}
                }
            }
        }

        self.overlay = Some(Overlay::Results {
            results,
            selected_index,
            expanded_index,
            focus,
            body_scroll,
        });
    }

    pub(super) fn open_result_recovery(&mut self, result: &OperationResult, recovery: ResultsAction) {
        let (action, target, description) = match (recovery, result.failure.as_ref()) {
            (ResultsAction::ReviewForceDeletion, Some(FailureCause::NotMerged)) => (
                BranchAction::DeleteLocalForce,
                ConfirmTarget::Branch(result.branch_name.clone()),
                "Permanently delete only this unmerged branch.",
            ),
            (
                ResultsAction::ReviewWorktreeRemoval,
                Some(FailureCause::CheckedOutInWorktree {
                    worktree_path,
                    is_main: false,
                }),
            ) => (
                BranchAction::DeleteBranchAndRemoveWorktree,
                ConfirmTarget::BranchWorktree {
                    branch: result.branch_name.clone(),
                    worktree: worktree_path.clone(),
                },
                "Remove only this linked worktree, then safely delete its branch.",
            ),
            _ => return,
        };
        let targets = vec![target.job_target()];
        let choice = ConfirmChoice {
            accelerator: 'y',
            action,
            targets,
            remote: None,
            command: confirmation_command_for_target(action, &target, None),
            description: description.into(),
            destructive_target: Some(target.clone()),
        };
        self.open_confirm_with_choices(
            self.return_view,
            DeletePreflight::default(),
            vec![choice.clone()],
            ConfirmStage::FinalDestructive { choice, target },
        );
    }

    pub(super) fn handle_settings_key(&mut self, key: KeyEvent, cursor: usize) {
        const NUM_ROWS: usize = 9;

        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.overlay = Some(Overlay::Settings {
                    cursor: (cursor + 1).min(NUM_ROWS - 1),
                });
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.overlay = Some(Overlay::Settings {
                    cursor: cursor.saturating_sub(1),
                });
            }
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Char(' ') | KeyCode::Enter => {
                match cursor {
                    0 => {
                        self.symbols = self.symbols.next();
                        self.reload_graph_for_symbol_change();
                    }
                    1 => {
                        self.theme = self.theme.next();
                    }
                    2 => {
                        self.cycle_view_sort(ViewId::Branches, true);
                    }
                    3 => {
                        self.cycle_view_sort(ViewId::Remotes, true);
                    }
                    4 => {
                        self.cycle_view_sort(ViewId::Tags, true);
                    }
                    5 => {
                        self.cycle_view_sort(ViewId::Worktrees, true);
                    }
                    6 => {
                        self.config.auto_fetch = Some(self.config.auto_fetch != Some(true));
                    }
                    7 => {
                        self.config.load_worktrees_on_launch =
                            Some(self.config.load_worktrees_on_launch != Some(true));
                    }
                    8 => self.toggle_horizontal_scrolling(),
                    _ => {}
                }
                self.save_settings_field(cursor);
                self.overlay = Some(Overlay::Settings { cursor });
            }
            KeyCode::Left | KeyCode::Char('h') => {
                match cursor {
                    0 => {
                        // backward = next() twice (3-cycle)
                        self.symbols = self.symbols.next();
                        self.symbols = self.symbols.next();
                        self.reload_graph_for_symbol_change();
                    }
                    1 => {
                        // backward = next() 3 times (4-cycle)
                        self.theme = self.theme.next();
                        self.theme = self.theme.next();
                        self.theme = self.theme.next();
                    }
                    2 => {
                        self.cycle_view_sort(ViewId::Branches, false);
                    }
                    3 => {
                        self.cycle_view_sort(ViewId::Remotes, false);
                    }
                    4 => {
                        self.cycle_view_sort(ViewId::Tags, false);
                    }
                    5 => {
                        self.cycle_view_sort(ViewId::Worktrees, false);
                    }
                    6 => {
                        self.config.auto_fetch = Some(self.config.auto_fetch != Some(true));
                    }
                    7 => {
                        self.config.load_worktrees_on_launch =
                            Some(self.config.load_worktrees_on_launch != Some(true));
                    }
                    8 => self.toggle_horizontal_scrolling(),
                    _ => {}
                }
                self.save_settings_field(cursor);
                self.overlay = Some(Overlay::Settings { cursor });
            }
            KeyCode::Esc => {
                // Close settings (overlay already taken)
            }
            _ => {
                self.overlay = Some(Overlay::Settings { cursor });
            }
        }
    }

    pub(super) fn handle_filter_key(&mut self, key: KeyEvent, cursor: usize) {
        let active_tokens = match self.active_view {
            ViewId::Graph => &[] as &[FilterTokenDef],
            ViewId::Branches => &self.branch_filter_tokens,
            ViewId::Remotes => &self.remote_filter_tokens,
            ViewId::Tags => &self.tag_filter_tokens,
            ViewId::Worktrees => &self.worktree_filter_tokens,
        };
        let max_cursor = active_tokens.len();
        let selected_token = active_tokens.get(cursor).map(|token| token.token);

        match key.code {
            KeyCode::Esc | KeyCode::Char('\\') => {
                // Close filter (overlay already taken); only Esc / \ dismiss it.
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.overlay = Some(Overlay::FilterSelection {
                    cursor: (cursor + 1).min(max_cursor),
                });
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.overlay = Some(Overlay::FilterSelection {
                    cursor: cursor.min(max_cursor).saturating_sub(1),
                });
            }
            KeyCode::Enter => {
                if let Some(token) = selected_token {
                    let current = self.active_filter_query();
                    self.set_active_filter(FilterSet::toggle_token(&current, token));
                } else {
                    self.set_active_filter(String::new());
                }
                self.overlay = Some(Overlay::FilterSelection {
                    cursor: cursor.min(max_cursor),
                });
            }
            KeyCode::Char('c') => {
                // Clear all filters, but keep the modal open.
                self.set_active_filter(String::new());
                self.overlay = Some(Overlay::FilterSelection {
                    cursor: cursor.min(max_cursor),
                });
            }
            KeyCode::Char(ch) => {
                // Toggle the matching filter token (if any). Either way the modal
                // stays open so the user can adjust several filters in a row.
                if let Some(token) = active_tokens
                    .iter()
                    .find(|token| token.key == ch)
                    .map(|token| token.token)
                {
                    let current = self.active_filter_query();
                    let new = FilterSet::toggle_token(&current, token);
                    self.set_active_filter(new);
                }
                self.overlay = Some(Overlay::FilterSelection {
                    cursor: cursor.min(max_cursor),
                });
            }
            _ => {
                self.overlay = Some(Overlay::FilterSelection {
                    cursor: cursor.min(max_cursor),
                });
            }
        }
    }
}
