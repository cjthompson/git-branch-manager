use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};

use crate::app::App;
use git_branch_manager::types::{MergeStatus, TrackingStatus};
use super::theme;

pub fn draw(frame: &mut Frame, app: &mut App) {
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

    // Header row
    let header = Row::new(vec![
        Cell::from(""),
        Cell::from("Branch"),
        Cell::from("Age"),
        Cell::from("A/B"),
        Cell::from("Status"),
    ])
    .style(theme::HEADER_STYLE)
    .bottom_margin(0);

    // Build table rows
    let rows: Vec<Row> = app
        .branches
        .iter()
        .enumerate()
        .map(|(i, branch)| {
            let is_selected = app.selected[i];

            // Checkbox column
            let (checkbox_text, checkbox_style) = if branch.is_base || branch.is_current {
                ("   ", Style::default())
            } else if is_selected {
                ("[x]", theme::SELECTED_STYLE)
            } else {
                ("[ ]", theme::SECONDARY_TEXT)
            };

            // Branch name column
            let current_marker = if branch.is_current { "* " } else { "  " };
            let base_suffix = if branch.is_base { " [base]" } else { "" };

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
                        " (gone)".to_string()
                    } else {
                        format!(" \u{2192} {}", remote_ref)
                    }
                }
                TrackingStatus::Local => " (local)".to_string(),
            };

            let name_cell = Cell::from(Line::from(vec![
                Span::styled(format!("{}{}", current_marker, branch.name), name_style),
                Span::styled(base_suffix, theme::SECONDARY_TEXT),
                Span::styled(tracking_text, theme::SECONDARY_TEXT),
            ]));

            // Age column
            let age = branch.age_display();
            let age_cell = Cell::from(Span::styled(age, theme::SECONDARY_TEXT));

            // Ahead/behind column
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
            let ab_cell = Cell::from(Span::styled(ahead_behind, theme::AHEAD_BEHIND_STYLE));

            // Status column
            let (status_text, status_style) = match branch.merge_status {
                MergeStatus::Merged => ("merged", theme::MERGED_STYLE),
                MergeStatus::SquashMerged => ("squash-merged", theme::SQUASH_MERGED_STYLE),
                MergeStatus::Unmerged => ("unmerged", theme::UNMERGED_STYLE),
            };
            let status_cell = Cell::from(Span::styled(status_text, status_style));

            Row::new(vec![
                Cell::from(Span::styled(checkbox_text, checkbox_style)),
                name_cell,
                age_cell,
                ab_cell,
                status_cell,
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(3),
        Constraint::Min(20),
        Constraint::Length(14),
        Constraint::Length(6),
        Constraint::Length(14),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .row_highlight_style(theme::CURSOR_STYLE)
        .highlight_symbol("> ");

    frame.render_stateful_widget(table, main_area, &mut app.table_state);

    // Status bar
    let selected_count = app.selection_count();
    let total = app.branches.len();
    let loading = if app.squash_rx.is_some() { " [loading\u{2026}]" } else { "" };
    let status_text = format!(
        " {}/{} selected{} \u{2014} [c]heckout [d]elete [D]el+remote [?]help [q]uit",
        selected_count, total, loading
    );
    let status = Paragraph::new(status_text).style(theme::STATUS_BAR_STYLE);
    frame.render_widget(status, status_area);
}
