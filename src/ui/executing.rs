use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::app::App;

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();

    // Centered overlay showing the operation label with a spinner
    let width = 40u16.min(area.width);
    let height = 5u16.min(area.height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let rect = Rect::new(x, y, width, height);

    let block = Block::default()
        .title("Running")
        .title_style(app.theme.title)
        .borders(Borders::ALL);

    let label = if app.executing_label.is_empty() {
        "Working..."
    } else {
        &app.executing_label
    };

    let paragraph = Paragraph::new(Line::from(Span::styled(
        label,
        app.theme.dim,
    )))
    .block(block)
    .alignment(Alignment::Center);

    frame.render_widget(Clear, rect);
    frame.render_widget(paragraph, rect);
}
