use chrono::{DateTime, Utc};
use crossterm::event::KeyCode;
use ratatui::prelude::*;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::Paragraph;

use crate::app::View;

/// Returns a color style for known branch name prefixes (text before the first `/`).
pub fn prefix_style(prefix: &str) -> Option<Style> {
    match prefix {
        "fix" => Some(Style::new().fg(Color::Red)),
        "feat" | "feature" => Some(Style::new().fg(Color::Green)),
        "chore" => Some(Style::new().fg(Color::Indexed(130))),
        "hotfix" => Some(Style::new().fg(Color::Magenta)),
        "release" => Some(Style::new().fg(Color::Cyan)),
        _ => None,
    }
}

/// Returns a color style based on how old a commit is.
pub fn age_style(date: &DateTime<Utc>) -> Style {
    let days = (Utc::now() - *date).num_days();
    if days < 7 {
        Style::new().fg(Color::Green)
    } else if days < 30 {
        Style::new().fg(Color::Yellow)
    } else if days < 90 {
        Style::new().fg(Color::Indexed(208)) // orange
    } else {
        Style::new().fg(Color::Red)
    }
}

/// Truncates `s` to fit within `max_chars`, appending `ellipsis` if truncated.
pub fn truncate(s: &str, max_chars: usize, ellipsis: &str) -> String {
    if s.len() <= max_chars {
        s.to_string()
    } else if max_chars > ellipsis.len() {
        format!("{}{}", &s[..max_chars - ellipsis.len()], ellipsis)
    } else {
        ellipsis.to_string()
    }
}

/// Builds a styled tab bar `Line` for use as a Block title.
/// The `active` parameter indicates which view is currently displayed.
/// The `title_style` is the style used for the active tab text.
pub fn tab_bar_line<'a>(active: &View, title_style: Style) -> Line<'a> {
    let tabs: &[(&str, View)] = &[
        ("Branches", View::BranchList),
        ("Remote", View::RemoteBranches),
        ("Worktrees", View::Worktrees),
        ("Help", View::Help),
    ];

    let inactive_style = Style::default().add_modifier(Modifier::DIM);
    let separator_style = Style::default().add_modifier(Modifier::DIM);

    let mut spans: Vec<Span<'a>> = Vec::new();
    spans.push(Span::styled(" ", Style::default()));

    for (i, (label, view)) in tabs.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" | ", separator_style));
        }
        if *active == *view {
            spans.push(Span::styled((*label).to_string(), title_style));
        } else {
            spans.push(Span::styled((*label).to_string(), inactive_style));
        }
    }

    spans.push(Span::styled(" ", Style::default()));

    Line::from(spans)
}

/// Renders a status bar Paragraph into `area`, parsing [X]word patterns.
/// Shortcut keys are styled bold with `accent_color` fg.
/// Returns clickable regions as (x_start, x_end_exclusive, KeyCode).
pub fn render_status_bar(
    frame: &mut Frame,
    area: Rect,
    text: &str,
    accent_color: Color,
    status_bar_style: Style,
) -> Vec<(u16, u16, KeyCode)> {
    // Phase A: build clickable regions
    let mut items: Vec<(u16, u16, KeyCode)> = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let base_x = area.x;
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '[' && i + 2 < chars.len() && chars[i + 2] == ']' {
            let key_char = chars[i + 1];
            let key_code = match key_char {
                '/' => KeyCode::Char('/'),
                '?' => KeyCode::Char('?'),
                'q' => KeyCode::Char('q'),
                'c' => KeyCode::Char('c'),
                'd' => KeyCode::Char('d'),
                'D' => KeyCode::Char('D'),
                'f' => KeyCode::Char('f'),
                'r' => KeyCode::Char('r'),
                'w' => KeyCode::Char('w'),
                't' => KeyCode::Char('t'),
                'p' => KeyCode::Char('p'),
                's' => KeyCode::Char('s'),
                'E' => KeyCode::Char('E'),
                'a' => KeyCode::Char('a'),
                'n' => KeyCode::Char('n'),
                'i' => KeyCode::Char('i'),
                _ => {
                    i += 1;
                    continue;
                }
            };
            let x_start = base_x + i as u16;
            let mut j = i + 3;
            while j < chars.len() && chars[j] != ' ' && chars[j] != '[' {
                j += 1;
            }
            let x_end = base_x + j as u16;
            items.push((x_start, x_end, key_code));
            i = j;
        } else {
            i += 1;
        }
    }

    // Phase B: build styled spans and render
    let key_style = Style::default()
        .fg(accent_color)
        .bg(status_bar_style.bg.unwrap_or(Color::Reset))
        .add_modifier(Modifier::BOLD);
    let mut spans: Vec<Span> = Vec::new();
    let mut remaining = text;
    while let Some(open) = remaining.find('[') {
        if open > 0 {
            spans.push(Span::styled(remaining[..open].to_string(), status_bar_style));
        }
        remaining = &remaining[open..];
        if let Some(close) = remaining.find(']') {
            spans.push(Span::styled("[".to_string(), status_bar_style));
            spans.push(Span::styled(remaining[1..close].to_string(), key_style));
            let after_close = &remaining[close..];
            let word_end = after_close[1..]
                .find(|c: char| c == ' ' || c == '[')
                .map(|idx| idx + 1)
                .unwrap_or(after_close.len());
            spans.push(Span::styled(after_close[..word_end].to_string(), status_bar_style));
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
