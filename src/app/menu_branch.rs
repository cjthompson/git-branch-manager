use super::*;

impl App {
    pub(super) fn build_branch_menu(&self) -> Vec<MenuItem> {
        let Some(branch) = self.branches.cursor_item() else {
            return vec![];
        };
        self.build_branch_menu_for(branch)
    }

    pub(super) fn build_branch_menu_for(&self, branch: &BranchInfo) -> Vec<MenuItem> {
        let has_remote = matches!(
            &branch.tracking,
            TrackingStatus::Tracked { gone: false, .. }
        );
        let tracking_remote = match &branch.tracking {
            TrackingStatus::Tracked { remote_ref, .. } => remote_ref
                .split_once('/')
                .map(|(remote, _)| remote.to_owned()),
            TrackingStatus::Local => None,
        };
        let has_pr = self.pr_map.contains_key(&branch.name);

        // While the background worktree loader hasn't finished, `worktree_presence_for`
        // can't be trusted: a branch may actually be checked out somewhere the app just
        // hasn't learned about yet, so a false "not checked out anywhere" could let an
        // unsafe operation through. Gate every capability that consults worktree state
        // behind "loading" until `self.worktrees.loading` clears. Push/Force push take
        // no worktree parameter at all (they never needed it — see
        // `git/capability.rs::can_push`/`can_force_push`), so they are left ungated, and
        // `Open PR in browser` is untouched per #094's scope.
        let loading = self.worktrees.loading;
        let gate = |cap: capability::Capability| {
            if loading {
                capability::Capability::no("loading")
            } else {
                cap
            }
        };

        vec![
            MenuItem {
                label: "Jump to Graph".into(),
                enabled: true,
                reason: None,
                shortcut: Some('g'),
                action: BranchAction::JumpToGraph,
                target: branch.name.clone(),
                remote: None,
            },
            MenuItem {
                label: BranchAction::CheckPrimaryBranchCode.label().into(),
                enabled: true,
                reason: None,
                shortcut: None,
                action: BranchAction::CheckPrimaryBranchCode,
                target: format!("refs/heads/{}", branch.name),
                remote: None,
            },
            self.row_from(
                "Checkout",
                Some('c'),
                BranchAction::Checkout,
                branch.name.clone(),
                None,
                gate(capability::can_checkout_local(
                    branch,
                    self.worktree_presence_for(&branch.name),
                )),
            ),
            self.row_from(
                "Delete local",
                Some('d'),
                BranchAction::DeleteLocal,
                branch.name.clone(),
                None,
                gate(capability::can_delete_local(
                    branch,
                    self.worktree_presence_for(&branch.name),
                )),
            ),
            self.row_from(
                "Delete local + remote",
                Some('D'),
                BranchAction::DeleteLocalAndRemote,
                branch.name.clone(),
                tracking_remote,
                gate(capability::can_delete_local_and_remote(
                    branch,
                    self.worktree_presence_for(&branch.name),
                    has_remote,
                )),
            ),
            self.row_from(
                "Force-delete local",
                Some('!'),
                BranchAction::DeleteLocalForce,
                branch.name.clone(),
                None,
                gate(capability::can_force_delete_local(
                    branch,
                    self.worktree_presence_for(&branch.name),
                )),
            ),
            self.row_from(
                "Fast-forward",
                Some('f'),
                BranchAction::FastForward,
                branch.name.clone(),
                None,
                gate(capability::can_fast_forward(
                    branch,
                    self.worktree_presence_for(&branch.name),
                    has_remote,
                )),
            ),
            self.row_from(
                "Push",
                Some('p'),
                BranchAction::Push,
                branch.name.clone(),
                None,
                capability::can_push(branch, self.has_configured_remote, true),
            ),
            self.row_from(
                "Force push",
                Some('P'),
                BranchAction::ForcePush,
                branch.name.clone(),
                None,
                capability::can_force_push(branch, has_remote),
            ),
            self.row_from(
                "Pull",
                Some('l'),
                BranchAction::Pull,
                branch.name.clone(),
                None,
                gate(capability::can_pull(
                    branch,
                    self.worktree_presence_for(&branch.name),
                    has_remote,
                )),
            ),
            self.row_from(
                "Merge into base",
                Some('m'),
                BranchAction::Merge,
                branch.name.clone(),
                None,
                gate(capability::can_merge_into_base(
                    branch,
                    self.worktree_presence_for_branch_or_base(&branch.name, &self.base_branch),
                )),
            ),
            self.row_from(
                "Squash merge into base",
                Some('s'),
                BranchAction::SquashMerge,
                branch.name.clone(),
                None,
                gate(capability::can_squash_into_base(branch)),
            ),
            self.row_from(
                "Rebase onto base",
                Some('r'),
                BranchAction::Rebase,
                branch.name.clone(),
                None,
                gate(capability::can_rebase(
                    branch,
                    self.worktree_presence_for(&branch.name),
                    true,
                )),
            ),
            self.row_from(
                "Cherry-pick onto current",
                Some('C'),
                BranchAction::CherryPickLocal,
                branch.name.clone(),
                None,
                capability::can_cherry_pick_local(branch),
            ),
            self.row_from(
                "Create worktree",
                Some('w'),
                BranchAction::Worktree,
                branch.name.clone(),
                None,
                gate(capability::can_create_worktree(
                    branch,
                    self.worktree_presence_for(&branch.name),
                )),
            ),
            MenuItem {
                label: "Open PR in browser".into(),
                enabled: has_pr,
                reason: if !has_pr { Some("no PR".into()) } else { None },
                shortcut: Some('o'),
                action: BranchAction::ViewRemotePR,
                target: branch.name.clone(),
                remote: None,
            },
        ]
    }
}
