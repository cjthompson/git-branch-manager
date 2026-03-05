use ratatui::prelude::*;
use ratatui::style::Modifier;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::app::{App, FilterSet};

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let query = &app.search_query;

    let key_style = Style::default()
        .fg(app.theme.title.fg.unwrap_or(Color::White))
        .add_modifier(Modifier::BOLD);
    let active_style = Style::default()
        .fg(Color::Green)
        .add_modifier(Modifier::BOLD);
    let label_style = Style::default();
    let section_style = Style::default()
        .fg(app.theme.title.fg.unwrap_or(Color::White))
        .add_modifier(Modifier::BOLD);

    let fl = |key, label, token| filter_line(key, label, token, query, key_style, active_style, label_style);

    let lines: Vec<Line> = vec![
        // Status section
        Line::from(Span::styled("Status", section_style)),
        fl("m", "Merged", "status:merged"),
        fl("s", "Squash-merged", "status:squash"),
        fl("u", "Unmerged", "status:unmerged"),
        Line::from(""),
        // PR section
        Line::from(Span::styled("Pull Requests", section_style)),
        fl("p", "Has PR", "pr:yes"),
        fl("P", "No PR", "pr:no"),
        Line::from(""),
        // Sync section
        Line::from(Span::styled("Sync", section_style)),
        fl("a", "Ahead (can push)", "sync:ahead"),
        fl("b", "Behind (can pull)", "sync:behind"),
        Line::from(""),
        // Age section
        Line::from(Span::styled("Age", section_style)),
        fl("1", "Newer than 7 days", "age:<7d"),
        fl("2", "Newer than 30 days", "age:<30d"),
        fl("3", "Older than 30 days", "age:>30d"),
        fl("4", "Older than 90 days", "age:>90d"),
        Line::from(vec![
            Span::styled("n", key_style),
            Span::styled("  Newer than (custom)", label_style),
        ]),
        Line::from(vec![
            Span::styled("o", key_style),
            Span::styled("  Older than (custom)", label_style),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("c", key_style),
            Span::styled("  Clear all filters", label_style),
        ]),
    ];

    let content_height = lines.len() as u16 + 2; // +2 for borders
    let width = 36u16.min(area.width);
    let height = content_height.min(area.height);

    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let rect = Rect::new(x, y, width.min(area.width), height.min(area.height));

    let block = Block::default()
        .title("Filters")
        .title_style(app.theme.title)
        .borders(Borders::ALL);

    let paragraph = Paragraph::new(lines).block(block);

    frame.render_widget(Clear, rect);
    frame.render_widget(paragraph, rect);
}

pub fn draw_tag_filter(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let query = &app.tag_search_query;

    let key_style = Style::default()
        .fg(app.theme.title.fg.unwrap_or(Color::White))
        .add_modifier(Modifier::BOLD);
    let active_style = Style::default()
        .fg(Color::Green)
        .add_modifier(Modifier::BOLD);
    let label_style = Style::default();
    let section_style = Style::default()
        .fg(app.theme.title.fg.unwrap_or(Color::White))
        .add_modifier(Modifier::BOLD);

    let fl = |key, label, token| filter_line(key, label, token, query, key_style, active_style, label_style);

    let lines: Vec<Line> = vec![
        Line::from(Span::styled("Age", section_style)),
        fl("1", "Newer than 7 days", "age:<7d"),
        fl("2", "Newer than 30 days", "age:<30d"),
        fl("3", "Older than 30 days", "age:>30d"),
        fl("4", "Older than 90 days", "age:>90d"),
        Line::from(vec![
            Span::styled("n", key_style),
            Span::styled("  Newer than (custom)", label_style),
        ]),
        Line::from(vec![
            Span::styled("o", key_style),
            Span::styled("  Older than (custom)", label_style),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("c", key_style),
            Span::styled("  Clear all filters", label_style),
        ]),
    ];

    let content_height = lines.len() as u16 + 2;
    let width = 36u16.min(area.width);
    let height = content_height.min(area.height);

    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let rect = Rect::new(x, y, width.min(area.width), height.min(area.height));

    let block = Block::default()
        .title("Tag Filters")
        .title_style(app.theme.title)
        .borders(Borders::ALL);

    let paragraph = Paragraph::new(lines).block(block);

    frame.render_widget(Clear, rect);
    frame.render_widget(paragraph, rect);
}

fn filter_line<'a>(
    key: &'a str,
    label: &'a str,
    token: &str,
    query: &str,
    key_style: Style,
    active_style: Style,
    label_style: Style,
) -> Line<'a> {
    let is_active = FilterSet::has_token(query, token);
    let marker = if is_active { "\u{25c9} " } else { "\u{25ef} " };
    let (marker_style, text_style) = if is_active {
        (active_style, active_style)
    } else {
        (label_style, label_style)
    };

    Line::from(vec![
        Span::styled(key, key_style),
        Span::styled(" ", label_style),
        Span::styled(marker, marker_style),
        Span::styled(label, text_style),
    ])
}
