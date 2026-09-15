use ratatui::prelude::*;
use ratatui::widgets::{Paragraph, Wrap};

use crate::theme::Theme;
use crate::types::ProgressUpdate;

use super::{
    modal::{draw_modal_shell, ModalFooter, ModalSpec},
    shared::{render_progress_bar, truncate_left},
};

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
    let display_label = if label.is_empty() {
        "Working..."
    } else {
        label
    };

    let inner_width = 46usize;

    let mut lines: Vec<Line> = Vec::new();

    if let Some(progress) = progress {
        // Line 1: label
        lines.push(Line::from(Span::styled(display_label, theme.dim)));

        // Line 2: progress bar  [========>          ] 3/10
        let bar = render_progress_bar(inner_width, progress.completed, progress.total);
        lines.push(Line::from(Span::styled(bar, theme.primary_text)));

        // Line 3: current item name
        let item_display = truncate_left(
            progress.current_item.as_str(),
            inner_width.saturating_sub(3),
        );
        lines.push(Line::from(Span::styled(item_display, theme.secondary_text)));
    } else {
        // No progress info yet, just show the label.
        lines.push(Line::from(Span::styled(display_label, theme.dim)));
        lines.push(Line::from(""));
    }

    // Height tracks actual content (accounting for word-wrap on a long
    // label) rather than a fixed 5/7-row guess, so it always fits content + 2 borders.
    let wrapped_height: usize = lines
        .iter()
        .map(|l| {
            let chars: usize = l.spans.iter().map(|s| s.content.chars().count()).sum();
            if chars == 0 {
                1
            } else {
                chars.div_ceil(inner_width.max(1))
            }
        })
        .sum();
    let height = (wrapped_height as u16 + 3).max(5);
    let areas = draw_modal_shell(
        frame,
        &ModalSpec::new(
            "Running",
            ModalFooter::hints(&[("Esc", "Cancel")]),
            50,
            height,
        ),
        theme,
    );

    let paragraph = Paragraph::new(lines)
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, areas.body);
}
