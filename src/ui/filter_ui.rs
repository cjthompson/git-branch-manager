use ratatui::prelude::*;
use ratatui::style::Modifier;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::theme::Theme;
use crate::view::filter::{FilterSet, FilterTokenDef};

use super::shared::centered_rect;

/// Renders the generic filter builder overlay.
///
/// This single function works for all views by accepting the view's filter token
/// definitions. Active tokens are highlighted in green.
///
/// `filter_tokens` defines which filter toggles to show.
/// `current_query` is the current filter query string.
/// `title` is the overlay title (e.g., "Filters", "Tag Filters").
pub fn draw_filter(
    frame: &mut Frame,
    filter_tokens: &[FilterTokenDef],
    current_query: &str,
    title: &str,
    theme: &Theme,
) {
    let area = frame.area();

    let key_style = Style::default()
        .fg(theme.accent_fg())
        .add_modifier(Modifier::BOLD);
    let active_style = Style::default()
        .fg(ratatui::style::Color::Green)
        .add_modifier(Modifier::BOLD);
    let label_style = Style::default();

    // Group tokens by category based on their token prefix
    let mut lines: Vec<Line> = Vec::new();
    let mut current_section: Option<&str> = None;

    for token_def in filter_tokens {
        let section = token_section(token_def.token);

        // Insert section header if we've entered a new section
        if current_section != Some(section) {
            if current_section.is_some() {
                lines.push(Line::from("")); // blank line between sections
            }
            let section_style = Style::default()
                .fg(theme.accent_fg())
                .add_modifier(Modifier::BOLD);
            lines.push(Line::from(Span::styled(section, section_style)));
            current_section = Some(section);
        }

        lines.push(filter_line(
            &token_def.key.to_string(),
            token_def.label,
            token_def.token,
            current_query,
            key_style,
            active_style,
            label_style,
        ));
    }

    // Add clear all option
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("c", key_style),
        Span::styled("  Clear all filters", label_style),
    ]));

    let content_height = lines.len() as u16 + 2; // +2 for borders
    let width = 36u16.min(area.width);
    let height = content_height.min(area.height);
    let rect = centered_rect(width, height, area);

    let block = Block::default()
        .title(title)
        .title_style(theme.title)
        .borders(Borders::ALL);

    let paragraph = Paragraph::new(lines).block(block);

    frame.render_widget(Clear, rect);
    frame.render_widget(paragraph, rect);
}

/// Determine the section name from a filter token's prefix.
fn token_section(token: &str) -> &'static str {
    if token.starts_with("status:") {
        "Status"
    } else if token.starts_with("pr:") {
        "Pull Requests"
    } else if token.starts_with("sync:") {
        "Sync"
    } else if token.starts_with("age:") {
        "Age"
    } else {
        "Other"
    }
}

fn filter_line<'a>(
    key: &str,
    label: &str,
    token: &str,
    query: &str,
    key_style: Style,
    active_style: Style,
    label_style: Style,
) -> Line<'a> {
    let is_active = FilterSet::has_token(query, token);
    let marker = if is_active { "\u{25c9} " } else { "\u{25ef} " }; // filled vs empty circle
    let (marker_style, text_style) = if is_active {
        (active_style, active_style)
    } else {
        (label_style, label_style)
    };

    Line::from(vec![
        Span::styled(key.to_string(), key_style),
        Span::styled(" ".to_string(), label_style),
        Span::styled(marker.to_string(), marker_style),
        Span::styled(label.to_string(), text_style),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_section_status() {
        assert_eq!(token_section("status:merged"), "Status");
    }

    #[test]
    fn token_section_pr() {
        assert_eq!(token_section("pr:yes"), "Pull Requests");
    }

    #[test]
    fn token_section_sync() {
        assert_eq!(token_section("sync:ahead"), "Sync");
    }

    #[test]
    fn token_section_age() {
        assert_eq!(token_section("age:<7d"), "Age");
    }
}
