use ratatui::style::{Color, Modifier, Style};

pub const SELECTED_STYLE: Style = Style::new()
    .fg(Color::Green)
    .add_modifier(Modifier::BOLD);

pub const CURSOR_STYLE: Style = Style::new().bg(Color::DarkGray);

pub const MERGED_STYLE: Style = Style::new().fg(Color::Green);

pub const SQUASH_MERGED_STYLE: Style = Style::new().fg(Color::Yellow);

pub const UNMERGED_STYLE: Style = Style::new();

pub const ERROR_STYLE: Style = Style::new()
    .fg(Color::Red)
    .add_modifier(Modifier::BOLD);

pub const DIM_STYLE: Style = Style::new().add_modifier(Modifier::DIM);

pub const STATUS_BAR_STYLE: Style = Style::new()
    .bg(Color::DarkGray)
    .fg(Color::White);

pub const TITLE_STYLE: Style = Style::new()
    .fg(Color::Cyan)
    .add_modifier(Modifier::BOLD);
