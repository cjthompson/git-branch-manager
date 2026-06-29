use ratatui::prelude::*;
use ratatui::style::Modifier;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::theme::Theme;
use crate::types::OperationResult;
use super::shared::centered_rect;

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

    // Calculate dynamic width based on maximum content width
    let content_max_width: usize = lines
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.chars().count()).sum::<usize>())
        .max()
        .unwrap_or(0);
    let width = (content_max_width as u16 + 4).max(50).min(area.width.saturating_sub(2));

    // Calculate height accounting for text wrapping
    let inner_width = width.saturating_sub(2) as usize;
    let wrapped_height: usize = lines
        .iter()
        .map(|l| {
            let char_count: usize = l.spans.iter().map(|s| s.content.chars().count()).sum();
            if char_count == 0 {
                1  // empty line still takes 1 row
            } else {
                char_count.div_ceil(inner_width.max(1))
            }
        })
        .sum();
    let content_height = (wrapped_height + 2) as u16; // +2 for block borders

    let max_height = (area.height * 80 / 100).max(10);
    let modal_height = content_height.min(max_height).min(area.height);

    let rect = centered_rect(width, modal_height, area);

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
