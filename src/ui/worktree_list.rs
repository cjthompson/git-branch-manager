use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::app::App;

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let block = Block::default()
        .title(" Worktrees ")
        .borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let status = if app.worktree_loading {
        "Loading worktrees...".to_string()
    } else if app.worktrees.is_empty() {
        "No worktrees found.".to_string()
    } else {
        format!("{} worktrees", app.worktrees.len())
    };

    let para = Paragraph::new(status);
    frame.render_widget(para, inner);
}
