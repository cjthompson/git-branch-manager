use super::*;

pub(super) fn first_enabled_index(items: &[MenuItem]) -> Option<usize> {
    items.iter().position(|item| item.enabled)
}

pub(super) fn previous_enabled_index(items: &[MenuItem], cursor: usize) -> Option<usize> {
    items
        .get(..cursor.min(items.len()))
        .and_then(|preceding| preceding.iter().rposition(|item| item.enabled))
}

pub(super) fn next_enabled_index(items: &[MenuItem], cursor: usize) -> Option<usize> {
    items
        .iter()
        .enumerate()
        .skip(cursor.saturating_add(1))
        .find_map(|(index, item)| item.enabled.then_some(index))
}

pub(super) fn normalize_info_action_state(
    cursor: usize,
    focus: InfoModalFocus,
    items: &[MenuItem],
) -> (usize, InfoModalFocus) {
    match first_enabled_index(items) {
        Some(first_enabled) => {
            let cursor = items
                .get(cursor)
                .filter(|item| item.enabled)
                .map_or(first_enabled, |_| cursor);
            (cursor, focus)
        }
        None => (0, InfoModalFocus::Info),
    }
}

pub(super) fn push_unique(items: &mut Vec<String>, item: &str) {
    if !items.iter().any(|existing| existing == item) {
        items.push(item.to_string());
    }
}

pub(super) fn requires_final_confirmation(action: BranchAction) -> bool {
    matches!(
        action,
        BranchAction::DeleteLocalForce
            | BranchAction::WorktreeForceRemove
            | BranchAction::DeleteBranchAndRemoveWorktreeForce
    )
}

pub(super) fn confirmation_command(action: BranchAction, targets: &[String], remote: Option<&str>) -> String {
    let targets = targets.join(" ");
    match action {
        BranchAction::DeleteLocal => format!("git branch -d -- {targets}"),
        BranchAction::DeleteLocalForce => format!("git branch -D -- {targets}"),
        BranchAction::WorktreeRemove => format!("git worktree remove -- {targets}"),
        BranchAction::WorktreeForceRemove => {
            format!("git worktree remove --force -- {targets}")
        }
        BranchAction::DeleteBranchAndRemoveWorktree => {
            format!("git worktree remove + git branch -d -- {targets}")
        }
        BranchAction::DeleteBranchAndRemoveWorktreeForce => {
            format!("git worktree remove --force + git branch -D -- {targets}")
        }
        BranchAction::DeleteRemoteBranch => remote.map_or_else(
            || format!("Delete remote branch {targets}"),
            |remote| format!("git push {remote} --delete -- {targets}"),
        ),
        BranchAction::CherryPickCommit => format!("git cherry-pick {targets}"),
        _ => format!("{} — {targets}", action.label()),
    }
}

pub(super) fn confirmation_command_for_target(
    action: BranchAction,
    target: &ConfirmTarget,
    remote: Option<&str>,
) -> String {
    match (action, target) {
        (BranchAction::DeleteLocalForce, ConfirmTarget::Branch(branch)) => {
            format!("git branch -D -- {branch}")
        }
        (BranchAction::WorktreeForceRemove, ConfirmTarget::Worktree(worktree)) => {
            format!("git worktree remove --force -- {}", worktree.display())
        }
        (
            BranchAction::DeleteBranchAndRemoveWorktree,
            ConfirmTarget::BranchWorktree { branch, worktree },
        ) => format!(
            "git worktree remove -- {} + git branch -d -- {branch}",
            worktree.display()
        ),
        (
            BranchAction::DeleteBranchAndRemoveWorktreeForce,
            ConfirmTarget::BranchWorktree { branch, worktree },
        ) => format!(
            "git worktree remove --force -- {} + git branch -D -- {branch}",
            worktree.display()
        ),
        _ => confirmation_command(action, &[target.job_target()], remote),
    }
}

pub(super) fn graph_affected_views(action: BranchAction) -> &'static [ViewId] {
    // P110: every action that mutates repository state (refs or OIDs) should
    // refresh the Graph view. UI-only and read-only actions stay empty.
    match action {
        BranchAction::JumpToGraph
        | BranchAction::ViewRemotePR
        | BranchAction::CheckPrimaryBranchCode => &[],
        BranchAction::DeleteLocal
        | BranchAction::DeleteLocalAndRemote
        | BranchAction::DeleteLocalForce
        | BranchAction::DeleteRemoteBranch
        | BranchAction::DeleteRemoteAndLocal => {
            &[ViewId::Branches, ViewId::Remotes, ViewId::Graph]
        }
        BranchAction::Checkout
        | BranchAction::FastForward
        | BranchAction::Merge
        | BranchAction::SquashMerge
        | BranchAction::Rebase => &[ViewId::Branches, ViewId::Worktrees, ViewId::Graph],
        BranchAction::Fetch | BranchAction::FetchPrune => {
            &[ViewId::Branches, ViewId::Remotes, ViewId::Tags, ViewId::Graph]
        }
        BranchAction::Push | BranchAction::ForcePush => {
            &[ViewId::Branches, ViewId::Remotes, ViewId::Graph]
        }
        BranchAction::Pull => {
            &[ViewId::Branches, ViewId::Remotes, ViewId::Worktrees, ViewId::Graph]
        }
        BranchAction::Worktree => &[ViewId::Worktrees, ViewId::Graph],
        BranchAction::DeleteTag | BranchAction::DeleteTagAndRemote | BranchAction::PushTag => {
            &[ViewId::Tags, ViewId::Graph]
        }
        BranchAction::CheckoutRemote => {
            &[ViewId::Branches, ViewId::Remotes, ViewId::Worktrees, ViewId::Graph]
        }
        BranchAction::FetchRemote => {
            &[ViewId::Branches, ViewId::Remotes, ViewId::Tags, ViewId::Graph]
        }
        BranchAction::PullRemote
        | BranchAction::MergeRemoteIntoCurrent
        | BranchAction::CherryPickRemote
        | BranchAction::CherryPickLocal
        | BranchAction::CherryPickCommit => {
            &[ViewId::Branches, ViewId::Remotes, ViewId::Worktrees, ViewId::Graph]
        }
        BranchAction::WorktreeRemove | BranchAction::WorktreeForceRemove => {
            &[ViewId::Worktrees, ViewId::Graph]
        }
        BranchAction::WorktreeRemoveAndDeleteBranch
        | BranchAction::WorktreeRemoveAndDeleteBranchRemote
        | BranchAction::DeleteBranchAndRemoveWorktree
        | BranchAction::DeleteBranchAndRemoveWorktreeForce
        | BranchAction::DeleteBranchAndRemoveWorktreeRemote => {
            &[ViewId::Branches, ViewId::Remotes, ViewId::Worktrees, ViewId::Graph]
        }
    }
}
