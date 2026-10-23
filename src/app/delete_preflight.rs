use super::*;

impl App {
    // ---- View-level action helpers ----

    pub(super) fn delete_selected_branches(&mut self, include_remote: bool) {
        let targets = if include_remote {
            // Drop base/current (cannot delete those) and drop no-remote (no path
            // to success). Worktree-checked-out and dirty branches still flow
            // through to `build_delete_preflight` so the user sees a structured
            // `DeleteRisk::CheckedOut` / `DeleteRisk::DirtyWorktree` warning in
            // the confirm overlay and can confirm or cancel.
            self.get_selected_branch_names_for(|b| {
                let has_remote = matches!(&b.tracking, TrackingStatus::Tracked { gone: false, .. });
                !b.is_pinned() && has_remote
            })
        } else {
            // Drop base/current only (same as the OLD `!is_pinned()` filter).
            // Worktree-checked-out and dirty branches reach
            // `build_delete_preflight` so the user gets the structured preflight
            // warning and the chance to confirm or cancel.
            self.get_selected_branch_names_for(|b| !b.is_pinned())
        };
        let action = if include_remote {
            BranchAction::DeleteLocalAndRemote
        } else {
            BranchAction::DeleteLocal
        };
        if include_remote {
            self.open_confirm(action, ViewId::Branches, targets);
        } else {
            let preflight = self.build_delete_preflight(&targets);
            let choices = self.build_delete_choices(&targets, None, &preflight);
            self.open_confirm_with_choices(
                ViewId::Branches,
                preflight,
                choices,
                ConfirmStage::Initial,
            );
        }
    }

    /// Return a `WorktreePresence` view of the worktree currently on the given
    /// branch, if any. `None` means "no worktree the app has loaded has this
    /// branch checked out." Pure shim over `self.worktrees.items()` — no I/O.
    /// Reuses the same lookup as `build_delete_preflight` below.
    ///
    /// Used by `build_branch_menu_for` (Step 4) to feed the `capability::can_*`
    /// evaluators. The `worktrees.loading` gate lives upstream of this call
    /// (in the menu builder), not here.
    pub(super) fn worktree_presence_for(&self, branch_name: &str) -> Option<capability::WorktreePresence<'_>> {
        self.worktrees
            .items()
            .iter()
            .find(|worktree| worktree.branch.as_deref() == Some(branch_name))
            .map(|worktree| capability::WorktreePresence {
                path: &worktree.path,
                is_main: worktree.is_main,
                is_clean: worktree.wt_status.is_clean(),
            })
    }

    /// Like `worktree_presence_for`, but tries `branch_name` first and falls
    /// back to `base_branch` if the branch itself isn't checked out anywhere.
    /// Consolidates the lookup shared by evaluators that need either the
    /// source branch's worktree (`can_rebase`) or the base branch's worktree
    /// (`can_merge_into_base`, `can_squash_into_base`). When `branch_name ==
    /// base_branch` this collapses to a single lookup.
    pub(super) fn worktree_presence_for_branch_or_base(
        &self,
        branch_name: &str,
        base_branch: &str,
    ) -> Option<capability::WorktreePresence<'_>> {
        self.worktree_presence_for(branch_name)
            .or_else(|| self.worktree_presence_for(base_branch))
    }

    /// Resolve the worktree-aware dispatch path for a confirmed job, per
    /// Design → "Worktree-aware merge/squash dispatch" (plan P010 Step 7).
    /// Only `Merge`/`SquashMerge`/`Rebase` resolve to `Some(path)`; every
    /// other action resolves to `None`, which `JobQueue::start`'s
    /// `unwrap_or_else` fallback (Step 6) turns into "run in
    /// `self.repo_path`" -- today's unchanged behavior.
    ///
    /// `targets` is the exact job-target list about to be enqueued: either
    /// `choice.targets` from `ConfirmStage::Initial`, or the single
    /// `vec![target.job_target()]` from `ConfirmStage::FinalDestructive`.
    /// Merge/SquashMerge/Rebase menu rows are only ever built per-branch by
    /// `build_branch_menu_for` and routed through `execute_menu_action`'s
    /// single-target `vec![item.target]` -- there is no bulk multi-select
    /// path for these three actions (bulk selection is Delete/Push-only,
    /// see `get_selected_branch_names_for`) -- so `targets.first()` is
    /// always the correct, and only, source branch.
    pub(super) fn dispatch_path_for(&self, action: BranchAction, targets: &[String]) -> Option<PathBuf> {
        let branch_name = targets.first()?;
        match action {
            BranchAction::Merge | BranchAction::SquashMerge => self
                .worktree_presence_for_branch_or_base(branch_name, &self.base_branch)
                .map(|wt| wt.path.to_path_buf()),
            BranchAction::Rebase => self
                .worktree_presence_for(branch_name)
                .map(|wt| wt.path.to_path_buf()),
            _ => None,
        }
    }

    /// Build a `MenuItem` from a `capability::Capability`. `remote` is passed
    /// through separately (not derived from `cap`) because `Delete local +
    /// remote` needs to carry the tracking remote name downstream through
    /// `execute_menu_action`/`open_confirm_with_remote`/`build_delete_choices`
    /// (see `src/app.rs:3013,3021`) — collapsing `remote` into this helper's
    /// signature as an always-`None` field would silently break that action.
    pub(super) fn row_from(
        &self,
        label: &str,
        shortcut: Option<char>,
        action: BranchAction,
        target: String,
        remote: Option<String>,
        cap: capability::Capability,
    ) -> MenuItem {
        MenuItem {
            label: label.into(),
            enabled: cap.available,
            reason: cap.reason.map(str::to_owned),
            shortcut,
            action,
            target,
            remote,
        }
    }

    /// Collect structured safety facts in stable branch/status order.
    pub(super) fn build_delete_preflight(&self, targets: &[String]) -> DeletePreflight {
        let mut risks = Vec::new();

        for name in targets {
            if let Some(branch) = self
                .branches
                .items()
                .iter()
                .find(|branch| branch.name == *name)
            {
                if matches!(
                    branch.merge_status,
                    MergeStatus::Unmerged | MergeStatus::LikelySquashMerged
                ) {
                    risks.push(DeleteRisk::UniqueCommits {
                        branch: name.clone(),
                        base: branch.base_branch.clone(),
                        status: branch.merge_status,
                        merge_base_commit: branch.merge_base_commit.clone(),
                    });
                }
            }

            if let Some(worktree) = self
                .worktrees
                .items()
                .iter()
                .find(|worktree| worktree.branch.as_deref() == Some(name.as_str()))
            {
                risks.push(DeleteRisk::CheckedOut {
                    branch: name.clone(),
                    worktree: worktree.path.clone(),
                    is_main: worktree.is_main,
                });

                if !worktree.wt_status.is_clean() {
                    let changed: Vec<ChangedFile> = worktree
                        .wt_status
                        .changed_files
                        .iter()
                        .filter(|file| {
                            matches!(
                                file.kind,
                                ChangedFileKind::Modified | ChangedFileKind::Untracked
                            )
                        })
                        .cloned()
                        .collect();
                    let omitted = changed.len().saturating_sub(5);
                    risks.push(DeleteRisk::DirtyWorktree {
                        worktree: worktree.path.clone(),
                        files: changed.into_iter().take(5).collect(),
                        omitted,
                    });
                }
            }
        }

        DeletePreflight { risks }
    }

    pub(super) fn build_delete_choices(
        &self,
        targets: &[String],
        remote: Option<String>,
        preflight: &DeletePreflight,
    ) -> Vec<ConfirmChoice> {
        let (known_merged_targets, safe_targets): (Vec<String>, Vec<String>) =
            targets.iter().cloned().partition(|name| {
                // Checked-out branches keep the safe path; their `w` choice owns removal.
                let checked_out = preflight.risks.iter().any(|risk| {
                    matches!(risk, DeleteRisk::CheckedOut { branch, .. } if branch == name)
                });
                self.branch_is_known_merged(name) && !checked_out
            });

        // Force `y` precedes safe `y`: both share the accelerator, and Enter runs every
        // same-accelerator row in order, so the force group is enqueued first.
        let mut choices = Vec::new();
        if !known_merged_targets.is_empty() {
            choices.push(ConfirmChoice {
                accelerator: 'y',
                action: BranchAction::DeleteLocalForce,
                targets: known_merged_targets.clone(),
                remote: None,
                command: confirmation_command(
                    BranchAction::DeleteLocalForce,
                    &known_merged_targets,
                    None,
                ),
                description: "Delete; branch already merged into base (squash/cherry-pick)"
                    .into(),
                destructive_target: None,
            });
        }
        if !safe_targets.is_empty() || known_merged_targets.is_empty() {
            choices.push(ConfirmChoice {
                accelerator: 'y',
                action: BranchAction::DeleteLocal,
                targets: safe_targets.clone(),
                remote: remote.clone(),
                command: confirmation_command(
                    BranchAction::DeleteLocal,
                    &safe_targets,
                    remote.as_deref(),
                ),
                description: "Ask Git to delete only branches that are fully merged.".into(),
                destructive_target: None,
            });
        }

        let mut primary_worktree_targets = Vec::new();
        let mut dirty_worktrees = Vec::new();
        for risk in &preflight.risks {
            match risk {
                DeleteRisk::CheckedOut {
                    branch,
                    is_main: true,
                    ..
                } => push_unique(&mut primary_worktree_targets, branch),
                DeleteRisk::DirtyWorktree { worktree, .. } => {
                    dirty_worktrees.push(worktree.clone());
                }
                _ => {}
            }
        }

        for branch in preflight.risks.iter().filter_map(|risk| match risk {
            DeleteRisk::UniqueCommits { branch, .. }
                if !primary_worktree_targets.contains(branch) =>
            {
                Some(branch)
            }
            _ => None,
        }) {
            let target = ConfirmTarget::Branch(branch.clone());
            let job_targets = vec![target.job_target()];
            choices.push(ConfirmChoice {
                accelerator: 'r',
                action: BranchAction::DeleteLocalForce,
                targets: job_targets.clone(),
                remote: None,
                command: confirmation_command(BranchAction::DeleteLocalForce, &job_targets, None),
                description: "Open final destructive confirmation for this branch.".into(),
                destructive_target: Some(target),
            });
        }

        for (branch, worktree) in preflight.risks.iter().filter_map(|risk| match risk {
            DeleteRisk::CheckedOut {
                branch,
                worktree,
                is_main: false,
            } => Some((branch, worktree)),
            _ => None,
        }) {
            let force = preflight.risks.iter().any(|risk| {
                matches!(risk, DeleteRisk::UniqueCommits { branch: risky_branch, .. } if risky_branch == branch)
            }) || self.branch_is_known_merged(branch)
                || dirty_worktrees.contains(worktree);
            let action = if force {
                BranchAction::DeleteBranchAndRemoveWorktreeForce
            } else {
                BranchAction::DeleteBranchAndRemoveWorktree
            };
            let target = ConfirmTarget::BranchWorktree {
                branch: branch.clone(),
                worktree: worktree.clone(),
            };
            choices.push(ConfirmChoice {
                accelerator: 'w',
                action,
                targets: vec![target.job_target()],
                remote: None,
                command: confirmation_command_for_target(action, &target, None),
                description: "Open final confirmation for exactly this branch/worktree pair."
                    .into(),
                destructive_target: Some(target),
            });
        }

        choices
    }

    pub(super) fn branch_is_known_merged(&self, name: &str) -> bool {
        self.branches
            .items()
            .iter()
            .find(|branch| branch.name == name)
            .is_some_and(|branch| is_known_merged(branch.merge_status))
    }
}

// Plain `git branch -d` cannot see squash or cherry-pick merges, so these need `-D`.
fn is_known_merged(status: MergeStatus) -> bool {
    matches!(
        status,
        MergeStatus::SquashMerged
            | MergeStatus::LocalSquashMerged
            | MergeStatus::RemoteSquashMerged
            | MergeStatus::CherryPicked
            | MergeStatus::LocalCherryPicked
            | MergeStatus::RemoteCherryPicked
    )
}
