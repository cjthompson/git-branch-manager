use ratatui::Frame;

use crate::app::{App, View};
use super::{branch_list, confirm, executing, filter, help, menu, remote_branch_list, results, settings, tag_list, worktree_list};

pub fn draw(frame: &mut Frame, app: &mut App) {
    match &app.view {
        View::BranchList => branch_list::draw(frame, app),
        View::Confirm { action } => {
            let is_tag_action = matches!(
                action,
                git_branch_manager::types::BranchAction::DeleteTag
                    | git_branch_manager::types::BranchAction::DeleteTagAndRemote
                    | git_branch_manager::types::BranchAction::PushTag
            );
            let is_remote_action = matches!(
                action,
                git_branch_manager::types::BranchAction::DeleteRemoteBranch
                    | git_branch_manager::types::BranchAction::CheckoutRemote
            );
            let is_worktree_action = matches!(
                action,
                git_branch_manager::types::BranchAction::WorktreeRemove
                    | git_branch_manager::types::BranchAction::WorktreeForceRemove
            );
            if is_tag_action {
                tag_list::draw(frame, app);
            } else if is_remote_action {
                remote_branch_list::draw(frame, app);
            } else if is_worktree_action {
                worktree_list::draw(frame, app);
            } else {
                branch_list::draw(frame, app);
            }
            confirm::draw(frame, &*app);
        }
        View::Executing => {
            branch_list::draw(frame, app);
            executing::draw(frame, &*app);
        }
        View::Results => results::draw(frame, &*app),
        View::Help => {
            branch_list::draw(frame, app);
            help::draw(frame, &*app);
        }
        View::Menu { cursor } => {
            let menu_cursor = *cursor;
            if app.prev_view == View::Worktrees {
                worktree_list::draw(frame, app);
                let items = app.build_worktree_menu_items();
                let anchor_row =
                    (app.worktree_cursor as u16).saturating_sub(app.worktree_table_state.offset() as u16) + 2;
                menu::draw(frame, &items, menu_cursor, anchor_row, &app.theme, app.symbols);
            } else {
                branch_list::draw(frame, app);
                let items = app.build_menu_items();
                // Calculate anchor row based on table cursor position
                // +2 accounts for the border and header row
                let anchor_row =
                    (app.cursor as u16).saturating_sub(app.table_state.offset() as u16) + 2;
                menu::draw(frame, &items, menu_cursor, anchor_row, &app.theme, app.symbols);
            }
        }
        View::Tags => tag_list::draw(frame, app),
        View::Settings { .. } => {
            branch_list::draw(frame, app);
            settings::draw(frame, app);
        }
        View::Filter => {
            branch_list::draw(frame, app);
            filter::draw(frame, &*app);
        }
        View::TagFilter => {
            tag_list::draw(frame, app);
            filter::draw_tag_filter(frame, &*app);
        }
        View::RemoteBranches => remote_branch_list::draw(frame, app),
        View::RemoteFilter => {
            remote_branch_list::draw(frame, app);
            filter::draw_remote_filter(frame, &*app);
        }
        View::Worktrees => worktree_list::draw(frame, app),
    }
}
