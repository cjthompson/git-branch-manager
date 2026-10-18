mod fields;
mod ref_fields;

use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use super::menu::MenuItem;
use super::modal::{draw_modal_shell, ModalActionRow, ModalFooter, ModalScroll, ModalSpec};
use crate::git::graph::GraphCommit;
use crate::symbols::SymbolSet;
use crate::theme::Theme;
use crate::types::*;
use fields::build_fields;

#[derive(Debug, Clone)]
pub enum InfoModalRow {
    GraphCommit(GraphCommit),
    Branch(BranchInfo),
    Remote(RemoteBranchInfo),
    Tag(TagInfo),
    Worktree(WorktreeInfo),
    PrimaryBranchCodeCheck {
        display_ref: String,
        base_branch: String,
        primary_reference: Option<String>,
        primary_oid: Option<String>,
        outcome: Result<PrimaryBranchCodeMatch, String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InfoModalFocus {
    Info,
    Actions,
}

struct InfoField {
    label: &'static str,
    value: String,
}

/// On-screen rectangle of one info value, recorded each frame so a left-click
/// can be mapped back to the field it landed on (for click-to-copy).
#[derive(Debug, Clone)]
pub struct InfoHitRegion {
    pub rect: Rect,
    pub label: String,
    pub value: String,
}

impl InfoModalRow {
    pub fn info_field_count(&self) -> usize {
        build_fields(self).len()
    }

    pub fn info_field(&self, index: usize) -> Option<(String, String)> {
        build_fields(self)
            .get(index)
            .map(|field| (field.label.to_string(), field.value.clone()))
    }
}

/// Where a field's lines landed within the built line list, before any
/// scroll offset or pane origin is applied.
struct FieldSpan {
    label: String,
    value: String,
    start_line: u16,
    line_count: u16,
}

#[allow(clippy::too_many_arguments)]
pub fn draw_info_modal(
    frame: &mut Frame,
    row: &InfoModalRow,
    items: &[MenuItem],
    cursor: usize,
    focus: InfoModalFocus,
    info_cursor: usize,
    scroll_offset: &mut u16,
    copied_msg: Option<&str>,
    hit_regions: &mut Vec<InfoHitRegion>,
    theme: &Theme,
    symbols: &SymbolSet,
) {
    // Recorded fresh every frame; the click handler reads the latest set.
    hit_regions.clear();

    let fields = build_fields(row);
    let title = get_title(row);
    let code_check = matches!(row, InfoModalRow::PrimaryBranchCodeCheck { .. });
    let footer = if code_check {
        ModalFooter::hints(&[("Esc", "Close")])
    } else {
        ModalFooter::hints(&[
            ("Tab", "Switch"),
            ("j/k", "Navigate"),
            ("Enter", "Invoke"),
            ("Esc", "Close"),
        ])
    };
    let areas = draw_modal_shell(frame, &ModalSpec::new(title, footer, 84, 22), theme);

    let selected_field = (focus == InfoModalFocus::Info).then_some(info_cursor);
    let (mut lines, field_spans) = build_info_lines(
        &fields,
        theme,
        areas.body.width.saturating_sub(1) as usize,
        selected_field,
    );
    if let Some(msg) = copied_msg {
        lines.push(Line::from(Span::styled(
            msg.to_string(),
            theme.modal_success,
        )));
    }
    let actions_start = if code_check {
        lines.len() as u16
    } else {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("Actions", theme.modal_title)));
        lines.len() as u16
    };
    if !code_check {
        for (index, item) in items.iter().enumerate() {
            let selected = focus == InfoModalFocus::Actions && index == cursor && item.enabled;
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
                        "  ".to_string()
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
    }

    let target = match focus {
        InfoModalFocus::Info => field_spans
            .get(info_cursor)
            .map(|span| span.start_line)
            .unwrap_or_default(),
        InfoModalFocus::Actions => actions_start.saturating_add(cursor as u16),
    };
    let mut scroll = ModalScroll::default();
    scroll.offset = *scroll_offset;
    scroll.ensure_visible(target, lines.len() as u16, areas.body.height);
    *scroll_offset = scroll.offset;
    frame.render_widget(Paragraph::new(lines).scroll((scroll.offset, 0)), areas.body);

    for span in field_spans {
        let visible_start = span.start_line.max(scroll.offset);
        let visible_end = (span.start_line + span.line_count)
            .min(scroll.offset.saturating_add(areas.body.height));
        if visible_end > visible_start {
            hit_regions.push(InfoHitRegion {
                rect: Rect {
                    x: areas.body.x,
                    y: areas.body.y + visible_start - scroll.offset,
                    width: areas.body.width,
                    height: visible_end - visible_start,
                },
                label: span.label,
                value: span.value,
            });
        }
    }
}

fn get_title(row: &InfoModalRow) -> String {
    match row {
        InfoModalRow::GraphCommit(commit) => {
            let short_oid: String = commit.oid.chars().take(7).collect();
            format!("{short_oid} {}", commit.summary)
        }
        InfoModalRow::Branch(b) => b.name.clone(),
        InfoModalRow::Remote(r) => r.short_name.clone(),
        InfoModalRow::Tag(t) => t.name.clone(),
        InfoModalRow::Worktree(w) => w.path.to_string_lossy().to_string(),
        InfoModalRow::PrimaryBranchCodeCheck { display_ref, .. } => {
            format!("Code in primary branch: {display_ref}")
        }
    }
}

/// Width of the label column: 15 chars of label + 1 trailing space.
const INFO_LABEL_WIDTH: usize = 16;

/// Wrap `value` to `width` columns. Words are kept whole when they fit; words
/// longer than `width` (e.g. file paths, which contain no spaces) are
/// hard-broken across lines. Always returns at least one (possibly empty) line.
fn wrap_value(value: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_len = 0usize;

    for word in value.split(' ') {
        let word_len = word.chars().count();

        // Hard-break a word that can't fit on a line by itself.
        if word_len > width {
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
                current_len = 0;
            }
            let chars: Vec<char> = word.chars().collect();
            let mut start = 0;
            while start < chars.len() {
                let end = (start + width).min(chars.len());
                lines.push(chars[start..end].iter().collect());
                start = end;
            }
            continue;
        }

        let needed = if current.is_empty() {
            word_len
        } else {
            word_len + 1
        };
        if current_len + needed > width && !current.is_empty() {
            lines.push(std::mem::take(&mut current));
            current_len = 0;
        }
        if !current.is_empty() {
            current.push(' ');
            current_len += 1;
        }
        current.push_str(word);
        current_len += word_len;
    }

    if !current.is_empty() || lines.is_empty() {
        lines.push(current);
    }
    lines
}

/// Build the info section as `label: value` lines, wrapping long values to
/// `width` with continuation lines indented under the value column.
fn build_info_lines(
    fields: &[InfoField],
    theme: &Theme,
    width: usize,
    selected_field: Option<usize>,
) -> (Vec<Line<'static>>, Vec<FieldSpan>) {
    let value_width = width.saturating_sub(INFO_LABEL_WIDTH).max(1);
    let mut lines = Vec::new();
    let mut spans = Vec::new();
    for f in fields {
        let start_line = lines.len() as u16;
        let chunks = wrap_value(&f.value, value_width);
        let line_count = chunks.len() as u16;
        for (i, chunk) in chunks.into_iter().enumerate() {
            let selected = selected_field == Some(spans.len());
            if i == 0 {
                let line = Line::from(vec![
                    Span::styled(format!("{:<15} ", f.label), theme.modal_secondary),
                    Span::styled(chunk, info_value_style(f.label, theme)),
                ]);
                lines.push(if selected {
                    line.style(theme.modal_action_selected)
                } else {
                    line
                });
            } else {
                let line = Line::from(vec![
                    Span::raw(" ".repeat(INFO_LABEL_WIDTH)),
                    Span::styled(chunk, info_value_style(f.label, theme)),
                ]);
                lines.push(if selected {
                    line.style(theme.modal_action_selected)
                } else {
                    line
                });
            }
        }
        spans.push(FieldSpan {
            label: f.label.to_string(),
            value: f.value.clone(),
            start_line,
            line_count,
        });
    }
    (lines, spans)
}

fn info_value_style(label: &str, theme: &Theme) -> Style {
    match label {
        "Commit" | "Merge Base" | "Parents" => theme.modal_commit,
        "Branch" | "Base Branch" | "Local Refs" | "Remote Refs" | "Tags" | "Remote" => {
            theme.modal_branch
        }
        "Worktree" | "Path" => theme.modal_worktree,
        _ => theme.modal_command,
    }
}

#[cfg(test)]
mod tests;
