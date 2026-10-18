use super::ListState;
use crate::types::MergeStatus;
use crate::view::ViewItem;

// --- Navigation free functions ---

/// Move cursor down one row in the display list
pub fn nav_down<T: ViewItem>(state: &mut ListState<T>) {
    let len = state.display_indices.len();
    if len == 0 {
        return;
    }
    let current = state.table_state.selected().unwrap_or(0);
    let next = (current + 1).min(len - 1);
    state.table_state.select(Some(next));
    if let Some(&raw_idx) = state.display_indices.get(next) {
        state.cursor = raw_idx;
    }
}

/// Move cursor up one row in the display list
pub fn nav_up<T: ViewItem>(state: &mut ListState<T>) {
    let current = state.table_state.selected().unwrap_or(0);
    let next = current.saturating_sub(1);
    state.table_state.select(Some(next));
    if let Some(&raw_idx) = state.display_indices.get(next) {
        state.cursor = raw_idx;
    }
}

/// Move cursor down one page
pub fn nav_page_down<T: ViewItem>(state: &mut ListState<T>, page_size: usize) {
    let len = state.display_indices.len();
    if len == 0 {
        return;
    }
    let current = state.table_state.selected().unwrap_or(0);
    let next = (current + page_size).min(len - 1);
    state.table_state.select(Some(next));
    if let Some(&raw_idx) = state.display_indices.get(next) {
        state.cursor = raw_idx;
    }
}

/// Move cursor up one page
pub fn nav_page_up<T: ViewItem>(state: &mut ListState<T>, page_size: usize) {
    let current = state.table_state.selected().unwrap_or(0);
    let next = current.saturating_sub(page_size);
    state.table_state.select(Some(next));
    if let Some(&raw_idx) = state.display_indices.get(next) {
        state.cursor = raw_idx;
    }
}

/// Jump to first item
pub fn nav_home<T: ViewItem>(state: &mut ListState<T>) {
    if state.display_indices.is_empty() {
        return;
    }
    state.table_state.select(Some(0));
    if let Some(&raw_idx) = state.display_indices.first() {
        state.cursor = raw_idx;
    }
}

/// Jump to last item
pub fn nav_end<T: ViewItem>(state: &mut ListState<T>) {
    let len = state.display_indices.len();
    if len == 0 {
        return;
    }
    state.table_state.select(Some(len - 1));
    if let Some(&raw_idx) = state.display_indices.last() {
        state.cursor = raw_idx;
    }
}

// --- Selection free functions ---

/// Toggle selection on the item under the cursor
pub fn select_toggle<T: ViewItem>(state: &mut ListState<T>) {
    if let Some(idx) = state.cursor_item_index() {
        if !state.items[idx].is_pinned() {
            state.selected[idx] = !state.selected[idx];
        }
    }
}

/// Select all non-pinned visible items
pub fn select_all<T: ViewItem>(state: &mut ListState<T>) {
    for &i in &state.display_indices {
        if !state.items[i].is_pinned() {
            state.selected[i] = true;
        }
    }
}

/// Deselect all items
pub fn deselect_all<T: ViewItem>(state: &mut ListState<T>) {
    state.selected.fill(false);
}

/// Invert selection (non-pinned only)
pub fn invert_selection<T: ViewItem>(state: &mut ListState<T>) {
    for &i in &state.display_indices {
        if !state.items[i].is_pinned() {
            state.selected[i] = !state.selected[i];
        }
    }
}

/// Select all merged + squash-merged items
pub fn select_merged<T: ViewItem>(state: &mut ListState<T>) {
    deselect_all(state);
    for &i in &state.display_indices {
        if state.items[i].is_pinned() {
            continue;
        }
        if let Some(status) = state.items[i].merge_status() {
            if matches!(
                status,
                MergeStatus::Merged | MergeStatus::SquashMerged | MergeStatus::CherryPicked
            ) {
                state.selected[i] = true;
            }
        }
    }
}
