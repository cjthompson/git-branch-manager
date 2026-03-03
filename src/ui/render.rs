use ratatui::Frame;

use crate::app::{App, View};
use super::{branch_list, confirm, help, results};

pub fn draw(frame: &mut Frame, app: &App) {
    match &app.view {
        View::BranchList => branch_list::draw(frame, app),
        View::Confirm { .. } => {
            branch_list::draw(frame, app);
            confirm::draw(frame, app);
        }
        View::Results => results::draw(frame, app),
        View::Help => {
            branch_list::draw(frame, app);
            help::draw(frame, app);
        }
    }
}
