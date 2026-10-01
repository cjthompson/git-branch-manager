use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use crate::git::commit_details::CommitFileDiff;
use crate::theme::Theme;

use super::modal::{draw_modal_shell, ModalFooter, ModalScroll, ModalSpec};

pub fn draw_commit_diff(
    frame: &mut Frame,
    diff: &CommitFileDiff,
    scroll: &mut ModalScroll,
    theme: &Theme,
) {
    let areas = draw_modal_shell(
        frame,
        &ModalSpec::new(
            format!("Diff: {}", diff.path),
            ModalFooter::hints(&[("j/k", "Scroll"), ("PgUp/Dn", "Page"), ("Esc", "Back")]),
            110,
            30,
        ),
        theme,
    );
    let lines: Vec<Line> = diff.patch.lines().map(Line::from).collect();
    scroll.clamp(lines.len() as u16, areas.body.height);
    frame.render_widget(Paragraph::new(lines).scroll((scroll.offset, 0)), areas.body);
}
