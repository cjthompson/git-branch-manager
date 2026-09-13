use chrono::{DateTime, Utc};
use ratatui::prelude::*;
use ratatui::style::{Color, Modifier, Style};
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
    let key_style = Style::default()
        .fg(theme.accent_fg())
        .add_modifier(Modifier::BOLD);
    vec![
        Span::styled("[", theme.dim),
        Span::styled(key.to_string(), key_style),
        Span::styled(format!("] {label}"), theme.dim),
    ]
}

/// Returns a bordered `Block` with the theme's dim border color and 1-column
/// horizontal padding, giving every overlay a consistent surface. Callers
/// chain `.title(...)`/`.title_style(...)`/`.borders(...)` afterward as
/// needed (e.g. to override to a partial border set for split panes).
pub fn block_panel(theme: &Theme) -> Block<'_> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(theme.dim)
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
mod tests {
    use super::*;
    use crate::theme::Theme;
    use chrono::Utc;

    #[test]
    fn prefix_style_known_prefixes() {
        let theme = Theme::dark();
        assert!(prefix_style("fix", &theme).is_some());
        assert!(prefix_style("feat", &theme).is_some());
        assert!(prefix_style("feature", &theme).is_some());
        assert!(prefix_style("chore", &theme).is_some());
        assert!(prefix_style("hotfix", &theme).is_some());
        assert!(prefix_style("release", &theme).is_some());
    }

    #[test]
    fn prefix_style_unknown_returns_none() {
        let theme = Theme::dark();
        assert!(prefix_style("unknown", &theme).is_none());
        assert!(prefix_style("main", &theme).is_none());
    }

    #[test]
    fn age_style_recent() {
        let theme = Theme::dark();
        let recent = Utc::now() - chrono::Duration::days(1);
        let style = age_style(&recent, &theme);
        assert_eq!(style.fg, Some(Color::Green));
    }

    #[test]
    fn age_style_week_old() {
        let theme = Theme::dark();
        let date = Utc::now() - chrono::Duration::days(10);
        let style = age_style(&date, &theme);
        assert_eq!(style.fg, Some(Color::Yellow));
    }

    #[test]
    fn age_style_month_old() {
        let theme = Theme::dark();
        let date = Utc::now() - chrono::Duration::days(45);
        let style = age_style(&date, &theme);
        assert_eq!(style.fg, Some(Color::Indexed(208)));
    }

    #[test]
    fn age_style_old() {
        let theme = Theme::dark();
        let date = Utc::now() - chrono::Duration::days(100);
        let style = age_style(&date, &theme);
        assert_eq!(style.fg, Some(Color::Red));
    }

    #[test]
    fn truncate_short_string() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn truncate_exact_length() {
        assert_eq!(truncate("hello", 5), "hello");
    }

    #[test]
    fn truncate_long_string() {
        let result = truncate("hello world", 6);
        assert_eq!(result, "hello\u{2026}");
    }

    #[test]
    fn truncate_width_one() {
        assert_eq!(truncate("hello", 1), "\u{2026}");
    }

    #[test]
    fn truncate_width_zero() {
        assert_eq!(truncate("hello", 0), "");
    }

    #[test]
    fn truncate_left_short_string() {
        assert_eq!(truncate_left("hello", 10), "hello");
    }

    #[test]
    fn truncate_left_keeps_tail() {
        // Keeps the last (max_width - 1) chars, prefixed with the ellipsis.
        assert_eq!(truncate_left("hello world", 6), "\u{2026}world");
    }

    #[test]
    fn truncate_left_width_one() {
        assert_eq!(truncate_left("hello", 1), "\u{2026}");
    }

    #[test]
    fn truncate_left_width_zero() {
        assert_eq!(truncate_left("hello", 0), "\u{2026}");
    }

    #[test]
    fn truncate_left_handles_non_ascii_without_panicking() {
        // Regression for a latent panic in draw_executing (ui/executing.rs),
        // which calls truncate_left on a branch's current_item name. Branch
        // names can contain multi-byte UTF-8 (e.g. accented/CJK characters);
        // truncate_left must slice on char boundaries, not bytes.
        let s = "feature/日本語-emoji-🚀-café-branch";
        let result = truncate_left(s, 10);
        assert!(result.chars().count() <= 10, "got: {result:?}");
        assert!(result.starts_with('\u{2026}'), "got: {result:?}");

        // Also verify the short-circuit path (no truncation needed) doesn't panic.
        let short = "日本語";
        assert_eq!(truncate_left(short, 10), "日本語");
    }

    #[test]
    fn abbreviate_path_fits_unchanged() {
        let p = std::path::Path::new("/Users/chris/dev/proj/feat");
        assert_eq!(abbreviate_path(p, 100), "/Users/chris/dev/proj/feat");
    }

    #[test]
    fn abbreviate_path_zero_width() {
        let p = std::path::Path::new("/Users/chris/dev/proj/feat");
        assert_eq!(abbreviate_path(p, 0), "");
    }

    #[test]
    fn abbreviate_path_abbreviates_leading_keeps_tail() {
        let p = std::path::Path::new("/Users/chris/dev/git-branch-manager/.claude/worktrees/feat");
        // Wide enough to keep the tail full but too narrow for the whole path.
        let result = abbreviate_path(p, 45);
        assert!(result.starts_with('/'), "got: {result:?}");
        assert!(result.ends_with("/feat"), "got: {result:?}");
        // Last component must be kept full (not abbreviated to "f").
        assert!(result.contains("/feat"), "got: {result:?}");
        assert!(result.chars().count() <= 45, "got: {result:?}");
    }

    #[test]
    fn abbreviate_path_left_truncates_when_very_narrow() {
        let p = std::path::Path::new("/Users/chris/dev/git-branch-manager/.claude/worktrees/feat");
        let result = abbreviate_path(p, 8);
        assert!(result.starts_with('\u{2026}'), "got: {result:?}");
        assert!(result.chars().count() <= 8, "got: {result:?}");
    }

    #[test]
    fn abbreviate_path_shortens_worktrees_before_clipping_tail() {
        // Real-world shape: long shared prefix, then `worktrees`, then a long
        // worktree name. At this width, keeping `worktrees` full would overflow,
        // so it must be abbreviated too — and the final name stays fully visible.
        let p = std::path::Path::new(
            "/Users/chris/workspace/zen/.claude/worktrees/idempotent-create-payroll-admin-rspec",
        );
        let result = abbreviate_path(p, 52);
        assert!(!result.contains("worktrees"), "got: {result:?}");
        assert!(
            result.contains("/w/"),
            "expected worktrees→w; got: {result:?}"
        );
        assert!(
            result.ends_with("idempotent-create-payroll-admin-rspec"),
            "tail must stay visible; got: {result:?}"
        );
        assert!(result.chars().count() <= 52, "got: {result:?}");
    }

    #[test]
    fn abbreviate_path_single_component_narrow() {
        let p = std::path::Path::new("my-feature");
        let result = abbreviate_path(p, 5);
        // No parents to abbreviate → left-truncated, end visible, no panic.
        assert!(result.starts_with('\u{2026}'), "got: {result:?}");
        assert!(result.ends_with("ure"), "got: {result:?}");
        assert!(result.chars().count() <= 5, "got: {result:?}");
    }

    #[test]
    fn centered_rect_basic() {
        let area = Rect::new(0, 0, 80, 24);
        let r = centered_rect(40, 10, area);
        assert_eq!(r.x, 20);
        assert_eq!(r.y, 7);
        assert_eq!(r.width, 40);
        assert_eq!(r.height, 10);
    }

    #[test]
    fn centered_rect_larger_than_area() {
        let area = Rect::new(0, 0, 40, 10);
        let r = centered_rect(80, 20, area);
        assert_eq!(r.width, 40);
        assert_eq!(r.height, 10);
    }

    #[test]
    fn centered_rect_pct_50() {
        let area = Rect::new(0, 0, 100, 50);
        let r = centered_rect_pct(50, 10, area);
        assert_eq!(r.width, 50);
        assert_eq!(r.x, 25);
    }

    #[test]
    fn render_progress_bar_empty() {
        let bar = render_progress_bar(20, 0, 10);
        assert!(bar.starts_with('['));
        assert!(bar.ends_with(" 0/10"));
    }

    #[test]
    fn render_progress_bar_full() {
        let bar = render_progress_bar(20, 10, 10);
        assert!(bar.ends_with(" 10/10"));
        // Fully filled: no spaces between the last '=' and the closing ']'.
        let inside = bar.split(']').next().unwrap();
        assert!(!inside.contains(' '));
    }

    #[test]
    fn render_progress_bar_zero_total_does_not_panic() {
        let bar = render_progress_bar(20, 0, 0);
        assert!(bar.ends_with(" 0/0"));
    }

    #[test]
    fn key_hint_produces_three_styled_spans() {
        let theme = Theme::dark();
        let spans = key_hint('d', "delete", &theme);

        assert_eq!(spans.len(), 3);

        assert_eq!(spans[0].content.as_ref(), "[");
        assert_eq!(spans[0].style, theme.dim);

        assert_eq!(spans[1].content.as_ref(), "d");
        assert_eq!(
            spans[1].style,
            Style::default()
                .fg(theme.accent_fg())
                .add_modifier(Modifier::BOLD)
        );

        assert_eq!(spans[2].content.as_ref(), "] delete");
        assert_eq!(spans[2].style, theme.dim);
    }

    #[test]
    fn block_panel_inner_accounts_for_border_and_padding() {
        let theme = Theme::dark();
        let block = block_panel(&theme);

        // Borders::ALL removes 1 cell per side; padding adds 1 more on
        // left/right only (Padding::horizontal(1), no vertical padding).
        let area = Rect::new(0, 0, 20, 10);
        let inner = block.inner(area);
        assert_eq!(inner, Rect::new(2, 1, 16, 8));
    }

    #[test]
    fn block_panel_draws_all_four_borders_in_theme_dim_style() {
        let theme = Theme::dark();
        let block = block_panel(&theme);

        let area = Rect::new(0, 0, 10, 5);
        let mut buf = Buffer::empty(area);
        block.render(area, &mut buf);

        // Top-left corner: a border glyph styled with the theme's dim fg.
        let corner = buf.cell((0, 0)).unwrap();
        assert_ne!(
            corner.symbol(),
            " ",
            "expected a border glyph at the top-left corner"
        );
        assert_eq!(corner.fg, theme.dim.fg.unwrap());

        // Borders::ALL: mid-point of every edge should be a non-blank glyph.
        assert_ne!(buf.cell((5, 0)).unwrap().symbol(), " ", "top border missing");
        assert_ne!(buf.cell((5, 4)).unwrap().symbol(), " ", "bottom border missing");
        assert_ne!(buf.cell((0, 2)).unwrap().symbol(), " ", "left border missing");
        assert_ne!(buf.cell((9, 2)).unwrap().symbol(), " ", "right border missing");
    }
}
