use ratatui::Frame;

use crate::app::{App, View};
use super::{branch_list, confirm, executing, help, menu, results, tag_list};

pub fn draw(frame: &mut Frame, app: &mut App) {
    match &app.view {
        View::BranchList => branch_list::draw(frame, app),
        View::Confirm { action } => {
            let is_tag_action = matches!(
                action,
                git_branch_manager::types::BranchAction::DeleteTag
                    | git_branch_manager::types::BranchAction::PushTag
            );
            if is_tag_action {
                tag_list::draw(frame, app);
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
            branch_list::draw(frame, app);
            let items = app.build_menu_items();
            // Calculate anchor row based on table cursor position
            // +2 accounts for the border and header row
            let anchor_row =
                (app.cursor as u16).saturating_sub(app.table_state.offset() as u16) + 2;
            menu::draw(frame, &items, menu_cursor, anchor_row, &app.theme);
        }
        View::Tags => tag_list::draw(frame, app),
    }
}
