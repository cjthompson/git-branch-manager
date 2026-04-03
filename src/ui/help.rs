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
r       Remote branches
w       Worktrees view
Tab     Next view
S-Tab   Prev view
l       Local branches
,       Settings
/       Search branches
\\      Filter menu
?       Toggle help
q       Quit";

fn style_entry<'a>(line: &str, key_style: Style) -> Vec<Span<'a>> {
    if let Some(pos) = line.find("  ") {
        let key_part = &line[..pos];
        let desc_part = &line[pos..];
        vec![
            Span::styled(key_part.to_string(), key_style),
            Span::styled(desc_part.to_string(), Style::default()),
        ]
    } else {
        vec![Span::raw(line.to_string())]
    }
}

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();

    let key_style = Style::default()
        .fg(app.theme.title.fg.unwrap_or(Color::White))
        .add_modifier(Modifier::BOLD);

    let all_lines: Vec<&str> = HELP_TEXT.lines().collect();
    let col_width = 38u16;
    let separator = "  \u{2502}  "; // " │ "

    // Use two-column layout if terminal is wide enough and too short for single column
    let content_height_single = all_lines.len() as u16 + 2;
    let use_two_cols = area.width >= col_width * 2 + separator.len() as u16 + 4
        && area.height < content_height_single;

    if use_two_cols {
        let mid = all_lines.len().div_ceil(2);
        let left = &all_lines[..mid];
        let right = &all_lines[mid..];

        let lines: Vec<Line> = (0..mid)
            .map(|i| {
                let mut spans = style_entry(left[i], key_style);
                // Pad left column to fixed width
                let left_text: String = spans.iter().map(|s| s.content.as_ref()).collect();
                let pad = col_width as usize - left_text.chars().count().min(col_width as usize);
                spans.push(Span::raw(" ".repeat(pad)));
                // Separator
                let sep_style = Style::default().add_modifier(Modifier::DIM);
                spans.push(Span::styled(separator.to_string(), sep_style));
                // Right column
                if let Some(&right_line) = right.get(i) {
                    spans.extend(style_entry(right_line, key_style));
                }
                Line::from(spans)
            })
            .collect();

        let width = (col_width * 2 + separator.len() as u16 + 4).min(area.width);
        let height = (mid as u16 + 2).min(area.height);
        let rect = centered_rect(width, height, area);

        let block = Block::default()
            .title("Help")
            .title_style(app.theme.title)
            .borders(Borders::ALL);
        let paragraph = Paragraph::new(lines).block(block);
        frame.render_widget(Clear, rect);
        frame.render_widget(paragraph, rect);
    } else {
        // Single column — original layout
        let lines: Vec<Line> = all_lines
            .iter()
            .map(|line| Line::from(style_entry(line, key_style)))
            .collect();
        let width = col_width.min(area.width) + 4;
        let height = content_height_single.min(area.height);
        let rect = centered_rect(width, height, area);

        let block = Block::default()
            .title("Help")
            .title_style(app.theme.title)
            .borders(Borders::ALL);
        let paragraph = Paragraph::new(lines).block(block);
        frame.render_widget(Clear, rect);
        frame.render_widget(paragraph, rect);
    }
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}
