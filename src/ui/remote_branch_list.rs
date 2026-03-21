use chrono::{DateTime, Utc};
use crossterm::event::KeyCode;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table};

use crate::app::{App, View};
use git_branch_manager::types::MergeStatus;
use super::shared::tab_bar_line;

/// Returns a color style for known branch name prefixes (text before the first `/`).
fn prefix_style(prefix: &str) -> Option<Style> {
    match prefix {
        "fix" => Some(Style::new().fg(Color::Red)),
        "feat" | "feature" => Some(Style::new().fg(Color::Green)),
        "chore" => Some(Style::new().fg(Color::Indexed(130))),
        "hotfix" => Some(Style::new().fg(Color::Magenta)),
        "release" => Some(Style::new().fg(Color::Cyan)),
        _ => None,
    }
}

/// Returns a color style based on how old a commit is.
fn age_style(date: &DateTime<Utc>) -> Style {
    let duration = Utc::now() - *date;
    let days = duration.num_days();

    if days < 7 {
        Style::new().fg(Color::Green)
    } else if days < 30 {
        Style::new().fg(Color::Yellow)
    } else if days < 90 {
        Style::new().fg(Color::Indexed(208)) // orange
    } else {
        Style::new().fg(Color::Red)
    }
}

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    app.terminal_rows = area.height;

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(area);

    let main_area = layout[0];
    let status_area = layout[1];

    let width = main_area.width as usize;
    let compact_age = width < 120;
    let hide_local = width < 80;
    let short_status = width < 70;
    let hide_age = width < 60;

    let tab_title = tab_bar_line(&View::RemoteBranches, app.theme.remote_title);
    let block = Block::default()
        .title(tab_title)
        .borders(Borders::ALL);

    let sort_arrow = if app.remote_sort_ascending { "\u{25b2}" } else { "\u{25bc}" };
    let sort_label = |col_index: usize, base: &str| -> String {
        if app.remote_sort_column == Some(col_index) {
            format!("{}{}", base, sort_arrow)
        } else {
            base.to_string()
        }
    };

    let mut header_cells = vec![
        Cell::from(""),
        Cell::from(sort_label(0, "Name")),
    ];
    if !hide_age {
        header_cells.push(Cell::from(
            Line::from(sort_label(1, "Age")).alignment(Alignment::Right),
        ));
    }
    if !hide_local {
        header_cells.push(Cell::from("Local"));
    }
    header_cells.push(Cell::from(
        Line::from(sort_label(2, "Status")).alignment(Alignment::Right),
    ));

    let header = Row::new(header_cells)
        .style(app.theme.remote_header)
        .bottom_margin(0);

    let display_indices = app.filtered_remote_indices();

    let display_cursor = display_indices
        .iter()
        .position(|&i| i == app.remote_cursor);
    if let Some(row_idx) = display_cursor {
        app.remote_table_state.select(Some(row_idx));
    }

    let status_min_width: u16 = if short_status { 4 } else { 15 };
    let local_col_width: u16 = 5;

    let highlight_width_for_name = app.symbols.cursor_prefix.len() as u16 + 1;
    let checkbox_width: u16 = 3;
    let age_width: u16 = if hide_age { 0 } else if compact_age { 5 } else { 14 };
    let local_width: u16 = if hide_local { 0 } else { local_col_width };
    let gap_count: u16 = 2
        + if hide_age { 0 } else { 1 }
        + if hide_local { 0 } else { 1 }
        + 1;
    let name_col_width = main_area
        .width
        .saturating_sub(2)
        .saturating_sub(highlight_width_for_name)
        .saturating_sub(checkbox_width)
        .saturating_sub(age_width)
        .saturating_sub(local_width)
        .saturating_sub(status_min_width)
        .saturating_sub(gap_count) as usize;

    let is_ascii = app.symbols.cursor_prefix == ">";
    let local_yes = if is_ascii { "Y" } else { "\u{2713}" };
    let local_no = if is_ascii { "-" } else { "\u{2014}" };

    let build_row = |i: usize| -> Row {
        let branch = &app.remote_branches[i];
        let is_selected = app.remote_selected.get(i).copied().unwrap_or(false);
        let is_pinned = branch.is_pinned();

        let (checkbox_text, checkbox_style) = if is_pinned {
            ("   ".to_string(), Style::default())
        } else if is_selected {
            (app.symbols.checkbox_on.to_string(), app.theme.selected)
        } else {
            (app.symbols.checkbox_off.to_string(), app.theme.secondary_text)
        };

        let name_style = if is_pinned {
            app.theme.pinned_row
        } else if is_selected {
            app.theme.selected
        } else {
            app.theme.primary_text
        };

        let ellipsis = if is_ascii { "..." } else { "\u{2026}" };
        let remote_prefix = format!("{}/", branch.remote);
        let name_available = name_col_width.saturating_sub(remote_prefix.len());
        let display_name =
            if branch.short_name.len() > name_available && name_available > ellipsis.len() {
                format!(
                    "{}{}",
                    &branch.short_name[..name_available - ellipsis.len()],
                    ellipsis
                )
            } else if branch.short_name.len() > name_available {
                ellipsis.to_string()
            } else {
                branch.short_name.clone()
            };

        let pinned_label = if is_pinned { " [base]" } else { "" };

        let mut name_spans: Vec<Span> = Vec::new();
        if is_pinned {
            name_spans.push(Span::styled(remote_prefix, app.theme.pinned_row));
        } else {
            name_spans.push(Span::styled(remote_prefix, app.theme.secondary_text));
        }
        if let Some((prefix_part, rest)) = display_name.split_once('/') {
            if let Some(pstyle) = prefix_style(prefix_part) {
                if is_pinned {
                    name_spans.push(Span::styled(
                        format!("{}/", prefix_part),
                        app.theme.pinned_row,
                    ));
                    name_spans.push(Span::styled(rest.to_string(), app.theme.pinned_row));
                } else {
                    name_spans.push(Span::styled(format!("{}/", prefix_part), pstyle));
                    name_spans.push(Span::styled(rest.to_string(), name_style));
                }
            } else {
                name_spans.push(Span::styled(display_name.clone(), name_style));
            }
        } else {
            name_spans.push(Span::styled(display_name.clone(), name_style));
        }
        name_spans.push(Span::styled(pinned_label, app.theme.secondary_text));

        let name_cell = Cell::from(Line::from(name_spans));

        let mut cells = vec![
            Cell::from(Span::styled(checkbox_text, checkbox_style)),
            name_cell,
        ];

        if !hide_age {
            let age = if compact_age {
                branch.age_short()
            } else {
                branch.age_display()
            };
            let a_style = if is_pinned {
                app.theme.pinned_row
            } else {
                age_style(&branch.last_commit_date)
            };
            cells.push(Cell::from(
                Line::from(Span::styled(age, a_style)).alignment(Alignment::Right),
            ));
        }

        if !hide_local {
            let (local_text, local_style) = if is_pinned {
                (String::new(), app.theme.pinned_row)
            } else if branch.has_local {
                (local_yes.to_string(), app.theme.merged)
            } else {
                (local_no.to_string(), app.theme.secondary_text)
            };
            cells.push(Cell::from(Span::styled(local_text, local_style)));
        }

        let (status_text, status_style) = if is_pinned {
            (String::new(), app.theme.pinned_row)
        } else if short_status {
            match branch.merge_status {
                MergeStatus::Merged => (
                    format!("m {}", app.symbols.status_merged),
                    app.theme.merged,
                ),
                MergeStatus::SquashMerged => (
                    format!("s {}", app.symbols.status_squash_merged),
                    app.theme.squash_merged,
                ),
                MergeStatus::Unmerged => (
                    format!("u {}", app.symbols.status_unmerged),
                    app.theme.unmerged,
                ),
            }
        } else {
            match branch.merge_status {
                MergeStatus::Merged => (
                    format!("merged {}", app.symbols.status_merged),
                    app.theme.merged,
                ),
                MergeStatus::SquashMerged => (
                    format!("squash-merged {}", app.symbols.status_squash_merged),
                    app.theme.squash_merged,
                ),
                MergeStatus::Unmerged => (
                    format!("unmerged {}", app.symbols.status_unmerged),
                    app.theme.unmerged,
                ),
            }
        };
        cells.push(Cell::from(
            Line::from(Span::styled(status_text, status_style)).alignment(Alignment::Right),
        ));

        if is_selected {
            Row::new(cells).style(app.theme.checked_row)
        } else {
            Row::new(cells)
        }
    };

    let all_rows: Vec<Row> = display_indices.iter().map(|&i| build_row(i)).collect();

    let mut widths: Vec<Constraint> = vec![
        Constraint::Length(3),
        Constraint::Min(20),
    ];
    if !hide_age {
        widths.push(Constraint::Length(if compact_age { 5 } else { 14 }));
    }
    if !hide_local {
        widths.push(Constraint::Length(local_col_width));
    }
    widths.push(Constraint::Length(status_min_width));

    {
        let mut col_positions: Vec<(u16, usize)> = Vec::new();
        let highlight_width = app.symbols.cursor_prefix.len() as u16 + 1;
        let x = main_area.x + 1 + highlight_width;

        let mut sort_col_map: Vec<Option<usize>> = vec![None];
        sort_col_map.push(Some(0));
        if !hide_age {
            sort_col_map.push(Some(1));
        }
        if !hide_local {
            sort_col_map.push(None);
        }
        sort_col_map.push(Some(2));

        let available = main_area.width.saturating_sub(2 + highlight_width);
        let resolved = Layout::horizontal(&widths).split(Rect::new(0, 0, available, 1));

        for (i, rect) in resolved.iter().enumerate() {
            if let Some(&Some(sort_idx)) = sort_col_map.get(i) {
                col_positions.push((x + rect.x, sort_idx));
            }
        }

        app.remote_header_columns = col_positions;
    }

    let highlight_sym = format!("{} ", app.symbols.cursor_prefix);

    let inner_area = block.inner(main_area);
    frame.render_widget(block, main_area);

    let table = Table::new(all_rows, widths)
        .header(header)
        .row_highlight_style(app.theme.cursor)
        .highlight_symbol(highlight_sym);

    frame.render_stateful_widget(table, inner_area, &mut app.remote_table_state);

    if app.remote_search_active {
        app.remote_status_bar_items.clear();
        let search_text = format!(" / {}_", app.remote_search_query);
        let search_bar = Paragraph::new(search_text).style(app.theme.search_bar);
        frame.render_widget(search_bar, status_area);
    } else if !app.remote_search_query.is_empty() {
        app.remote_status_bar_items.clear();
        let filter_text = format!(
            " filter: \"{}\" ({}/{} shown) \u{2014} [\\]filter [/]edit [Esc in /]clear",
            app.remote_search_query,
            display_indices.len(),
            app.remote_branches.len()
        );
        let status = Paragraph::new(filter_text).style(app.theme.search_bar);
        frame.render_widget(status, status_area);
    } else {
        let selected_count = app.remote_selected.iter().filter(|&&s| s).count();
        let total = app.remote_branches.len();
        let merged_count = app
            .remote_branches
            .iter()
            .filter(|b| b.merge_status == MergeStatus::Merged)
            .count();
        let squash_count = app
            .remote_branches
            .iter()
            .filter(|b| b.merge_status == MergeStatus::SquashMerged)
            .count();
        let progress = if app.remote_squash_total > 0
            && app.remote_squash_checked < app.remote_squash_total
        {
            format!(
                " | checking {}/{}",
                app.remote_squash_checked, app.remote_squash_total
            )
        } else {
            String::new()
        };

        let status_text = if width < 80 {
            format!(
                " {}br {}sel {}m {}s{} \u{2014} [/]search [d]el [c]heckout [q]back",
                total, selected_count, merged_count, squash_count, progress
            )
        } else {
            format!(
                " {} remote branches | {} selected | {} merged | {} squashed{} \u{2014} [/]search [\\]filter [d]el [c]heckout [f]etch [?]help [q]back",
                total, selected_count, merged_count, squash_count, progress
            )
        };

        {
            let mut items: Vec<(u16, u16, KeyCode)> = Vec::new();
            let chars: Vec<char> = status_text.chars().collect();
            let base_x = status_area.x;
            let mut i = 0;
            while i < chars.len() {
                if chars[i] == '[' && i + 2 < chars.len() && chars[i + 2] == ']' {
                    let key_char = chars[i + 1];
                    let key_code = match key_char {
                        '/' => KeyCode::Char('/'),
                        '?' => KeyCode::Char('?'),
                        'q' => KeyCode::Char('q'),
                        'c' => KeyCode::Char('c'),
                        'd' => KeyCode::Char('d'),
                        'f' => KeyCode::Char('f'),
                        _ => {
                            i += 1;
                            continue;
                        }
                    };
                    let x_start = base_x + i as u16;
                    let mut j = i + 3;
                    while j < chars.len() && chars[j] != ' ' && chars[j] != '[' {
                        j += 1;
                    }
                    let x_end = base_x + j as u16;
                    items.push((x_start, x_end, key_code));
                    i = j;
                } else {
                    i += 1;
                }
            }
            app.remote_status_bar_items = items;
        }

        let key_style = Style::default()
            .fg(app.theme.remote_title.fg.unwrap_or(Color::White))
            .bg(app.theme.status_bar.bg.unwrap_or(Color::Reset))
            .add_modifier(ratatui::style::Modifier::BOLD);
        let mut spans: Vec<Span> = Vec::new();
        let mut remaining = status_text.as_str();
        while let Some(open) = remaining.find('[') {
            if open > 0 {
                spans.push(Span::styled(
                    remaining[..open].to_string(),
                    app.theme.status_bar,
                ));
            }
            remaining = &remaining[open..];
            if let Some(close) = remaining.find(']') {
                spans.push(Span::styled("[".to_string(), app.theme.status_bar));
                spans.push(Span::styled(remaining[1..close].to_string(), key_style));
                let after_close = &remaining[close..];
                let word_end = after_close[1..]
                    .find(' ')
                    .map(|idx| idx + 1)
                    .unwrap_or(after_close.len());
                spans.push(Span::styled(
                    after_close[..word_end].to_string(),
                    app.theme.status_bar,
                ));
                remaining = &after_close[word_end..];
            } else {
                spans.push(Span::styled(remaining.to_string(), app.theme.status_bar));
                remaining = "";
            }
        }
        if !remaining.is_empty() {
            spans.push(Span::styled(remaining.to_string(), app.theme.status_bar));
        }
        let status = Paragraph::new(Line::from(spans)).style(app.theme.status_bar);
        frame.render_widget(status, status_area);
    }

    // Toast overlay while fetching remote branches
    if app.remote_loading {
        let msg = " Fetching remote branches\u{2026} ";
        let toast_width = msg.len() as u16 + 2; // +2 for border
        let toast_height: u16 = 3;
        let x = area.width.saturating_sub(toast_width).saturating_sub(1);
        let y = area.height.saturating_sub(toast_height).saturating_sub(2); // above status bar
        let toast_area = Rect::new(x, y, toast_width, toast_height);

        let toast = Paragraph::new(msg)
            .style(app.theme.toast_text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(app.theme.toast_border),
            )
            .alignment(Alignment::Center);
        frame.render_widget(Clear, toast_area);
        frame.render_widget(toast, toast_area);
    }
}
