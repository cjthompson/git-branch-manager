use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};

use crate::app::App;

/// Returns a color style based on how old a tag's commit is.
fn age_style(date: &chrono::DateTime<chrono::Utc>) -> Style {
    let duration = chrono::Utc::now() - *date;
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

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(area);

    let main_area = layout[0];
    let status_area = layout[1];

    let width = main_area.width as usize;
    let compact_age = width < 120;
    let hide_message = width < 80;

    let block = Block::default()
        .title("Tags")
        .title_style(app.theme.title)
        .borders(Borders::ALL);

    // Build filtered indices
    let filtered_indices = app.filtered_tag_indices();

    // Header row
    let sort_indicator = if app.tag_sort_by_name { " [name]" } else { " [date]" };
    let mut header_cells = vec![
        Cell::from(""),  // checkbox column
        Cell::from(format!("Tag{}", if app.tag_sort_by_name { sort_indicator } else { "" })),
        Cell::from(Line::from(format!("Age{}", if !app.tag_sort_by_name { sort_indicator } else { "" })).alignment(Alignment::Right)),
        Cell::from("Commit"),
    ];
    if !hide_message {
        header_cells.push(Cell::from("Message"));
    }

    let header = Row::new(header_cells)
        .style(app.theme.header)
        .bottom_margin(0);

    // Build table rows from filtered indices
    let rows: Vec<Row> = filtered_indices
        .iter()
        .map(|&idx| {
            let tag = &app.tags[idx];
            let is_selected = app.tag_selected.get(idx).copied().unwrap_or(false);

            // Checkbox
            let (checkbox_text, checkbox_style) = if is_selected {
                (app.symbols.checkbox_on.to_string(), app.theme.selected)
            } else {
                (app.symbols.checkbox_off.to_string(), app.theme.secondary_text)
            };
            let checkbox_cell = Cell::from(Span::styled(checkbox_text, checkbox_style));

            let name_style = if is_selected {
                app.theme.selected
            } else {
                app.theme.primary_text
            };
            let name_cell = Cell::from(Span::styled(&tag.name, name_style));

            let age = if compact_age {
                tag.age_short()
            } else {
                tag.age_display()
            };
            let age_cell = Cell::from(
                Line::from(Span::styled(age, age_style(&tag.date))).alignment(Alignment::Right),
            );

            let short_hash = if tag.commit_hash.len() >= 7 {
                &tag.commit_hash[..7]
            } else {
                &tag.commit_hash
            };
            let hash_cell = Cell::from(Span::styled(
                short_hash,
                app.theme.secondary_text,
            ));

            let mut cells = vec![checkbox_cell, name_cell, age_cell, hash_cell];

            if !hide_message {
                let msg = tag.message.as_deref().unwrap_or("");
                let msg_display = if msg.len() > 60 {
                    format!("{}\u{2026}", &msg[..59])
                } else {
                    msg.to_string()
                };
                cells.push(Cell::from(Span::styled(
                    msg_display,
                    app.theme.secondary_text,
                )));
            }

            let row_style = if is_selected {
                app.theme.checked_row
            } else {
                Style::default()
            };

            Row::new(cells).style(row_style)
        })
        .collect();

    // Column widths
    let mut widths: Vec<Constraint> = vec![
        Constraint::Length(3),                                   // checkbox
        Constraint::Min(20),                                     // tag name
        Constraint::Length(if compact_age { 5 } else { 14 }),    // age
        Constraint::Length(9),                                   // commit hash
    ];
    if !hide_message {
        widths.push(Constraint::Min(15)); // message
    }

    let highlight_sym = format!("{} ", app.symbols.cursor_prefix);
    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .row_highlight_style(app.theme.cursor)
        .highlight_symbol(highlight_sym);

    frame.render_stateful_widget(table, main_area, &mut app.tag_table_state);

    // Status bar
    let total = app.tags.len();
    let shown = filtered_indices.len();
    let selected_count = app.tag_selection_count();

    let status_text = if app.tag_search_active {
        format!(" / {}_", app.tag_search_query)
    } else if !app.tag_search_query.is_empty() {
        format!(
            " filter: \"{}\" ({}/{} shown) \u{2014} [\\]filter [/]edit [Esc in /]clear",
            app.tag_search_query, shown, total
        )
    } else if width < 80 {
        if selected_count > 0 {
            format!(
                " {} tags ({} sel) \u{2014} [d]el [D]el+remote [p]ush [q]back",
                total, selected_count
            )
        } else {
            format!(
                " {} tags \u{2014} [d]el [p]ush [/]search [q]back",
                total
            )
        }
    } else if selected_count > 0 {
        format!(
            " {} tags ({} selected) \u{2014} [Space]toggle [a]ll [n]one [i]nvert [d]elete [D]el+remote [s]ort [q]back",
            total, selected_count
        )
    } else {
        format!(
            " {} tags \u{2014} [Space]select [a]ll [d]elete [D]el+remote [p]ush [/]search [\\]filter [s]ort [q]back [?]help",
            total
        )
    };

    if app.tag_search_active {
        let search_bar = Paragraph::new(Line::from(status_text.as_str()))
            .style(app.theme.search_bar);
        frame.render_widget(search_bar, status_area);
    } else {
        let key_style = Style::default().fg(app.theme.title.fg.unwrap_or(Color::White));
        let mut spans: Vec<Span> = Vec::new();
        let mut remaining = status_text.as_str();
        while let Some(open) = remaining.find('[') {
            if open > 0 {
                spans.push(Span::styled(remaining[..open].to_string(), app.theme.status_bar));
            }
            remaining = &remaining[open..];
            if let Some(close) = remaining.find(']') {
                spans.push(Span::styled("[", app.theme.status_bar));
                spans.push(Span::styled(remaining[1..close].to_string(), key_style));
                remaining = &remaining[close..];
                if let Some(next_open) = remaining.find('[') {
                    spans.push(Span::styled(remaining[..next_open].to_string(), app.theme.status_bar));
                    remaining = &remaining[next_open..];
                } else {
                    spans.push(Span::styled(remaining.to_string(), app.theme.status_bar));
                    remaining = "";
                    break;
                }
            } else {
                spans.push(Span::styled(remaining.to_string(), app.theme.status_bar));
                remaining = "";
                break;
            }
        }
        if !remaining.is_empty() {
            spans.push(Span::styled(remaining.to_string(), app.theme.status_bar));
        }
        let status = Paragraph::new(Line::from(spans)).style(app.theme.status_bar);
        frame.render_widget(status, status_area);
    }
}
