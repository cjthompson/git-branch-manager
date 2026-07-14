use ratatui::prelude::*;
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};

const VERSION: &str = env!("CARGO_PKG_VERSION");

use crate::symbols::SymbolSet;
use crate::theme::Theme;
use crate::view::column::ColumnDef;
use crate::view::list_state::ListState;
use crate::view::ViewId;
use crate::view::ViewItem;

use super::tab_bar::tab_bar_line;

/// Context passed to per-view row rendering callbacks.
pub struct CellContext<'a> {
    pub theme: &'a Theme,
    pub symbols: &'a SymbolSet,
    pub area_width: u16,
    pub compact: bool,
    /// Resolved render widths for visible data columns, in the same order as
    /// the `visible_col_indices` passed to row renderers.
    pub data_col_widths: Vec<u16>,
    /// Resolved render width of the first (stretchy) data column, in cells.
    /// Used by views that fit content to the first column (e.g. worktree paths).
    pub first_col_width: u16,
}

/// Type alias for the row-rendering callback function pointer.
///
/// Parameters: (item, raw_index, is_selected, is_cursor_row, visible_col_indices, context)
/// Returns: Vec of Lines for the data columns (checkbox is handled automatically).
pub type RowRenderer<T> = fn(&T, usize, bool, bool, &[usize], &CellContext) -> Vec<Line<'static>>;

/// Bundles all parameters needed for generic list rendering.
pub struct ListRenderParams<'a, T: ViewItem> {
    pub state: &'a mut ListState<T>,
    pub columns: &'a [ColumnDef<T>],
    pub active_view: ViewId,
    pub render_row: RowRenderer<T>,
    pub theme: &'a Theme,
    pub symbols: &'a SymbolSet,
}

/// One column's sizing inputs for the responsive compaction-ladder decision
/// (BL-022 stage 1). Deliberately independent of `ColumnDef<T>`'s `T: ViewItem`
/// generic so the ladder algorithm — and its tests — don't need a concrete
/// row type.
struct LadderColumn {
    key: &'static str,
    min_width: u16,
    wide_width: Option<u16>,
    is_stretchy: bool,
}

impl LadderColumn {
    fn wide_or_min(&self) -> u16 {
        self.wide_width.unwrap_or(self.min_width)
    }
}

/// The ladder's last rung: every column compact, including ones with no
/// named tier (e.g. worktree Status, stretchy wide floors). This matches the
/// pre-ladder behavior's single "short" state, kept as the ultimate fallback
/// when even demoting every named tier doesn't leave the stretchy column
/// enough room.
const FULLY_COMPACT_LEVEL: u8 = 3;

/// Whether the column identified by `key` should render in its compact form
/// at the given ladder `level`. Follows BL-022's priority order as directed
/// for this task: Age is demoted first (level 1), then Merge (level 2), then
/// everything else together — including A/B and PR, which BL-022's own text
/// lists together as one "least important" tier — at `FULLY_COMPACT_LEVEL`
/// (level 3).
fn demoted_at_level(key: &str, level: u8) -> bool {
    match level {
        0 => false,
        1 => key == "age",
        2 => matches!(key, "age" | "merge"),
        _ => true,
    }
}

/// Resolve the compaction ladder level (`0..=FULLY_COMPACT_LEVEL`) for a set
/// of visible columns at a given width.
///
/// Walks the ladder from level 0 (nothing compact) upward, stopping at the
/// first level where the stretchy (Branch/Path) column gets at least its own
/// wide floor and at least as much space as the fixed columns combined —
/// the same ">=" room check the old binary toggle used, now re-evaluated one
/// level at a time instead of once. `area_width` is the raw outer terminal
/// width (for the flat `< 70` floor, same as before); `available` is the row
/// width left after the border/highlight-symbol columns are removed (i.e.
/// `columns_area.width`), used for the arithmetic itself.
fn resolve_ladder_level(columns: &[LadderColumn], area_width: u16, available: u32) -> u8 {
    let gaps = columns.len() as u32; // N+1 segments (checkbox + N columns) -> N gaps
    let mut level = 0u8;
    while level < FULLY_COMPACT_LEVEL {
        let (mut stretchy_wide_floor, mut fixed_total) = (0u32, 0u32);
        for col in columns {
            let w = if demoted_at_level(col.key, level) {
                col.min_width
            } else {
                col.wide_or_min()
            } as u32;
            if col.is_stretchy {
                stretchy_wide_floor += w;
            } else {
                fixed_total += w;
            }
        }
        let stretchy_actual = available.saturating_sub(3 + gaps + fixed_total); // 3 = checkbox
        let room_ok = stretchy_actual >= stretchy_wide_floor && stretchy_actual >= fixed_total;
        if room_ok && area_width >= 70 {
            break;
        }
        level += 1;
    }
    level
}

/// Decide whether a header label should render right-aligned, given whether
/// right alignment is wanted at all (Age / the last column, to match their
/// data cells) and the column's actual resolved width.
///
/// Right alignment truncates overflow by dropping *leading* characters
/// (ratatui preserves the tail), which reads as garbage — e.g. "Merge"
/// becomes "erge" — when a narrow terminal forces the column below its
/// label's width. Falling back to left alignment in that case truncates the
/// tail instead, producing a legible partial word (e.g. "Merg").
fn header_alignment(wants_right: bool, label_len: usize, resolved_width: u16) -> Alignment {
    if wants_right && resolved_width as usize >= label_len {
        Alignment::Right
    } else {
        Alignment::Left
    }
}

/// Renders any list view generically.
///
/// The checkbox cell is automatically prepended; the `render_row` callback should
/// NOT include it. It receives visible column indices so it knows which cells to produce.
#[allow(clippy::too_many_arguments)]
pub fn render_list_view<T: ViewItem>(
    frame: &mut Frame,
    area: Rect,
    params: &mut ListRenderParams<T>,
) {
    let width = area.width as usize;
    let compact = width < 120;
    let columns = params.columns;
    let state = &mut *params.state;
    let theme = params.theme;
    let symbols = params.symbols;

    // Determine which columns are visible at this width
    let visible_col_indices: Vec<usize> = columns
        .iter()
        .enumerate()
        .filter(|(_, col)| {
            col.hide_below_width
                .is_none_or(|threshold| area.width >= threshold)
        })
        .map(|(i, _)| i)
        .collect();

    let visible_columns: Vec<&ColumnDef<T>> =
        visible_col_indices.iter().map(|&i| &columns[i]).collect();

    // Sort-direction arrow appended to the active sort column's header label.
    let sort_arrow = if state.sort_ascending() {
        "\u{25b2}"
    } else {
        "\u{25bc}"
    };

    // Build column widths: checkbox + visible columns.
    // Branches/remotes/tags keep the original shape: first data column stretches,
    // later columns are fixed. Worktrees also lets Branch stretch because Path
    // would otherwise take all spare width and squeeze branch names.
    let highlight_width = symbols.cursor_prefix.len() as u16 + 1;
    let table_width = area.width.saturating_sub(2);
    let [_highlight_area, columns_area] =
        Layout::horizontal([Constraint::Length(highlight_width), Constraint::Fill(0)])
            .areas(Rect::new(0, 0, table_width, 1));

    // The stretchy (first, growable) column, identified by name rather than
    // position: "Branch" (Branches' first column; also Worktrees' second
    // stretchy column), "Name" (Remotes'/Tags' first column), and "Path"
    // (Worktrees' first column). Matching by name — not column index — means
    // moving another column ahead of the stretchy column won't silently
    // change which column claims the priority width.
    let is_stretchy = |col: &ColumnDef<T>| -> bool {
        matches!(col.name, "Branch" | "Name" | "Path")
    };

    // Give the stretchy column priority via a staged compaction ladder
    // (BL-022 stage 1) instead of flipping every fixed column between wide
    // and compact at once: demote one priority tier at a time (Age, then
    // Merge, then A/B+PR together with everything else) until the stretchy
    // column gets at least as much space as it needs and at least as much
    // as the fixed columns combined. See `resolve_ladder_level` /
    // `demoted_at_level` above for the algorithm and the rationale for using
    // direct arithmetic instead of resolving a trial `Layout`.
    let ladder_columns: Vec<LadderColumn> = visible_columns
        .iter()
        .enumerate()
        .map(|(_i, col)| LadderColumn {
            key: col.key,
            min_width: col.min_width,
            wide_width: col.wide_width,
            is_stretchy: is_stretchy(col),
        })
        .collect();
    let available = columns_area.width as u32;
    let level = resolve_ladder_level(&ladder_columns, area.width, available);

    let mut widths: Vec<Constraint> = vec![Constraint::Length(3)]; // checkbox
    for (_i, col) in visible_columns.iter().enumerate() {
        let col_width = if demoted_at_level(col.key, level) {
            col.min_width
        } else {
            col.wide_width.unwrap_or(col.min_width)
        };
        if is_stretchy(col) {
            widths.push(Constraint::Min(col_width));
        } else {
            widths.push(Constraint::Length(col_width));
        }
    }

    // Resolve the constraint widths once so row renderers can fit text to real
    // cells. This mirrors ratatui Table's width calculation: the table first
    // reserves highlight-symbol space, then applies column spacing.
    let resolved = Layout::horizontal(&widths).spacing(1).split(columns_area);
    // resolved[0] is the checkbox; resolved[1] is the first data column.
    let data_col_widths: Vec<u16> = resolved.iter().skip(1).map(|r| r.width).collect();
    let first_col_width = data_col_widths.first().copied().unwrap_or(0);

    // Build header row. Computed after `data_col_widths` (rather than up front)
    // so alignment can check each column's actual resolved width.
    let mut header_cells: Vec<Cell> = vec![Cell::from("")]; // checkbox header (empty)

    for (pos, &col_idx) in visible_col_indices.iter().enumerate() {
        let col = &columns[col_idx];
        let label = if state.sort_column() == Some(col_idx) && col.compare.is_some() {
            format!("{}{}", col.name, sort_arrow)
        } else {
            col.name.to_string()
        };
        let wants_right = col.name == "Age" || col_idx == columns.len() - 1;
        let resolved_width = data_col_widths.get(pos).copied().unwrap_or(0);
        let cell = if header_alignment(wants_right, label.chars().count(), resolved_width)
            == Alignment::Right
        {
            Cell::from(Line::from(label).alignment(Alignment::Right))
        } else {
            Cell::from(label)
        };
        header_cells.push(cell.style(theme.header));
    }

    let header = Row::new(header_cells).height(1);

    let ctx = CellContext {
        theme,
        symbols,
        area_width: area.width,
        compact,
        data_col_widths,
        first_col_width,
    };

    // Build rows from display indices
    let display_indices: Vec<usize> = state.display_indices().to_vec();

    if display_indices.is_empty() && state.loading {
        let tab_title = tab_bar_line(params.active_view, theme);
        let block = Block::default()
            .title(tab_title)
            .title_top(Line::from(format!(" v{VERSION} ")).right_aligned())
            .borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let loading =
            Paragraph::new(Span::styled("Loading...", theme.dim)).alignment(Alignment::Left);
        frame.render_widget(loading, inner);
        return;
    }

    let rows: Vec<Row> = display_indices
        .iter()
        .enumerate()
        .map(|(display_pos, &raw_idx)| {
            let item = &state.items()[raw_idx];
            let is_selected = state.selected()[raw_idx];
            let is_cursor = state.table_state().selected() == Some(display_pos);
            let is_pinned = item.is_pinned();

            // Build checkbox cell
            let (checkbox_text, checkbox_style) = if is_pinned {
                ("   ".to_string(), Style::default())
            } else if is_selected {
                (symbols.checkbox_on.to_string(), theme.selected)
            } else {
                (symbols.checkbox_off.to_string(), theme.secondary_text)
            };
            let checkbox_cell = Cell::from(Span::styled(checkbox_text, checkbox_style));

            // Get view-specific cells
            let mut cells = vec![checkbox_cell];
            cells.extend(
                (params.render_row)(
                    item,
                    raw_idx,
                    is_selected,
                    is_cursor,
                    &visible_col_indices,
                    &ctx,
                )
                .into_iter()
                .map(Cell::from),
            );

            if is_selected {
                Row::new(cells).style(theme.checked_row)
            } else {
                Row::new(cells)
            }
        })
        .collect();

    // Build block with tab bar title
    let tab_title = tab_bar_line(params.active_view, theme);
    let block = Block::default()
            .title(tab_title)
            .title_top(Line::from(format!(" v{VERSION} ")).right_aligned())
            .borders(Borders::ALL);

    // Store header column positions for mouse click sorting
    {
        let x = area.x + 1 + highlight_width; // +1 for left border

        // Build sort column map: checkbox=skip, then visible columns -> sort indices
        let mut sort_col_map: Vec<Option<usize>> = vec![None]; // checkbox
        for &col_idx in &visible_col_indices {
            if columns[col_idx].compare.is_some() {
                sort_col_map.push(Some(col_idx));
            } else {
                sort_col_map.push(None);
            }
        }

        // `resolved` (the per-column rects) was computed above when building ctx.
        let mut col_positions: Vec<(u16, usize)> = Vec::new();
        for (i, rect) in resolved.iter().enumerate() {
            if let Some(&Some(sort_idx)) = sort_col_map.get(i) {
                col_positions.push((x + rect.x, sort_idx));
            }
        }

        state.header_columns = col_positions;
    }

    let highlight_sym = format!("{} ", symbols.cursor_prefix);

    // Render
    let inner_area = block.inner(area);
    frame.render_widget(block, area);

    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(theme.cursor)
        .highlight_symbol(highlight_sym)
        .highlight_spacing(ratatui::widgets::HighlightSpacing::Always);

    frame.render_stateful_widget(table, inner_area, state.table_state_mut());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(
        key: &'static str,
        min_width: u16,
        wide_width: Option<u16>,
        is_stretchy: bool,
    ) -> LadderColumn {
        LadderColumn {
            key,
            min_width,
            wide_width,
            is_stretchy,
        }
    }

    // A Branches-shaped column set mirroring `view::branches::BranchesViewDef`:
    // Branch (stretchy) + Up (no compact form) + A/B + PR + Age + Merge.
    fn branches_like_columns() -> Vec<LadderColumn> {
        vec![
            col("name", 15, None, true),           // Branch (stretchy)
            col("remote", 4, None, false),          // Up (no wide form)
            col("ahead_behind", 3, Some(8), false), // A/B
            col("pr", 2, Some(9), false),           // PR
            col("age", 5, Some(14), false),         // Age
            col("merge", 5, Some(16), false),       // Merge
        ]
    }

    #[test]
    fn demoted_at_level_matches_bl_022_priority_order() {
        assert!(!demoted_at_level("age", 0));
        assert!(!demoted_at_level("merge", 0));
        assert!(!demoted_at_level("ahead_behind", 0));
        assert!(!demoted_at_level("pr", 0));

        assert!(demoted_at_level("age", 1));
        assert!(!demoted_at_level("merge", 1));
        assert!(!demoted_at_level("ahead_behind", 1));
        assert!(!demoted_at_level("pr", 1));

        assert!(demoted_at_level("age", 2));
        assert!(demoted_at_level("merge", 2));
        assert!(!demoted_at_level("ahead_behind", 2));
        assert!(!demoted_at_level("pr", 2));

        // A/B and PR are demoted together, only at the final level.
        assert!(demoted_at_level("age", 3));
        assert!(demoted_at_level("merge", 3));
        assert!(demoted_at_level("ahead_behind", 3));
        assert!(demoted_at_level("pr", 3));
        // FULLY_COMPACT_LEVEL also demotes columns with no named tier.
        assert!(demoted_at_level("remote", 3));
        assert!(demoted_at_level("status", 3));
    }

    #[test]
    fn stretchy_column_not_demoted_until_final_level() {
        for level in 0..FULLY_COMPACT_LEVEL {
            assert!(!demoted_at_level("name", level));
        }
        assert!(demoted_at_level("name", FULLY_COMPACT_LEVEL));
    }

    #[test]
    fn wide_terminal_keeps_everything_wide() {
        let columns = branches_like_columns();
        let level = resolve_ladder_level(&columns, 160, 160);
        assert_eq!(level, 0);
    }

    #[test]
    fn narrow_width_only_age_compacts() {
        let columns = branches_like_columns();
        let level = resolve_ladder_level(&columns, 100, 100);
        assert_eq!(level, 1);
    }

    #[test]
    fn header_right_aligns_when_label_fits() {
        assert_eq!(header_alignment(true, 5, 5), Alignment::Right);
        assert_eq!(header_alignment(true, 5, 16), Alignment::Right);
    }

    #[test]
    fn header_falls_back_to_left_when_too_narrow_for_label() {
        // Regression: at resolved width 4, right-aligning "Merge" (len 5)
        // used to drop the leading "M", rendering "erge".
        assert_eq!(header_alignment(true, 5, 4), Alignment::Left);
        assert_eq!(header_alignment(true, 5, 0), Alignment::Left);
    }

    #[test]
    fn header_never_right_aligns_when_not_wanted() {
        assert_eq!(header_alignment(false, 5, 16), Alignment::Left);
    }

    #[test]
    fn narrower_width_age_and_merge_compact() {
        let columns = branches_like_columns();
        let level = resolve_ladder_level(&columns, 80, 80);
        assert_eq!(level, 2);
    }

    #[test]
    fn very_narrow_width_compacts_age_merge_and_ab_pr() {
        let columns = branches_like_columns();
        let level = resolve_ladder_level(&columns, 100, 50);
        assert_eq!(level, FULLY_COMPACT_LEVEL);
    }

    #[test]
    fn flat_width_below_70_forces_fully_compact_even_with_room() {
        let columns = branches_like_columns();
        let level = resolve_ladder_level(&columns, 69, 500);
        assert_eq!(level, FULLY_COMPACT_LEVEL);
    }
}
