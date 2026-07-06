use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::prelude::*;
use ratatui::widgets::{
    Block, Borders, Clear, List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation,
    ScrollbarState,
};

use super::menu::MenuItem;
use super::shared::centered_rect_pct;
use crate::symbols::SymbolSet;
use crate::theme::Theme;
use crate::types::*;

#[derive(Debug, Clone)]
pub enum InfoModalRow {
    Branch(BranchInfo),
    Remote(RemoteBranchInfo),
    Tag(TagInfo),
    Worktree(WorktreeInfo),
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
    scroll_offset: u16,
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
        InfoModalRow::Branch(b) => b.name.clone(),
        InfoModalRow::Remote(r) => r.short_name.clone(),
        InfoModalRow::Tag(t) => t.name.clone(),
        InfoModalRow::Worktree(w) => w.path.to_string_lossy().to_string(),
    }
}

fn build_fields(row: &InfoModalRow) -> Vec<InfoField> {
    match row {
        InfoModalRow::Branch(b) => branch_fields(b),
        InfoModalRow::Remote(r) => remote_fields(r),
        InfoModalRow::Tag(t) => tag_fields(t),
        InfoModalRow::Worktree(w) => worktree_fields(w),
    }
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
        MergeStatus::Unmerged => "Unmerged",
        MergeStatus::Pending => "Pending",
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
) -> (Vec<Line<'static>>, Vec<FieldSpan>) {
    let value_width = width.saturating_sub(INFO_LABEL_WIDTH).max(1);
    let mut lines = Vec::new();
    let mut spans = Vec::new();
    for f in fields {
        let start_line = lines.len() as u16;
        let chunks = wrap_value(&f.value, value_width);
        let line_count = chunks.len() as u16;
        for (i, chunk) in chunks.into_iter().enumerate() {
            if i == 0 {
                lines.push(Line::from(vec![
                    Span::styled(format!("{:<15} ", f.label), theme.title),
                    Span::raw(chunk),
                ]));
            } else {
                lines.push(Line::from(vec![
                    Span::raw(" ".repeat(INFO_LABEL_WIDTH)),
                    Span::raw(chunk),
                ]));
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
    let block = Block::default()
        .title(title)
        .title_alignment(Alignment::Left)
        .title_style(theme.title)
        .borders(Borders::LEFT | Borders::TOP | Borders::BOTTOM | Borders::RIGHT);
    frame.render_widget(block, info_rect);

    let info_inner = Rect {
        x: info_rect.x + 1,
        y: info_rect.y + 1,
        width: info_rect.width.saturating_sub(2),
        height: info_rect.height.saturating_sub(1),
    };

    // Reserve the bottom row of the info pane for the copied-confirmation message.
    let content_height = info_inner.height.saturating_sub(1);
    let content_rect = Rect {
        x: info_inner.x,
        y: info_inner.y,
        width: info_inner.width,
        height: content_height,
    };

    let (info_lines, field_spans) = build_info_lines(fields, theme, info_inner.width as usize);

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
    }

    // Render actions pane on the right
    let block = Block::default()
        .title("Actions")
        .title_alignment(Alignment::Left)
        .title_style(theme.title)
        .borders(Borders::RIGHT | Borders::TOP | Borders::BOTTOM);
    frame.render_widget(block, actions_rect);

    let actions_inner = Rect {
        x: actions_rect.x + 1,
        y: actions_rect.y + 1,
        width: actions_rect.width.saturating_sub(1),
        height: actions_rect.height.saturating_sub(1),
    };

    let list_items: Vec<ListItem> = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let prefix = if i == cursor {
                format!("{} ", symbols.cursor_prefix)
            } else {
                "  ".to_string()
            };

            let item_style = if !item.enabled {
                theme.secondary_text
            } else if i == cursor {
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
                        theme.title
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
    scroll_offset: u16,
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
    let content_width = modal_rect.width.saturating_sub(3) as usize;
    let (info_lines, field_spans) = build_info_lines(fields, theme, content_width);
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
    for (i, item) in items.iter().enumerate() {
        let prefix = if i == cursor {
            format!("{} ", symbols.cursor_prefix)
        } else {
            "  ".to_string()
        };

        let item_style = if !item.enabled {
            theme.secondary_text
        } else if i == cursor {
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
                    theme.title
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
        "j/k navigate  Enter invoke  Esc close",
        theme.secondary_text,
    )));

    let total_lines = all_lines.len() as u16;

    let block = Block::default()
        .title(title)
        .title_alignment(Alignment::Left)
        .title_style(theme.title)
        .borders(Borders::ALL);

    frame.render_widget(block, modal_rect);

    let inner = Rect {
        x: modal_rect.x + 1,
        y: modal_rect.y + 1,
        width: modal_rect.width.saturating_sub(3), // -2 for borders, -1 for scrollbar
        height: modal_rect.height.saturating_sub(2),
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
    let clamped_offset = scroll_offset.min(max_scroll);

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
