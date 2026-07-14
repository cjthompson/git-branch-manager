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
pub(crate) fn merge_status_parts(
    status: &MergeStatus,
    ctx: &CellContext,
    col_width: Option<usize>,
) -> (String, Style) {
    let theme = ctx.theme;
    let symbols = ctx.symbols;
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

/// Ahead/behind spans. Zero and `None` counts are omitted; both non-zero counts
/// render with a space separator.
pub(crate) fn ahead_behind_spans(
    ahead: Option<u32>,
    behind: Option<u32>,
    ctx: &CellContext,
) -> Vec<Span<'static>> {
    let mut parts: Vec<Span<'static>> = Vec::new();
    if let Some(a) = ahead {
        if a > 0 {
            parts.push(Span::styled(
                format!("{}{a}", ctx.symbols.arrow_up),
                ctx.theme.ahead_behind,
            ));
        }
    }
    if let Some(b) = behind {
        if b > 0 {
            if !parts.is_empty() {
                parts.push(Span::raw(" "));
            }
            parts.push(Span::styled(
                format!("{}{b}", ctx.symbols.arrow_down),
                ctx.theme.ahead_behind,
            ));
        }
    }
    parts
}

// ── Public Line builders ────────────────────────────────────────────────────

/// Ahead/behind line (branch and remote rows).
pub fn ahead_behind_line(
    ahead: Option<u32>,
    behind: Option<u32>,
    ctx: &CellContext,
) -> Line<'static> {
    Line::from(ahead_behind_spans(ahead, behind, ctx))
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
mod tests {
    use super::*;
    use crate::symbols::SymbolSet;
    use crate::theme::Theme;

    fn theme_and_symbols() -> (Theme, SymbolSet) {
        (Theme::dark(), SymbolSet::ascii())
    }

    // --- merge_status_parts: abbreviated when the column is too narrow ---

    #[test]
    fn merge_status_narrow_is_abbreviated() {
        let (theme, symbols) = theme_and_symbols();
        let ctx = CellContext {
            theme: &theme,
            symbols: &symbols,
            area_width: 60,
            compact: true,
            data_col_widths: Vec::new(),
            first_col_width: 80,
        };
        // Column width 4 fits "sm ~"/"u -"/"m +" but not the full words.
        let w = Some(4);
        assert_eq!(merge_status_parts(&MergeStatus::Merged, &ctx, w).0, "m +");
        assert_eq!(merge_status_parts(&MergeStatus::InSync, &ctx, w).0, "is =");
        assert_eq!(
            merge_status_parts(&MergeStatus::SquashMerged, &ctx, w).0,
            "sm ~"
        );
        assert_eq!(merge_status_parts(&MergeStatus::Unmerged, &ctx, w).0, "u -");
        assert_eq!(
            merge_status_parts(&MergeStatus::Pending, &ctx, w).0,
            "\u{2026}"
        );
    }

    #[test]
    fn merge_status_wide_is_full_text() {
        let (theme, symbols) = theme_and_symbols();
        let ctx = CellContext {
            theme: &theme,
            symbols: &symbols,
            area_width: 80,
            compact: false,
            data_col_widths: Vec::new(),
            first_col_width: 80,
        };
        let w = Some(15);
        assert_eq!(
            merge_status_parts(&MergeStatus::Merged, &ctx, w).0,
            "merged +"
        );
        assert_eq!(
            merge_status_parts(&MergeStatus::InSync, &ctx, w).0,
            "in-sync ="
        );
        assert_eq!(
            merge_status_parts(&MergeStatus::SquashMerged, &ctx, w).0,
            "squash-merged ~"
        );
        assert_eq!(
            merge_status_parts(&MergeStatus::Unmerged, &ctx, w).0,
            "unmerged -"
        );
        assert_eq!(
            merge_status_parts(&MergeStatus::Pending, &ctx, w).0,
            "pending \u{2026}"
        );
    }

    #[test]
    fn merge_status_local_remote_variants() {
        let (theme, symbols) = theme_and_symbols();
        let ctx = CellContext {
            theme: &theme,
            symbols: &symbols,
            area_width: 80,
            compact: false,
            data_col_widths: Vec::new(),
            first_col_width: 80,
        };
        let w = Some(20);
        assert_eq!(
            merge_status_parts(&MergeStatus::RemoteMerged, &ctx, w).0,
            "remote-merged +v"
        );
        assert_eq!(
            merge_status_parts(&MergeStatus::LocalMerged, &ctx, w).0,
            "local-merged +^"
        );
        assert_eq!(
            merge_status_parts(&MergeStatus::RemoteSquashMerged, &ctx, w).0,
            "remote-squash ~v"
        );
        assert_eq!(
            merge_status_parts(&MergeStatus::LocalSquashMerged, &ctx, w).0,
            "local-squash ~^"
        );
        // abbreviated forms
        assert_eq!(
            merge_status_parts(&MergeStatus::RemoteMerged, &ctx, Some(5)).0,
            "rm +v"
        );
        assert_eq!(
            merge_status_parts(&MergeStatus::LocalMerged, &ctx, Some(5)).0,
            "lm +^"
        );
    }

    #[test]
    fn merge_status_fits_full_at_exact_width_abbrev_one_less() {
        let (theme, symbols) = theme_and_symbols();
        let ctx = CellContext {
            theme: &theme,
            symbols: &symbols,
            area_width: 80,
            compact: false,
            data_col_widths: Vec::new(),
            first_col_width: 80,
        };
        // "merged +" is 8 chars: width 8 shows full, width 7 abbreviates.
        assert_eq!(
            merge_status_parts(&MergeStatus::Merged, &ctx, Some(8)).0,
            "merged +"
        );
        assert_eq!(
            merge_status_parts(&MergeStatus::Merged, &ctx, Some(7)).0,
            "m +"
        );
    }

    // --- worktree_status_parts: full when wide, single letters when narrow ---

    #[test]
    fn worktree_status_full_and_abbreviated() {
        let (theme, symbols) = theme_and_symbols();
        let ctx = CellContext {
            theme: &theme,
            symbols: &symbols,
            area_width: 80,
            compact: false,
            data_col_widths: Vec::new(),
            first_col_width: 80,
        };
        let clean = WorkingTreeStatus::clean();
        let dirty = WorkingTreeStatus {
            has_staged: true,
            has_modified: true,
            has_untracked: false,
            changed_files: Vec::new(),
        };
        // Wide enough for the full label.
        assert_eq!(worktree_status_parts(&clean, &ctx, Some(9)).0, "clean");
        assert_eq!(
            worktree_status_parts(&dirty, &ctx, Some(15)).0,
            "staged+modified"
        );
        // Too narrow: abbreviate.
        assert_eq!(worktree_status_parts(&clean, &ctx, Some(3)).0, "c");
        assert_eq!(worktree_status_parts(&dirty, &ctx, Some(3)).0, "s+m");
    }

    // --- merge_status_line_for_branch: base is blank ---

    #[test]
    fn branch_base_status_is_blank() {
        let (theme, symbols) = theme_and_symbols();
        let ctx = CellContext {
            theme: &theme,
            symbols: &symbols,
            area_width: 80,
            compact: false,
            data_col_widths: Vec::new(),
            first_col_width: 80,
        };
        assert_eq!(
            merge_status_line_for_branch(&MergeStatus::Merged, true, &ctx, Some(15)),
            Line::from("")
        );
        assert_ne!(
            merge_status_line_for_branch(&MergeStatus::Merged, false, &ctx, Some(15)),
            Line::from("")
        );
    }

    // --- pr_parts: style per PrStatus, empty when None ---

    #[test]
    fn pr_none_is_empty() {
        let (theme, symbols) = theme_and_symbols();
        let ctx = CellContext {
            theme: &theme,
            symbols: &symbols,
            area_width: 80,
            compact: false,
            data_col_widths: Vec::new(),
            first_col_width: 80,
        };
        let (text, style) = pr_parts(None, &ctx, Some(9));
        assert_eq!(text, "");
        assert_eq!(style, Style::default());
    }

    #[test]
    fn pr_styles_match_status() {
        let (theme, symbols) = theme_and_symbols();
        let ctx = CellContext {
            theme: &theme,
            symbols: &symbols,
            area_width: 80,
            compact: false,
            data_col_widths: Vec::new(),
            first_col_width: 80,
        };
        for (status, expected) in [
            (PrStatus::Draft, theme.pr_draft),
            (PrStatus::Open, theme.pr_open),
            (PrStatus::Merged, theme.pr_merged),
            (PrStatus::Closed, theme.pr_closed),
        ] {
            let pr = PrInfo { number: 42, status };
            let (text, style) = pr_parts(Some(&pr), &ctx, Some(9));
            assert_eq!(text, "#42");
            assert_eq!(style, expected);
        }
    }

    #[test]
    fn pr_wide_column_shows_full_number() {
        let (theme, symbols) = theme_and_symbols();
        let ctx = CellContext {
            theme: &theme,
            symbols: &symbols,
            area_width: 80,
            compact: false,
            data_col_widths: Vec::new(),
            first_col_width: 80,
        };
        let pr = PrInfo {
            number: 357902,
            status: PrStatus::Open,
        };
        let (text, _) = pr_parts(Some(&pr), &ctx, Some(9));
        assert_eq!(text, "#357902");
    }

    #[test]
    fn pr_narrow_column_shows_icon_only() {
        let (theme, symbols) = theme_and_symbols();
        let ctx = CellContext {
            theme: &theme,
            symbols: &symbols,
            area_width: 60,
            compact: true,
            data_col_widths: Vec::new(),
            first_col_width: 80,
        };
        let pr = PrInfo {
            number: 357902,
            status: PrStatus::Open,
        };
        let (text, _) = pr_parts(Some(&pr), &ctx, Some(2));
        assert_eq!(text, symbols.pr_indicator);
    }

    // --- ahead_behind_spans: omit zero/None, separator for both non-zero ---

    #[test]
    fn ahead_behind_omits_zero_and_none() {
        let (theme, symbols) = theme_and_symbols();
        let ctx = CellContext {
            theme: &theme,
            symbols: &symbols,
            area_width: 80,
            compact: false,
            data_col_widths: Vec::new(),
            first_col_width: 80,
        };
        assert!(ahead_behind_spans(Some(0), Some(0), &ctx).is_empty());
        assert!(ahead_behind_spans(None, None, &ctx).is_empty());
    }

    #[test]
    fn ahead_behind_single_side() {
        let (theme, symbols) = theme_and_symbols();
        let ctx = CellContext {
            theme: &theme,
            symbols: &symbols,
            area_width: 80,
            compact: false,
            data_col_widths: Vec::new(),
            first_col_width: 80,
        };
        let ahead = ahead_behind_spans(Some(3), Some(0), &ctx);
        assert_eq!(ahead.len(), 1);
        assert_eq!(ahead[0].content.as_ref(), "+3");

        let behind = ahead_behind_spans(None, Some(2), &ctx);
        assert_eq!(behind.len(), 1);
        assert_eq!(behind[0].content.as_ref(), "-2");
    }

    #[test]
    fn ahead_behind_both_nonzero_has_separator() {
        let (theme, symbols) = theme_and_symbols();
        let ctx = CellContext {
            theme: &theme,
            symbols: &symbols,
            area_width: 80,
            compact: false,
            data_col_widths: Vec::new(),
            first_col_width: 80,
        };
        let spans = ahead_behind_spans(Some(1), Some(2), &ctx);
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].content.as_ref(), "+1");
        assert_eq!(spans[1].content.as_ref(), " ");
        assert_eq!(spans[2].content.as_ref(), "-2");
    }
}
