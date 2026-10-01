use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use crate::config::Config;
use crate::symbols::SymbolSet;
use crate::theme::Theme;

use super::modal::{draw_modal_shell, ModalActionRow, ModalFooter, ModalScroll, ModalSpec};

/// A settings row definition with current value display.
pub struct SettingsRow {
    pub label: &'static str,
    pub value: String,
}

/// Build the settings rows from current configuration state.
pub fn settings_rows(
    symbols: &SymbolSet,
    theme: &Theme,
    config: &Config,
    branch_sort_display: &str,
    remote_sort_display: &str,
    tag_sort_display: &str,
    worktree_sort_display: &str,
) -> Vec<SettingsRow> {
    let auto_fetch_display = if config.auto_fetch == Some(true) {
        "on".to_string()
    } else {
        "off".to_string()
    };
    let load_worktrees_display = if config.load_worktrees_on_launch == Some(true) {
        "on".to_string()
    } else {
        "off".to_string()
    };
    let horizontal_scrolling_display = if config.horizontal_scrolling == Some(true) {
        "on".to_string()
    } else {
        "off".to_string()
    };

    vec![
        SettingsRow {
            label: "Symbol set",
            value: symbols.name.to_string(),
        },
        SettingsRow {
            label: "Theme",
            value: theme.name.to_string(),
        },
        SettingsRow {
            label: "Branches sort",
            value: branch_sort_display.to_string(),
        },
        SettingsRow {
            label: "Remotes sort",
            value: remote_sort_display.to_string(),
        },
        SettingsRow {
            label: "Tags sort",
            value: tag_sort_display.to_string(),
        },
        SettingsRow {
            label: "Worktrees sort",
            value: worktree_sort_display.to_string(),
        },
        SettingsRow {
            label: "Auto-fetch on launch",
            value: auto_fetch_display,
        },
        SettingsRow {
            label: "Load worktrees on launch",
            value: load_worktrees_display,
        },
        SettingsRow {
            label: "Horizontal table scrolling",
            value: horizontal_scrolling_display,
        },
    ]
}

/// Renders the settings panel overlay.
///
/// `cursor` is the currently highlighted setting row.
/// `rows` should be built via `settings_rows()`.
pub fn draw_settings(frame: &mut Frame, cursor: usize, rows: &[SettingsRow], theme: &Theme) {
    let areas = draw_modal_shell(
        frame,
        &ModalSpec::new(
            "Settings",
            ModalFooter::hints(&[]),
            60,
            rows.len() as u16 + 3,
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
    scroll.ensure_visible(cursor as u16, rows.len() as u16, areas.body.height);

    let lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            ModalActionRow::new(None, row.label, row.value.clone()).render(i == cursor, theme)
        })
        .collect();

    frame.render_widget(Paragraph::new(lines).scroll((scroll.offset, 0)), areas.body);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn horizontal_scrolling_setting_is_off_by_default_and_shows_enabled_value() {
        let symbols = SymbolSet::ascii();
        let theme = Theme::dark();
        let sort = "Name (ascending)";

        let rows = settings_rows(&symbols, &theme, &Config::default(), sort, sort, sort, sort);
        let horizontal = rows.last().expect("horizontal scrolling row");
        assert_eq!(horizontal.label, "Horizontal table scrolling");
        assert_eq!(horizontal.value, "off");

        let enabled_rows = settings_rows(
            &symbols,
            &theme,
            &Config {
                horizontal_scrolling: Some(true),
                ..Config::default()
            },
            sort,
            sort,
            sort,
            sort,
        );
        assert_eq!(enabled_rows.last().unwrap().value, "on");
    }
}
