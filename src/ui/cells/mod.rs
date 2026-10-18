//! Shared row-cell builders.
//!
//! The generic table renderer in `list_render` delegates per-row cell
//! construction to each view's `RowRenderer`. Several cells (age, merge status,
//! ahead/behind, PR) render identically across views; this module centralizes
//! them so visual and responsive behavior cannot drift per tab.
//!
//! The cell builders take a `CellContext` (theme, symbols, width, compact) and
//! return a ratatui `Line`. Pure formatting logic is factored into small
//! `(String, Style)` / `Vec<Span>` helpers so it can be unit-tested without
//! introspecting an opaque `Cell`.

use chrono::{DateTime, Utc};
use ratatui::prelude::*;
use ratatui::style::Modifier;
use ratatui::text::Line;

use crate::types::{MergeStatus, PrInfo, PrStatus, WorkingTreeStatus};
use crate::ui::list_render::CellContext;
use crate::ui::shared::age_style;
use crate::{symbols::SymbolSet, theme::Theme};

// ── Pure formatting helpers (testable without building a Cell) ──────────────

/// Choose between a full and abbreviated label based on the resolved column
/// width: show `full` when it fits, otherwise `short`. When the width is
/// unknown (`None`), fall back to `compact` (the global narrow-terminal flag).
///
/// This is the single responsive-label rule shared by the Age, Merge, and
/// worktree Status columns so their narrow/wide behavior cannot drift.
pub fn fit_text(full: String, short: String, col_width: Option<usize>, compact: bool) -> String {
    match col_width {
        Some(width) if full.chars().count() <= width => full,
        Some(_) => short,
        None if compact => short,
        None => full,
    }
}

/// Merge-status text + style, fit to the resolved column width: full words
/// (`merged`/`squash-merged`/`unmerged`) when the column is wide enough,
/// abbreviations (`m`/`sm`/`u`) when narrow. The status symbol is always shown.
fn merge_status_labels(
    status: &MergeStatus,
    theme: &Theme,
    symbols: &SymbolSet,
) -> (String, String, Style) {
    let (full, short, style) = match status {
        MergeStatus::Merged => (
            format!("merged {}", symbols.status_merged),
            format!("m {}", symbols.status_merged),
            theme.merged,
        ),
        MergeStatus::InSync => (
            format!("in-sync {}", symbols.status_in_sync),
            format!("is {}", symbols.status_in_sync),
            theme.in_sync,
        ),
        MergeStatus::SquashMerged => (
            format!("squash-merged {}", symbols.status_squash_merged),
            format!("sm {}", symbols.status_squash_merged),
            theme.squash_merged,
        ),
        MergeStatus::LocalMerged => (
            format!(
                "local-merged {}{}",
                symbols.status_merged, symbols.status_local_suffix
            ),
            format!(
                "lm {}{}",
                symbols.status_merged, symbols.status_local_suffix
            ),
            theme.merged.add_modifier(Modifier::ITALIC),
        ),
        MergeStatus::RemoteMerged => (
            format!(
                "remote-merged {}{}",
                symbols.status_merged, symbols.status_remote_suffix
            ),
            format!(
                "rm {}{}",
                symbols.status_merged, symbols.status_remote_suffix
            ),
            theme.merged.add_modifier(Modifier::ITALIC),
        ),
        MergeStatus::LocalSquashMerged => (
            format!(
                "local-squash {}{}",
                symbols.status_squash_merged, symbols.status_local_suffix
            ),
            format!(
                "ls {}{}",
                symbols.status_squash_merged, symbols.status_local_suffix
            ),
            theme.squash_merged.add_modifier(Modifier::ITALIC),
        ),
        MergeStatus::RemoteSquashMerged => (
            format!(
                "remote-squash {}{}",
                symbols.status_squash_merged, symbols.status_remote_suffix
            ),
            format!(
                "rs {}{}",
                symbols.status_squash_merged, symbols.status_remote_suffix
            ),
            theme.squash_merged.add_modifier(Modifier::ITALIC),
        ),
        MergeStatus::LikelySquashMerged => (
            format!("possible-squash {}", symbols.status_squash_merged),
            format!("ps {}", symbols.status_squash_merged),
            theme.squash_merged.add_modifier(Modifier::DIM),
        ),
        MergeStatus::CherryPicked => (
            format!("cherry-picked {}", symbols.status_cherry_picked),
            format!("cp {}", symbols.status_cherry_picked),
            theme.cherry_picked,
        ),
        MergeStatus::LocalCherryPicked => (
            format!(
                "local-cherry {}{}",
                symbols.status_cherry_picked, symbols.status_local_suffix
            ),
            format!(
                "lc {}{}",
                symbols.status_cherry_picked, symbols.status_local_suffix
            ),
            theme.cherry_picked.add_modifier(Modifier::ITALIC),
        ),
        MergeStatus::RemoteCherryPicked => (
            format!(
                "remote-cherry {}{}",
                symbols.status_cherry_picked, symbols.status_remote_suffix
            ),
            format!(
                "rc {}{}",
                symbols.status_cherry_picked, symbols.status_remote_suffix
            ),
            theme.cherry_picked.add_modifier(Modifier::ITALIC),
        ),
        MergeStatus::Unmerged => (
            format!("unmerged {}", symbols.status_unmerged),
            format!("u {}", symbols.status_unmerged),
            theme.unmerged,
        ),
        MergeStatus::Pending => (
            "pending \u{2026}".to_string(),
            "\u{2026}".to_string(),
            theme.dim,
        ),
    };
    (full, short, style)
}

pub(crate) fn merge_status_parts(
    status: &MergeStatus,
    ctx: &CellContext,
    col_width: Option<usize>,
) -> (String, Style) {
    let (full, short, style) = merge_status_labels(status, ctx.theme, ctx.symbols);
    (fit_text(full, short, col_width, ctx.compact), style)
}

/// Worktree working-tree-status text + style, fit to the resolved column width:
/// full words (`clean`/`staged`/`unstaged`/…) when wide, abbreviations
/// (`c`/`s`/`u`/`t`) when narrow.
pub(crate) fn worktree_status_parts(
    status: &WorkingTreeStatus,
    ctx: &CellContext,
    col_width: Option<usize>,
) -> (String, Style) {
    if status.is_clean() {
        (
            fit_text("clean".to_string(), "c".to_string(), col_width, ctx.compact),
            ctx.theme.merged,
        )
    } else {
        (
            fit_text(
                status.summary(),
                status.short_summary(),
                col_width,
                ctx.compact,
            ),
            ctx.theme.unmerged,
        )
    }
}

/// PR cell text + style, fit to the resolved column width: the full `#<number>`
/// when the column is wide enough, just the PR indicator icon when narrow.
/// Empty when there is no PR.
pub(crate) fn pr_parts(
    pr: Option<&PrInfo>,
    ctx: &CellContext,
    col_width: Option<usize>,
) -> (String, Style) {
    match pr {
        Some(pr) => {
            let style = match pr.status {
                PrStatus::Draft => ctx.theme.pr_draft,
                PrStatus::Open => ctx.theme.pr_open,
                PrStatus::Merged => ctx.theme.pr_merged,
                PrStatus::Closed => ctx.theme.pr_closed,
            };
            let full = format!("#{}", pr.number);
            let short = ctx.symbols.pr_indicator.to_string();
            (fit_text(full, short, col_width, ctx.compact), style)
        }
        None => (String::new(), Style::default()),
    }
}

/// Ahead/behind spans, fit to the resolved column width per segment. Zero and
/// `None` counts are omitted. Each present segment is styled independently
/// (`theme.ahead` / `theme.behind`); a literal space separates them when both
/// are present.
pub(crate) fn ahead_behind_parts(
    ahead: Option<u32>,
    behind: Option<u32>,
    ctx: &CellContext,
    col_width: Option<usize>,
) -> Vec<Span<'static>> {
    let a = ahead.filter(|&a| a > 0);
    let b = behind.filter(|&b| b > 0);
    let mut spans = Vec::new();
    if let Some(a) = a {
        let full = format!("{}{a}", ctx.symbols.arrow_up);
        let short = ctx.symbols.arrow_up.to_string();
        spans.push(Span::styled(
            fit_text(full, short, col_width, ctx.compact),
            ctx.theme.ahead,
        ));
    }
    if let Some(b) = b {
        if !spans.is_empty() {
            spans.push(Span::raw(" "));
        }
        let full = format!("{}{b}", ctx.symbols.arrow_down);
        let short = ctx.symbols.arrow_down.to_string();
        spans.push(Span::styled(
            fit_text(full, short, col_width, ctx.compact),
            ctx.theme.behind,
        ));
    }
    spans
}

// ── Public Line builders ────────────────────────────────────────────────────

/// Ahead/behind line (branch and remote rows). `col_width` is the resolved
/// column width, used to choose counts vs arrows-only.
pub fn ahead_behind_line(
    ahead: Option<u32>,
    behind: Option<u32>,
    ctx: &CellContext,
    col_width: Option<usize>,
) -> Line<'static> {
    Line::from(ahead_behind_parts(ahead, behind, ctx, col_width))
}

/// PR number line (branch and remote rows). `col_width` is the resolved
/// column width, used to choose the full number vs the icon-only form.
pub fn pr_line(pr: Option<&PrInfo>, ctx: &CellContext, col_width: Option<usize>) -> Line<'static> {
    let (text, style) = pr_parts(pr, ctx, col_width);
    Line::from(Span::styled(text, style))
}

/// Age line, right-aligned. The caller passes the already-formatted age string
/// (`item.age_short()` or `item.age_display()` depending on `ctx.compact`); the
/// date drives the age-based color.
pub fn age_line(age_text: String, date: &DateTime<Utc>, ctx: &CellContext) -> Line<'static> {
    let style = age_style(date, ctx.theme);
    Line::from(Span::styled(age_text, style)).alignment(Alignment::Right)
}

/// Merge-status line, right-aligned (remote and worktree rows). `col_width` is
/// the resolved column width, used to choose full vs abbreviated text.
pub fn merge_status_line(
    status: &MergeStatus,
    ctx: &CellContext,
    col_width: Option<usize>,
) -> Line<'static> {
    let (text, style) = merge_status_parts(status, ctx, col_width);
    Line::from(Span::styled(text, style)).alignment(Alignment::Right)
}

/// Merge-status line for branch rows: the base branch shows a blank status,
/// otherwise this delegates to [`merge_status_line`].
pub fn merge_status_line_for_branch(
    status: &MergeStatus,
    is_base: bool,
    ctx: &CellContext,
    col_width: Option<usize>,
) -> Line<'static> {
    if is_base {
        Line::from("")
    } else {
        merge_status_line(status, ctx, col_width)
    }
}

/// Worktree working-tree-status line (left-aligned). `col_width` is the resolved
/// column width, used to choose full vs abbreviated text.
pub fn worktree_status_line(
    status: &WorkingTreeStatus,
    ctx: &CellContext,
    col_width: Option<usize>,
) -> Line<'static> {
    let (text, style) = worktree_status_parts(status, ctx, col_width);
    Line::from(Span::styled(text, style))
}

#[cfg(test)]
mod tests;
