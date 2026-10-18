use super::*;

impl App {
    pub(super) fn push_selected_branches(&mut self) {
        let targets = self.get_selected_branch_names_for(|b| {
            capability::can_push(b, self.has_configured_remote, true).available
        });
        self.open_confirm(BranchAction::Push, ViewId::Branches, targets);
    }

    pub(super) fn delete_selected_remote_branches(&mut self) {
        let targets = list_state::collect_targets(&self.remotes, |b| {
            (!b.is_pinned()).then(|| b.short_name.clone())
        });
        self.open_confirm(BranchAction::DeleteRemoteBranch, ViewId::Remotes, targets);
    }

    pub(super) fn delete_selected_tags(&mut self, include_remote: bool) {
        let targets = list_state::collect_targets(&self.tags, |t| Some(t.name.clone()));
        let action = if include_remote {
            BranchAction::DeleteTagAndRemote
        } else {
            BranchAction::DeleteTag
        };
        self.open_confirm(action, ViewId::Tags, targets);
    }

    pub(super) fn push_selected_tags(&mut self) {
        let targets = list_state::collect_targets(&self.tags, |t| Some(t.name.clone()));
        self.open_confirm(BranchAction::PushTag, ViewId::Tags, targets);
    }

    pub(super) fn remove_selected_worktrees(&mut self, force: bool) {
        self.worktree_enrich_rx = None;
        let targets = list_state::collect_targets(&self.worktrees, |wt| {
            (!wt.is_main).then(|| wt.path.to_string_lossy().to_string())
        });
        let action = if force {
            BranchAction::WorktreeForceRemove
        } else {
            BranchAction::WorktreeRemove
        };
        self.open_confirm(action, ViewId::Worktrees, targets);
    }

    pub(super) fn get_selected_branch_names_for(
        &self,
        action_check: impl Fn(&BranchInfo) -> bool,
    ) -> Vec<String> {
        list_state::collect_targets(&self.branches, |b| action_check(b).then(|| b.name.clone()))
    }

    /// Open a confirm overlay for `action` over `targets`, returning to
    /// `return_view` when it closes. No-op when `targets` is empty.
    pub(super) fn open_confirm(&mut self, action: BranchAction, return_view: ViewId, targets: Vec<String>) {
        self.open_confirm_with_remote(action, return_view, targets, None);
    }

    pub(super) fn open_confirm_with_remote(
        &mut self,
        action: BranchAction,
        return_view: ViewId,
        targets: Vec<String>,
        remote: Option<String>,
    ) {
        if targets.is_empty() {
            return;
        }
        let destructive = requires_final_confirmation(action);
        let choices = if destructive {
            targets
                .into_iter()
                .filter_map(|job_target| {
                    let target = self.confirm_target(action, &job_target)?;
                    Some(ConfirmChoice {
                        accelerator: 'y',
                        action,
                        targets: vec![target.job_target()],
                        remote: remote.clone(),
                        command: confirmation_command_for_target(
                            action,
                            &target,
                            remote.as_deref(),
                        ),
                        description: "Open final confirmation for exactly this target.".into(),
                        destructive_target: Some(target),
                    })
                })
                .collect()
        } else {
            vec![ConfirmChoice {
                accelerator: 'y',
                action,
                command: confirmation_command(action, &targets, remote.as_deref()),
                description: format!(
                    "Apply {} to exactly {} target(s).",
                    action.label(),
                    targets.len()
                ),
                targets,
                remote,
                destructive_target: None,
            }]
        };
        let stage = match choices.as_slice() {
            [choice] if destructive => ConfirmStage::FinalDestructive {
                choice: choice.clone(),
                target: choice
                    .destructive_target
                    .clone()
                    .expect("destructive choice has an exact target"),
            },
            _ => ConfirmStage::Initial,
        };
        self.open_confirm_with_choices(return_view, DeletePreflight::default(), choices, stage);
    }

    pub(super) fn confirm_target(&self, action: BranchAction, job_target: &str) -> Option<ConfirmTarget> {
        match action {
            BranchAction::DeleteLocalForce => Some(ConfirmTarget::Branch(job_target.to_string())),
            BranchAction::WorktreeForceRemove => {
                Some(ConfirmTarget::Worktree(PathBuf::from(job_target)))
            }
            BranchAction::DeleteBranchAndRemoveWorktreeForce => self
                .worktrees
                .items()
                .iter()
                .find(|worktree| worktree.branch.as_deref() == Some(job_target))
                .map(|worktree| ConfirmTarget::BranchWorktree {
                    branch: job_target.to_string(),
                    worktree: worktree.path.clone(),
                }),
            _ => None,
        }
    }

    /// Install the typed confirmation state after target and risk discovery.
    pub(super) fn open_confirm_with_choices(
        &mut self,
        return_view: ViewId,
        preflight: DeletePreflight,
        choices: Vec<ConfirmChoice>,
        stage: ConfirmStage,
    ) {
        if choices.is_empty() {
            return;
        }
        self.return_view = return_view;
        self.overlay = Some(Overlay::Confirm {
            preflight,
            choices,
            selected: 0,
            body_scroll: ModalScroll::default(),
            stage,
        });
    }

    // ---- Sorting ----

    pub(super) fn cycle_sort(&mut self) {
        match self.active_view {
            ViewId::Graph => {}
            ViewId::Branches => {
                list_state::cycle_sort_and_apply(&mut self.branches, &self.branch_columns)
            }
            ViewId::Remotes => {
                list_state::cycle_sort_and_apply(&mut self.remotes, &self.remote_columns)
            }
            ViewId::Tags => list_state::cycle_sort_and_apply(&mut self.tags, &self.tag_columns),
            ViewId::Worktrees => {
                list_state::cycle_sort_and_apply(&mut self.worktrees, &self.worktree_columns)
            }
        }
        self.save_sort_field(self.active_view);
    }

    pub(super) fn toggle_sort_direction(&mut self) {
        match self.active_view {
            ViewId::Graph => {}
            ViewId::Branches => list_state::toggle_sort_direction_and_apply(
                &mut self.branches,
                &self.branch_columns,
            ),
            ViewId::Remotes => {
                list_state::toggle_sort_direction_and_apply(&mut self.remotes, &self.remote_columns)
            }
            ViewId::Tags => {
                list_state::toggle_sort_direction_and_apply(&mut self.tags, &self.tag_columns)
            }
            ViewId::Worktrees => list_state::toggle_sort_direction_and_apply(
                &mut self.worktrees,
                &self.worktree_columns,
            ),
        }
        self.save_sort_field(self.active_view);
    }
}
