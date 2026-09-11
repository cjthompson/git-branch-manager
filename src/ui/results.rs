use ratatui::prelude::*;
use ratatui::style::Modifier;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use super::shared::centered_rect;
use crate::theme::Theme;
use crate::types::{FailureCause, OperationResult};

/// Per-row recovery hint derived from a typed `FailureCause`. `None`
/// means "no recovery on this screen" — either `BranchNotFound` (the
/// branch is already gone, the row will vanish on next refresh) or
/// `Other` (a generic libgit2 error that needs human investigation).
fn recovery_hint(failure: Option<&FailureCause>) -> Option<&'static str> {
    match failure {
        Some(FailureCause::NotMerged) => Some("(press ! to force-delete)"),
        Some(FailureCause::CheckedOutInWorktree { is_main: false, .. }) => {
            Some("(press r to remove worktree + delete)")
        }
        Some(FailureCause::CheckedOutInWorktree { is_main: true, .. }) => None,
        Some(FailureCause::BranchNotFound) | Some(FailureCause::Other { .. }) | None => None,
    }
}

pub fn draw_results(frame: &mut Frame, results: &[OperationResult], theme: &Theme) {
    let area = frame.area();

    let mut lines: Vec<Line> = results
        .iter()
        .map(|r| {
            let (status, style, message_style) = if r.success {
                (" OK ", theme.merged, theme.dim)
            } else {
                ("FAIL", theme.error, theme.error)
            };

            let mut spans = vec![
                Span::styled(status, style),
                Span::raw("  "),
                Span::styled(
                    r.branch_name.clone(),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(r.message.clone(), message_style),
            ];
            if let Some(hint) = recovery_hint(r.failure.as_ref()) {
                spans.push(Span::raw("  "));
                spans.push(Span::styled(hint, theme.dim));
            }
            Line::from(spans)
        })
        .collect();

    lines.push(Line::from(""));
    let key_style = Style::default().fg(theme.title.fg.unwrap_or(Color::White));
    // Advertise `!`/`r` recovery keys only when at least one row is
    // actually recoverable — keeps the footer honest and prevents the
    // "press ! to force-delete" hint from appearing on rows where it
    // would do nothing.
    let has_force_recoverable = results
        .iter()
        .any(|r| matches!(&r.failure, Some(FailureCause::NotMerged)));
    let has_worktree_recoverable = results
        .iter()
        .any(|r| {
            matches!(
                &r.failure,
                Some(FailureCause::CheckedOutInWorktree {
                    is_main: false,
                    ..
                })
            )
        });
    let mut footer_spans = vec![Span::styled("Press ", theme.dim)];
    if has_force_recoverable {
        footer_spans.push(Span::styled("!", key_style));
        footer_spans.push(Span::styled(" force", theme.dim));
        footer_spans.push(Span::raw("  "));
    }
    if has_worktree_recoverable {
        footer_spans.push(Span::styled("r", key_style));
        footer_spans.push(Span::styled(" remove worktree", theme.dim));
        footer_spans.push(Span::raw("  "));
    }
    footer_spans.push(Span::styled("Enter", key_style));
    footer_spans.push(Span::styled("/", theme.dim));
    footer_spans.push(Span::styled("Esc", key_style));
    footer_spans.push(Span::styled(" to continue", theme.dim));
    lines.push(Line::from(footer_spans));

    // Calculate dynamic width based on maximum content width
    let content_max_width: usize = lines
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.chars().count())
                .sum::<usize>()
        })
        .max()
        .unwrap_or(0);
    let width = (content_max_width as u16 + 4)
        .max(50)
        .min(area.width.saturating_sub(2));

    // Calculate height accounting for text wrapping
    let inner_width = width.saturating_sub(2) as usize;
    let wrapped_height: usize = lines
        .iter()
        .map(|l| {
            let char_count: usize = l.spans.iter().map(|s| s.content.chars().count()).sum();
            if char_count == 0 {
                1 // empty line still takes 1 row
            } else {
                char_count.div_ceil(inner_width.max(1))
            }
        })
        .sum();
    let content_height = (wrapped_height + 2) as u16; // +2 for block borders

    let max_height = (area.height * 80 / 100).max(10);
    let modal_height = content_height.min(max_height).min(area.height);

    let rect = centered_rect(width, modal_height, area);

    let paragraph = Paragraph::new(lines)
        .block(
            Block::default()
                .title("Results")
                .title_style(theme.title)
                .borders(Borders::ALL),
        )
        .wrap(Wrap { trim: false });

    frame.render_widget(Clear, rect);
    frame.render_widget(paragraph, rect);
}
