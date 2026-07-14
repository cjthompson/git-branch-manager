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

    // Build header row
    let sort_arrow = if state.sort_ascending() {
        "\u{25b2}"
    } else {
        "\u{25bc}"
    };

    let mut header_cells: Vec<Cell> = vec![Cell::from("")]; // checkbox header (empty)

    for &col_idx in &visible_col_indices {
        let col = &columns[col_idx];
        let label = if state.sort_column() == Some(col_idx) && col.compare.is_some() {
            format!("{}{}", col.name, sort_arrow)
        } else {
            col.name.to_string()
        };
        // Right-align Age and last column (Status/Merge)
        let cell = if col.name == "Age" || col_idx == columns.len() - 1 {
            Cell::from(Line::from(label).alignment(Alignment::Right))
        } else {
            Cell::from(label)
        };
        header_cells.push(cell.style(theme.header));
    }

    let header = Row::new(header_cells).height(1);

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

    let build_widths = |wide: bool| -> Vec<Constraint> {
        let mut widths: Vec<Constraint> = vec![Constraint::Length(3)]; // checkbox
        for col in visible_columns.iter() {
            let col_width = if wide {
                col.wide_width.unwrap_or(col.min_width)
            } else {
                col.min_width
            };
            if is_stretchy(col) {
                widths.push(Constraint::Min(col_width));
            } else {
                widths.push(Constraint::Length(col_width));
            }
        }
        widths
    };

    // ratatui's layout solver honors `Length` constraints over `Min`, so the
    // fixed columns always render at their full requested width even when that
    // leaves the stretchy (branch/name) column a minority sliver of the row.
    // Give the stretchy column priority: if the fixed columns' wide widths would
    // claim at least as much space as the stretchy column, fall back to compact
    // (min_width) sizing for every column instead, so the stretchy column gets
    // the majority of the row and the fixed columns show their abbreviated
    // forms — triggered by actual leftover space, not just a flat terminal-width
    // threshold.
    //
    // This is computed by direct arithmetic rather than by resolving a trial
    // `Layout` and reading back its widths: when the "wide" hypothesis is
    // itself infeasible (total demand exceeds the available row width — e.g.
    // Worktrees' two stretchy columns, Path and Branch, both wanting their
    // wide floor at once), ratatui's solver has to violate constraints to
    // produce *some* answer, and those violated numbers are unreliable input
    // for this decision — they can spuriously say "wide fits" when it doesn't.
    let (stretchy_wide_floor, fixed_wide_total) = {
        let (mut stretchy, mut fixed) = (0u32, 0u32);
        for col in visible_columns.iter() {
            let w = col.wide_width.unwrap_or(col.min_width) as u32;
            if is_stretchy(col) {
                stretchy += w;
            } else {
                fixed += w;
            }
        }
        (stretchy, fixed)
    };
    let gaps = visible_columns.len() as u32; // N+1 segments (checkbox + N columns) -> N gaps
    let available = columns_area.width as u32;
    let stretchy_actual_wide = available.saturating_sub(3 + gaps + fixed_wide_total); // 3 = checkbox
    let wide_leaves_room =
        stretchy_actual_wide >= stretchy_wide_floor && stretchy_actual_wide >= fixed_wide_total;
    let short_status = area.width < 70 || !wide_leaves_room;
    let widths = build_widths(!short_status);

    // Resolve the constraint widths once so row renderers can fit text to real
    // cells. This mirrors ratatui Table's width calculation: the table first
    // reserves highlight-symbol space, then applies column spacing.
    let resolved = Layout::horizontal(&widths).spacing(1).split(columns_area);
    // resolved[0] is the checkbox; resolved[1] is the first data column.
    let data_col_widths: Vec<u16> = resolved.iter().skip(1).map(|r| r.width).collect();
    let first_col_width = data_col_widths.first().copied().unwrap_or(0);

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
