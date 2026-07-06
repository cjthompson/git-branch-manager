use ratatui::prelude::*;
use ratatui::style::Modifier;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::config::Config;
use crate::symbols::SymbolSet;
use crate::theme::Theme;

use super::shared::centered_rect;

/// A settings row definition with current value display.
pub struct SettingsRow {
    pub label: &'static str,
    pub value: String,
}

/// Build the settings rows from current configuration state.
pub fn settings_rows(symbols: &SymbolSet, theme: &Theme, config: &Config) -> Vec<SettingsRow> {
    let sort_col_display = match config.sort_column.as_deref() {
        Some("name") => "name",
        Some("remote") => "remote",
        Some("age") => "age",
        Some("ahead") => "ahead",
        Some("pr") => "pr",
        Some("status") => "status",
        _ => "none",
    }
    .to_string();
    let sort_dir_display = if config.sort_asc != Some(false) {
        "ascending".to_string()
    } else {
        "descending".to_string()
    };
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
            label: "Default sort column",
            value: sort_col_display,
        },
        SettingsRow {
            label: "Default sort direction",
            value: sort_dir_display,
        },
        SettingsRow {
            label: "Auto-fetch on launch",
            value: auto_fetch_display,
        },
        SettingsRow {
            label: "Load worktrees on launch",
            value: load_worktrees_display,
        },
    ]
}

/// Renders the settings panel overlay.
///
/// `cursor` is the currently highlighted setting row.
/// `rows` should be built via `settings_rows()`.
pub fn draw_settings(frame: &mut Frame, cursor: usize, rows: &[SettingsRow], theme: &Theme) {
    let area = frame.area();
    let width = 60u16.min(area.width);
    let height = (rows.len() as u16 + 4).min(area.height); // +4 for borders + instructions
    let rect = centered_rect(width, height, area);

    let block = Block::default()
        .title(" Settings ")
        .title_style(theme.title)
        .borders(Borders::ALL);

    let inner = block.inner(rect);
    frame.render_widget(Clear, rect);
    frame.render_widget(block, rect);

    let mut lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let style = if i == cursor {
                theme.cursor
            } else {
                Style::default()
            };
            Line::from(vec![
                Span::styled(format!("  {:<30}", row.label), style),
                Span::styled(row.value.clone(), style.add_modifier(Modifier::BOLD)),
            ])
        })
        .collect();

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  \u{2190}/\u{2192} cycle   Esc close",
        theme.dim,
    )));

    frame.render_widget(Paragraph::new(lines), inner);
}
