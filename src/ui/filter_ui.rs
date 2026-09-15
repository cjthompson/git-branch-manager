use ratatui::prelude::*;
use ratatui::widgets::{Paragraph, Wrap};

use crate::theme::Theme;
use crate::view::filter::{FilterSet, FilterTokenDef};

use super::modal::{draw_modal_shell, ModalActionRow, ModalFooter, ModalScroll, ModalSpec};

/// Renders the generic filter builder overlay.
///
/// This single function works for all views by accepting the view's filter token
/// definitions. Active tokens use the modal success role.
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
    draw_filter_with_selection(frame, filter_tokens, current_query, title, None, theme);
}

/// Renders the interactive filter action list with a selected action.
pub(crate) fn draw_filter_selected(
    frame: &mut Frame,
    filter_tokens: &[FilterTokenDef],
    current_query: &str,
    title: &str,
    cursor: usize,
    theme: &Theme,
) {
    draw_filter_with_selection(
        frame,
        filter_tokens,
        current_query,
        title,
        Some(cursor),
        theme,
    );
}

fn draw_filter_with_selection(
    frame: &mut Frame,
    filter_tokens: &[FilterTokenDef],
    current_query: &str,
    title: &str,
    cursor: Option<usize>,
    theme: &Theme,
) {
    // Group tokens by category based on their token prefix
    let mut lines: Vec<Line> = Vec::new();
    let mut selected_row = None;
    let mut current_section: Option<&str> = None;

    for (index, token_def) in filter_tokens.iter().enumerate() {
        let section = token_section(token_def.token);

        // Insert section header if we've entered a new section
        if current_section != Some(section) {
            if current_section.is_some() {
                lines.push(Line::from("")); // blank line between sections
            }
            lines.push(Line::from(Span::styled(section, theme.modal_title)));
            current_section = Some(section);
        }

        let selected = cursor == Some(index);
        if selected {
            selected_row = Some(lines.len().min(u16::MAX as usize) as u16);
        }
        let marker = if FilterSet::has_token(current_query, token_def.token) {
            "\u{25c9}"
        } else {
            "\u{25ef}"
        };
        let state = if FilterSet::has_token(current_query, token_def.token) {
            "Active"
        } else {
            "Inactive"
        };
        lines.push(
            ModalActionRow::new(
                Some(token_def.key),
                format!("{marker} {}", token_def.label),
                state,
            )
            .render(selected, theme),
        );
    }

    // Add clear all option
    lines.push(Line::from(""));
    let clear_selected = cursor == Some(filter_tokens.len());
    if clear_selected {
        selected_row = Some(lines.len().min(u16::MAX as usize) as u16);
    }
    lines.push(
        ModalActionRow::new(Some('c'), "Clear all filters", "Remove every active filter")
            .render(clear_selected, theme),
    );

    let content_max_width = lines
        .iter()
        .map(|l: &Line| {
            l.spans
                .iter()
                .map(|s| s.content.chars().count())
                .sum::<usize>()
        })
        .max()
        .unwrap_or(0) as u16;
    let areas = draw_modal_shell(
        frame,
        &ModalSpec::new(
            title.to_string(),
            ModalFooter::hints(&[]),
            (content_max_width + 4).max(48),
            lines.len() as u16 + 3,
        ),
        theme,
    );

    ModalFooter::adaptive_hints(
        areas.footer,
        &[("j/k", "Select"), ("Enter", "Choose"), ("Esc", "Close")],
        &["j/k", "Enter", "Esc"],
    )
    .render(theme, areas.footer, frame);
    let mut scroll = ModalScroll::default();
    if let Some(selected_row) = selected_row {
        scroll.ensure_visible(selected_row, lines.len() as u16, areas.body.height);
    }

    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((scroll.offset, 0)),
        areas.body,
    );
}

/// Determine the section name from a filter token's prefix.
fn token_section(token: &str) -> &'static str {
    if token.starts_with("merge:") {
        "Merge Status"
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_section_merge() {
        assert_eq!(token_section("merge:merged"), "Merge Status");
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
