use super::*;

impl App {
    pub(super) fn build_remote_menu(&self) -> Vec<MenuItem> {
        let Some(branch) = self.remotes.cursor_item() else {
            return vec![];
        };
        self.build_remote_menu_for(branch)
    }

    pub(super) fn build_remote_menu_for(&self, branch: &RemoteBranchInfo) -> Vec<MenuItem> {
        let pinned = branch.is_pinned();
        let has_local = branch.has_local;
        let has_pr = self.pr_map.contains_key(&branch.short_name);

        vec![
            MenuItem {
                label: "Jump to Graph".into(),
                enabled: true,
                reason: None,
                shortcut: Some('g'),
                action: BranchAction::JumpToGraph,
                target: branch.full_ref.clone(),
                remote: None,
            },
            MenuItem {
                label: BranchAction::CheckPrimaryBranchCode.label().into(),
                enabled: true,
                reason: None,
                shortcut: None,
                action: BranchAction::CheckPrimaryBranchCode,
                target: format!("refs/remotes/{}", branch.full_ref),
                remote: None,
            },
            MenuItem {
                label: "Checkout".into(),
                enabled: !pinned && !has_local,
                reason: if pinned {
                    Some("base".into())
                } else if has_local {
                    Some("local exists".into())
                } else {
                    None
                },
                shortcut: Some('c'),
                action: BranchAction::CheckoutRemote,
                target: branch.short_name.clone(),
                remote: Some(branch.remote.clone()),
            },
            MenuItem {
                label: "Delete remote branch".into(),
                enabled: !pinned,
                reason: if pinned { Some("base".into()) } else { None },
                shortcut: Some('d'),
                action: BranchAction::DeleteRemoteBranch,
                target: branch.short_name.clone(),
                remote: Some(branch.remote.clone()),
            },
            MenuItem {
                label: "Delete remote + local".into(),
                enabled: !pinned && has_local,
                reason: if pinned {
                    Some("base".into())
                } else if !has_local {
                    Some("no local".into())
                } else {
                    None
                },
                shortcut: Some('D'),
                action: BranchAction::DeleteRemoteAndLocal,
                target: branch.short_name.clone(),
                remote: Some(branch.remote.clone()),
            },
            MenuItem {
                label: "Fetch remote".into(),
                enabled: !pinned,
                reason: if pinned { Some("base".into()) } else { None },
                shortcut: Some('f'),
                action: BranchAction::FetchRemote,
                target: branch.short_name.clone(),
                remote: Some(branch.remote.clone()),
            },
            MenuItem {
                label: "Pull remote".into(),
                enabled: !pinned && has_local,
                reason: if pinned {
                    Some("base".into())
                } else if !has_local {
                    Some("no local".into())
                } else {
                    None
                },
                shortcut: Some('l'),
                action: BranchAction::PullRemote,
                target: branch.short_name.clone(),
                remote: Some(branch.remote.clone()),
            },
            MenuItem {
                label: "Merge into current".into(),
                enabled: !pinned,
                reason: if pinned { Some("base".into()) } else { None },
                shortcut: Some('m'),
                action: BranchAction::MergeRemoteIntoCurrent,
                target: branch.short_name.clone(),
                remote: Some(branch.remote.clone()),
            },
            MenuItem {
                label: "Cherry-pick latest".into(),
                enabled: !pinned,
                reason: if pinned { Some("base".into()) } else { None },
                shortcut: Some('p'),
                action: BranchAction::CherryPickRemote,
                target: branch.short_name.clone(),
                remote: Some(branch.remote.clone()),
            },
            MenuItem {
                label: "View PR in browser".into(),
                enabled: has_pr && !pinned,
                reason: if pinned {
                    Some("base".into())
                } else if !has_pr {
                    Some("no PR".into())
                } else {
                    None
                },
                shortcut: Some('o'),
                action: BranchAction::ViewRemotePR,
                target: branch.short_name.clone(),
                remote: Some(branch.remote.clone()),
            },
        ]
    }

    pub(super) fn build_tag_menu(&self) -> Vec<MenuItem> {
        let Some(tag) = self.tags.cursor_item() else {
            return vec![];
        };
        self.build_tag_menu_for(tag)
    }

    pub(super) fn build_tag_menu_for(&self, tag: &TagInfo) -> Vec<MenuItem> {
        vec![
            MenuItem {
                label: "Jump to Graph".into(),
                enabled: true,
                reason: None,
                shortcut: Some('g'),
                action: BranchAction::JumpToGraph,
                target: tag.name.clone(),
                remote: None,
            },
            MenuItem {
                label: BranchAction::CheckPrimaryBranchCode.label().into(),
                enabled: true,
                reason: None,
                shortcut: None,
                action: BranchAction::CheckPrimaryBranchCode,
                target: format!("refs/tags/{}", tag.name),
                remote: None,
            },
            MenuItem {
                label: "Delete tag".into(),
                enabled: true,
                reason: None,
                shortcut: Some('d'),
                action: BranchAction::DeleteTag,
                target: tag.name.clone(),
                remote: None,
            },
            MenuItem {
                label: "Delete tag (local + remote)".into(),
                enabled: true,
                reason: None,
                shortcut: Some('D'),
                action: BranchAction::DeleteTagAndRemote,
                target: tag.name.clone(),
                remote: None,
            },
            MenuItem {
                label: "Push tag to remote".into(),
                enabled: true,
                reason: None,
                shortcut: Some('p'),
                action: BranchAction::PushTag,
                target: tag.name.clone(),
                remote: None,
            },
        ]
    }
}
