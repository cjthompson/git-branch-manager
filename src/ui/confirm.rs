use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::app::{App, View};

pub fn draw(frame: &mut Frame, app: &App) {
    let View::Confirm { action } = &app.view else {
        return;
    };

    let action_label = action.label();

    // Build content: Tag operations show the tag name(s); cursor-branch operations show the cursor branch; bulk operations show selected branches
    let is_tag_action = matches!(
        action,
        git_branch_manager::types::BranchAction::DeleteTag
            | git_branch_manager::types::BranchAction::DeleteTagAndRemote
            | git_branch_manager::types::BranchAction::PushTag
    );
    let is_cursor_action = matches!(
        action,
        git_branch_manager::types::BranchAction::Checkout
            | git_branch_manager::types::BranchAction::FastForward
            | git_branch_manager::types::BranchAction::Merge
            | git_branch_manager::types::BranchAction::SquashMerge
            | git_branch_manager::types::BranchAction::Rebase
            | git_branch_manager::types::BranchAction::Worktree
    );
    let mut lines = if is_tag_action {
        let target_names = app.target_tag_names();
        let count = target_names.len();
        let mut l = vec![
            Line::from(format!("{} {} tag(s)?", action_label, count)),
            Line::from(""),
        ];
        for name in &target_names {
            l.push(Line::from(Span::styled(
                format!("  {}", name),
                app.theme.selected,
            )));
        }
        l
    } else if is_cursor_action {
        let cursor_name = &app.branches[app.cursor].name;
        vec![
            Line::from(format!("{} branch?", action_label)),
            Line::from(""),
            Line::from(Span::styled(
                format!("  {}", cursor_name),
                app.theme.selected,
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
                app.theme.selected,
            )));
        }
        l
    };
    lines.push(Line::from(""));
    let key_style = Style::default().fg(app.theme.title.fg.unwrap_or(Color::White));
    lines.push(Line::from(vec![
        Span::styled("[", app.theme.dim),
        Span::styled("y", key_style),
        Span::styled("]es  [", app.theme.dim),
        Span::styled("n", key_style),
        Span::styled("]o", app.theme.dim),
    ]));

    // Calculate overlay size
    let area = frame.area();
    let max_height = (area.height * 60 / 100).max(8);
    let inner_max = max_height.saturating_sub(2) as usize; // subtract borders

    // If content exceeds available space, truncate the branch list and add "...N more"
    if lines.len() > inner_max {
        // Find how many branch name lines we can keep.
        // Layout: header lines at top, then branch names, then blank + [y]es [n]o at bottom.
        // The last 2 lines are always: blank line + "[y]es  [n]o"
        // We also need 1 line for the "...N more" indicator.
        // So available for branch names = inner_max - (non-branch lines count) - 1 (for "...N more")
        let footer_lines = 2; // blank + yes/no
        let header_lines = 2; // action question + blank
        let available_for_branches = inner_max.saturating_sub(header_lines + footer_lines + 1);

        // Rebuild: keep header, truncated branch list, "...N more", footer
        let total_branches = lines.len() - header_lines - footer_lines;
        let hidden = total_branches.saturating_sub(available_for_branches);

        if hidden > 0 {
            // Remove footer (last 2 lines)
            let footer: Vec<Line> = lines.split_off(lines.len() - 2);
            // Truncate branch list: keep header + available_for_branches items
            lines.truncate(header_lines + available_for_branches);
            // Add "...N more"
            lines.push(Line::from(Span::styled(
                format!("  ...{} more", hidden),
                app.theme.dim,
            )));
            // Re-add footer
            lines.extend(footer);
        }
    }

    let content_height = lines.len() as u16 + 2; // +2 for borders
    let height = content_height.min(max_height).min(area.height);
    let width = (area.width / 2).max(40).min(area.width);

    let rect = centered_rect(width, height, area);

    let block = Block::default()
        .title(format!("Confirm {}", action_label))
        .title_style(app.theme.title)
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
