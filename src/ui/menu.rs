use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem};

use super::symbols::SymbolSet;
use super::theme::Theme;

pub struct MenuItem {
    pub label: String,
    pub enabled: bool,
    pub reason: Option<String>,
    pub shortcut: Option<char>,
}

pub fn draw(frame: &mut Frame, items: &[MenuItem], menu_cursor: usize, anchor_row: u16, theme: &Theme, symbols: &SymbolSet) {
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
            let prefix = if i == menu_cursor { format!("{} ", symbols.cursor_prefix) } else { "  ".to_string() };
            let item_style = if !item.enabled {
                theme.secondary_text
            } else if i == menu_cursor {
                theme.cursor
            } else {
                Style::default()
            };
            let prefix_span = Span::styled(prefix, item_style);
            let mut spans = vec![prefix_span];
            if let Some(ch) = item.shortcut {
                spans.push(Span::styled("[", item_style));
                spans.push(Span::styled(ch.to_string(), if item.enabled { theme.title } else { item_style }));
                spans.push(Span::styled(format!("] {}", item.label), item_style));
            } else {
                spans.push(Span::styled(item.label.clone(), item_style));
            }
            if let Some(reason) = &item.reason {
                spans.push(Span::styled(format!(" ({})", reason), item_style));
            }
            let line = Line::from(spans);
            ListItem::new(line)
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
