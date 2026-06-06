use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};

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
}

/// Type alias for the row-rendering callback function pointer.
///
/// Parameters: (item, raw_index, is_selected, is_cursor_row, visible_col_indices, context)
/// Returns: Vec of cells for the data columns (checkbox is handled automatically).
pub type RowRenderer<T> = fn(&T, usize, bool, bool, &[usize], &CellContext) -> Vec<Cell<'static>>;

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
    // First visible column (name) gets Min (stretchy), rest get Length (fixed).
    // This matches the original app's pattern and ensures ratatui fills all cells.
    let short_status = area.width < 70;
    let mut widths: Vec<Constraint> = vec![Constraint::Length(3)]; // checkbox
    for (i, col) in visible_columns.iter().enumerate() {
        if i == 0 {
            widths.push(Constraint::Min(col.min_width));
        } else {
            let col_width = if !short_status {
                col.wide_width.unwrap_or(col.min_width)
            } else {
                col.min_width
            };
            widths.push(Constraint::Length(col_width));
        }
    }

    let ctx = CellContext {
        theme,
        symbols,
        area_width: area.width,
        compact,
    };

    // Build rows from display indices
    let display_indices: Vec<usize> = state.display_indices().to_vec();

    if display_indices.is_empty() && state.loading {
        let tab_title = tab_bar_line(params.active_view, theme);
        let block = Block::default().title(tab_title).borders(Borders::ALL);
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
            cells.extend((params.render_row)(
                item,
                raw_idx,
                is_selected,
                is_cursor,
                &visible_col_indices,
                &ctx,
            ));

            if is_selected {
                Row::new(cells).style(theme.checked_row)
            } else {
                Row::new(cells)
            }
        })
        .collect();

    // Build block with tab bar title
    let tab_title = tab_bar_line(params.active_view, theme);
    let block = Block::default().title(tab_title).borders(Borders::ALL);

    // Store header column positions for mouse click sorting
    {
        let highlight_width = symbols.cursor_prefix.len() as u16 + 1;
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

        // Resolve constraint widths
        let available = area.width.saturating_sub(2 + highlight_width);
        let resolved = Layout::horizontal(&widths).split(Rect::new(0, 0, available, 1));

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
