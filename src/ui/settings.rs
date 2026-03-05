use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::app::{App, View};

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let width = 60u16.min(area.width);
    let height = 10u16.min(area.height);
    let rect = centered_rect(width, height, area);

    let block = Block::default()
        .title(" Settings ")
        .title_style(app.theme.title)
        .borders(Borders::ALL);

    let inner = block.inner(rect);
    frame.render_widget(Clear, rect);
    frame.render_widget(block, rect);

    let cursor = if let View::Settings { cursor } = app.view { cursor } else { 0 };

    let rows: &[(&str, String)] = &[
        ("Symbol set", crate::ui::symbols::name(app.symbols).to_string()),
        ("Theme", app.theme.name.to_string()),
    ];

    let mut lines: Vec<Line> = rows.iter().enumerate().map(|(i, (label, value))| {
        let style = if i == cursor { app.theme.cursor } else { Style::default() };
        Line::from(vec![
            Span::styled(format!("  {:<30}", label), style),
            Span::styled(value.clone(), style.add_modifier(Modifier::BOLD)),
        ])
    }).collect();

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("  ←/→ cycle   Esc close", app.theme.dim)));

    frame.render_widget(Paragraph::new(lines), inner);
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}
