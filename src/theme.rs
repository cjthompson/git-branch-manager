use ratatui::style::{Color, Modifier, Style};

#[derive(Debug, Clone)]
pub struct Theme {
    pub name: &'static str,
    pub merged: Style,
    pub squash_merged: Style,
    pub unmerged: Style,
    pub primary_text: Style,
    pub secondary_text: Style,
    pub cursor: Style,
    pub selected: Style,
    pub current_branch: Style,
    pub ahead_behind: Style,
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
}

impl Theme {
    pub fn dark() -> Self {
        Self {
            name: "dark",
            merged: Style::new().fg(Color::Green).add_modifier(Modifier::BOLD),
            squash_merged: Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            unmerged: Style::new().fg(Color::Red),
            primary_text: Style::new().fg(Color::White).add_modifier(Modifier::BOLD),
            secondary_text: Style::new().fg(Color::DarkGray),
            cursor: Style::new().bg(Color::Indexed(24)),
            selected: Style::new().fg(Color::Green).add_modifier(Modifier::BOLD),
            current_branch: Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ahead_behind: Style::new().fg(Color::Cyan),
            pinned_row: Style::new().add_modifier(Modifier::DIM),
            checked_row: Style::new().bg(Color::Indexed(236)),
            error: Style::new().fg(Color::Red).add_modifier(Modifier::BOLD),
            dim: Style::new().add_modifier(Modifier::DIM),
            status_bar: Style::new().bg(Color::DarkGray).fg(Color::White),
            title: Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            header: Style::new()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
                .add_modifier(Modifier::UNDERLINED),
            search_bar: Style::new().bg(Color::Indexed(236)).fg(Color::Yellow),
            pr_draft: Style::new().fg(Color::DarkGray),
            pr_open: Style::new().fg(Color::Green),
            pr_merged: Style::new().fg(Color::Indexed(141)),
            pr_closed: Style::new().fg(Color::Red),
            remote_title: Style::new()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
            remote_header: Style::new()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD)
                .add_modifier(Modifier::UNDERLINED),
            toast_border: Style::new().fg(Color::Yellow),
            toast_text: Style::new()
                .fg(Color::Yellow)
                .add_modifier(Modifier::ITALIC),
        }
    }

    pub fn light() -> Self {
        Self {
            name: "light",
            merged: Style::new()
                .fg(Color::Indexed(28))
                .add_modifier(Modifier::BOLD), // dark green
            squash_merged: Style::new()
                .fg(Color::Indexed(172))
                .add_modifier(Modifier::BOLD), // dark orange
            unmerged: Style::new().fg(Color::Red),
            primary_text: Style::new().fg(Color::Black).add_modifier(Modifier::BOLD),
            secondary_text: Style::new().fg(Color::DarkGray),
            cursor: Style::new().bg(Color::Indexed(153)), // light blue
            selected: Style::new()
                .fg(Color::Indexed(28))
                .add_modifier(Modifier::BOLD), // dark green
            current_branch: Style::new().fg(Color::Blue).add_modifier(Modifier::BOLD),
            ahead_behind: Style::new().fg(Color::Blue),
            pinned_row: Style::new().add_modifier(Modifier::DIM),
            checked_row: Style::new().bg(Color::Indexed(229)),
            error: Style::new().fg(Color::Red).add_modifier(Modifier::BOLD),
            dim: Style::new().add_modifier(Modifier::DIM),
            status_bar: Style::new().bg(Color::Indexed(252)).fg(Color::Black),
            title: Style::new().fg(Color::Blue).add_modifier(Modifier::BOLD),
            header: Style::new()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD)
                .add_modifier(Modifier::UNDERLINED),
            search_bar: Style::new()
                .bg(Color::Indexed(254))
                .fg(Color::Indexed(130)),
            pr_draft: Style::new().fg(Color::DarkGray),
            pr_open: Style::new().fg(Color::Indexed(28)),
            pr_merged: Style::new().fg(Color::Indexed(92)),
            pr_closed: Style::new().fg(Color::Red),
            remote_title: Style::new()
                .fg(Color::Indexed(92))
                .add_modifier(Modifier::BOLD),
            remote_header: Style::new()
                .fg(Color::Indexed(92))
                .add_modifier(Modifier::BOLD)
                .add_modifier(Modifier::UNDERLINED),
            toast_border: Style::new().fg(Color::Indexed(172)),
            toast_text: Style::new()
                .fg(Color::Indexed(172))
                .add_modifier(Modifier::ITALIC),
        }
    }

    pub fn solarized() -> Self {
        // Solarized palette
        let green = Color::Indexed(64);
        let yellow = Color::Indexed(136);
        let red = Color::Indexed(160);
        let blue = Color::Indexed(33);
        let cyan = Color::Indexed(37);
        let base0 = Color::Indexed(244);
        let base01 = Color::Indexed(240);

        Self {
            name: "solarized",
            merged: Style::new().fg(green).add_modifier(Modifier::BOLD),
            squash_merged: Style::new().fg(yellow).add_modifier(Modifier::BOLD),
            unmerged: Style::new().fg(red),
            primary_text: Style::new().fg(base0).add_modifier(Modifier::BOLD),
            secondary_text: Style::new().fg(base01),
            cursor: Style::new().bg(Color::Indexed(236)),
            selected: Style::new().fg(green).add_modifier(Modifier::BOLD),
            current_branch: Style::new().fg(cyan).add_modifier(Modifier::BOLD),
            ahead_behind: Style::new().fg(cyan),
            pinned_row: Style::new().add_modifier(Modifier::DIM),
            checked_row: Style::new().bg(Color::Indexed(22)),
            error: Style::new().fg(red).add_modifier(Modifier::BOLD),
            dim: Style::new().add_modifier(Modifier::DIM),
            status_bar: Style::new().bg(Color::Indexed(235)).fg(base0),
            title: Style::new().fg(blue).add_modifier(Modifier::BOLD),
            header: Style::new()
                .fg(blue)
                .add_modifier(Modifier::BOLD)
                .add_modifier(Modifier::UNDERLINED),
            search_bar: Style::new().bg(Color::Indexed(235)).fg(yellow),
            pr_draft: Style::new().fg(base01),
            pr_open: Style::new().fg(green),
            pr_merged: Style::new().fg(Color::Indexed(61)),
            pr_closed: Style::new().fg(red),
            remote_title: Style::new()
                .fg(Color::Indexed(125))
                .add_modifier(Modifier::BOLD),
            remote_header: Style::new()
                .fg(Color::Indexed(125))
                .add_modifier(Modifier::BOLD)
                .add_modifier(Modifier::UNDERLINED),
            toast_border: Style::new().fg(Color::Indexed(136)),
            toast_text: Style::new()
                .fg(Color::Indexed(136))
                .add_modifier(Modifier::ITALIC),
        }
    }

    pub fn dracula() -> Self {
        let purple = Color::Indexed(141);
        let green = Color::Indexed(84);
        let yellow = Color::Indexed(228);
        let red = Color::Indexed(210);
        let cyan = Color::Indexed(117);
        let fg = Color::Indexed(253); // foreground

        Self {
            name: "dracula",
            merged: Style::new().fg(green).add_modifier(Modifier::BOLD),
            squash_merged: Style::new().fg(yellow).add_modifier(Modifier::BOLD),
            unmerged: Style::new().fg(red),
            primary_text: Style::new().fg(fg).add_modifier(Modifier::BOLD),
            secondary_text: Style::new().fg(Color::Indexed(245)),
            cursor: Style::new().bg(Color::Indexed(238)),
            selected: Style::new().fg(green).add_modifier(Modifier::BOLD),
            current_branch: Style::new().fg(cyan).add_modifier(Modifier::BOLD),
            ahead_behind: Style::new().fg(cyan),
            pinned_row: Style::new().add_modifier(Modifier::DIM),
            checked_row: Style::new().bg(Color::Indexed(22)),
            error: Style::new().fg(red).add_modifier(Modifier::BOLD),
            dim: Style::new().add_modifier(Modifier::DIM),
            status_bar: Style::new().bg(Color::Indexed(236)).fg(fg),
            title: Style::new().fg(purple).add_modifier(Modifier::BOLD),
            header: Style::new()
                .fg(purple)
                .add_modifier(Modifier::BOLD)
                .add_modifier(Modifier::UNDERLINED),
            search_bar: Style::new().bg(Color::Indexed(236)).fg(yellow),
            pr_draft: Style::new().fg(Color::Indexed(245)),
            pr_open: Style::new().fg(green),
            pr_merged: Style::new().fg(purple),
            pr_closed: Style::new().fg(red),
            remote_title: Style::new()
                .fg(Color::Indexed(212))
                .add_modifier(Modifier::BOLD),
            remote_header: Style::new()
                .fg(Color::Indexed(212))
                .add_modifier(Modifier::BOLD)
                .add_modifier(Modifier::UNDERLINED),
            toast_border: Style::new().fg(Color::Indexed(228)),
            toast_text: Style::new()
                .fg(Color::Indexed(228))
                .add_modifier(Modifier::ITALIC),
        }
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
    }

    #[test]
    fn light_theme_has_expected_styles() {
        let t = Theme::light();
        assert_eq!(t.primary_text.fg, Some(Color::Black));
    }
}
