use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use crate::git::commit_details::{CommitDetailMode, CommitDetails};
use crate::git::graph::GraphCommit;
use crate::symbols::SymbolSet;
use crate::theme::Theme;

use super::menu::MenuItem;
use super::modal::{draw_modal_shell, ModalActionRow, ModalFooter, ModalScroll, ModalSpec};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitDetailsFocus {
    Files,
    Actions,
}

#[allow(clippy::too_many_arguments)]
pub fn draw_commit_details(
    frame: &mut Frame,
    commit: &GraphCommit,
    details: &CommitDetails,
    items: &[MenuItem],
    cursor: usize,
    file_cursor: usize,
    focus: CommitDetailsFocus,
    scroll: &mut ModalScroll,
    theme: &Theme,
    symbols: &SymbolSet,
) {
    let short_oid: String = details.oid.chars().take(7).collect();
    let areas = draw_modal_shell(
        frame,
        &ModalSpec::new(
            format!("{short_oid} {}", details.summary),
            ModalFooter::hints(&[
                ("j/k", "Navigate"),
                ("Enter", "Diff/action"),
                ("Tab", "Switch"),
                ("Esc", "Close"),
            ]),
            96,
            26,
        ),
        theme,
    );

    let mut lines = Vec::new();
    lines.push(Line::from(vec![
        Span::styled("Commit ", theme.modal_secondary),
        Span::styled(details.oid.clone(), theme.modal_commit),
    ]));
    if !commit.is_possible_squash_merge && !commit.is_cherry_picked_commit {
        if let Some(fuzzy) = commit.fuzzy_squash_match.as_ref() {
            lines.push(Line::from(vec![
                Span::styled("Possible squash merge (fuzzy): ", theme.modal_secondary),
                Span::styled(
                    format!("{}% similarity", fuzzy.similarity_percent),
                    theme.modal_commit,
                ),
            ]));
        }
    }
    let author = match (
        details.author_name.is_empty(),
        details.author_email.is_empty(),
    ) {
        (false, false) => format!("{} <{}>", details.author_name, details.author_email),
        (false, true) => details.author_name.clone(),
        (true, false) => details.author_email.clone(),
        (true, true) => String::new(),
    };
    if !author.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("Author ", theme.modal_secondary),
            Span::styled(author, theme.modal_commit),
        ]));
    }
    if let Some(date) = details.authored_at.as_ref() {
        lines.push(Line::from(vec![
            Span::styled("Date ", theme.modal_secondary),
            Span::styled(date.to_rfc3339(), theme.modal_commit),
        ]));
    }

    lines.push(Line::from(Span::styled("Message", theme.modal_title)));
    for line in &details.message_lines {
        lines.push(Line::from(line.clone()));
    }
    if let CommitDetailMode::BranchTip { branch, base, .. } = &details.mode {
        lines.push(Line::from(Span::styled(
            format!("Branch tip {branch} ({base}..{branch})"),
            theme.modal_title,
        )));
        for line in &details.branch_log {
            lines.push(Line::from(line.clone()));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Changed Files", theme.modal_title)));
    let files_start = lines.len() as u16;
    for (index, file) in details.files.iter().enumerate() {
        let selected = focus == CommitDetailsFocus::Files && index == file_cursor;
        let marker = if selected { symbols.cursor_prefix } else { " " };
        let old = file
            .old_path
            .as_ref()
            .map(|old| format!(" ({old} -> {})", file.path))
            .unwrap_or_default();
        lines.push(Line::from(vec![
            Span::styled(
                format!("{marker} "),
                if selected {
                    theme.modal_action_selected
                } else {
                    Style::default()
                },
            ),
            Span::styled(
                format!("{} {}{}", file.kind.label(), file.path, old),
                if selected {
                    theme.modal_action_selected
                } else {
                    Style::default()
                },
            ),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Actions", theme.modal_title)));
    let actions_start = lines.len() as u16;
    for (index, item) in items.iter().enumerate() {
        let selected = focus == CommitDetailsFocus::Actions && index == cursor && item.enabled;
        let mut line = ModalActionRow::new(
            item.shortcut,
            item.label.clone(),
            item.reason.clone().unwrap_or_default(),
        )
        .render_with_availability(selected, item.enabled, theme);
        line.spans.insert(
            0,
            Span::styled(
                if selected {
                    format!("{} ", symbols.cursor_prefix)
                } else {
                    "  ".to_owned()
                },
                if item.enabled {
                    Style::default()
                } else {
                    theme.modal_action_unavailable
                },
            ),
        );
        lines.push(line);
    }

    let focused_row = match focus {
        CommitDetailsFocus::Files if file_cursor < details.files.len() => {
            Some(files_start.saturating_add(file_cursor as u16))
        }
        CommitDetailsFocus::Actions if items.get(cursor).is_some_and(|item| item.enabled) => {
            Some(actions_start.saturating_add(cursor as u16))
        }
        _ => None,
    };
    scroll.keep_focus_visible(focused_row, lines.len() as u16, areas.body.height);
    frame.render_widget(Paragraph::new(lines).scroll((scroll.offset, 0)), areas.body);
}
