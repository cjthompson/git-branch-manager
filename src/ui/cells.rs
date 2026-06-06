//! Shared row-cell builders.
//!
//! The generic table renderer in `list_render` delegates per-row cell
//! construction to each view's `RowRenderer`. Several cells (age, merge status,
//! ahead/behind, PR) render identically across views; this module centralizes
//! them so visual and responsive behavior cannot drift per tab.
//!
//! The cell builders take a `CellContext` (theme, symbols, width, compact) and
//! return a ratatui `Cell`. Pure formatting logic is factored into small
//! `(String, Style)` / `Vec<Span>` helpers so it can be unit-tested without
//! introspecting an opaque `Cell`.

use chrono::{DateTime, Utc};
use ratatui::prelude::*;
use ratatui::widgets::Cell;

use crate::types::{MergeStatus, PrInfo, PrStatus};
use crate::ui::list_render::CellContext;
use crate::ui::shared::age_style;

// ── Pure formatting helpers (testable without building a Cell) ──────────────

/// Merge-status text + style. Symbol-only below width 70, full text at 70+.
pub(crate) fn merge_status_parts(status: &MergeStatus, ctx: &CellContext) -> (String, Style) {
    let theme = ctx.theme;
    let symbols = ctx.symbols;
    if ctx.area_width < 70 {
        match status {
            MergeStatus::Merged => (symbols.status_merged.to_string(), theme.merged),
            MergeStatus::SquashMerged => (
                symbols.status_squash_merged.to_string(),
                theme.squash_merged,
            ),
            MergeStatus::Unmerged => (symbols.status_unmerged.to_string(), theme.unmerged),
            MergeStatus::Pending => ("\u{2026}".to_string(), theme.dim),
        }
    } else {
        match status {
            MergeStatus::Merged => (format!("merged {}", symbols.status_merged), theme.merged),
            MergeStatus::SquashMerged => (
                format!("squash-merged {}", symbols.status_squash_merged),
                theme.squash_merged,
            ),
            MergeStatus::Unmerged => (
                format!("unmerged {}", symbols.status_unmerged),
                theme.unmerged,
            ),
            MergeStatus::Pending => ("pending \u{2026}".to_string(), theme.dim),
        }
    }
}

/// PR cell text + style. Empty when there is no PR.
pub(crate) fn pr_parts(pr: Option<&PrInfo>, ctx: &CellContext) -> (String, Style) {
    match pr {
        Some(pr) => {
            let style = match pr.status {
                PrStatus::Draft => ctx.theme.pr_draft,
                PrStatus::Open => ctx.theme.pr_open,
                PrStatus::Merged => ctx.theme.pr_merged,
                PrStatus::Closed => ctx.theme.pr_closed,
            };
            (format!("#{}", pr.number), style)
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

// ── Public Cell builders ────────────────────────────────────────────────────

/// Ahead/behind cell (branch and remote rows).
pub fn ahead_behind_cell(
    ahead: Option<u32>,
    behind: Option<u32>,
    ctx: &CellContext,
) -> Cell<'static> {
    Cell::from(Line::from(ahead_behind_spans(ahead, behind, ctx)))
}

/// PR number cell (branch and remote rows).
pub fn pr_cell(pr: Option<&PrInfo>, ctx: &CellContext) -> Cell<'static> {
    let (text, style) = pr_parts(pr, ctx);
    Cell::from(Span::styled(text, style))
}

/// Age cell, right-aligned. The caller passes the already-formatted age string
/// (`item.age_short()` or `item.age_display()` depending on `ctx.compact`); the
/// date drives the age-based color.
pub fn age_cell(age_text: String, date: &DateTime<Utc>, ctx: &CellContext) -> Cell<'static> {
    let style = age_style(date, ctx.theme);
    Cell::from(Line::from(Span::styled(age_text, style)).alignment(Alignment::Right))
}

/// Merge-status cell, right-aligned (remote and worktree rows).
pub fn merge_status_cell(status: &MergeStatus, ctx: &CellContext) -> Cell<'static> {
    let (text, style) = merge_status_parts(status, ctx);
    Cell::from(Line::from(Span::styled(text, style)).alignment(Alignment::Right))
}

/// Merge-status cell for branch rows: the base branch shows a blank status,
/// otherwise this delegates to [`merge_status_cell`].
pub fn merge_status_cell_for_branch(
    status: &MergeStatus,
    is_base: bool,
    ctx: &CellContext,
) -> Cell<'static> {
    if is_base {
        Cell::from("")
    } else {
        merge_status_cell(status, ctx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbols::SymbolSet;
    use crate::theme::Theme;

    fn theme_and_symbols() -> (Theme, SymbolSet) {
        (Theme::dark(), SymbolSet::ascii())
    }

    // --- merge_status_parts: symbol-only < 70, full text >= 70 ---

    #[test]
    fn merge_status_narrow_is_symbol_only() {
        let (theme, symbols) = theme_and_symbols();
        let ctx = CellContext {
            theme: &theme,
            symbols: &symbols,
            area_width: 60,
            compact: true,
        };
        assert_eq!(merge_status_parts(&MergeStatus::Merged, &ctx).0, "+");
        assert_eq!(merge_status_parts(&MergeStatus::SquashMerged, &ctx).0, "~");
        assert_eq!(merge_status_parts(&MergeStatus::Unmerged, &ctx).0, "-");
        assert_eq!(
            merge_status_parts(&MergeStatus::Pending, &ctx).0,
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
        };
        assert_eq!(merge_status_parts(&MergeStatus::Merged, &ctx).0, "merged +");
        assert_eq!(
            merge_status_parts(&MergeStatus::SquashMerged, &ctx).0,
            "squash-merged ~"
        );
        assert_eq!(
            merge_status_parts(&MergeStatus::Unmerged, &ctx).0,
            "unmerged -"
        );
        assert_eq!(
            merge_status_parts(&MergeStatus::Pending, &ctx).0,
            "pending \u{2026}"
        );
    }

    #[test]
    fn merge_status_boundary_70_is_wide_69_is_narrow() {
        let (theme, symbols) = theme_and_symbols();
        let wide = CellContext {
            theme: &theme,
            symbols: &symbols,
            area_width: 70,
            compact: false,
        };
        let narrow = CellContext {
            theme: &theme,
            symbols: &symbols,
            area_width: 69,
            compact: false,
        };
        assert_eq!(
            merge_status_parts(&MergeStatus::Merged, &wide).0,
            "merged +"
        );
        assert_eq!(merge_status_parts(&MergeStatus::Merged, &narrow).0, "+");
    }

    // --- merge_status_cell_for_branch: base is blank ---

    #[test]
    fn branch_base_status_is_blank() {
        let (theme, symbols) = theme_and_symbols();
        let ctx = CellContext {
            theme: &theme,
            symbols: &symbols,
            area_width: 80,
            compact: false,
        };
        assert_eq!(
            merge_status_cell_for_branch(&MergeStatus::Merged, true, &ctx),
            Cell::from("")
        );
        assert_ne!(
            merge_status_cell_for_branch(&MergeStatus::Merged, false, &ctx),
            Cell::from("")
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
        };
        let (text, style) = pr_parts(None, &ctx);
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
        };
        for (status, expected) in [
            (PrStatus::Draft, theme.pr_draft),
            (PrStatus::Open, theme.pr_open),
            (PrStatus::Merged, theme.pr_merged),
            (PrStatus::Closed, theme.pr_closed),
        ] {
            let pr = PrInfo { number: 42, status };
            let (text, style) = pr_parts(Some(&pr), &ctx);
            assert_eq!(text, "#42");
            assert_eq!(style, expected);
        }
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
        };
        let spans = ahead_behind_spans(Some(1), Some(2), &ctx);
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].content.as_ref(), "+1");
        assert_eq!(spans[1].content.as_ref(), " ");
        assert_eq!(spans[2].content.as_ref(), "-2");
    }
}
