use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use crate::symbols::SymbolSet;
use crate::theme::Theme;
use crate::types::BranchAction;

use super::modal::{draw_modal_shell, ModalActionRow, ModalFooter, ModalScroll, ModalSpec};

/// A single item in the context menu overlay.
#[derive(Debug, Clone)]
pub struct MenuItem {
    pub label: String,
    pub shortcut: Option<char>,
    pub action: BranchAction,
    /// Exact operation target resolved from the authoritative backing row.
    pub target: String,
    /// Remote name for remote-branch operations, when the target is a remote ref.
    pub remote: Option<String>,
    pub enabled: bool,
    pub reason: Option<String>,
}

/// Renders a context menu overlay through the shared modal shell.
pub fn draw_menu(
    frame: &mut Frame,
    items: &[MenuItem],
    cursor: usize,
    theme: &Theme,
    symbols: &SymbolSet,
) {
    let areas = draw_modal_shell(
        frame,
        &ModalSpec::new(
            "Actions",
            ModalFooter::hints(&[("j/k", "Navigate"), ("Enter", "Select"), ("Esc", "Close")]),
            54,
            items.len() as u16 + 3,
        ),
        theme,
    );
    let mut scroll = ModalScroll::default();
    scroll.ensure_visible(cursor as u16, items.len() as u16, areas.body.height);

    let lines: Vec<Line> = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let selected = i == cursor && item.enabled;
            let prefix = if selected {
                format!("{} ", symbols.cursor_prefix)
            } else {
                "  ".to_string()
            };

            let mut line = ModalActionRow::new(
                item.shortcut,
                item.label.clone(),
                item.reason.clone().unwrap_or_default(),
            )
            .render_with_availability(selected, item.enabled, theme);
            line.spans.insert(
                0,
                Span::styled(
                    prefix,
                    if item.enabled {
                        Style::default()
                    } else {
                        theme.modal_action_unavailable
                    },
                ),
            );
            line
        })
        .collect();

    frame.render_widget(Paragraph::new(lines).scroll((scroll.offset, 0)), areas.body);
}
