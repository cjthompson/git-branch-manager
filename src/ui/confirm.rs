use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::app::{App, View};
use super::theme;

pub fn draw(frame: &mut Frame, app: &App) {
    let View::Confirm { action } = &app.view else {
        return;
    };

    let action_label = action.label();

    // Build content: Cursor-branch operations show the cursor branch; bulk operations show selected branches
    let is_cursor_action = matches!(
        action,
        git_branch_manager::types::BranchAction::Checkout
            | git_branch_manager::types::BranchAction::FastForward
            | git_branch_manager::types::BranchAction::Merge
            | git_branch_manager::types::BranchAction::SquashMerge
            | git_branch_manager::types::BranchAction::Rebase
    );
    let mut lines = if is_cursor_action {
        let cursor_name = &app.branches[app.cursor].name;
        vec![
            Line::from(format!("{} branch?", action_label)),
            Line::from(""),
            Line::from(Span::styled(
                format!("  {}", cursor_name),
                theme::SELECTED_STYLE,
            )),
        ]
    } else {
        let target_names = app.target_branch_names();
        let count = target_names.len();
        let mut l = vec![
            Line::from(format!("{} {} branch(es)?", action_label, count)),
            Line::from(""),
        ];
        for name in &target_names {
            l.push(Line::from(Span::styled(
                format!("  {}", name),
                theme::SELECTED_STYLE,
            )));
        }
        l
    };
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "[y]es  [n]o",
        theme::DIM_STYLE,
    )));

    // Calculate overlay size
    let area = frame.area();
    let content_height = lines.len() as u16 + 2; // +2 for borders
    let max_height = (area.height * 60 / 100).max(8);
    let height = content_height.min(max_height).min(area.height);
    let width = (area.width / 2).max(40).min(area.width);

    let rect = centered_rect(width, height, area);

    let block = Block::default()
        .title(format!("Confirm {}", action_label))
        .title_style(theme::TITLE_STYLE)
        .borders(Borders::ALL);

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });

    frame.render_widget(Clear, rect);
    frame.render_widget(paragraph, rect);
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}
