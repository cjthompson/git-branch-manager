use chrono::{DateTime, Utc};
use ratatui::prelude::*;
use ratatui::style::{Color, Style};

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

/// Returns a centered rectangle of given dimensions within the provided area.
/// `width_pct` is a percentage (0-100) of the area width; `height` is absolute rows.
pub fn centered_rect_pct(width_pct: u16, height: u16, area: Rect) -> Rect {
    let width = (area.width as u32 * width_pct as u32 / 100) as u16;
    centered_rect(width, height, area)
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
}
