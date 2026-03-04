use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::app::App;
use super::theme;

const HELP_TEXT: &str = "\
j/\u{2193}     Move down
k/\u{2191}     Move up
SPACE   Toggle selection
a       Select all
n       Deselect all
m       Select merged
i       Invert selection
c       Checkout cursor branch
x       Delete cursor branch
d       Delete local (selected)
D       Delete local + remote (selected)
f       Fetch
F       Fetch + prune
R       Force recheck (clear cache)
?       Toggle help
q/Esc   Quit";

pub fn draw(frame: &mut Frame, _app: &App) {
    let area = frame.area();

    let lines: Vec<Line> = HELP_TEXT.lines().map(Line::from).collect();
    let content_height = lines.len() as u16 + 2; // +2 for borders
    let width = 42u16.min(area.width);
    let height = content_height.min(area.height);

    let rect = centered_rect(width, height, area);

    let block = Block::default()
        .title("Help")
        .title_style(theme::TITLE_STYLE)
        .borders(Borders::ALL);

    let paragraph = Paragraph::new(lines).block(block);

    frame.render_widget(Clear, rect);
    frame.render_widget(paragraph, rect);
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}
