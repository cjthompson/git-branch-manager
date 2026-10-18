use super::*;

impl App {
    // ---- Overlay key handling ----

    pub(super) fn handle_overlay_key(&mut self, key: KeyEvent) {
        let overlay = self.overlay.take();
        match overlay {
            Some(Overlay::Help { scroll }) => match key.code {
                KeyCode::Char('j') | KeyCode::Down => {
                    self.overlay = Some(Overlay::Help {
                        scroll: scroll.saturating_add(1),
                    });
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.overlay = Some(Overlay::Help {
                        scroll: scroll.saturating_sub(1),
                    });
                }
                KeyCode::PageDown => {
                    self.overlay = Some(Overlay::Help {
                        scroll: scroll.saturating_add(5),
                    });
                }
                KeyCode::PageUp => {
                    self.overlay = Some(Overlay::Help {
                        scroll: scroll.saturating_sub(5),
                    });
                }
                KeyCode::Esc | KeyCode::Char('q') => {}
                _ => self.overlay = Some(Overlay::Help { scroll }),
            },
            Some(Overlay::Confirm {
                preflight,
                choices,
                selected,
                mut body_scroll,
                stage,
            }) => match stage {
                ConfirmStage::Initial => match key.code {
                    KeyCode::Char('j') | KeyCode::Down => {
                        let selected = selected
                            .saturating_add(1)
                            .min(choices.len().saturating_sub(1));
                        self.overlay = Some(Overlay::Confirm {
                            preflight,
                            choices,
                            selected,
                            body_scroll,
                            stage: ConfirmStage::Initial,
                        });
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        self.overlay = Some(Overlay::Confirm {
                            preflight,
                            choices,
                            selected: selected.saturating_sub(1),
                            body_scroll,
                            stage: ConfirmStage::Initial,
                        });
                    }
                    KeyCode::PageDown => {
                        body_scroll.page_down(5);
                        self.overlay = Some(Overlay::Confirm {
                            preflight,
                            choices,
                            selected,
                            body_scroll,
                            stage: ConfirmStage::Initial,
                        });
                    }
                    KeyCode::PageUp => {
                        body_scroll.page_up(5);
                        self.overlay = Some(Overlay::Confirm {
                            preflight,
                            choices,
                            selected,
                            body_scroll,
                            stage: ConfirmStage::Initial,
                        });
                    }
                    KeyCode::Enter => {
                        if let Some(choice) = choices.get(selected).cloned() {
                            if let Some(target) = choice.destructive_target.clone() {
                                self.overlay = Some(Overlay::Confirm {
                                    preflight,
                                    choices,
                                    selected,
                                    body_scroll: ModalScroll::default(),
                                    stage: ConfirmStage::FinalDestructive { choice, target },
                                });
                            } else {
                                let dispatch_path =
                                    self.dispatch_path_for(choice.action, &choice.targets);
                                self.job_queue.enqueue_or_start_with_remote(
                                    choice.action,
                                    choice.targets,
                                    choice.remote,
                                    self.return_view,
                                    dispatch_path,
                                );
                            }
                        }
                    }
                    KeyCode::Char('n') | KeyCode::Esc => {}
                    KeyCode::Char(accelerator) => {
                        let selected = if choices
                            .get(selected)
                            .is_some_and(|choice| choice.accelerator == accelerator)
                        {
                            selected
                        } else {
                            choices
                                .iter()
                                .position(|choice| choice.accelerator == accelerator)
                                .unwrap_or(selected)
                        };
                        self.overlay = Some(Overlay::Confirm {
                            preflight,
                            choices,
                            selected,
                            body_scroll,
                            stage: ConfirmStage::Initial,
                        });
                    }
                    _ => {
                        self.overlay = Some(Overlay::Confirm {
                            preflight,
                            choices,
                            selected,
                            body_scroll,
                            stage: ConfirmStage::Initial,
                        });
                    }
                },
                ConfirmStage::FinalDestructive { choice, target } => match key.code {
                    KeyCode::PageDown => {
                        body_scroll.page_down(5);
                        self.overlay = Some(Overlay::Confirm {
                            preflight,
                            choices,
                            selected,
                            body_scroll,
                            stage: ConfirmStage::FinalDestructive { choice, target },
                        });
                    }
                    KeyCode::PageUp => {
                        body_scroll.page_up(5);
                        self.overlay = Some(Overlay::Confirm {
                            preflight,
                            choices,
                            selected,
                            body_scroll,
                            stage: ConfirmStage::FinalDestructive { choice, target },
                        });
                    }
                    KeyCode::Char('y') | KeyCode::Enter => {
                        let job_targets = vec![target.job_target()];
                        let dispatch_path = self.dispatch_path_for(choice.action, &job_targets);
                        self.job_queue.enqueue_or_start_with_remote(
                            choice.action,
                            job_targets,
                            choice.remote,
                            self.return_view,
                            dispatch_path,
                        );
                    }
                    KeyCode::Char('n') | KeyCode::Esc => {}
                    _ => {
                        self.overlay = Some(Overlay::Confirm {
                            preflight,
                            choices,
                            selected,
                            body_scroll,
                            stage: ConfirmStage::FinalDestructive { choice, target },
                        });
                    }
                },
            },
            Some(Overlay::ConfirmCancelJob { message }) => match key.code {
                KeyCode::Char('y') | KeyCode::Enter => {
                    self.job_queue.cancel_current();
                }
                KeyCode::Char('n') | KeyCode::Esc => {
                    // Keep the job running untouched -- don't put overlay back
                }
                _ => {
                    self.overlay = Some(Overlay::ConfirmCancelJob { message });
                }
            },
            Some(Overlay::Menu { cursor, items }) => {
                let cursor = items
                    .get(cursor)
                    .filter(|item| item.enabled)
                    .map_or_else(|| first_enabled_index(&items).unwrap_or(0), |_| cursor);
                match key.code {
                    KeyCode::Char('j') | KeyCode::Down => {
                        let mut new_cursor = cursor + 1;
                        while new_cursor < items.len() && !items[new_cursor].enabled {
                            new_cursor += 1;
                        }
                        if new_cursor < items.len() {
                            self.overlay = Some(Overlay::Menu {
                                cursor: new_cursor,
                                items,
                            });
                        } else {
                            self.overlay = Some(Overlay::Menu { cursor, items });
                        }
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        let new_cursor = previous_enabled_index(&items, cursor).unwrap_or(cursor);
                        self.overlay = Some(Overlay::Menu {
                            cursor: new_cursor,
                            items,
                        });
                    }
                    KeyCode::Enter => {
                        if let Some(item) = items.get(cursor) {
                            if item.enabled {
                                self.execute_menu_action(item.clone());
                            } else {
                                self.overlay = Some(Overlay::Menu { cursor, items });
                            }
                        }
                    }
                    KeyCode::Esc | KeyCode::Char('q') => {} // close
                    KeyCode::Char(c) => {
                        if let Some((_, item)) = items
                            .iter()
                            .enumerate()
                            .find(|(_, mi)| mi.shortcut == Some(c) && mi.enabled)
                        {
                            self.execute_menu_action(item.clone());
                        } else {
                            self.overlay = Some(Overlay::Menu { cursor, items });
                        }
                    }
                    _ => {
                        self.overlay = Some(Overlay::Menu { cursor, items });
                    }
                }
            }
            Some(Overlay::CommitDetails {
                commit,
                details,
                items,
                cursor,
                file_cursor,
                focus,
                mut scroll,
            }) => {
                let cursor = normalize_info_action_state(cursor, InfoModalFocus::Actions, &items).0;
                match key.code {
                    KeyCode::Tab | KeyCode::BackTab => {
                        let focus = match focus {
                            CommitDetailsFocus::Files if !items.is_empty() => {
                                CommitDetailsFocus::Actions
                            }
                            CommitDetailsFocus::Actions if !details.files.is_empty() => {
                                CommitDetailsFocus::Files
                            }
                            other => other,
                        };
                        self.overlay = Some(Overlay::CommitDetails {
                            commit,
                            details,
                            items,
                            cursor,
                            file_cursor,
                            focus,
                            scroll,
                        });
                    }
                    KeyCode::Char('j') | KeyCode::Down if focus == CommitDetailsFocus::Files => {
                        let file_cursor = file_cursor
                            .saturating_add(1)
                            .min(details.files.len().saturating_sub(1));
                        self.overlay = Some(Overlay::CommitDetails {
                            commit,
                            details,
                            items,
                            cursor,
                            file_cursor,
                            focus,
                            scroll,
                        });
                    }
                    KeyCode::Char('k') | KeyCode::Up if focus == CommitDetailsFocus::Files => {
                        let file_cursor = file_cursor.saturating_sub(1);
                        self.overlay = Some(Overlay::CommitDetails {
                            commit,
                            details,
                            items,
                            cursor,
                            file_cursor,
                            focus,
                            scroll,
                        });
                    }
                    KeyCode::Char('j') | KeyCode::Down if focus == CommitDetailsFocus::Actions => {
                        let cursor = next_enabled_index(&items, cursor).unwrap_or(cursor);
                        self.overlay = Some(Overlay::CommitDetails {
                            commit,
                            details,
                            items,
                            cursor,
                            file_cursor,
                            focus,
                            scroll,
                        });
                    }
                    KeyCode::Char('k') | KeyCode::Up if focus == CommitDetailsFocus::Actions => {
                        let cursor = previous_enabled_index(&items, cursor).unwrap_or(cursor);
                        self.overlay = Some(Overlay::CommitDetails {
                            commit,
                            details,
                            items,
                            cursor,
                            file_cursor,
                            focus,
                            scroll,
                        });
                    }
                    KeyCode::PageDown => {
                        scroll.page_down(5);
                        self.overlay = Some(Overlay::CommitDetails {
                            commit,
                            details,
                            items,
                            cursor,
                            file_cursor,
                            focus,
                            scroll,
                        });
                    }
                    KeyCode::PageUp => {
                        scroll.page_up(5);
                        self.overlay = Some(Overlay::CommitDetails {
                            commit,
                            details,
                            items,
                            cursor,
                            file_cursor,
                            focus,
                            scroll,
                        });
                    }
                    KeyCode::Enter if focus == CommitDetailsFocus::Files => {
                        if let Some(file) = details.files.get(file_cursor).cloned() {
                            let target = (details.oid.clone(), file.path.clone());
                            self.commit_file_diff_target = Some(target.clone());
                            let loader = commit_details::spawn_commit_file_diff_loader(
                                self.repo_path.clone(),
                                details.clone(),
                                file,
                            );
                            let (tx, rx) = mpsc::channel();
                            let target = target.clone();
                            std::thread::spawn(move || {
                                if let Ok(result) = loader.recv() {
                                    let _ = tx.send((target, result));
                                }
                            });
                            self.commit_file_diff_rx = Some(rx);
                        }
                        self.overlay = Some(Overlay::CommitDetails {
                            commit,
                            details,
                            items,
                            cursor,
                            file_cursor,
                            focus,
                            scroll,
                        });
                    }
                    KeyCode::Enter if focus == CommitDetailsFocus::Actions => {
                        if let Some(item) = items.get(cursor).cloned().filter(|item| item.enabled) {
                            self.invalidate_commit_file_diff_request();
                            self.execute_menu_action(item);
                        } else {
                            self.overlay = Some(Overlay::CommitDetails {
                                commit,
                                details,
                                items,
                                cursor,
                                file_cursor,
                                focus,
                                scroll,
                            });
                        }
                    }
                    KeyCode::Char(c) if focus == CommitDetailsFocus::Actions => {
                        if let Some(item) = items
                            .iter()
                            .find(|item| item.shortcut == Some(c) && item.enabled)
                            .cloned()
                        {
                            self.invalidate_commit_file_diff_request();
                            self.execute_menu_action(item);
                        } else {
                            self.overlay = Some(Overlay::CommitDetails {
                                commit,
                                details,
                                items,
                                cursor,
                                file_cursor,
                                focus,
                                scroll,
                            });
                        }
                    }
                    KeyCode::Esc | KeyCode::Char('q') => self.invalidate_commit_file_diff_request(),
                    _ => {
                        self.overlay = Some(Overlay::CommitDetails {
                            commit,
                            details,
                            items,
                            cursor,
                            file_cursor,
                            focus,
                            scroll,
                        })
                    }
                }
            }
            Some(Overlay::CommitDiff {
                commit,
                details,
                items,
                cursor,
                file_cursor,
                focus,
                details_scroll,
                diff,
                mut scroll,
            }) => match key.code {
                KeyCode::Char('j') | KeyCode::Down => {
                    scroll.offset = scroll.offset.saturating_add(1);
                    self.overlay = Some(Overlay::CommitDiff {
                        commit,
                        details,
                        items,
                        cursor,
                        file_cursor,
                        focus,
                        details_scroll,
                        diff,
                        scroll,
                    });
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    scroll.offset = scroll.offset.saturating_sub(1);
                    self.overlay = Some(Overlay::CommitDiff {
                        commit,
                        details,
                        items,
                        cursor,
                        file_cursor,
                        focus,
                        details_scroll,
                        diff,
                        scroll,
                    });
                }
                KeyCode::PageDown => {
                    scroll.page_down(5);
                    self.overlay = Some(Overlay::CommitDiff {
                        commit,
                        details,
                        items,
                        cursor,
                        file_cursor,
                        focus,
                        details_scroll,
                        diff,
                        scroll,
                    });
                }
                KeyCode::PageUp => {
                    scroll.page_up(5);
                    self.overlay = Some(Overlay::CommitDiff {
                        commit,
                        details,
                        items,
                        cursor,
                        file_cursor,
                        focus,
                        details_scroll,
                        diff,
                        scroll,
                    });
                }
                KeyCode::Esc | KeyCode::Char('q') => {
                    self.invalidate_commit_file_diff_request();
                    self.overlay = Some(Overlay::CommitDetails {
                        commit,
                        details,
                        items,
                        cursor,
                        file_cursor,
                        focus,
                        scroll: details_scroll,
                    });
                }
                _ => {
                    self.overlay = Some(Overlay::CommitDiff {
                        commit,
                        details,
                        items,
                        cursor,
                        file_cursor,
                        focus,
                        details_scroll,
                        diff,
                        scroll,
                    })
                }
            },
            Some(Overlay::InfoModal {
                cursor,
                info_cursor,
                focus,
                items,
                row,
            }) => {
                let (cursor, focus) = normalize_info_action_state(cursor, focus, &items);
                match key.code {
                    KeyCode::Tab | KeyCode::BackTab => {
                        self.overlay = Some(Overlay::InfoModal {
                            cursor,
                            info_cursor,
                            focus: if focus == InfoModalFocus::Info
                                && first_enabled_index(&items).is_some()
                            {
                                InfoModalFocus::Actions
                            } else if focus == InfoModalFocus::Actions {
                                InfoModalFocus::Info
                            } else {
                                focus
                            },
                            items,
                            row,
                        });
                    }
                    KeyCode::Char('j') | KeyCode::Down if focus == InfoModalFocus::Actions => {
                        let mut new_cursor = cursor + 1;
                        while new_cursor < items.len() && !items[new_cursor].enabled {
                            new_cursor += 1;
                        }
                        if new_cursor < items.len() {
                            self.overlay = Some(Overlay::InfoModal {
                                cursor: new_cursor,
                                info_cursor,
                                focus,
                                items,
                                row,
                            });
                        } else {
                            self.overlay = Some(Overlay::InfoModal {
                                cursor,
                                info_cursor,
                                focus,
                                items,
                                row,
                            });
                        }
                    }
                    KeyCode::Char('k') | KeyCode::Up if focus == InfoModalFocus::Actions => {
                        let new_cursor = previous_enabled_index(&items, cursor).unwrap_or(cursor);
                        self.overlay = Some(Overlay::InfoModal {
                            cursor: new_cursor,
                            info_cursor,
                            focus,
                            items,
                            row,
                        });
                    }
                    KeyCode::Char('u') | KeyCode::PageUp => {
                        self.info_modal_scroll_offset =
                            self.info_modal_scroll_offset.saturating_sub(5);
                        self.overlay = Some(Overlay::InfoModal {
                            cursor,
                            info_cursor,
                            focus,
                            items,
                            row,
                        });
                    }
                    KeyCode::Char('d') | KeyCode::PageDown => {
                        self.info_modal_scroll_offset =
                            self.info_modal_scroll_offset.saturating_add(5);
                        self.overlay = Some(Overlay::InfoModal {
                            cursor,
                            info_cursor,
                            focus,
                            items,
                            row,
                        });
                    }
                    KeyCode::Down if focus == InfoModalFocus::Info => {
                        let info_cursor = info_cursor
                            .saturating_add(1)
                            .min(row.info_field_count().saturating_sub(1));
                        self.overlay = Some(Overlay::InfoModal {
                            cursor,
                            info_cursor,
                            focus,
                            items,
                            row,
                        });
                    }
                    KeyCode::Up if focus == InfoModalFocus::Info => {
                        let info_cursor = info_cursor.saturating_sub(1);
                        self.overlay = Some(Overlay::InfoModal {
                            cursor,
                            info_cursor,
                            focus,
                            items,
                            row,
                        });
                    }
                    KeyCode::Enter if focus == InfoModalFocus::Info => {
                        if let Some((label, value)) = row.info_field(info_cursor) {
                            self.copy_info_value(label, value);
                        }
                        self.overlay = Some(Overlay::InfoModal {
                            cursor,
                            info_cursor,
                            focus,
                            items,
                            row,
                        });
                    }
                    KeyCode::Char('y') if focus == InfoModalFocus::Info => {
                        if let Some((label, value)) = row.info_field(info_cursor) {
                            self.copy_info_value(label, value);
                        }
                        self.overlay = Some(Overlay::InfoModal {
                            cursor,
                            info_cursor,
                            focus,
                            items,
                            row,
                        });
                    }
                    KeyCode::Enter if focus == InfoModalFocus::Actions => {
                        if let Some(item) = items.get(cursor) {
                            if item.enabled {
                                self.execute_menu_action(item.clone());
                            } else {
                                self.overlay = Some(Overlay::InfoModal {
                                    cursor,
                                    info_cursor,
                                    focus,
                                    items,
                                    row,
                                });
                            }
                        }
                    }
                    KeyCode::Esc | KeyCode::Char('q') => {} // close
                    KeyCode::Char(c) if focus == InfoModalFocus::Actions => {
                        if let Some((_, item)) = items
                            .iter()
                            .enumerate()
                            .find(|(_, mi)| mi.shortcut == Some(c) && mi.enabled)
                        {
                            self.execute_menu_action(item.clone());
                        } else {
                            self.overlay = Some(Overlay::InfoModal {
                                cursor,
                                info_cursor,
                                focus,
                                items,
                                row,
                            });
                        }
                    }
                    _ => {
                        self.overlay = Some(Overlay::InfoModal {
                            cursor,
                            info_cursor,
                            focus,
                            items,
                            row,
                        });
                    }
                }
            }
            Some(Overlay::Results {
                results,
                selected_index,
                expanded_index,
                focus,
                body_scroll,
            }) => self.handle_results_key(
                key.code,
                results,
                selected_index,
                expanded_index,
                focus,
                body_scroll,
            ),
            Some(Overlay::Executing { label, progress }) => {
                if key.code == KeyCode::Esc {
                    if let Some(request) = self.primary_code_check_request.take() {
                        request.cancel.store(true, Ordering::Relaxed);
                        if self
                            .cancel_flag
                            .as_ref()
                            .is_some_and(|cancel| Arc::ptr_eq(cancel, &request.cancel))
                        {
                            self.cancel_flag = None;
                        }
                    }
                    if let Some(flag) = &self.cancel_flag {
                        flag.store(true, Ordering::Relaxed);
                    }
                    // Keep the fetch receiver alive so partial repository
                    // changes are observed and reconciled after cancellation.
                    self.fetch_overlay_dismissed = true;
                    self.diag_rx = None;
                    self.progress_rx = None;
                    self.progress = None;
                    // overlay stays None (already taken at top of function)
                    return;
                }
                // overlay was taken; put it back for any other key
                self.overlay = Some(Overlay::Executing { label, progress });
            }
            Some(Overlay::Settings { cursor }) => {
                self.handle_settings_key(key, cursor);
            }
            Some(Overlay::Filter) => {
                self.handle_filter_key(key, 0);
            }
            Some(Overlay::FilterSelection { cursor }) => {
                self.handle_filter_key(key, cursor);
            }
            Some(Overlay::GraphOptions {
                cursor,
                include_remotes,
            }) => match key.code {
                KeyCode::Char('j') | KeyCode::Down => {
                    self.overlay = Some(Overlay::GraphOptions {
                        cursor: (cursor + 1).min(1),
                        include_remotes,
                    });
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.overlay = Some(Overlay::GraphOptions {
                        cursor: cursor.saturating_sub(1),
                        include_remotes,
                    });
                }
                KeyCode::Char(' ') => {
                    self.overlay = Some(Overlay::GraphOptions {
                        cursor,
                        include_remotes: !include_remotes,
                    });
                }
                KeyCode::Enter => {
                    self.apply_graph_options(cursor, include_remotes);
                    self.save_include_remotes();
                }
                KeyCode::Esc | KeyCode::Char('q') => {}
                _ => {
                    self.overlay = Some(Overlay::GraphOptions {
                        cursor,
                        include_remotes,
                    });
                }
            },
            Some(Overlay::Diagnostics { cursor }) => {
                let count = DiagnosticAction::ALL.len();
                match key.code {
                    KeyCode::Char('j') | KeyCode::Down => {
                        self.overlay = Some(Overlay::Diagnostics {
                            cursor: (cursor + 1).min(count.saturating_sub(1)),
                        });
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        self.overlay = Some(Overlay::Diagnostics {
                            cursor: cursor.saturating_sub(1),
                        });
                    }
                    KeyCode::Enter => {
                        if let Some(action) = DiagnosticAction::ALL.get(cursor) {
                            match action {
                                DiagnosticAction::VerifyCache => self.run_cache_audit(),
                            }
                        }
                    }
                    KeyCode::Esc | KeyCode::Char('q') => {} // close
                    _ => {
                        self.overlay = Some(Overlay::Diagnostics { cursor });
                    }
                }
            }
            Some(Overlay::DiagnosticsReport { audit, scroll }) => match key.code {
                KeyCode::Char('j') | KeyCode::Down => {
                    self.overlay = Some(Overlay::DiagnosticsReport {
                        audit,
                        scroll: scroll + 1,
                    });
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.overlay = Some(Overlay::DiagnosticsReport {
                        audit,
                        scroll: scroll.saturating_sub(1),
                    });
                }
                KeyCode::PageDown => {
                    self.overlay = Some(Overlay::DiagnosticsReport {
                        audit,
                        scroll: scroll.saturating_add(5),
                    });
                }
                KeyCode::PageUp => {
                    self.overlay = Some(Overlay::DiagnosticsReport {
                        audit,
                        scroll: scroll.saturating_sub(5),
                    });
                }
                KeyCode::Char('f') if !audit.is_clean() => {
                    self.apply_cache_fix(audit);
                }
                KeyCode::Esc | KeyCode::Char('q') => {} // close
                _ => {
                    self.overlay = Some(Overlay::DiagnosticsReport { audit, scroll });
                }
            },
            None => {}
        }
    }
}
