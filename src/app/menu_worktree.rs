use super::*;

impl App {
    pub(super) fn build_worktree_menu(&self) -> Vec<MenuItem> {
        let Some(wt) = self.worktrees.cursor_item() else {
            return vec![];
        };
        let is_main = wt.is_main;
        let is_dirty = !wt.wt_status.is_clean();
        let is_detached = wt.branch.is_none();
        let is_base = wt.is_base;
        let can_delete_branch = !is_main && !is_dirty && !is_detached && !is_base;
        let delete_branch_reason = if is_main {
            Some("main worktree".into())
        } else if is_dirty {
            Some("dirty".into())
        } else if is_detached {
            Some("detached HEAD".into())
        } else if is_base {
            Some("base branch".into())
        } else {
            None
        };
        let has_remote = wt.branch.as_deref().is_some_and(|name| {
            self.branches.items().iter().any(|b| {
                b.name == name && matches!(b.tracking, TrackingStatus::Tracked { gone: false, .. })
            })
        });
        let can_delete_branch_remote = can_delete_branch && has_remote;
        let delete_branch_remote_reason = delete_branch_reason
            .clone()
            .or((!has_remote).then(|| "no remote".into()));
        let target = wt.path.to_string_lossy().to_string();
        let graph_target = wt.branch.clone().unwrap_or_else(|| wt.commit_hash.clone());

        vec![
            MenuItem {
                label: "Jump to Graph".into(),
                enabled: true,
                reason: None,
                shortcut: Some('g'),
                action: BranchAction::JumpToGraph,
                target: graph_target,
                remote: None,
            },
            MenuItem {
                label: BranchAction::CheckPrimaryBranchCode.label().into(),
                enabled: true,
                reason: None,
                shortcut: None,
                action: BranchAction::CheckPrimaryBranchCode,
                target: wt
                    .branch
                    .as_ref()
                    .map(|branch| format!("refs/heads/{branch}"))
                    .unwrap_or_else(|| wt.commit_hash.clone()),
                remote: None,
            },
            MenuItem {
                label: "Remove worktree".into(),
                enabled: !is_main && !is_dirty,
                reason: if is_main {
                    Some("main worktree".into())
                } else if is_dirty {
                    Some("dirty".into())
                } else {
                    None
                },
                shortcut: Some('d'),
                action: BranchAction::WorktreeRemove,
                target: target.clone(),
                remote: None,
            },
            MenuItem {
                label: "Force remove worktree".into(),
                enabled: !is_main,
                reason: if is_main {
                    Some("main worktree".into())
                } else {
                    None
                },
                shortcut: Some('D'),
                action: BranchAction::WorktreeForceRemove,
                target: target.clone(),
                remote: None,
            },
            MenuItem {
                label: "Remove worktree + branch".into(),
                enabled: can_delete_branch,
                reason: delete_branch_reason.clone(),
                shortcut: Some('b'),
                action: BranchAction::WorktreeRemoveAndDeleteBranch,
                target: target.clone(),
                remote: None,
            },
            MenuItem {
                label: "Remove worktree + branch (local + remote)".into(),
                enabled: can_delete_branch_remote,
                reason: delete_branch_remote_reason,
                shortcut: Some('B'),
                action: BranchAction::WorktreeRemoveAndDeleteBranchRemote,
                target,
                remote: None,
            },
        ]
    }

    pub(super) fn execute_menu_action(&mut self, item: MenuItem) {
        let action = item.action;
        if action == BranchAction::CheckPrimaryBranchCode {
            self.start_primary_code_check(item.target);
            return;
        }
        if action == BranchAction::JumpToGraph {
            self.jump_to_graph(item.target, self.return_view == ViewId::Remotes);
            return;
        }

        // View PR -- fire and forget, no confirm
        if action == BranchAction::ViewRemotePR {
            let name = item.target;
            let repo_path = self.repo_path.clone();
            std::thread::spawn(move || {
                let _ = std::process::Command::new("gh")
                    .args(["pr", "view", "--web", &name])
                    .current_dir(&repo_path)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn();
            });
            return;
        }

        if action == BranchAction::CherryPickLocal {
            let head = self.branches.items().iter().find(|branch| branch.is_current);
            let head_is_base =
                head.is_some_and(|branch| branch.is_base || branch.name == self.base_branch);
            if !head_is_base {
                let head = head.map(|branch| branch.name.clone());
                let description = format!(
                    "Apply the latest commit of {} onto {}.",
                    item.target,
                    head.as_deref().unwrap_or("detached HEAD")
                );
                let preflight = DeletePreflight {
                    risks: vec![DeleteRisk::CherryPickTarget {
                        branch: item.target.clone(),
                        head,
                        base: self.base_branch.clone(),
                    }],
                };
                let choices = vec![ConfirmChoice {
                    accelerator: 'y',
                    action,
                    targets: vec![item.target.clone()],
                    remote: None,
                    command: format!("git cherry-pick refs/heads/{}", item.target),
                    description,
                    destructive_target: None,
                }];
                self.open_confirm_with_choices(
                    self.return_view,
                    preflight,
                    choices,
                    ConfirmStage::Initial,
                );
                return;
            }
        }

        let targets = vec![item.target];
        if action == BranchAction::DeleteLocal {
            let preflight = self.build_delete_preflight(&targets);
            let choices = self.build_delete_choices(&targets, item.remote, &preflight);
            self.open_confirm_with_choices(
                self.return_view,
                preflight,
                choices,
                ConfirmStage::Initial,
            );
        } else {
            self.open_confirm_with_remote(action, self.return_view, targets, item.remote);
        }
    }
}
