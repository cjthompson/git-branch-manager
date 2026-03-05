use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::app::App;

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let width = 60u16.min(area.width);
    let height = 10u16.min(area.height);
    let rect = centered_rect(width, height, area);

    let block = Block::default()
        .title("Settings")
        .title_style(app.theme.title)
        .borders(Borders::ALL);

    let paragraph = Paragraph::new(vec![]).block(block);

    frame.render_widget(Clear, rect);
    frame.render_widget(paragraph, rect);
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}
