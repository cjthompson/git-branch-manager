use ratatui::prelude::*;
use ratatui::widgets::{Clear, Paragraph, Wrap};

use crate::theme::Theme;
use crate::types::BranchAction;

use super::render::ConfirmExtraKey;
use super::shared::{block_panel, centered_rect, key_hint};

/// Renders a confirmation dialog overlay.
///
/// `action` is the operation about to be performed.
/// `target_names` is the list of items the action will affect.
/// `reason` (plan P005 §6) is an optional pre-flight reason block
/// rendered above the target list when the build_delete_preflight pass
/// detected unmerged commits or a worktree holding the branch.
/// `extra_keys` are the alternate-action keys (`!`, `r`) shown after
/// `[y]es [n]o` so the user can switch to the recovery variant
/// without leaving the confirm flow.
pub fn draw_confirm(
    frame: &mut Frame,
    action: BranchAction,
    target_names: &[String],
    reason: Option<&str>,
    extra_keys: &[ConfirmExtraKey],
    theme: &Theme,
) {
    let action_label = action.label();
    let count = target_names.len();

    let mut lines = vec![
        Line::from(Span::styled(
            format!("{action_label} {count} item(s)?"),
            theme.title,
        )),
        Line::from(""),
    ];

    // Optional pre-flight reason block (plan P005 §6). Rendered as one
    // wrapped paragraph so long worktree paths flow on narrow terminals
    // instead of being mid-word truncated.
    if let Some(reason_text) = reason {
        for raw_line in reason_text.split('\n') {
            lines.push(Line::from(Span::styled(
                format!("  {}", raw_line),
                theme.secondary_text,
            )));
        }
        lines.push(Line::from(""));
    }

    for name in target_names {
        lines.push(Line::from(Span::styled(
            format!("  {}", name),
            theme.selected,
        )));
    }

    lines.push(Line::from(""));
    let key_style = Style::default().fg(theme.accent_fg()).add_modifier(Modifier::BOLD);
    let mut footer = vec![
        Span::styled("[", theme.dim),
        Span::styled("y", key_style),
        Span::styled("]es  [", theme.dim),
        Span::styled("n", key_style),
        Span::styled("]o", theme.dim),
    ];
    // Append the alternate-action hints so the user can see what `!`
    // and `r` would do at a glance.
    for extra in extra_keys {
        footer.push(Span::raw("  "));
        footer.extend(key_hint(extra.key, &extra.label, theme));
    }
    lines.push(Line::from(footer));

    // Calculate overlay size
    let area = frame.area();
    let max_height = (area.height * 60 / 100).max(8);
    let inner_max = max_height.saturating_sub(2) as usize; // subtract borders

    // Truncate if content exceeds available space. Header now has the
    // action line + blank + (optional reason block + blank) + targets +
    // blank + footer; the math below assumes a fixed 3-line footer
    // (blank + yes/no + extras still wraps to one line on most widths)
    // and recomputes the available budget for the target list.
    if lines.len() > inner_max {
        // Fixed blocks at top: action line, blank, optional reason
        // (re-counted below), blank, then footer (2 lines: blank +
        // key list). We compute header_lines dynamically so the
        // optional reason block fits.
        let action_lines = 2; // action question + blank
        let reason_lines = reason.map_or(0, |r| {
            // Each reason line plus a trailing blank. The pre-flight
            // produces reason_text.split('\n') joined by ", OR\n  ", so
            // count explicit newlines. Worst case this over-estimates
            // by one blank line, which only eats one target line.
            r.matches('\n').count() + 2
        });
        let footer_lines = 2; // blank + yes/no (extras wrap inline)
        let header_lines = action_lines + reason_lines;
        let available_for_items =
            inner_max.saturating_sub(header_lines + footer_lines + 1);

        let targets_start = header_lines;
        let targets_end = lines.len() - footer_lines;
        let total_items = targets_end.saturating_sub(targets_start);
        let hidden = total_items.saturating_sub(available_for_items);

        if hidden > 0 {
            let footer: Vec<Line> = lines.split_off(lines.len() - footer_lines);
            lines.truncate(targets_start + available_for_items);
            lines.push(Line::from(Span::styled(
                format!("  ...{} more", hidden),
                theme.dim,
            )));
            lines.extend(footer);
        }
    }

    // Expand width to fit the longest content line, avoiding mid-word wraps.
    let content_max_width = lines
        .iter()
        .map(|l: &Line| {
            l.spans
                .iter()
                .map(|s| s.content.chars().count())
                .sum::<usize>()
        })
        .max()
        .unwrap_or(0) as u16;
    let width_cap = (area.width * 80 / 100).min(100);
    let width = (content_max_width + 4)
        .max(40)
        .min(area.width.saturating_sub(2))
        .min(width_cap);

    // Simulate wrapping at the actual inner width so the height accounts for
    // any lines that still wrap (e.g. very long paths in narrow terminals).
    let inner_width = width.saturating_sub(4) as usize; // 2 for borders + 2 for block_panel's horizontal padding
    let wrapped_height: usize = lines
        .iter()
        .map(|l| {
            let chars: usize = l.spans.iter().map(|s| s.content.chars().count()).sum();
            if chars == 0 {
                1
            } else {
                chars.div_ceil(inner_width.max(1))
            }
        })
        .sum();
    let content_height = (wrapped_height as u16) + 2 + 2; // +2 borders, +2 word-wrap slack (ratatui's Wrap{trim:false} is word-wrap, not char-wrap, so the div_ceil simulation above can under-count)
    let height = content_height.min(max_height).min(area.height);

    let rect = centered_rect(width, height, area);

    let block = block_panel(theme)
        .title(format!("Confirm {}", action_label))
        .title_style(theme.title);

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });

    frame.render_widget(Clear, rect);
    frame.render_widget(paragraph, rect);
}
