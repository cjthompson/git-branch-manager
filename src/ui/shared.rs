use chrono::{DateTime, Utc};
use ratatui::prelude::*;
use ratatui::style::{Color, Modifier, Style};

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
