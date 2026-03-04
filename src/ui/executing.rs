use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::app::App;

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();

    let has_progress = app.progress.is_some();
    let width = 50u16.min(area.width);
    let height = if has_progress { 7u16 } else { 5u16 };
    let height = height.min(area.height);
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

    let inner_width = width.saturating_sub(2) as usize; // account for borders

    let mut lines: Vec<Line> = Vec::new();

    if let Some(progress) = &app.progress {
        // Line 1: label
        lines.push(Line::from(Span::styled(label, app.theme.dim)));

        // Line 2: progress bar  [========>          ] 3/10
        let count_text = format!(" {}/{}", progress.completed, progress.total);
        let bar_width = inner_width.saturating_sub(count_text.len()).saturating_sub(2); // -2 for []

        let fraction = if progress.total > 0 {
            progress.completed as f64 / progress.total as f64
        } else {
            0.0
        };
        let filled = (fraction * bar_width as f64) as usize;
        let empty = bar_width.saturating_sub(filled);

        let bar = format!(
            "[{}{}]{}",
            "=".repeat(filled),
            " ".repeat(empty),
            count_text,
        );
        lines.push(Line::from(Span::styled(bar, app.theme.primary_text)));

        // Line 3: current item name
        let item_display = if progress.current_item.len() > inner_width {
            format!("...{}", &progress.current_item[progress.current_item.len() - (inner_width - 3)..])
        } else {
            progress.current_item.clone()
        };
        lines.push(Line::from(Span::styled(item_display, app.theme.secondary_text)));

        // Line 4: cancel hint
        lines.push(Line::from(Span::styled("Esc to cancel", app.theme.dim)));
    } else {
        // No progress info yet, just show label and cancel hint
        lines.push(Line::from(Span::styled(label, app.theme.dim)));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("Esc to cancel", app.theme.dim)));
    }

    let paragraph = Paragraph::new(lines)
        .block(block)
        .alignment(Alignment::Center);

    frame.render_widget(Clear, rect);
    frame.render_widget(paragraph, rect);
}
