use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::app::App;

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(area);

    let main_area = layout[0];
    let footer_area = layout[1];

    let block = Block::default()
        .title("Results")
        .title_style(app.theme.title)
        .borders(Borders::ALL);

    let lines: Vec<Line> = app
        .results
        .iter()
        .map(|r| {
            let (status, style) = if r.success {
                (" OK ", app.theme.merged)
            } else {
                ("FAIL", app.theme.error)
            };

            Line::from(vec![
                Span::styled(status, style),
                Span::raw("  "),
                Span::styled(&r.branch_name, Style::default().add_modifier(Modifier::BOLD)),
                Span::raw("  "),
                Span::styled(&r.message, app.theme.dim),
            ])
        })
        .collect();

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, main_area);

    let footer = Paragraph::new(" Press any key to continue").style(app.theme.status_bar);
    frame.render_widget(footer, footer_area);
}
