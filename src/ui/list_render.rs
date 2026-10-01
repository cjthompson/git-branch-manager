use ratatui::prelude::*;
use ratatui::text::Line;
use ratatui::widgets::{
    Block, Borders, Cell, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState, Table,
};

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
    /// Resolved render width of the first data column, in cells.
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
    pub horizontal_scrolling: bool,
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

/// Return the indices of columns that remain visible at the given terminal
/// width. Kept separate so the final narrow-width fallback can be tested
/// against a real view's column definitions.
fn visible_column_indices<T: ViewItem>(columns: &[ColumnDef<T>], area_width: u16) -> Vec<usize> {
    columns
        .iter()
        .enumerate()
        .filter(|(_, col)| {
            col.hide_below_width
                .is_none_or(|threshold| area_width >= threshold)
        })
        .map(|(i, _)| i)
        .collect()
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
    let horizontal_scrolling = params.horizontal_scrolling;
    let columns = params.columns;
    let state = &mut *params.state;
    let display_indices = state.display_indices().to_vec();
    let theme = params.theme;
    let symbols = params.symbols;

    // Determine which columns are visible at this width
    let visible_col_indices = if horizontal_scrolling {
        (0..columns.len()).collect()
    } else {
        visible_column_indices(columns, area.width)
    };

    let visible_columns: Vec<&ColumnDef<T>> =
        visible_col_indices.iter().map(|&i| &columns[i]).collect();

    // Sort-direction arrow appended to the active sort column's header label.
    let sort_arrow = if state.sort_ascending() {
        "\u{25b2}"
    } else {
        "\u{25bc}"
    };

    // Build column widths: checkbox + visible columns. Branch (and the
    // Worktrees Branch column) stretches by name, so a preceding fixed
    // indicator such as Branches Up does not change which column grows.
    let highlight_width = symbols.cursor_prefix.len() as u16 + 1;
    let wide_data_width: u32 = visible_columns
        .iter()
        .map(|col| col.wide_width.unwrap_or(col.min_width) as u32)
        .sum();
    let minimum_full_width =
        2u32 + highlight_width as u32 + 3 + wide_data_width + visible_columns.len() as u32;
    let logical_width = if horizontal_scrolling {
        (area.width as u32)
            .max(120)
            .max(minimum_full_width)
            .min(u16::MAX as u32) as u16
    } else {
        area.width
    };
    let compact = !horizontal_scrolling && width < 120;
    let table_width = logical_width.saturating_sub(2);
    let [_highlight_area, columns_area] =
        Layout::horizontal([Constraint::Length(highlight_width), Constraint::Fill(0)])
            .areas(Rect::new(0, 0, table_width, 1));

    // The stretchy column is identified by name rather than position:
    // "Branch" (Branches and Worktrees), "Name" (Remotes/Tags), and "Path"
    // (Worktrees). Matching by name means a fixed indicator can move ahead of
    // Branch without changing which column claims the priority width.
    let is_stretchy =
        |col: &ColumnDef<T>| -> bool { matches!(col.name, "Branch" | "Name" | "Path") };

    // Give the stretchy column priority via a staged compaction ladder
    // (BL-022 stage 1) instead of flipping every fixed column between wide
    // and compact at once: demote one priority tier at a time (Age, then
    // Merge, then A/B+PR together with everything else) until the stretchy
    // column gets at least as much space as it needs and at least as much
    // as the fixed columns combined. See `resolve_ladder_level` /
    // `demoted_at_level` above for the algorithm and the rationale for using
    // direct arithmetic instead of resolving a trial `Layout`.
    let effective_min_widths: Vec<u16> = visible_columns
        .iter()
        .map(|col| {
            display_indices
                .iter()
                .map(|&raw_idx| col.min_width_for(&state.items()[raw_idx]))
                .max()
                .unwrap_or(col.min_width)
        })
        .collect();
    let ladder_columns: Vec<LadderColumn> = visible_columns
        .iter()
        .zip(&effective_min_widths)
        .map(|(col, &min_width)| LadderColumn {
            key: col.key,
            min_width,
            wide_width: col.wide_width,
            is_stretchy: is_stretchy(col),
        })
        .collect();
    let available = columns_area.width as u32;
    let level = if horizontal_scrolling {
        0
    } else {
        resolve_ladder_level(&ladder_columns, area.width, available)
    };

    let mut widths: Vec<Constraint> = vec![Constraint::Length(3)]; // checkbox
    for (col, &min_width) in visible_columns.iter().zip(&effective_min_widths) {
        let col_width = if demoted_at_level(col.key, level) {
            min_width
        } else {
            col.wide_width.unwrap_or(min_width)
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
        let label = if !col.show_header {
            String::new()
        } else if state.sort_column() == Some(col_idx) && col.compare.is_some() {
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
        area_width: logical_width,
        compact,
        data_col_widths,
        first_col_width,
    };

    let viewport_width = area.width.saturating_sub(2);
    let logical_viewport_width = logical_width.saturating_sub(2);
    let max_horizontal_offset = if horizontal_scrolling {
        logical_viewport_width.saturating_sub(viewport_width) as usize
    } else {
        0
    };
    state.set_horizontal_scroll_bounds(max_horizontal_offset);
    let overflow = max_horizontal_offset > 0;

    // Build rows from display indices
    if display_indices.is_empty() && state.loading {
        state.header_columns.clear();
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

    let highlight_sym = format!("{} ", symbols.cursor_prefix);

    // Render the border on the real viewport. When horizontal scrolling is
    // needed, render the table into a full-width buffer and copy the visible
    // slice into the viewport inside this border.
    let inner_area = block.inner(area);
    frame.render_widget(block.clone(), area);

    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(theme.cursor)
        .highlight_symbol(highlight_sym)
        .highlight_spacing(ratatui::widgets::HighlightSpacing::Always);

    let reserve_scrollbar = overflow && inner_area.height >= 2 && inner_area.width > 0;
    let table_height = inner_area
        .height
        .saturating_sub(u16::from(reserve_scrollbar));

    if overflow {
        let virtual_area = Rect::new(0, 0, logical_width, area.height);
        let virtual_inner = block.inner(virtual_area);
        let virtual_table_area = Rect::new(
            virtual_inner.x,
            virtual_inner.y,
            virtual_inner.width,
            table_height,
        );
        let mut virtual_buffer = ratatui::buffer::Buffer::empty(virtual_area);
        ratatui::widgets::StatefulWidget::render(
            table,
            virtual_table_area,
            &mut virtual_buffer,
            state.table_state_mut(),
        );

        let source_x = virtual_inner
            .x
            .saturating_add(state.horizontal_offset().min(u16::MAX as usize) as u16);
        for row in 0..table_height {
            let source_y = virtual_table_area.y + row;
            let target_y = inner_area.y + row;
            for column in 0..inner_area.width {
                let from_x = source_x + column;
                let to_x = inner_area.x + column;
                frame
                    .buffer_mut()
                    .cell_mut((to_x, target_y))
                    .expect("list viewport cell is in the frame")
                    .clone_from(
                        virtual_buffer
                            .cell((from_x, source_y))
                            .expect("scrolled table cell is in its virtual buffer"),
                    );
            }
        }

        if reserve_scrollbar {
            let scrollbar_area = Rect::new(
                inner_area.x,
                inner_area.y + inner_area.height - 1,
                inner_area.width,
                1,
            );
            let scrollbar = Scrollbar::new(ScrollbarOrientation::HorizontalBottom)
                .track_symbol(Some("─"))
                .thumb_symbol("━")
                .begin_symbol(Some("◂"))
                .end_symbol(Some("▸"))
                .track_style(theme.dim)
                .thumb_style(theme.title);
            let mut scrollbar_state = ScrollbarState::new(logical_viewport_width as usize)
                .position(state.horizontal_offset())
                .viewport_content_length(inner_area.width as usize);
            frame.render_stateful_widget(scrollbar, scrollbar_area, &mut scrollbar_state);
        }
    } else {
        frame.render_stateful_widget(
            table,
            Rect::new(inner_area.x, inner_area.y, inner_area.width, table_height),
            state.table_state_mut(),
        );
    }

    // Keep exact bounded sort hit regions. Clipping the original cell ranges
    // against the rendered viewport makes sorting work after a horizontal
    // scroll even when non-sortable or hidden columns precede a header.
    let virtual_inner = block.inner(Rect::new(0, 0, logical_width, area.height));
    let source_view_start = virtual_inner
        .x
        .saturating_add(state.horizontal_offset().min(u16::MAX as usize) as u16);
    let source_view_end = source_view_start.saturating_add(inner_area.width);
    let mut sort_col_map: Vec<Option<usize>> = vec![None]; // checkbox
    sort_col_map.extend(
        visible_col_indices
            .iter()
            .map(|&col_idx| columns[col_idx].compare.is_some().then_some(col_idx)),
    );
    state.header_columns = resolved
        .iter()
        .enumerate()
        .filter_map(|(i, rect)| {
            let sort_idx = sort_col_map.get(i).copied().flatten()?;
            let source_start = virtual_inner.x.saturating_add(rect.x);
            let source_end = source_start.saturating_add(rect.width);
            let clipped_start = source_start.max(source_view_start);
            let clipped_end = source_end.min(source_view_end);
            (clipped_start < clipped_end).then(|| {
                let screen_start = inner_area.x + clipped_start - source_view_start;
                let screen_end = screen_start + clipped_end - clipped_start;
                (screen_start, screen_end, sort_idx)
            })
        })
        .collect();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::branches::BranchesViewDef;
    use crate::types::{BranchInfo, MergeStatus, TrackingStatus};
    use chrono::Utc;
    use ratatui::backend::TestBackend;

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
    // Up (compact unless a gone upstream needs four cells), Branch (stretchy),
    // A/B, PR, Age, and Merge.
    fn branches_like_columns() -> Vec<LadderColumn> {
        vec![
            col("remote", 2, None, false),          // Up indicator
            col("name", 15, None, true),            // Branch (stretchy)
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

    fn scroll_test_branch(name: &str) -> BranchInfo {
        BranchInfo {
            name: name.to_string(),
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
        }
    }

    fn scroll_test_columns() -> Vec<ColumnDef<BranchInfo>> {
        vec![
            ColumnDef {
                key: "name",
                name: "Name",
                show_header: true,
                min_width: 12,
                content_min_width: None,
                wide_width: None,
                hide_below_width: None,
                compare: Some(|a, b| a.name.cmp(&b.name)),
            },
            ColumnDef {
                key: "extra",
                name: "Extra",
                show_header: true,
                min_width: 10,
                content_min_width: None,
                wide_width: Some(12),
                hide_below_width: Some(80),
                compare: None,
            },
            ColumnDef {
                key: "late",
                name: "Late",
                show_header: true,
                min_width: 10,
                content_min_width: None,
                wide_width: Some(12),
                hide_below_width: Some(100),
                compare: Some(|a, b| a.name.cmp(&b.name)),
            },
        ]
    }

    fn scroll_test_row(
        _item: &BranchInfo,
        _raw_index: usize,
        _is_selected: bool,
        _is_cursor_row: bool,
        visible_columns: &[usize],
        _context: &CellContext,
    ) -> Vec<Line<'static>> {
        visible_columns
            .iter()
            .map(|index| Line::from(format!("value-{index}")))
            .collect()
    }

    fn scroll_test_output(terminal: &Terminal<TestBackend>) -> String {
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn horizontal_scrolling_reveals_all_columns_and_preserves_row_styles() {
        let theme = Theme::dark();
        let symbols = SymbolSet::ascii();
        let columns = scroll_test_columns();
        let mut state = ListState::new(vec![
            scroll_test_branch("cursor"),
            scroll_test_branch("selected"),
        ]);
        state.selected_mut()[1] = true;
        let mut terminal = Terminal::new(TestBackend::new(48, 8)).unwrap();

        terminal
            .draw(|frame| {
                let mut params = ListRenderParams {
                    state: &mut state,
                    columns: &columns,
                    active_view: ViewId::Branches,
                    render_row: scroll_test_row,
                    theme: &theme,
                    symbols: &symbols,
                    horizontal_scrolling: true,
                };
                let area = frame.area();
                render_list_view(frame, area, &mut params);
            })
            .unwrap();
        assert!(state.max_horizontal_offset() > 0);

        let max_offset = state.max_horizontal_offset();
        state.set_horizontal_scroll_bounds(max_offset);
        for _ in 0..max_offset {
            state.scroll_right();
        }
        terminal
            .draw(|frame| {
                let mut params = ListRenderParams {
                    state: &mut state,
                    columns: &columns,
                    active_view: ViewId::Branches,
                    render_row: scroll_test_row,
                    theme: &theme,
                    symbols: &symbols,
                    horizontal_scrolling: true,
                };
                let area = frame.area();
                render_list_view(frame, area, &mut params);
            })
            .unwrap();

        let output = scroll_test_output(&terminal);
        assert!(
            output.contains("Late"),
            "right header was not revealed: {output}"
        );
        assert!(
            output.contains("value-2"),
            "right cell was not revealed: {output}"
        );
        assert!(
            output.contains('━'),
            "horizontal scrollbar is missing: {output}"
        );
        assert!(
            terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .any(|cell| theme.checked_row.bg.is_some_and(|bg| cell.bg == bg)),
            "scrolling should preserve the checked-row styling"
        );
    }

    #[test]
    fn horizontal_scrolling_disabled_keeps_responsive_column_hiding() {
        let theme = Theme::dark();
        let symbols = SymbolSet::ascii();
        let columns = scroll_test_columns();
        let mut state = ListState::new(vec![scroll_test_branch("feature/x")]);
        let mut terminal = Terminal::new(TestBackend::new(48, 8)).unwrap();

        terminal
            .draw(|frame| {
                let mut params = ListRenderParams {
                    state: &mut state,
                    columns: &columns,
                    active_view: ViewId::Branches,
                    render_row: scroll_test_row,
                    theme: &theme,
                    symbols: &symbols,
                    horizontal_scrolling: false,
                };
                let area = frame.area();
                render_list_view(frame, area, &mut params);
            })
            .unwrap();

        let output = scroll_test_output(&terminal);
        assert!(
            !output.contains("Extra"),
            "hidden column appeared: {output}"
        );
        assert!(!output.contains("Late"), "hidden column appeared: {output}");
        assert_eq!(state.max_horizontal_offset(), 0);
    }

    #[test]
    fn flat_width_below_70_forces_fully_compact_even_with_room() {
        let columns = branches_like_columns();
        let level = resolve_ladder_level(&columns, 69, 500);
        assert_eq!(level, FULLY_COMPACT_LEVEL);
    }

    #[test]
    fn age_is_hidden_only_after_the_final_compact_width_rung() {
        let columns = BranchesViewDef.columns();

        assert_eq!(visible_column_indices(&columns, 60), vec![0, 2, 3, 4, 5]);
        assert_eq!(visible_column_indices(&columns, 59), vec![0, 2, 3, 5]);
    }
}
