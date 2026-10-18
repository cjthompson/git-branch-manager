use chrono::{DateTime, Utc};
use ratatui::prelude::*;
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders, Padding};

use crate::theme::Theme;

/// Returns a color style for known branch name prefixes (text before the first `/`).
/// Colors are theme-independent since they represent semantic categories.
pub fn prefix_style(prefix: &str, _theme: &Theme) -> Option<Style> {
    match prefix {
        "fix" => Some(Style::new().fg(Color::Red)),
        "feat" | "feature" => Some(Style::new().fg(Color::Green)),
        "chore" => Some(Style::new().fg(Color::Indexed(130))), // amber
        "hotfix" => Some(Style::new().fg(Color::Magenta)),
        "release" => Some(Style::new().fg(Color::Cyan)),
        _ => None,
    }
}

/// Returns a color style based on how old a commit is.
/// <7d green, <30d yellow, <90d orange, >90d red.
pub fn age_style(date: &DateTime<Utc>, _theme: &Theme) -> Style {
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

/// Truncates `s` to fit within `max_width` characters, appending an ellipsis if truncated.
/// Uses unicode ellipsis by default.
pub fn truncate(s: &str, max_width: usize) -> String {
    if s.chars().count() <= max_width {
        s.to_string()
    } else if max_width > 1 {
        let truncated: String = s.chars().take(max_width - 1).collect();
        format!("{}\u{2026}", truncated)
    } else if max_width == 1 {
        "\u{2026}".to_string()
    } else {
        String::new()
    }
}

/// Truncates `s` from the LEFT to fit `max_width`, prefixing `…` so the END
/// of the string stays visible. Mirror of [`truncate`], which drops the tail.
pub fn truncate_left(s: &str, max_width: usize) -> String {
    let count = s.chars().count();
    if count <= max_width {
        s.to_string()
    } else if max_width <= 1 {
        "\u{2026}".to_string()
    } else {
        let skip = count - (max_width - 1);
        let tail: String = s.chars().skip(skip).collect();
        format!("\u{2026}{tail}")
    }
}

/// Joins path `segs` with `/`, restoring a leading slash when `had_root`.
fn join_path(segs: &[String], had_root: bool) -> String {
    let body = segs.join("/");
    if had_root {
        format!("/{body}")
    } else {
        body
    }
}

/// Formats a filesystem path to fit `max_width`, keeping the END visible.
///
/// - If the full path fits, returns it unchanged.
/// - Otherwise abbreviates leading directory components to their first
///   character, left-to-right, stopping as soon as it fits (the final
///   component is always kept full).
/// - If even the fully-abbreviated form is too wide, left-truncates with `…`.
///
/// Example (narrowing): `/Users/chris/dev/git-branch-manager/.claude/worktrees/feat`
///   → `/U/c/d/git-branch-manager/.claude/worktrees/feat`
///   → `…/worktrees/feat`
pub fn abbreviate_path(path: &std::path::Path, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    let full = path.to_string_lossy();
    if full.chars().count() <= max_width {
        return full.into_owned();
    }

    let had_root = full.starts_with('/');
    let mut segs: Vec<String> = full
        .split('/')
        .filter(|p| !p.is_empty())
        .map(|p| p.to_string())
        .collect();

    if segs.len() > 1 {
        let last = segs.len() - 1;
        for i in 0..last {
            if let Some(c) = segs[i].chars().next() {
                segs[i] = c.to_string();
            }
            let candidate = join_path(&segs, had_root);
            if candidate.chars().count() <= max_width {
                return candidate;
            }
        }
    }

    truncate_left(&join_path(&segs, had_root), max_width)
}

/// Returns a centered rectangle of given dimensions within the provided area.
/// `width_pct` is a percentage (0-100) of the area width; `height` is absolute rows.
pub fn centered_rect_pct(width_pct: u16, height: u16, area: Rect) -> Rect {
    let width = (area.width as u32 * width_pct as u32 / 100) as u16;
    centered_rect(width, height, area)
}

/// Renders a `[====>   ] completed/total` progress bar string sized to fit
/// within `inner_width` characters. Shared by the `Executing` modal (used by
/// fetch/cache-audit) and the non-modal job-status area (used by confirmed
/// actions), so both draw identical bars.
pub fn render_progress_bar(inner_width: usize, completed: usize, total: usize) -> String {
    let count_text = format!(" {completed}/{total}");
    let bar_width = inner_width
        .saturating_sub(count_text.len())
        .saturating_sub(2); // -2 for []

    let fraction = if total > 0 {
        completed as f64 / total as f64
    } else {
        0.0
    };
    let filled = (fraction * bar_width as f64) as usize;
    let empty = bar_width.saturating_sub(filled);

    format!(
        "[{}{}]{}",
        "=".repeat(filled),
        " ".repeat(empty),
        count_text
    )
}

/// Returns the canonical `[x] label` hint spans used across overlay footers
/// and menu rows: dim brackets around a bold-accent key, followed by a
/// dim-styled label. Shared by confirm/menu/info-modal/status-bar hint text
/// so every overlay renders shortcut hints identically.
pub fn key_hint(key: char, label: &str, theme: &Theme) -> Vec<Span<'static>> {
    key_hint_str(&key.to_string(), label, theme)
}

/// Returns the canonical `[key] label` hint spans for string-capable shortcut
/// keys, including multi-character keys such as `Esc`.
pub fn key_hint_str(key: &str, label: &str, theme: &Theme) -> Vec<Span<'static>> {
    vec![
        Span::styled("[", theme.modal_footer),
        Span::styled(key.to_string(), theme.modal_key),
        Span::styled(format!("] {label}"), theme.modal_footer),
    ]
}

/// Returns a bordered `Block` with the theme's modal border and surface styles
/// plus 1-column horizontal padding, giving every overlay a consistent panel.
/// Callers chain `.title(...)`/`.title_style(...)`/`.borders(...)` afterward as
/// needed (e.g. to override to a partial border set for split panes).
pub fn block_panel(theme: &Theme) -> Block<'_> {
    Block::default()
        .borders(Borders::ALL)
        .style(theme.modal_surface)
        .border_style(theme.modal_border)
        .padding(Padding::horizontal(1))
}

/// Returns a centered rectangle with absolute width and height within the provided area.
pub fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect::new(x, y, w, h)
}

#[cfg(test)]
mod tests;
