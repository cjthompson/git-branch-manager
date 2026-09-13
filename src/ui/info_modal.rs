use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::prelude::*;
use ratatui::widgets::{
    Borders, Clear, List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation,
    ScrollbarState,
};

use super::menu::MenuItem;
use super::shared::{block_panel, centered_rect_pct};
use crate::git::graph::{GraphCommit, GraphRefKind};
use crate::symbols::SymbolSet;
use crate::theme::Theme;
use crate::types::*;

#[derive(Debug, Clone)]
pub enum InfoModalRow {
    GraphCommit(GraphCommit),
    Branch(BranchInfo),
    Remote(RemoteBranchInfo),
    Tag(TagInfo),
    Worktree(WorktreeInfo),
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

    let area = frame.area();
    let fields = build_fields(row);
    let title = get_title(row);
    let width = area.width;

    if width >= 100 {
        // Two-column layout: info left, actions right
        draw_info_modal_wide(
            frame,
            &title,
            &fields,
            items,
            cursor,
            focus,
            info_cursor,
            copied_msg,
            hit_regions,
            theme,
            symbols,
        );
    } else {
        // Single-column layout with scrolling
        draw_info_modal_narrow(
            frame,
            &title,
            &fields,
            items,
            cursor,
            focus,
            info_cursor,
            scroll_offset,
            copied_msg,
            hit_regions,
            theme,
            symbols,
        );
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
    }
}

fn build_fields(row: &InfoModalRow) -> Vec<InfoField> {
    match row {
        InfoModalRow::GraphCommit(commit) => graph_commit_fields(commit),
        InfoModalRow::Branch(b) => branch_fields(b),
        InfoModalRow::Remote(r) => remote_fields(r),
        InfoModalRow::Tag(t) => tag_fields(t),
        InfoModalRow::Worktree(w) => worktree_fields(w),
    }
}

fn graph_commit_fields(commit: &GraphCommit) -> Vec<InfoField> {
    let mut fields = vec![
        InfoField {
            label: "Commit",
            value: commit.oid.clone(),
        },
        InfoField {
            label: "Summary",
            value: commit.summary.clone(),
        },
    ];

    if !commit.parents.is_empty() {
        fields.push(InfoField {
            label: "Parents",
            value: commit.parents.join(", "),
        });
    }

    // NEW: Author row — single combined "Name <email>" format
    let has_author_name = !commit.author_name.is_empty();
    let has_author_email = !commit.author_email.is_empty();
    if has_author_name || has_author_email {
        let author_value = match (has_author_name, has_author_email) {
            (true, true) => format!("{} <{}>", commit.author_name, commit.author_email),
            (true, false) => commit.author_name.clone(),
            (false, true) => commit.author_email.clone(),
            (false, false) => unreachable!(),
        };
        fields.push(InfoField {
            label: "Author",
            value: author_value,
        });
    }

    // Date row — absolute local time + relative age, when the loader provided a date.
    if let Some(authored_at) = commit.authored_at.as_ref() {
        let local = crate::types::format_local_absolute(authored_at);
        let age = crate::types::format_age(authored_at);
        fields.push(InfoField {
            label: "Date",
            value: format!("{local} ({age})"),
        });
    }

    if let Some(branch) = &commit.branch {
        fields.push(InfoField {
            label: "Branch",
            value: branch.name.clone(),
        });
    }

    for (label, kind) in [
        ("Local Refs", GraphRefKind::LocalBranch),
        ("Remote Refs", GraphRefKind::RemoteBranch),
        ("Tags", GraphRefKind::Tag),
    ] {
        let names = commit
            .refs
            .iter()
            .filter(|reference| reference.kind == kind)
            .map(|reference| reference.name.as_str())
            .collect::<Vec<_>>();
        if !names.is_empty() {
            fields.push(InfoField {
                label,
                value: names.join(", "),
            });
        }
    }

    if commit.is_possible_squash_merge {
        fields.push(InfoField {
            label: "Possible Squash Merge",
            value: "yes".into(),
        });
    }

    fields
}

fn branch_fields(b: &BranchInfo) -> Vec<InfoField> {
    let mut fields = vec![
        InfoField {
            label: "Name",
            value: b.name.clone(),
        },
        InfoField {
            label: "Current",
            value: if b.is_current { "yes" } else { "no" }.to_string(),
        },
        InfoField {
            label: "Base",
            value: if b.is_base { "yes" } else { "no" }.to_string(),
        },
    ];

    match &b.tracking {
        TrackingStatus::Tracked { remote_ref, gone } => {
            fields.push(InfoField {
                label: "Remote",
                value: remote_ref.clone(),
            });
            if *gone {
                fields.push(InfoField {
                    label: "Tracking",
                    value: "gone".to_string(),
                });
            }
        }
        TrackingStatus::Local => {
            fields.push(InfoField {
                label: "Tracking",
                value: "local".to_string(),
            });
        }
    }

    if let Some(ahead) = b.ahead {
        fields.push(InfoField {
            label: "Ahead",
            value: ahead.to_string(),
        });
    }

    if let Some(behind) = b.behind {
        fields.push(InfoField {
            label: "Behind",
            value: behind.to_string(),
        });
    }

    fields.push(InfoField {
        label: "Merge Status",
        value: merge_status_str(&b.merge_status).to_string(),
    });

    if let Some(confidence) = &b.squash_confidence {
        fields.push(InfoField {
            label: "Confidence",
            value: squash_confidence_str(confidence),
        });
    }

    fields.push(InfoField {
        label: "Base Branch",
        value: b.base_branch.clone(),
    });

    if let Some(hash) = &b.merge_base_commit {
        fields.push(InfoField {
            label: "Merge Base",
            value: hash.clone(),
        });
    }

    fields.push(InfoField {
        label: "Last Commit",
        value: b.age_display(),
    });

    if let Some(pr) = &b.pr {
        fields.push(InfoField {
            label: "PR",
            value: format!("#{} ({})", pr.number, pr_status_str(&pr.status)),
        });
    }

    fields
}

fn remote_fields(r: &RemoteBranchInfo) -> Vec<InfoField> {
    let mut fields = vec![
        InfoField {
            label: "Full Ref",
            value: r.full_ref.clone(),
        },
        InfoField {
            label: "Remote",
            value: r.remote.clone(),
        },
        InfoField {
            label: "Short Name",
            value: r.short_name.clone(),
        },
        InfoField {
            label: "Has Local",
            value: if r.has_local { "yes" } else { "no" }.to_string(),
        },
        InfoField {
            label: "Base",
            value: if r.is_base { "yes" } else { "no" }.to_string(),
        },
    ];

    if let Some(ahead) = r.ahead {
        fields.push(InfoField {
            label: "Ahead",
            value: ahead.to_string(),
        });
    }

    if let Some(behind) = r.behind {
        fields.push(InfoField {
            label: "Behind",
            value: behind.to_string(),
        });
    }

    fields.push(InfoField {
        label: "Merge Status",
        value: merge_status_str(&r.merge_status).to_string(),
    });

    if let Some(confidence) = &r.squash_confidence {
        fields.push(InfoField {
            label: "Confidence",
            value: squash_confidence_str(confidence),
        });
    }

    fields.push(InfoField {
        label: "Last Commit",
        value: r.age_display(),
    });

    if let Some(pr) = &r.pr {
        fields.push(InfoField {
            label: "PR",
            value: format!("#{} ({})", pr.number, pr_status_str(&pr.status)),
        });
    }

    fields
}

fn tag_fields(t: &TagInfo) -> Vec<InfoField> {
    let mut fields = vec![
        InfoField {
            label: "Name",
            value: t.name.clone(),
        },
        InfoField {
            label: "Commit Hash",
            value: t.commit_hash.clone(),
        },
        InfoField {
            label: "Date",
            value: t.age_display(),
        },
        InfoField {
            label: "Annotated",
            value: if t.is_annotated { "yes" } else { "no" }.to_string(),
        },
    ];

    if let Some(msg) = &t.message {
        fields.push(InfoField {
            label: "Message",
            value: msg.clone(),
        });
    }

    fields
}

fn worktree_fields(w: &WorktreeInfo) -> Vec<InfoField> {
    let mut fields = vec![
        InfoField {
            label: "Path",
            value: w.path.to_string_lossy().to_string(),
        },
        InfoField {
            label: "Main",
            value: if w.is_main { "yes" } else { "no" }.to_string(),
        },
        InfoField {
            label: "Commit Hash",
            value: w.commit_hash.clone(),
        },
    ];

    if let Some(branch) = &w.branch {
        fields.push(InfoField {
            label: "Branch",
            value: branch.clone(),
        });
    }

    fields.push(InfoField {
        label: "Status",
        value: w.wt_status.summary(),
    });

    if let Some(ahead) = w.ahead {
        fields.push(InfoField {
            label: "Ahead",
            value: ahead.to_string(),
        });
    }

    if let Some(behind) = w.behind {
        fields.push(InfoField {
            label: "Behind",
            value: behind.to_string(),
        });
    }

    fields.push(InfoField {
        label: "Merge Status",
        value: merge_status_str(&w.merge_status).to_string(),
    });

    fields.push(InfoField {
        label: "Last Commit",
        value: w.age_display(),
    });

    if let Some(pr_status) = &w.pr {
        fields.push(InfoField {
            label: "PR",
            value: pr_status_str(pr_status).to_string(),
        });
    }

    for (i, file) in w.wt_status.changed_files.iter().enumerate() {
        fields.push(InfoField {
            label: if i == 0 { "Changed Files" } else { "" },
            value: format!("{} ({})", file.path, file.kind.label()),
        });
    }

    fields
}

fn merge_status_str(status: &MergeStatus) -> &'static str {
    match status {
        MergeStatus::Merged => "Merged",
        MergeStatus::InSync => "In Sync",
        MergeStatus::SquashMerged => "Squash Merged",
        MergeStatus::LocalMerged => "Local Merged",
        MergeStatus::RemoteMerged => "Remote Merged",
        MergeStatus::LocalSquashMerged => "Local Squash Merged",
        MergeStatus::RemoteSquashMerged => "Remote Squash Merged",
        MergeStatus::LikelySquashMerged => "Possible Squash Merge",
        MergeStatus::CherryPicked => "Cherry Picked",
        MergeStatus::LocalCherryPicked => "Local Cherry Picked",
        MergeStatus::RemoteCherryPicked => "Remote Cherry Picked",
        MergeStatus::Unmerged => "Unmerged",
        MergeStatus::Pending => "Pending",
    }
}

fn squash_confidence_str(confidence: &SquashConfidence) -> String {
    match confidence {
        SquashConfidence::MergeTreeConfirmed => "Merge-tree confirmed".to_string(),
        SquashConfidence::FuzzyMatch { similarity_percent } => {
            format!("Fuzzy match ({similarity_percent}%)")
        }
    }
}

fn pr_status_str(status: &PrStatus) -> &'static str {
    match status {
        PrStatus::Draft => "Draft",
        PrStatus::Open => "Open",
        PrStatus::Merged => "Merged",
        PrStatus::Closed => "Closed",
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
                    Span::styled(format!("{:<15} ", f.label), theme.title),
                    Span::raw(chunk),
                ]);
                lines.push(if selected {
                    line.style(theme.cursor)
                } else {
                    line
                });
            } else {
                let line = Line::from(vec![
                    Span::raw(" ".repeat(INFO_LABEL_WIDTH)),
                    Span::raw(chunk),
                ]);
                lines.push(if selected {
                    line.style(theme.cursor)
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

#[allow(clippy::too_many_arguments)]
fn draw_info_modal_wide(
    frame: &mut Frame,
    title: &str,
    fields: &[InfoField],
    items: &[MenuItem],
    cursor: usize,
    focus: InfoModalFocus,
    info_cursor: usize,
    copied_msg: Option<&str>,
    hit_regions: &mut Vec<InfoHitRegion>,
    theme: &Theme,
    symbols: &SymbolSet,
) {
    let area = frame.area();
    let modal_rect = centered_rect_pct(85, 70, area);

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(modal_rect);

    let info_rect = chunks[0];
    let actions_rect = chunks[1];

    // Clear background
    frame.render_widget(Clear, modal_rect);

    // Render info pane on the left. The RIGHT border is the vertical
    // separator between the info and actions columns.
    let block = block_panel(theme)
        .title(title)
        .title_alignment(Alignment::Left)
        .title_style(theme.title)
        .borders(Borders::LEFT | Borders::TOP | Borders::BOTTOM | Borders::RIGHT);
    let info_inner = block.inner(info_rect);
    frame.render_widget(block, info_rect);

    // Reserve the bottom row of the info pane for the copied-confirmation message.
    let content_height = info_inner.height.saturating_sub(1);
    let content_rect = Rect {
        x: info_inner.x,
        y: info_inner.y,
        width: info_inner.width,
        height: content_height,
    };

    let selected_field = (focus == InfoModalFocus::Info).then_some(info_cursor);
    let (info_lines, field_spans) =
        build_info_lines(fields, theme, info_inner.width as usize, selected_field);

    let info_para = Paragraph::new(info_lines);
    frame.render_widget(info_para, content_rect);

    // Record click-to-copy hit regions for the visible portion of each value.
    for span in &field_spans {
        if span.start_line >= content_height {
            continue;
        }
        let visible = content_height - span.start_line;
        let height = span.line_count.min(visible);
        if height == 0 {
            continue;
        }
        hit_regions.push(InfoHitRegion {
            rect: Rect {
                x: content_rect.x,
                y: content_rect.y + span.start_line,
                width: content_rect.width,
                height,
            },
            label: span.label.clone(),
            value: span.value.clone(),
        });
    }

    // Copied-confirmation message at the bottom of the info view.
    if let Some(msg) = copied_msg {
        let msg_rect = Rect {
            x: info_inner.x,
            y: info_inner.y + content_height,
            width: info_inner.width,
            height: 1,
        };
        let para = Paragraph::new(Line::from(Span::styled(msg.to_string(), theme.merged)));
        frame.render_widget(para, msg_rect);
    } else {
        let hint_rect = Rect {
            x: info_inner.x,
            y: info_inner.y + content_height,
            width: info_inner.width,
            height: 1,
        };
        let hint = if focus == InfoModalFocus::Info {
            "Tab switch  ↑/↓ navigate  Enter/y copy  Esc close"
        } else {
            "Tab switch  j/k navigate  Enter invoke  Esc close"
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(hint, theme.secondary_text))),
            hint_rect,
        );
    }

    // Render actions pane on the right
    let block = block_panel(theme)
        .title("Actions")
        .title_alignment(Alignment::Left)
        .title_style(theme.title)
        .borders(Borders::RIGHT | Borders::TOP | Borders::BOTTOM);
    let actions_inner = block.inner(actions_rect);
    frame.render_widget(block, actions_rect);

    let list_items: Vec<ListItem> = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let prefix = if focus == InfoModalFocus::Actions && i == cursor {
                format!("{} ", symbols.cursor_prefix)
            } else {
                "  ".to_string()
            };

            let item_style = if !item.enabled {
                theme.secondary_text
            } else if focus == InfoModalFocus::Actions && i == cursor {
                theme.cursor
            } else {
                Style::default()
            };

            let prefix_span = Span::styled(prefix, item_style);
            let mut spans = vec![prefix_span];

            if let Some(ch) = item.shortcut {
                spans.push(Span::styled("[", item_style));
                spans.push(Span::styled(
                    ch.to_string(),
                    if item.enabled {
                        item_style.patch(theme.title)
                    } else {
                        item_style
                    },
                ));
                spans.push(Span::styled(format!("] {}", item.label), item_style));
            } else {
                spans.push(Span::styled(item.label.clone(), item_style));
            }

            if let Some(reason) = &item.reason {
                spans.push(Span::styled(format!(" ({})", reason), item_style));
            }

            ListItem::new(Line::from(spans))
        })
        .collect();

    let list = List::new(list_items);
    frame.render_widget(list, actions_inner);
}

#[allow(clippy::too_many_arguments)]
fn draw_info_modal_narrow(
    frame: &mut Frame,
    title: &str,
    fields: &[InfoField],
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
    let area = frame.area();
    let modal_rect = centered_rect_pct(85, 70, area);

    // Clear background
    frame.render_widget(Clear, modal_rect);

    // Build combined content: info lines, separator, actions header, action lines, hint
    let mut all_lines = Vec::new();

    // Info section (wrapped to the content width: borders + scrollbar = 3).
    // Info lines come first, so each FieldSpan's start_line is also its index
    // within all_lines.
    let content_width = modal_rect.width.saturating_sub(5) as usize;   // 2 borders + 1 scrollbar + 2 padding
    let selected_field = (focus == InfoModalFocus::Info).then_some(info_cursor);
    let (info_lines, field_spans) = build_info_lines(fields, theme, content_width, selected_field);
    all_lines.extend(info_lines);

    // Separator
    all_lines.push(Line::from(""));
    all_lines.push(Line::from(Span::styled(
        "─".repeat(modal_rect.width.saturating_sub(2) as usize),
        theme.secondary_text,
    )));
    all_lines.push(Line::from(""));

    // Actions header
    all_lines.push(Line::from(Span::styled("Actions", theme.title)));

    // Action items
    let actions_start_line = all_lines.len() as u16;
    for (i, item) in items.iter().enumerate() {
        let prefix = if focus == InfoModalFocus::Actions && i == cursor {
            format!("{} ", symbols.cursor_prefix)
        } else {
            "  ".to_string()
        };

        let item_style = if !item.enabled {
            theme.secondary_text
        } else if focus == InfoModalFocus::Actions && i == cursor {
            theme.cursor
        } else {
            Style::default()
        };

        let prefix_span = Span::styled(prefix, item_style);
        let mut spans = vec![prefix_span];

        if let Some(ch) = item.shortcut {
            spans.push(Span::styled("[", item_style));
            spans.push(Span::styled(
                ch.to_string(),
                if item.enabled {
                    item_style.patch(theme.title)
                } else {
                    item_style
                },
            ));
            spans.push(Span::styled(format!("] {}", item.label), item_style));
        } else {
            spans.push(Span::styled(item.label.clone(), item_style));
        }

        if let Some(reason) = &item.reason {
            spans.push(Span::styled(format!(" ({})", reason), item_style));
        }

        all_lines.push(Line::from(spans));
    }

    // Hint line
    all_lines.push(Line::from(""));
    all_lines.push(Line::from(Span::styled(
        if focus == InfoModalFocus::Info {
            "Tab switch  ↑/↓ navigate  Enter/y copy  Esc close"
        } else {
            "Tab switch  j/k navigate  Enter invoke  Esc close"
        },
        theme.secondary_text,
    )));

    let total_lines = all_lines.len() as u16;

    let block = block_panel(theme)
        .title(title)
        .title_alignment(Alignment::Left)
        .title_style(theme.title);
    let block_inner = block.inner(modal_rect);
    frame.render_widget(block, modal_rect);

    let inner = Rect {
        x: block_inner.x,
        y: block_inner.y,
        width: block_inner.width.saturating_sub(1), // reserve the scrollbar column
        height: block_inner.height,
    };

    // Reserve the bottom inner row for the copied-confirmation message.
    let content_height = inner.height.saturating_sub(1);
    let content_rect = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: content_height,
    };

    let max_scroll = total_lines.saturating_sub(content_height);

    // Auto-scroll to keep the current selection in view. This pane mixes
    // wrapped info text with one-line action rows in a single scroll
    // buffer, so the target line is computed by hand rather than via a
    // shared ListState.
    let target_line = match focus {
        InfoModalFocus::Actions => actions_start_line + cursor as u16,
        InfoModalFocus::Info => field_spans
            .get(info_cursor)
            .map(|span| span.start_line)
            .unwrap_or(0),
    };
    let mut offset = *scroll_offset;
    if target_line < offset {
        offset = target_line;
    } else if content_height > 0 && target_line >= offset + content_height {
        offset = target_line + 1 - content_height;
    }
    let clamped_offset = offset.min(max_scroll);
    *scroll_offset = clamped_offset;

    let para = Paragraph::new(all_lines).scroll((clamped_offset, 0));
    frame.render_widget(para, content_rect);

    // Record click-to-copy hit regions for the visible part of each info value,
    // accounting for the scroll offset (a field may be partly scrolled off).
    for span in &field_spans {
        let vis_start = span.start_line.max(clamped_offset);
        let vis_end = (span.start_line + span.line_count).min(clamped_offset + content_height);
        if vis_end <= vis_start {
            continue;
        }
        hit_regions.push(InfoHitRegion {
            rect: Rect {
                x: content_rect.x,
                y: content_rect.y + (vis_start - clamped_offset),
                width: content_rect.width,
                height: vis_end - vis_start,
            },
            label: span.label.clone(),
            value: span.value.clone(),
        });
    }

    // Copied-confirmation message on the bottom inner row.
    if let Some(msg) = copied_msg {
        let msg_rect = Rect {
            x: inner.x,
            y: inner.y + content_height,
            width: inner.width,
            height: 1,
        };
        let para = Paragraph::new(Line::from(Span::styled(msg.to_string(), theme.merged)));
        frame.render_widget(para, msg_rect);
    }

    // Render scrollbar on the right
    let scrollbar_rect = Rect {
        x: modal_rect.x + modal_rect.width - 1,
        y: modal_rect.y + 1,
        width: 1,
        height: modal_rect.height.saturating_sub(2),
    };

    let mut scrollbar_state = ScrollbarState::new(total_lines as usize);
    scrollbar_state = scrollbar_state.position(clamped_offset as usize);

    let scrollbar = Scrollbar::default().orientation(ScrollbarOrientation::VerticalRight);
    frame.render_stateful_widget(scrollbar, scrollbar_rect, &mut scrollbar_state);
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone, Utc};

    #[test]
    fn info_field_accessors_match_built_fields() {
        let row = InfoModalRow::Tag(TagInfo {
            name: "v1.2".into(),
            commit_hash: "abc123".into(),
            date: Utc::now(),
            message: Some("release".into()),
            is_annotated: true,
        });
        let fields = build_fields(&row);

        assert_eq!(row.info_field_count(), fields.len());
        for (index, field) in fields.iter().enumerate() {
            assert_eq!(
                row.info_field(index),
                Some((field.label.into(), field.value.clone()))
            );
        }
        assert_eq!(row.info_field(fields.len()), None);
    }

    #[test]
    fn graph_commit_fields_includes_author_and_date_rows() {
        let commit = GraphCommit {
            oid: "abcdef1234567".into(),
            summary: "test".into(),
            parents: vec![],
            lane: None,
            branch: None,
            refs: vec![],
            is_possible_squash_merge: false,
            fuzzy_squash_match: None,
            is_cherry_picked_commit: false,
            author_name: "Jane Doe".into(),
            author_email: "jane@example.com".into(),
            authored_at: Some(Utc::now() - Duration::hours(3)),
        };
        let fields = graph_commit_fields(&commit);
        let labels: Vec<&str> = fields.iter().map(|f| f.label).collect();
        assert!(labels.contains(&"Author"), "expected Author field, got {labels:?}");
        assert!(labels.contains(&"Date"), "expected Date field, got {labels:?}");
        let author = fields.iter().find(|f| f.label == "Author").unwrap();
        assert_eq!(author.value, "Jane Doe <jane@example.com>");
        let date = fields.iter().find(|f| f.label == "Date").unwrap();
        assert!(date.value.contains("ago"), "expected relative age, got: {}", date.value);
    }

    #[test]
    fn graph_commit_fields_omits_author_row_when_both_name_and_email_empty() {
        let commit = GraphCommit {
            oid: "x".into(),
            summary: "s".into(),
            parents: vec![],
            lane: None,
            branch: None,
            refs: vec![],
            is_possible_squash_merge: false,
            fuzzy_squash_match: None,
            is_cherry_picked_commit: false,
            author_name: "".into(),
            author_email: "".into(),
            authored_at: Some(Utc::now()),
        };
        let labels: Vec<&str> = graph_commit_fields(&commit)
            .iter()
            .map(|f| f.label)
            .collect();
        assert!(!labels.contains(&"Author"));
    }

    #[test]
    fn graph_commit_fields_shows_date_row_for_epoch_authored_at() {
        let commit = GraphCommit {
            oid: "x".into(),
            summary: "s".into(),
            parents: vec![],
            lane: None,
            branch: None,
            refs: vec![],
            is_possible_squash_merge: false,
            fuzzy_squash_match: None,
            is_cherry_picked_commit: false,
            author_name: "X".into(),
            author_email: "x@y".into(),
            authored_at: Some(Utc.timestamp_opt(0, 0).unwrap()),
        };
        let labels: Vec<&str> = graph_commit_fields(&commit)
            .iter()
            .map(|f| f.label)
            .collect();
        assert!(labels.contains(&"Date"));
    }

    #[test]
    fn graph_commit_fields_omits_date_row_when_authored_at_is_unknown() {
        let commit = GraphCommit {
            oid: "x".into(),
            summary: "s".into(),
            parents: vec![],
            lane: None,
            branch: None,
            refs: vec![],
            is_possible_squash_merge: false,
            fuzzy_squash_match: None,
            is_cherry_picked_commit: false,
            author_name: "X".into(),
            author_email: "x@y".into(),
            authored_at: None,
        };
        let labels: Vec<&str> = graph_commit_fields(&commit)
            .iter()
            .map(|f| f.label)
            .collect();
        assert!(!labels.contains(&"Date"));
    }

    #[test]
    fn branch_fields_includes_confidence_when_likely_squash_merged() {
        let b = BranchInfo {
            name: "feature/x".into(),
            is_current: false,
            is_base: false,
            tracking: TrackingStatus::Local,
            ahead: None,
            behind: None,
            last_commit_date: Utc::now(),
            merge_status: MergeStatus::LikelySquashMerged,
            base_branch: "main".into(),
            merge_base_commit: None,
            pr: None,
            squash_confidence: Some(SquashConfidence::FuzzyMatch {
                similarity_percent: 82,
            }),
        };
        let fields = branch_fields(&b);
        let confidence_field = fields
            .iter()
            .find(|f| f.label == "Confidence")
            .expect("Confidence field should be present");
        assert_eq!(confidence_field.value, "Fuzzy match (82%)");
    }

    #[test]
    fn branch_fields_omits_confidence_when_none() {
        let b = BranchInfo {
            name: "feature/y".into(),
            is_current: false,
            is_base: false,
            tracking: TrackingStatus::Local,
            ahead: None,
            behind: None,
            last_commit_date: Utc::now(),
            merge_status: MergeStatus::Unmerged,
            base_branch: "main".into(),
            merge_base_commit: None,
            pr: None,
            squash_confidence: None,
        };
        let fields = branch_fields(&b);
        assert!(!fields.iter().any(|f| f.label == "Confidence"));
    }

    #[test]
    fn remote_fields_includes_confidence_when_present() {
        let r = RemoteBranchInfo {
            full_ref: "origin/feature/x".into(),
            remote: "origin".into(),
            short_name: "feature/x".into(),
            has_local: false,
            is_base: false,
            last_commit_date: Utc::now(),
            merge_status: MergeStatus::LikelySquashMerged,
            ahead: None,
            behind: None,
            disjoint: false,
            pr: None,
            squash_confidence: Some(SquashConfidence::MergeTreeConfirmed),
        };
        let fields = remote_fields(&r);
        let confidence_field = fields
            .iter()
            .find(|f| f.label == "Confidence")
            .expect("Confidence field should be present");
        assert_eq!(confidence_field.value, "Merge-tree confirmed");
    }
}
