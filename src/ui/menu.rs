use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem};

use super::theme::Theme;

pub struct MenuItem {
    pub label: String,
    pub enabled: bool,
    pub reason: Option<String>,
}

pub fn draw(frame: &mut Frame, items: &[MenuItem], menu_cursor: usize, anchor_row: u16, theme: &Theme) {
    let area = frame.area();
    let menu_width = 35u16.min(area.width);
    let menu_height = (items.len() as u16 + 2).min(area.height); // +2 for borders

    // Position near cursor row, right side of screen
    let y = anchor_row.min(area.height.saturating_sub(menu_height));
    let x = area.width.saturating_sub(menu_width + 1);
    let rect = Rect::new(x, y, menu_width, menu_height);

    let list_items: Vec<ListItem> = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let prefix = if i == menu_cursor { "\u{25b8} " } else { "  " };
            let style = if !item.enabled {
                theme.secondary_text
            } else if i == menu_cursor {
                theme.primary_text
            } else {
                Style::default()
            };
            let text = if let Some(reason) = &item.reason {
                format!("{}{} ({})", prefix, item.label, reason)
            } else {
                format!("{}{}", prefix, item.label)
            };
            ListItem::new(text).style(style)
        })
        .collect();

    let block = Block::default()
        .title("Actions")
        .title_style(theme.title)
        .borders(Borders::ALL);

    let list = List::new(list_items).block(block);
    frame.render_widget(Clear, rect);
    frame.render_widget(list, rect);
}
