use ratatui::prelude::*;
use ratatui::style::Modifier;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::theme::Theme;
use crate::types::OperationResult;

/// Renders the full-screen results view after an operation completes.
///
/// Shows success/failure per item. The footer instructs the user to press any key to return.
pub fn draw_results(
    frame: &mut Frame,
    results: &[OperationResult],
    theme: &Theme,
) {
    let area = frame.area();

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(area);

    let main_area = layout[0];
    let footer_area = layout[1];

    let block = Block::default()
        .title("Results")
        .title_style(theme.title)
        .borders(Borders::ALL);

    let lines: Vec<Line> = results
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

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, main_area);

    let footer = Paragraph::new(" Press any key to continue").style(theme.status_bar);
    frame.render_widget(footer, footer_area);
}
