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
    let wt_status = app.working_tree_status.summary();
    let title = format!(
        "git-branch-manager \u{2014} base: {} [{}]",
        app.base_branch, wt_status
    );
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
            let is_cursor = i == app.cursor;
            let is_selected = app.selected[i];

            let checkbox = if is_selected { "[x]" } else { "[ ]" };
            let checkbox_style = if is_selected {
                theme::SELECTED_STYLE
            } else {
                theme::SECONDARY_TEXT
            };

            let name_style = if branch.is_current {
                theme::CURRENT_BRANCH_STYLE
            } else if is_selected {
                theme::SELECTED_STYLE
            } else {
                theme::PRIMARY_TEXT
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

            let ahead_behind = match (branch.ahead, branch.behind) {
                (Some(a), Some(b)) if a > 0 || b > 0 => {
                    let mut parts = Vec::new();
                    if a > 0 {
                        parts.push(format!("\u{2191}{}", a));
                    }
                    if b > 0 {
                        parts.push(format!("\u{2193}{}", b));
                    }
                    parts.join("")
                }
                _ => String::new(),
            };

            let age = branch.age_display();

            let (status_text, status_style) = match branch.merge_status {
                MergeStatus::Merged => ("merged", theme::MERGED_STYLE),
                MergeStatus::SquashMerged => ("squash-merged", theme::SQUASH_MERGED_STYLE),
                MergeStatus::Unmerged => ("unmerged", theme::UNMERGED_STYLE),
            };

            let current_marker = if branch.is_current { "* " } else { "  " };
            let base_marker = if branch.is_base { " [base]" } else { "" };

            let cursor_prefix = if is_cursor { "> " } else { "  " };
            let cursor_prefix_style = if is_cursor {
                theme::CURSOR_PREFIX_STYLE
            } else {
                Style::default()
            };

            let mut spans = vec![
                Span::styled(cursor_prefix, cursor_prefix_style),
                Span::styled(format!("{} ", checkbox), checkbox_style),
                Span::styled(format!("{}{}", current_marker, branch.name), name_style),
                Span::styled(base_marker, theme::SECONDARY_TEXT),
                Span::raw("  "),
                Span::styled(tracking_text, theme::SECONDARY_TEXT),
            ];
            if !ahead_behind.is_empty() {
                spans.push(Span::raw(" "));
                spans.push(Span::styled(ahead_behind, theme::AHEAD_BEHIND_STYLE));
            }
            spans.extend([
                Span::raw("  "),
                Span::styled(age, theme::SECONDARY_TEXT),
                Span::raw("  "),
                Span::styled(status_text, status_style),
            ]);
            let line = Line::from(spans);

            let mut item = ListItem::new(line);
            if is_cursor {
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
    let loading = if app.squash_rx.is_some() { " [loading\u{2026}]" } else { "" };
    let status_text = format!(
        " {}/{} selected{} \u{2014} [d]elete [D]el+remote [?]help [q]uit",
        selected_count, total, loading
    );
    let status = Paragraph::new(status_text).style(theme::STATUS_BAR_STYLE);
    frame.render_widget(status, status_area);
}
