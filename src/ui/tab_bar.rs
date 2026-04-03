use ratatui::style::Modifier;
use ratatui::text::{Line, Span};

use crate::theme::Theme;
use crate::view::ViewId;

/// Builds the tab bar Line for the block title.
/// Shows all 4 tabs with the active tab highlighted.
pub fn tab_bar_line(active: ViewId, theme: &Theme) -> Line<'static> {
    let mut spans = Vec::new();
    spans.push(Span::styled(" ", ratatui::style::Style::default()));

    for (i, view_id) in ViewId::ALL.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(
                " \u{2502} ",
                ratatui::style::Style::default().fg(theme.dim_fg()),
            ));
        }

        let label = view_id.label();
        if *view_id == active {
            spans.push(Span::styled(
                label.to_string(),
                theme.title.add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::styled(label.to_string(), theme.secondary_text));
        }
    }

    spans.push(Span::styled(" ", ratatui::style::Style::default()));

    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;
    use crate::view::ViewId;

    #[test]
    fn tab_bar_contains_all_labels() {
        let theme = Theme::dark();
        let line = tab_bar_line(ViewId::Branches, &theme);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("Branches"));
        assert!(text.contains("Remote"));
        assert!(text.contains("Tags"));
        assert!(text.contains("Worktrees"));
    }

    #[test]
    fn tab_bar_different_active() {
        let theme = Theme::dark();
        let line = tab_bar_line(ViewId::Tags, &theme);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("Tags"));
    }
}
