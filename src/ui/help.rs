use ratatui::prelude::*;
use ratatui::style::Modifier;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::app::App;

const HELP_TEXT: &str = "\
j/\u{2193}     Move down
k/\u{2191}     Move up
PgUp/Dn Page scroll
ENTER   Operations menu
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
s       Cycle sort column
S       Reverse sort order
R       Force recheck (clear cache)
T       Cycle theme
Y       Cycle symbols
t       Tags view
,       Settings
/       Search branches
\\      Filter menu
?       Toggle help
q       Quit";

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();

    let key_style = Style::default()
        .fg(app.theme.title.fg.unwrap_or(Color::White))
        .add_modifier(Modifier::BOLD);
    let lines: Vec<Line> = HELP_TEXT
        .lines()
        .map(|line| {
            // Split on first occurrence of two or more spaces to get key_part and desc_part
            if let Some(pos) = line.find("  ") {
                let key_part = &line[..pos];
                let desc_part = &line[pos..];
                Line::from(vec![
                    Span::styled(key_part.to_string(), key_style),
                    Span::styled(desc_part.to_string(), Style::default()),
                ])
            } else {
                Line::from(line)
            }
        })
        .collect();
    let content_height = lines.len() as u16 + 2; // +2 for borders
    let width = 42u16.min(area.width);
    let height = content_height.min(area.height);

    let rect = centered_rect(width, height, area);

    let block = Block::default()
        .title("Help")
        .title_style(app.theme.title)
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
