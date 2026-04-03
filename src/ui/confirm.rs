use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::theme::Theme;
use crate::types::BranchAction;

use super::shared::centered_rect;

/// Renders a confirmation dialog overlay.
///
/// `action` is the operation about to be performed.
/// `target_names` is the list of items the action will affect.
pub fn draw_confirm(
    frame: &mut Frame,
    action: BranchAction,
    target_names: &[String],
    theme: &Theme,
) {
    let action_label = action.label();
    let count = target_names.len();

    let mut lines = vec![
        Line::from(format!("{} {} item(s)?", action_label, count)),
        Line::from(""),
    ];

    for name in target_names {
        lines.push(Line::from(Span::styled(
            format!("  {}", name),
            theme.selected,
        )));
    }

    lines.push(Line::from(""));
    let key_style = Style::default().fg(theme.accent_fg());
    lines.push(Line::from(vec![
        Span::styled("[", theme.dim),
        Span::styled("y", key_style),
        Span::styled("]es  [", theme.dim),
        Span::styled("n", key_style),
        Span::styled("]o", theme.dim),
    ]));

    // Calculate overlay size
    let area = frame.area();
    let max_height = (area.height * 60 / 100).max(8);
    let inner_max = max_height.saturating_sub(2) as usize; // subtract borders

    // Truncate if content exceeds available space
    if lines.len() > inner_max {
        let footer_lines = 2; // blank + yes/no
        let header_lines = 2; // action question + blank
        let available_for_items = inner_max.saturating_sub(header_lines + footer_lines + 1);

        let total_items = lines.len() - header_lines - footer_lines;
        let hidden = total_items.saturating_sub(available_for_items);

        if hidden > 0 {
            let footer: Vec<Line> = lines.split_off(lines.len() - 2);
            lines.truncate(header_lines + available_for_items);
            lines.push(Line::from(Span::styled(
                format!("  ...{} more", hidden),
                theme.dim,
            )));
            lines.extend(footer);
        }
    }

    let content_height = lines.len() as u16 + 2; // +2 for borders
    let height = content_height.min(max_height).min(area.height);
    let width = (area.width / 2).max(40).min(area.width);

    let rect = centered_rect(width, height, area);

    let block = Block::default()
        .title(format!("Confirm {}", action_label))
        .title_style(theme.title)
        .borders(Borders::ALL);

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });

    frame.render_widget(Clear, rect);
    frame.render_widget(paragraph, rect);
}
