use ratatui::text::Line;

use crate::symbols::SymbolSet;
use crate::theme::Theme;
use crate::view::column::ColumnDef;
use crate::view::list_state::ListState;
use crate::view::ViewId;
use crate::view::ViewItem;

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
