use ratatui::prelude::*;
use ratatui::style::Modifier;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::theme::Theme;
use crate::types::OperationResult;

pub fn draw_results(frame: &mut Frame, results: &[OperationResult], theme: &Theme) {
    let area = frame.area();

    let mut lines: Vec<Line> = results
        .iter()
        .map(|r| {
            let (status, style) = if r.success {
                (" OK ", theme.merged)
            } else {
                ("FAIL", theme.error)
            };

            Line::from(vec![
                Span::styled(status, style),
                Span::raw("  "),
                Span::styled(
                    r.branch_name.clone(),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(r.message.clone(), theme.dim),
            ])
        })
        .collect();

    lines.push(Line::from(""));
    let key_style = Style::default().fg(theme.title.fg.unwrap_or(Color::White));
    lines.push(Line::from(vec![
        Span::styled("Press ", theme.dim),
        Span::styled("Enter", key_style),
        Span::styled(" or ", theme.dim),
        Span::styled("Esc", key_style),
        Span::styled(" to continue", theme.dim),
    ]));

    let content_height = lines.len() as u16 + 2;
    let max_height = (area.height * 60 / 100).max(8);
    let height = content_height.min(max_height).min(area.height);
    let width = (area.width / 2).max(50).min(area.width);

    let rect = centered_rect(width, height, area);

    let paragraph = Paragraph::new(lines)
        .block(
            Block::default()
                .title("Results")
                .title_style(theme.title)
                .borders(Borders::ALL),
        )
        .wrap(Wrap { trim: false });

    frame.render_widget(Clear, rect);
    frame.render_widget(paragraph, rect);
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}
