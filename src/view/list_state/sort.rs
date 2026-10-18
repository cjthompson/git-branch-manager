use super::ListState;
use crate::view::column::ColumnDef;
use crate::view::ViewItem;

// --- Sorting free functions ---

/// Sort a slice of items with pinned items always at the top (base first).
/// Non-pinned items are sorted by the provided compare function.
pub fn sort_items<T: ViewItem>(
    items: &mut [T],
    compare: fn(&T, &T) -> std::cmp::Ordering,
    ascending: bool,
) {
    // Stable-partition: move all pinned items to the front (base first),
    // then sort only the non-pinned tail.
    items.sort_by(|a, b| {
        match (a.is_pinned(), b.is_pinned()) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            (true, true) => {
                // Base branch always comes first among pinned items
                b.is_base().cmp(&a.is_base())
            }
            (false, false) => {
                let ord = compare(a, b);
                if ascending {
                    ord
                } else {
                    ord.reverse()
                }
            }
        }
    });
}

/// Apply sort based on current sort_column and sort_ascending.
/// Pinned items always stay at the top, only non-pinned items are sorted.
pub fn apply_sort<T: ViewItem>(state: &mut ListState<T>, columns: &[ColumnDef<T>]) {
    let (cursor_id, cursor_display_position, selected_ids) = state.snapshot_row_state();
    if let Some(compare) = state
        .sort_column
        .and_then(|col_idx| columns.get(col_idx))
        .and_then(|column| column.compare)
    {
        sort_items(&mut state.items, compare, state.sort_ascending);
    }
    state.restore_row_state(cursor_id, cursor_display_position, selected_ids);
}

/// Cycle to next sort column (None -> first sortable -> ... -> None), skipping non-sortable columns
pub fn cycle_sort_column<T: ViewItem>(state: &mut ListState<T>, columns: &[ColumnDef<T>]) {
    let sortable: Vec<usize> = columns
        .iter()
        .enumerate()
        .filter(|(_, col)| col.compare.is_some())
        .map(|(i, _)| i)
        .collect();

    if sortable.is_empty() {
        return;
    }

    state.sort_column = match state.sort_column {
        None => Some(sortable[0]),
        Some(current) => {
            let next_pos = sortable
                .iter()
                .position(|&i| i == current)
                .map(|pos| pos + 1)
                .unwrap_or(0);
            if next_pos < sortable.len() {
                Some(sortable[next_pos])
            } else {
                None
            }
        }
    };
}

/// Toggle sort direction (no-op when no sort column is active)
pub fn toggle_sort_direction<T: ViewItem>(state: &mut ListState<T>) {
    if state.sort_column.is_some() {
        state.sort_ascending = !state.sort_ascending;
    }
}

/// Handle a header-column click: if `col` is already the active sort column,
/// toggle direction; otherwise sort ascending by `col`. Always re-applies.
pub fn sort_by_column_click<T: ViewItem>(
    state: &mut ListState<T>,
    columns: &[ColumnDef<T>],
    col: usize,
) {
    if state.sort_column() == Some(col) {
        toggle_sort_direction(state);
    } else {
        state.set_sort(Some(col), true);
    }
    apply_sort(state, columns);
}

/// Advance to the next sort column and re-apply the sort.
pub fn cycle_sort_and_apply<T: ViewItem>(state: &mut ListState<T>, columns: &[ColumnDef<T>]) {
    cycle_sort_column(state, columns);
    apply_sort(state, columns);
}

/// Toggle sort direction and re-apply the sort.
pub fn toggle_sort_direction_and_apply<T: ViewItem>(
    state: &mut ListState<T>,
    columns: &[ColumnDef<T>],
) {
    toggle_sort_direction(state);
    apply_sort(state, columns);
}

/// Collect confirm-overlay targets from a list state. Applies `mapper` to each
/// item at the selected indices (or the cursor item when nothing is checked),
/// keeping only the `Some` values in order. The returned `Vec` is owned, so the
/// borrow of `state` ends when this returns.
pub fn collect_targets<T: ViewItem>(
    state: &ListState<T>,
    mapper: impl Fn(&T) -> Option<String>,
) -> Vec<String> {
    state
        .selected_indices()
        .iter()
        .filter_map(|&i| mapper(&state.items()[i]))
        .collect()
}
