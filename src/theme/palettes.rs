use ratatui::style::{Color, Modifier, Style};

use super::Theme;

impl Theme {
    pub fn dark() -> Self {
        Self {
            name: "dark",
            merged: Style::new().fg(Color::Green).add_modifier(Modifier::BOLD),
            squash_merged: Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            cherry_picked: Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            in_sync: Style::new()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
            unmerged: Style::new().fg(Color::Red),
            primary_text: Style::new().fg(Color::White).add_modifier(Modifier::BOLD),
            secondary_text: Style::new().fg(Color::DarkGray),
            cursor: Style::new().bg(Color::Indexed(24)),
            selected: Style::new().fg(Color::Green).add_modifier(Modifier::BOLD),
            current_branch: Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ahead: Style::new().fg(Color::Green),
            behind: Style::new().fg(Color::Yellow),
            pinned_row: Style::new().add_modifier(Modifier::DIM),
            checked_row: Style::new().bg(Color::Indexed(236)),
            error: Style::new().fg(Color::Red).add_modifier(Modifier::BOLD),
            dim: Style::new().fg(Color::DarkGray).add_modifier(Modifier::DIM),
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
            remote_title: Style::new().fg(Color::Magenta).add_modifier(Modifier::BOLD),
            remote_header: Style::new()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD)
                .add_modifier(Modifier::UNDERLINED),
            toast_border: Style::new().fg(Color::Yellow),
            toast_text: Style::new()
                .fg(Color::Yellow)
                .add_modifier(Modifier::ITALIC),
            modal_border: Style::new()
                .fg(Color::Rgb(200, 200, 200))
                .add_modifier(Modifier::DIM),
            modal_surface: Style::new().bg(Color::Rgb(29, 59, 74)),
            modal_title: Style::new()
                .fg(Color::Rgb(139, 233, 253))
                .add_modifier(Modifier::BOLD),
            modal_footer: Style::new()
                .fg(Color::Rgb(200, 200, 200))
                .add_modifier(Modifier::DIM),
            modal_key: Style::new()
                .fg(Color::Rgb(248, 248, 242))
                .add_modifier(Modifier::BOLD),
            modal_branch: Style::new()
                .fg(Color::Rgb(248, 248, 242))
                .add_modifier(Modifier::BOLD),
            modal_worktree: Style::new()
                .fg(Color::Rgb(248, 248, 242))
                .add_modifier(Modifier::BOLD),
            modal_commit: Style::new().fg(Color::Yellow),
            modal_command: Style::new()
                .fg(Color::Rgb(248, 248, 242))
                .add_modifier(Modifier::BOLD),
            modal_success: Style::new().fg(Color::Green).add_modifier(Modifier::BOLD),
            modal_failure: Style::new().fg(Color::Red).add_modifier(Modifier::BOLD),
            modal_secondary: Style::new().fg(Color::Rgb(248, 248, 242)),
            modal_action_unavailable: Style::new()
                .fg(Color::Rgb(200, 200, 200))
                .add_modifier(Modifier::DIM),
            modal_action_selected: Style::new().bg(Color::Rgb(0, 114, 168)),
            modal_warning: Style::new()
                .fg(Color::Rgb(23, 23, 23))
                .bg(Color::Rgb(255, 85, 85)),
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
            cherry_picked: Style::new()
                .fg(Color::Indexed(30))
                .add_modifier(Modifier::BOLD), // teal
            in_sync: Style::new()
                .fg(Color::Indexed(245))
                .add_modifier(Modifier::ITALIC), // muted gray
            unmerged: Style::new().fg(Color::Red),
            primary_text: Style::new().fg(Color::Black).add_modifier(Modifier::BOLD),
            secondary_text: Style::new().fg(Color::DarkGray),
            cursor: Style::new().bg(Color::Indexed(153)), // light blue
            selected: Style::new()
                .fg(Color::Indexed(28))
                .add_modifier(Modifier::BOLD), // dark green
            current_branch: Style::new().fg(Color::Blue).add_modifier(Modifier::BOLD),
            ahead: Style::new().fg(Color::Indexed(28)),
            behind: Style::new().fg(Color::Indexed(172)),
            pinned_row: Style::new().add_modifier(Modifier::DIM),
            checked_row: Style::new().bg(Color::Indexed(229)),
            error: Style::new().fg(Color::Red).add_modifier(Modifier::BOLD),
            dim: Style::new()
                .fg(Color::Indexed(245))
                .add_modifier(Modifier::DIM),
            status_bar: Style::new().bg(Color::Indexed(252)).fg(Color::Black),
            title: Style::new().fg(Color::Blue).add_modifier(Modifier::BOLD),
            header: Style::new()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD)
                .add_modifier(Modifier::UNDERLINED),
            search_bar: Style::new().bg(Color::Indexed(254)).fg(Color::Indexed(130)),
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
            modal_border: Style::new()
                .fg(Color::Rgb(80, 80, 80))
                .add_modifier(Modifier::DIM),
            modal_surface: Style::new().bg(Color::Rgb(219, 234, 246)),
            modal_title: Style::new()
                .fg(Color::Rgb(0, 95, 175))
                .add_modifier(Modifier::BOLD),
            modal_footer: Style::new()
                .fg(Color::Rgb(80, 80, 80))
                .add_modifier(Modifier::DIM),
            modal_key: Style::new()
                .fg(Color::Rgb(32, 32, 32))
                .add_modifier(Modifier::BOLD),
            modal_branch: Style::new()
                .fg(Color::Rgb(32, 32, 32))
                .add_modifier(Modifier::BOLD),
            modal_worktree: Style::new()
                .fg(Color::Rgb(32, 32, 32))
                .add_modifier(Modifier::BOLD),
            modal_commit: Style::new().fg(Color::Indexed(172)),
            modal_command: Style::new()
                .fg(Color::Rgb(32, 32, 32))
                .add_modifier(Modifier::BOLD),
            modal_success: Style::new()
                .fg(Color::Indexed(28))
                .add_modifier(Modifier::BOLD),
            modal_failure: Style::new().fg(Color::Red).add_modifier(Modifier::BOLD),
            modal_secondary: Style::new().fg(Color::Rgb(32, 32, 32)),
            modal_action_unavailable: Style::new()
                .fg(Color::Rgb(80, 80, 80))
                .add_modifier(Modifier::DIM),
            modal_action_selected: Style::new().bg(Color::Rgb(133, 189, 232)),
            modal_warning: Style::new()
                .fg(Color::Rgb(32, 32, 32))
                .bg(Color::Rgb(255, 215, 175)),
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
            cherry_picked: Style::new().fg(cyan),
            in_sync: Style::new().fg(base01).add_modifier(Modifier::ITALIC),
            unmerged: Style::new().fg(red),
            primary_text: Style::new().fg(base0).add_modifier(Modifier::BOLD),
            secondary_text: Style::new().fg(base01),
            cursor: Style::new().bg(Color::Indexed(236)),
            selected: Style::new().fg(green).add_modifier(Modifier::BOLD),
            current_branch: Style::new().fg(cyan).add_modifier(Modifier::BOLD),
            ahead: Style::new().fg(green),
            behind: Style::new().fg(yellow),
            pinned_row: Style::new().add_modifier(Modifier::DIM),
            checked_row: Style::new().bg(Color::Indexed(22)),
            error: Style::new().fg(red).add_modifier(Modifier::BOLD),
            dim: Style::new().fg(base01).add_modifier(Modifier::DIM),
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
            modal_border: Style::new()
                .fg(Color::Rgb(188, 188, 188))
                .add_modifier(Modifier::DIM),
            modal_surface: Style::new().bg(Color::Rgb(11, 61, 75)),
            modal_title: Style::new()
                .fg(Color::Rgb(77, 182, 255))
                .add_modifier(Modifier::BOLD),
            modal_footer: Style::new()
                .fg(Color::Rgb(188, 188, 188))
                .add_modifier(Modifier::DIM),
            modal_key: Style::new()
                .fg(Color::Rgb(230, 230, 230))
                .add_modifier(Modifier::BOLD),
            modal_branch: Style::new()
                .fg(Color::Rgb(230, 230, 230))
                .add_modifier(Modifier::BOLD),
            modal_worktree: Style::new()
                .fg(Color::Rgb(230, 230, 230))
                .add_modifier(Modifier::BOLD),
            modal_commit: Style::new().fg(yellow),
            modal_command: Style::new()
                .fg(Color::Rgb(230, 230, 230))
                .add_modifier(Modifier::BOLD),
            modal_success: Style::new().fg(green).add_modifier(Modifier::BOLD),
            modal_failure: Style::new().fg(red).add_modifier(Modifier::BOLD),
            modal_secondary: Style::new().fg(Color::Rgb(230, 230, 230)),
            modal_action_unavailable: Style::new()
                .fg(Color::Rgb(188, 188, 188))
                .add_modifier(Modifier::DIM),
            modal_action_selected: Style::new().bg(Color::Rgb(11, 111, 141)),
            modal_warning: Style::new().fg(Color::White).bg(Color::Rgb(215, 0, 0)),
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
            cherry_picked: Style::new().fg(cyan),
            in_sync: Style::new()
                .fg(Color::Indexed(245))
                .add_modifier(Modifier::ITALIC),
            unmerged: Style::new().fg(red),
            primary_text: Style::new().fg(fg).add_modifier(Modifier::BOLD),
            secondary_text: Style::new().fg(Color::Indexed(245)),
            cursor: Style::new().bg(Color::Indexed(238)),
            selected: Style::new().fg(green).add_modifier(Modifier::BOLD),
            current_branch: Style::new().fg(cyan).add_modifier(Modifier::BOLD),
            ahead: Style::new().fg(green),
            behind: Style::new().fg(yellow),
            pinned_row: Style::new().add_modifier(Modifier::DIM),
            checked_row: Style::new().bg(Color::Indexed(22)),
            error: Style::new().fg(red).add_modifier(Modifier::BOLD),
            dim: Style::new()
                .fg(Color::Indexed(245))
                .add_modifier(Modifier::DIM),
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
            modal_border: Style::new()
                .fg(Color::Rgb(208, 208, 208))
                .add_modifier(Modifier::DIM),
            modal_surface: Style::new().bg(Color::Rgb(59, 61, 79)),
            modal_title: Style::new()
                .fg(Color::Rgb(205, 170, 255))
                .add_modifier(Modifier::BOLD),
            modal_footer: Style::new()
                .fg(Color::Rgb(208, 208, 208))
                .add_modifier(Modifier::DIM),
            modal_key: Style::new()
                .fg(Color::Rgb(248, 248, 242))
                .add_modifier(Modifier::BOLD),
            modal_branch: Style::new()
                .fg(Color::Rgb(248, 248, 242))
                .add_modifier(Modifier::BOLD),
            modal_worktree: Style::new()
                .fg(Color::Rgb(248, 248, 242))
                .add_modifier(Modifier::BOLD),
            modal_commit: Style::new().fg(yellow),
            modal_command: Style::new()
                .fg(Color::Rgb(248, 248, 242))
                .add_modifier(Modifier::BOLD),
            modal_success: Style::new().fg(green).add_modifier(Modifier::BOLD),
            modal_failure: Style::new().fg(red).add_modifier(Modifier::BOLD),
            modal_secondary: Style::new().fg(Color::Rgb(248, 248, 242)),
            modal_action_unavailable: Style::new()
                .fg(Color::Rgb(208, 208, 208))
                .add_modifier(Modifier::DIM),
            modal_action_selected: Style::new().bg(Color::Rgb(106, 109, 157)),
            modal_warning: Style::new()
                .fg(Color::Rgb(23, 23, 23))
                .bg(Color::Rgb(255, 135, 135)),
        }
    }
}
