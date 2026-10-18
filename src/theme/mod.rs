mod palettes;

use ratatui::style::{Color, Style};

#[derive(Debug, Clone)]
pub struct Theme {
    pub name: &'static str,
    pub merged: Style,
    pub squash_merged: Style,
    pub cherry_picked: Style,
    /// Style for branches whose tip is literally the base tip (no integration
    /// event). Distinct from `merged` so users can tell "no work was done" from
    /// "work was integrated".
    pub in_sync: Style,
    pub unmerged: Style,
    pub primary_text: Style,
    pub secondary_text: Style,
    pub cursor: Style,
    pub selected: Style,
    pub current_branch: Style,
    pub ahead: Style,
    pub behind: Style,
    pub pinned_row: Style,
    pub checked_row: Style,
    pub error: Style,
    pub dim: Style,
    pub status_bar: Style,
    pub title: Style,
    pub header: Style,
    pub search_bar: Style,
    pub pr_draft: Style,
    pub pr_open: Style,
    pub pr_merged: Style,
    pub pr_closed: Style,
    pub remote_title: Style,
    pub remote_header: Style,
    pub toast_border: Style,
    pub toast_text: Style,
    pub modal_border: Style,
    pub modal_surface: Style,
    pub modal_title: Style,
    pub modal_footer: Style,
    pub modal_key: Style,
    pub modal_branch: Style,
    pub modal_worktree: Style,
    pub modal_commit: Style,
    pub modal_command: Style,
    pub modal_success: Style,
    pub modal_failure: Style,
    pub modal_secondary: Style,
    pub modal_action_unavailable: Style,
    pub modal_action_selected: Style,
    pub modal_warning: Style,
}

impl Theme {
    /// Returns the dim foreground color (from the `dim` style's fg, or DarkGray fallback).
    pub fn dim_fg(&self) -> Color {
        self.dim.fg.unwrap_or(Color::DarkGray)
    }

    /// Returns the accent color (from the `title` style's fg, or White fallback).
    pub fn accent_fg(&self) -> Color {
        self.title.fg.unwrap_or(Color::White)
    }

    pub fn from_name(name: &str) -> Self {
        match name {
            "light" => Self::light(),
            "solarized" => Self::solarized(),
            "dracula" => Self::dracula(),
            _ => Self::dark(),
        }
    }

    pub fn next(&self) -> Self {
        match self.name {
            "dark" => Self::light(),
            "light" => Self::solarized(),
            "solarized" => Self::dracula(),
            "dracula" => Self::dark(),
            _ => Self::dark(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_cycle() {
        let t = Theme::dark();
        let t2 = t.next();
        assert_eq!(t2.name, "light");
        let t3 = t2.next();
        assert_eq!(t3.name, "solarized");
        let t4 = t3.next();
        assert_eq!(t4.name, "dracula");
        let t5 = t4.next();
        assert_eq!(t5.name, "dark");
    }

    #[test]
    fn theme_from_name() {
        let t = Theme::from_name("dracula");
        assert_eq!(t.name, "dracula");
        let t = Theme::from_name("invalid");
        assert_eq!(t.name, "dark"); // default
    }

    #[test]
    fn theme_from_name_all_variants() {
        assert_eq!(Theme::from_name("dark").name, "dark");
        assert_eq!(Theme::from_name("light").name, "light");
        assert_eq!(Theme::from_name("solarized").name, "solarized");
        assert_eq!(Theme::from_name("dracula").name, "dracula");
    }

    #[test]
    fn dark_theme_has_expected_styles() {
        let t = Theme::dark();
        // Just verify a few key styles are configured
        assert_eq!(t.merged.fg, Some(Color::Green));
        assert_eq!(t.unmerged.fg, Some(Color::Red));
        assert_eq!(t.current_branch.fg, Some(Color::Cyan));
        assert_eq!(t.ahead.fg, Some(Color::Green));
        assert_eq!(t.behind.fg, Some(Color::Yellow));
    }

    #[test]
    fn light_theme_has_expected_styles() {
        let t = Theme::light();
        assert_eq!(t.primary_text.fg, Some(Color::Black));
    }

    #[test]
    fn ahead_and_behind_use_distinct_colors_in_every_theme() {
        for theme in [
            Theme::dark(),
            Theme::light(),
            Theme::solarized(),
            Theme::dracula(),
        ] {
            assert_ne!(
                theme.ahead.fg, theme.behind.fg,
                "{} theme should color ahead/behind differently",
                theme.name
            );
        }
    }

    #[test]
    fn dim_style_has_a_foreground_color_in_every_theme() {
        // Regression: `dim_fg()` and `key_hint`/`block_panel` (ui/shared.rs)
        // rely on `theme.dim.fg` being set — a theme that forgot to set it
        // would silently fall back to `dim_fg()`'s DarkGray default instead
        // of failing loudly.
        for theme in [
            Theme::dark(),
            Theme::light(),
            Theme::solarized(),
            Theme::dracula(),
        ] {
            assert!(
                theme.dim.fg.is_some(),
                "{} theme's `dim` style must set an fg color",
                theme.name
            );
        }
    }
}
