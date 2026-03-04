use ratatui::style::{Color, Modifier, Style};

// Merge status colors (pcu severity pattern: green/yellow/red)
pub const MERGED_STYLE: Style = Style::new()
    .fg(Color::Green)
    .add_modifier(Modifier::BOLD);

pub const SQUASH_MERGED_STYLE: Style = Style::new()
    .fg(Color::Yellow)
    .add_modifier(Modifier::BOLD);

pub const UNMERGED_STYLE: Style = Style::new().fg(Color::Red);

// Text hierarchy
pub const PRIMARY_TEXT: Style = Style::new()
    .fg(Color::White)
    .add_modifier(Modifier::BOLD);

pub const SECONDARY_TEXT: Style = Style::new().fg(Color::DarkGray);

// Cursor row — dark teal-blue background for clear contrast
pub const CURSOR_STYLE: Style = Style::new().bg(Color::Indexed(24));

pub const CURSOR_PREFIX_STYLE: Style = Style::new()
    .fg(Color::Cyan)
    .add_modifier(Modifier::BOLD);

// Selected item checkbox/name
pub const SELECTED_STYLE: Style = Style::new()
    .fg(Color::Green)
    .add_modifier(Modifier::BOLD);

// Current branch indicator
pub const CURRENT_BRANCH_STYLE: Style = Style::new()
    .fg(Color::Cyan)
    .add_modifier(Modifier::BOLD);

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
