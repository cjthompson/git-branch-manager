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
    pub error: Style,
    pub dim: Style,
    pub status_bar: Style,
    pub title: Style,
    pub header: Style,
    pub search_bar: Style,
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
            error: Style::new().fg(Color::Red).add_modifier(Modifier::BOLD),
            dim: Style::new().add_modifier(Modifier::DIM),
            status_bar: Style::new().bg(Color::DarkGray).fg(Color::White),
            title: Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            header: Style::new()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
                .add_modifier(Modifier::UNDERLINED),
            search_bar: Style::new().bg(Color::Indexed(236)).fg(Color::Yellow),
        }
    }

    pub fn light() -> Self {
        Self {
            name: "light",
            merged: Style::new().fg(Color::Indexed(28)).add_modifier(Modifier::BOLD), // dark green
            squash_merged: Style::new().fg(Color::Indexed(172)).add_modifier(Modifier::BOLD), // dark orange
            unmerged: Style::new().fg(Color::Red),
            primary_text: Style::new().fg(Color::Black).add_modifier(Modifier::BOLD),
            secondary_text: Style::new().fg(Color::DarkGray),
            cursor: Style::new().bg(Color::Indexed(153)), // light blue
            selected: Style::new().fg(Color::Indexed(28)).add_modifier(Modifier::BOLD), // dark green
            current_branch: Style::new().fg(Color::Blue).add_modifier(Modifier::BOLD),
            ahead_behind: Style::new().fg(Color::Blue),
            pinned_row: Style::new().add_modifier(Modifier::DIM),
            error: Style::new().fg(Color::Red).add_modifier(Modifier::BOLD),
            dim: Style::new().add_modifier(Modifier::DIM),
            status_bar: Style::new().bg(Color::Indexed(252)).fg(Color::Black),
            title: Style::new().fg(Color::Blue).add_modifier(Modifier::BOLD),
            header: Style::new()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD)
                .add_modifier(Modifier::UNDERLINED),
            search_bar: Style::new().bg(Color::Indexed(254)).fg(Color::Indexed(130)),
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
            error: Style::new().fg(red).add_modifier(Modifier::BOLD),
            dim: Style::new().add_modifier(Modifier::DIM),
            status_bar: Style::new().bg(Color::Indexed(235)).fg(base0),
            title: Style::new().fg(blue).add_modifier(Modifier::BOLD),
            header: Style::new()
                .fg(blue)
                .add_modifier(Modifier::BOLD)
                .add_modifier(Modifier::UNDERLINED),
            search_bar: Style::new().bg(Color::Indexed(235)).fg(yellow),
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
            error: Style::new().fg(red).add_modifier(Modifier::BOLD),
            dim: Style::new().add_modifier(Modifier::DIM),
            status_bar: Style::new().bg(Color::Indexed(236)).fg(fg),
            title: Style::new().fg(purple).add_modifier(Modifier::BOLD),
            header: Style::new()
                .fg(purple)
                .add_modifier(Modifier::BOLD)
                .add_modifier(Modifier::UNDERLINED),
            search_bar: Style::new().bg(Color::Indexed(236)).fg(yellow),
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
