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

    // Header row
    let mut header_cells = vec![
        Cell::from("Tag"),
        Cell::from(Line::from("Age").alignment(Alignment::Right)),
        Cell::from("Commit"),
    ];
    if !hide_message {
        header_cells.push(Cell::from("Message"));
    }

    let header = Row::new(header_cells)
        .style(app.theme.header)
        .bottom_margin(0);

    // Build table rows
    let rows: Vec<Row> = app
        .tags
        .iter()
        .map(|tag| {
            let name_cell = Cell::from(Span::styled(
                &tag.name,
                app.theme.primary_text,
            ));

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

            let mut cells = vec![name_cell, age_cell, hash_cell];

            if !hide_message {
                let msg = tag.message.as_deref().unwrap_or("");
                // Truncate long messages
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

            Row::new(cells)
        })
        .collect();

    // Column widths
    let mut widths: Vec<Constraint> = vec![
        Constraint::Min(20),                                    // tag name
        Constraint::Length(if compact_age { 5 } else { 14 }),   // age
        Constraint::Length(9),                                  // commit hash
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
    let status_text = if width < 80 {
        format!(
            " {} tags \u{2014} [d]el [p]ush [q]back",
            total
        )
    } else {
        format!(
            " {} tags \u{2014} [d]elete [p]ush to remote [q]/[Esc]/[t] back to branches [?]help",
            total
        )
    };
    let status = Paragraph::new(status_text).style(app.theme.status_bar);
    frame.render_widget(status, status_area);
}
