use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::theme::Theme;
use crate::types::ProgressUpdate;

use super::shared::{centered_rect, render_progress_bar};

/// Renders the executing/progress overlay.
///
/// `label` is the operation name (e.g. "Deleting branches").
/// `progress` is the current progress state (if available).
pub fn draw_executing(
    frame: &mut Frame,
    label: &str,
    progress: Option<&ProgressUpdate>,
    theme: &Theme,
) {
    let area = frame.area();

    let has_progress = progress.is_some();
    let width = 50u16.min(area.width);
    let height = if has_progress { 7u16 } else { 5u16 };
    let height = height.min(area.height);
    let rect = centered_rect(width, height, area);

    let block = Block::default()
        .title("Running")
        .title_style(theme.title)
        .borders(Borders::ALL);

    let display_label = if label.is_empty() {
        "Working..."
    } else {
        label
    };

    let inner_width = width.saturating_sub(2) as usize; // account for borders

    let mut lines: Vec<Line> = Vec::new();

    if let Some(progress) = progress {
        // Line 1: label
        lines.push(Line::from(Span::styled(display_label, theme.dim)));

        // Line 2: progress bar  [========>          ] 3/10
        let bar = render_progress_bar(inner_width, progress.completed, progress.total);
        lines.push(Line::from(Span::styled(bar, theme.primary_text)));

        // Line 3: current item name
        let item_display = if progress.current_item.len() > inner_width {
            format!(
                "...{}",
                &progress.current_item[progress.current_item.len() - (inner_width - 3)..]
            )
        } else {
            progress.current_item.clone()
        };
        lines.push(Line::from(Span::styled(item_display, theme.secondary_text)));

        // Line 4: cancel hint
        lines.push(Line::from(Span::styled("Esc to cancel", theme.dim)));
    } else {
        // No progress info yet, just show label and cancel hint
        lines.push(Line::from(Span::styled(display_label, theme.dim)));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("Esc to cancel", theme.dim)));
    }

    let paragraph = Paragraph::new(lines)
        .block(block)
        .alignment(Alignment::Center);

    frame.render_widget(Clear, rect);
    frame.render_widget(paragraph, rect);
}
