use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::app::App;
use git_branch_manager::types::{MergeStatus, TrackingStatus};
use super::theme;

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(area);

    let main_area = layout[0];
    let status_area = layout[1];

    // Main branch list
    let title = format!("git-branch-manager \u{2014} base: {}", app.base_branch);
    let block = Block::default()
        .title(title)
        .title_style(theme::TITLE_STYLE)
        .borders(Borders::ALL);

    let inner_height = block.inner(main_area).height as usize;

    // Compute scroll offset to keep cursor visible
    let scroll_offset = {
        let mut offset = app.list_scroll_offset;
        if inner_height > 0 {
            if app.cursor < offset {
                offset = app.cursor;
            }
            if app.cursor >= offset + inner_height {
                offset = app.cursor - inner_height + 1;
            }
        }
        offset
    };

    let items: Vec<ListItem> = app
        .branches
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(inner_height)
        .map(|(i, branch)| {
            let checkbox = if app.selected[i] { "[x]" } else { "[ ]" };

            let name_style = if branch.is_current {
                Style::default().add_modifier(Modifier::BOLD)
            } else if app.selected[i] {
                theme::SELECTED_STYLE
            } else {
                Style::default()
            };

            let tracking_text = match &branch.tracking {
                TrackingStatus::Tracked { remote_ref, gone } => {
                    if *gone {
                        "gone".to_string()
                    } else {
                        remote_ref.clone()
                    }
                }
                TrackingStatus::Local => "(local)".to_string(),
            };

            let age = branch.age_display();

            let (status_text, status_style) = match branch.merge_status {
                MergeStatus::Merged => ("merged", theme::MERGED_STYLE),
                MergeStatus::SquashMerged => ("squash-merged", theme::SQUASH_MERGED_STYLE),
                MergeStatus::Unmerged => ("unmerged", theme::UNMERGED_STYLE),
            };

            let current_marker = if branch.is_current { "* " } else { "  " };
            let base_marker = if branch.is_base { " [base]" } else { "" };

            let cursor_prefix = if i == app.cursor { "> " } else { "  " };

            let line = Line::from(vec![
                Span::raw(cursor_prefix),
                Span::styled(
                    format!("{} {}{}", checkbox, current_marker, branch.name),
                    name_style,
                ),
                Span::styled(base_marker, theme::DIM_STYLE),
                Span::raw("  "),
                Span::styled(tracking_text, theme::DIM_STYLE),
                Span::raw("  "),
                Span::styled(age, theme::DIM_STYLE),
                Span::raw("  "),
                Span::styled(status_text, status_style),
            ]);

            let mut item = ListItem::new(line);
            if i == app.cursor {
                item = item.style(theme::CURSOR_STYLE);
            }
            item
        })
        .collect();

    let list = List::new(items).block(block);
    frame.render_widget(list, main_area);

    // Status bar
    let selected_count = app.selection_count();
    let total = app.branches.len();
    let status_text = format!(
        " {}/{} selected \u{2014} [d]elete [D]el+remote [?]help [q]uit",
        selected_count, total
    );
    let status = Paragraph::new(status_text).style(theme::STATUS_BAR_STYLE);
    frame.render_widget(status, status_area);
}
