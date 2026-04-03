use crossterm::event::KeyCode;
use ratatui::prelude::*;
use ratatui::style::Modifier;
use ratatui::widgets::Paragraph;

use crate::theme::Theme;

/// A clickable region in the status bar.
#[derive(Debug, Clone)]
pub struct StatusBarItem {
    pub x_start: u16,
    pub x_end: u16,
    pub key: KeyCode,
}

/// Renders a status bar into `area`, parsing `[X]word` patterns.
/// Shortcut keys are styled bold with the theme's accent color.
/// Returns clickable regions for mouse support.
pub fn render_status_bar(
    frame: &mut Frame,
    area: Rect,
    text: &str,
    theme: &Theme,
) -> Vec<StatusBarItem> {
    let accent_color = theme.accent_fg();
    let status_bar_style = theme.status_bar;

    // Phase A: build clickable regions
    let mut items: Vec<StatusBarItem> = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let base_x = area.x;
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '[' && i + 2 < chars.len() && chars[i + 2] == ']' {
            let key_char = chars[i + 1];
            let key_code = char_to_keycode(key_char);
            if let Some(key) = key_code {
                let x_start = base_x + i as u16;
                let mut j = i + 3;
                while j < chars.len() && chars[j] != ' ' && chars[j] != '[' {
                    j += 1;
                }
                let x_end = base_x + j as u16;
                items.push(StatusBarItem {
                    x_start,
                    x_end,
                    key,
                });
                i = j;
            } else {
                i += 1;
            }
        } else {
            i += 1;
        }
    }

    // Phase B: build styled spans and render
    let key_style = Style::default()
        .fg(accent_color)
        .bg(status_bar_style.bg.unwrap_or(ratatui::style::Color::Reset))
        .add_modifier(Modifier::BOLD);
    let mut spans: Vec<Span> = Vec::new();
    let mut remaining = text;
    while let Some(open) = remaining.find('[') {
        if open > 0 {
            spans.push(Span::styled(
                remaining[..open].to_string(),
                status_bar_style,
            ));
        }
        remaining = &remaining[open..];
        if let Some(close) = remaining.find(']') {
            spans.push(Span::styled("[".to_string(), status_bar_style));
            spans.push(Span::styled(remaining[1..close].to_string(), key_style));
            let after_close = &remaining[close..];
            let word_end = after_close[1..]
                .find([' ', '['])
                .map(|idx| idx + 1)
                .unwrap_or(after_close.len());
            spans.push(Span::styled(
                after_close[..word_end].to_string(),
                status_bar_style,
            ));
            remaining = &after_close[word_end..];
        } else {
            spans.push(Span::styled(remaining.to_string(), status_bar_style));
            remaining = "";
        }
    }
    if !remaining.is_empty() {
        spans.push(Span::styled(remaining.to_string(), status_bar_style));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(status_bar_style),
        area,
    );

    items
}

/// Renders a search bar into the status bar area.
pub fn render_search_bar(frame: &mut Frame, area: Rect, query: &str, theme: &Theme) {
    let search_text = format!(" / {}_", query);
    let search_bar = Paragraph::new(search_text).style(theme.search_bar);
    frame.render_widget(search_bar, area);
}

/// Renders a filter indicator in the status bar.
pub fn render_filter_indicator(
    frame: &mut Frame,
    area: Rect,
    query: &str,
    visible_count: usize,
    total_count: usize,
    theme: &Theme,
) {
    let filter_text = format!(
        " filter: \"{}\" ({}/{} shown) \u{2014} [\\]filter [/]edit [Esc in /]clear",
        query, visible_count, total_count
    );
    let status = Paragraph::new(filter_text).style(theme.search_bar);
    frame.render_widget(status, area);
}

fn char_to_keycode(ch: char) -> Option<KeyCode> {
    // Accept any printable character as a shortcut key
    if ch.is_ascii_graphic() {
        Some(KeyCode::Char(ch))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn char_to_keycode_letters() {
        assert_eq!(char_to_keycode('q'), Some(KeyCode::Char('q')));
        assert_eq!(char_to_keycode('D'), Some(KeyCode::Char('D')));
    }

    #[test]
    fn char_to_keycode_special() {
        assert_eq!(char_to_keycode('/'), Some(KeyCode::Char('/')));
        assert_eq!(char_to_keycode('?'), Some(KeyCode::Char('?')));
        assert_eq!(char_to_keycode('\\'), Some(KeyCode::Char('\\')));
    }

    #[test]
    fn char_to_keycode_space_returns_none() {
        assert_eq!(char_to_keycode(' '), None);
    }
}
