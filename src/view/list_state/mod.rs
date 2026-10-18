mod nav;
mod sort;

pub use nav::*;
pub use sort::*;

use super::ViewItem;
use ratatui::widgets::TableState;
use std::collections::HashSet;

/// Generic state for any list view. Holds items, cursor, selection,
/// sort, filter, and search state. Shared by all 4 primary views.
pub struct ListState<T: ViewItem> {
    items: Vec<T>,
    selected: Vec<bool>,
    cursor: usize,
    table_state: TableState,
    sort_column: Option<usize>,
    sort_ascending: bool,
    search_query: String,
    search_active: bool,
    filter_query: String,
    horizontal_offset: usize,
    max_horizontal_offset: usize,
    /// Cached display indices (after filter + pinned-first reorder).
    /// Recalculated when items, filter, or search change.
    display_indices: Vec<usize>,
    /// Column positions for mouse click detection on headers
    pub header_columns: Vec<(u16, u16, usize)>,
    /// Status bar clickable regions: (x_start, x_end, key_code)
    pub status_bar_items: Vec<(u16, u16, crossterm::event::KeyCode)>,
    /// Loading state for lazy-loaded views
    pub loading: bool,
}

impl<T: ViewItem> ListState<T> {
    pub fn new(items: Vec<T>) -> Self {
        let len = items.len();
        let display_indices: Vec<usize> = (0..len).collect();
        let mut table_state = TableState::default();
        if len > 0 {
            table_state.select(Some(0));
        }
        // Rebuild display indices with pinned-first ordering
        let mut state = Self {
            items,
            selected: vec![false; len],
            cursor: 0,
            table_state,
            sort_column: None,
            sort_ascending: true,
            search_query: String::new(),
            search_active: false,
            filter_query: String::new(),
            horizontal_offset: 0,
            max_horizontal_offset: 0,
            display_indices,
            header_columns: Vec::new(),
            status_bar_items: Vec::new(),
            loading: false,
        };
        state.rebuild_display_indices();
        state
    }

    pub fn empty() -> Self {
        Self::new(Vec::new())
    }

    // --- Accessors ---

    pub fn items(&self) -> &[T] {
        &self.items
    }
    pub fn items_mut(&mut self) -> &mut Vec<T> {
        &mut self.items
    }
    pub fn cursor(&self) -> usize {
        self.cursor
    }
    pub fn selected(&self) -> &[bool] {
        &self.selected
    }
    pub fn selected_mut(&mut self) -> &mut Vec<bool> {
        &mut self.selected
    }
    pub fn table_state(&self) -> &TableState {
        &self.table_state
    }
    pub fn table_state_mut(&mut self) -> &mut TableState {
        &mut self.table_state
    }
    pub fn sort_column(&self) -> Option<usize> {
        self.sort_column
    }
    pub fn sort_ascending(&self) -> bool {
        self.sort_ascending
    }
    pub fn search_query(&self) -> &str {
        &self.search_query
    }
    pub fn search_active(&self) -> bool {
        self.search_active
    }
    pub fn filter_query(&self) -> &str {
        &self.filter_query
    }
    pub fn display_indices(&self) -> &[usize] {
        &self.display_indices
    }
    pub fn horizontal_offset(&self) -> usize {
        self.horizontal_offset
    }
    pub fn max_horizontal_offset(&self) -> usize {
        self.max_horizontal_offset
    }

    // --- Mutators ---

    pub fn set_items(&mut self, items: Vec<T>) {
        let (cursor_id, cursor_display_position, selected_ids) = self.snapshot_row_state();
        self.items = items;
        self.reset_horizontal_scroll();
        self.restore_row_state(cursor_id, cursor_display_position, selected_ids);
    }

    /// Update the maximum horizontal offset after resolving the current
    /// content and viewport widths. Shrinking the terminal clamps the current
    /// position immediately.
    pub fn set_horizontal_scroll_bounds(&mut self, max_offset: usize) {
        self.max_horizontal_offset = max_offset;
        self.horizontal_offset = self.horizontal_offset.min(max_offset);
    }

    pub fn scroll_left(&mut self) {
        self.horizontal_offset = self.horizontal_offset.saturating_sub(1);
    }

    pub fn scroll_right(&mut self) {
        self.horizontal_offset = self
            .horizontal_offset
            .saturating_add(1)
            .min(self.max_horizontal_offset);
    }

    pub fn reset_horizontal_scroll(&mut self) {
        self.horizontal_offset = 0;
        self.max_horizontal_offset = 0;
    }

    pub fn set_cursor(&mut self, cursor: usize) {
        self.cursor = cursor;
        // Map cursor to display position for table_state
        if let Some(pos) = self.display_indices.iter().position(|&i| i == cursor) {
            self.table_state.select(Some(pos));
        }
    }

    pub fn set_sort(&mut self, column: Option<usize>, ascending: bool) {
        self.sort_column = column;
        self.sort_ascending = ascending;
    }

    pub fn set_search_query(&mut self, query: String) {
        self.search_query = query;
        self.rebuild_display_indices();
    }

    pub fn set_search_active(&mut self, active: bool) {
        self.search_active = active;
    }

    pub fn set_filter_query(&mut self, query: String) {
        self.filter_query = query;
        self.rebuild_display_indices();
    }

    /// Rebuild display indices from current items + search + filter.
    /// Pinned items always come first.
    pub fn rebuild_display_indices(&mut self) {
        let search_lower = self.search_query.to_lowercase();
        let filter = super::filter::FilterSet::parse(&self.filter_query);

        let mut pinned = Vec::new();
        let mut non_pinned = Vec::new();

        for (i, item) in self.items.iter().enumerate() {
            // Search filter
            if !search_lower.is_empty()
                && !item.display_name().to_lowercase().contains(&search_lower)
            {
                continue;
            }

            // Token filters (only apply to non-pinned)
            if !item.is_pinned() && !filter.is_empty() && !self.matches_filter(item, &filter) {
                continue;
            }

            if item.is_pinned() {
                if item.is_base() {
                    pinned.insert(0, i); // base always first
                } else {
                    pinned.push(i);
                }
            } else {
                non_pinned.push(i);
            }
        }

        pinned.extend(non_pinned);
        self.display_indices = pinned;
    }

    fn matches_filter(&self, item: &T, filter: &super::filter::FilterSet) -> bool {
        // Status filter
        if !filter.statuses.is_empty() {
            if let Some(status) = item.merge_status() {
                if !filter.statuses.contains(status) {
                    return false;
                }
            }
        }

        // Age filters
        if let Some(newer_secs) = filter.age_newer_secs {
            let age_secs = chrono::Utc::now()
                .signed_duration_since(*item.last_commit_date())
                .num_seconds();
            if age_secs > newer_secs {
                return false;
            }
        }
        if let Some(older_secs) = filter.age_older_secs {
            let age_secs = chrono::Utc::now()
                .signed_duration_since(*item.last_commit_date())
                .num_seconds();
            if age_secs < older_secs {
                return false;
            }
        }

        // Ahead/behind filters
        if filter.sync_ahead && item.ahead().unwrap_or(0) == 0 {
            return false;
        }
        if filter.sync_behind && item.behind().unwrap_or(0) == 0 {
            return false;
        }

        // PR filters
        if filter.pr_yes && item.pr_info().is_none() {
            return false;
        }
        if filter.pr_no && item.pr_info().is_some() {
            return false;
        }

        true
    }

    /// Get the raw item index for the current cursor position
    pub fn cursor_item_index(&self) -> Option<usize> {
        let display_pos = self.table_state.selected()?;
        self.display_indices.get(display_pos).copied()
    }

    /// Get the item under the cursor
    pub fn cursor_item(&self) -> Option<&T> {
        self.cursor_item_index().and_then(|i| self.items.get(i))
    }

    /// Get indices of all selected items (or cursor item if none selected)
    pub fn selected_indices(&self) -> Vec<usize> {
        let selected: Vec<usize> = self
            .selected
            .iter()
            .enumerate()
            .filter(|(_, &s)| s)
            .map(|(i, _)| i)
            .collect();

        if selected.is_empty() {
            self.cursor_item_index().into_iter().collect()
        } else {
            selected
        }
    }

    fn snapshot_row_state(&self) -> (Option<String>, usize, HashSet<String>) {
        let cursor_id = self.cursor_item().map(ViewItem::identity);
        let cursor_display_position = self.table_state.selected().unwrap_or(0);
        let selected_ids = self
            .items
            .iter()
            .zip(&self.selected)
            .filter(|(_, selected)| **selected)
            .map(|(item, _)| item.identity())
            .collect();
        (cursor_id, cursor_display_position, selected_ids)
    }

    fn restore_row_state(
        &mut self,
        cursor_id: Option<String>,
        cursor_display_position: usize,
        selected_ids: HashSet<String>,
    ) {
        self.rebuild_display_indices();
        self.selected = self
            .items
            .iter()
            .map(|item| selected_ids.contains(&item.identity()))
            .collect();

        let cursor_raw_index = cursor_id.as_deref().and_then(|identity| {
            self.items
                .iter()
                .position(|item| item.identity() == identity)
        });
        let display_position = cursor_raw_index
            .and_then(|raw_index| {
                self.display_indices
                    .iter()
                    .position(|&index| index == raw_index)
            })
            .unwrap_or_else(|| {
                cursor_display_position.min(self.display_indices.len().saturating_sub(1))
            });

        if let Some(&raw_index) = self.display_indices.get(display_position) {
            self.cursor = raw_index;
            self.table_state.select(Some(display_position));
        } else {
            self.cursor = 0;
            self.table_state.select(None);
        }
    }
}

#[cfg(test)]
mod tests;
