use super::column::ColumnDef;
use super::ViewItem;
use crate::types::MergeStatus;
use ratatui::widgets::TableState;

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
    /// Cached display indices (after filter + pinned-first reorder).
    /// Recalculated when items, filter, or search change.
    display_indices: Vec<usize>,
    /// Column positions for mouse click detection on headers
    pub header_columns: Vec<(u16, usize)>,
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

    // --- Mutators ---

    pub fn set_items(&mut self, items: Vec<T>) {
        let len = items.len();
        self.items = items;
        self.selected = vec![false; len];
        self.cursor = 0;
        if len > 0 {
            self.table_state.select(Some(0));
        } else {
            self.table_state.select(None);
        }
        self.rebuild_display_indices();
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
}

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
            if matches!(status, MergeStatus::Merged | MergeStatus::SquashMerged) {
                state.selected[i] = true;
            }
        }
    }
}

// --- Sorting free functions ---

/// Apply sort based on current sort_column and sort_ascending.
/// Pinned items always stay at the top, only non-pinned items are sorted.
pub fn apply_sort<T: ViewItem>(state: &mut ListState<T>, columns: &[ColumnDef<T>]) {
    let Some(col_idx) = state.sort_column else {
        return;
    };
    let Some(column) = columns.get(col_idx) else {
        return;
    };
    let Some(compare) = column.compare else {
        return;
    };

    let asc = state.sort_ascending;

    // Stable-partition: move all pinned items to the front (base first),
    // then sort only the non-pinned tail.
    state.items.sort_by(|a, b| {
        match (a.is_pinned(), b.is_pinned()) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            (true, true) => {
                // Base branch always comes first among pinned items
                b.is_base().cmp(&a.is_base())
            }
            (false, false) => {
                let ord = compare(a, b);
                if asc {
                    ord
                } else {
                    ord.reverse()
                }
            }
        }
    });

    // Reset selection and cursor
    state.selected = vec![false; state.items.len()];
    state.cursor = 0;
    state.table_state.select(if state.items.is_empty() {
        None
    } else {
        Some(0)
    });
    state.rebuild_display_indices();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;
    use chrono::Utc;

    fn sample_branches() -> Vec<BranchInfo> {
        vec![
            BranchInfo {
                name: "main".into(),
                is_current: false,
                is_base: true,
                tracking: TrackingStatus::Local,
                ahead: None,
                behind: None,
                last_commit_date: Utc::now(),
                merge_status: MergeStatus::Unmerged,
                base_branch: "main".into(),
                merge_base_commit: None,
                pr: None,
            },
            BranchInfo {
                name: "feature/a".into(),
                is_current: false,
                is_base: false,
                tracking: TrackingStatus::Local,
                ahead: None,
                behind: None,
                last_commit_date: Utc::now() - chrono::Duration::days(1),
                merge_status: MergeStatus::Unmerged,
                base_branch: "main".into(),
                merge_base_commit: None,
                pr: None,
            },
            BranchInfo {
                name: "feature/b".into(),
                is_current: false,
                is_base: false,
                tracking: TrackingStatus::Local,
                ahead: None,
                behind: None,
                last_commit_date: Utc::now() - chrono::Duration::days(2),
                merge_status: MergeStatus::Merged,
                base_branch: "main".into(),
                merge_base_commit: None,
                pr: None,
            },
        ]
    }

    #[test]
    fn collect_targets_all_selected_no_filter() {
        let mut state = ListState::new(sample_branches());
        state.selected_mut().iter_mut().for_each(|s| *s = true);
        let targets = collect_targets(&state, |b| Some(b.name.clone()));
        assert_eq!(targets, vec!["main", "feature/a", "feature/b"]);
    }

    #[test]
    fn collect_targets_filters_pinned() {
        let mut state = ListState::new(sample_branches());
        state.selected_mut().iter_mut().for_each(|s| *s = true);
        let targets = collect_targets(&state, |b| (!b.is_pinned()).then(|| b.name.clone()));
        assert_eq!(targets, vec!["feature/a", "feature/b"]);
    }

    #[test]
    fn collect_targets_cursor_fallback_when_none_selected() {
        let state = ListState::new(sample_branches());
        // Nothing checked: selected_indices falls back to the cursor item (index 0).
        let targets = collect_targets(&state, |b| Some(b.name.clone()));
        assert_eq!(targets, vec!["main"]);
    }

    #[test]
    fn collect_targets_empty_when_all_selected_are_filtered() {
        let mut state = ListState::new(sample_branches());
        // Select only the pinned base branch; the mapper filters pinned items.
        state.selected_mut()[0] = true;
        let targets = collect_targets(&state, |b| (!b.is_pinned()).then(|| b.name.clone()));
        assert!(targets.is_empty());
    }

    fn name_column() -> ColumnDef<BranchInfo> {
        ColumnDef::<BranchInfo> {
            name: "Name",
            min_width: 10,
            wide_width: None,
            hide_below_width: None,
            compare: Some(|a, b| a.name.cmp(&b.name)),
        }
    }

    #[test]
    fn sort_by_column_click_sets_column_and_applies() {
        let columns = vec![name_column()];
        let mut state = ListState::new(sample_branches());
        assert_eq!(state.sort_column(), None);

        sort_by_column_click(&mut state, &columns, 0);

        assert_eq!(state.sort_column(), Some(0));
        assert!(state.sort_ascending());
        // main is the base branch (pinned first), then ascending by name.
        assert_eq!(state.items()[0].name, "main");
        assert_eq!(state.items()[1].name, "feature/a");
        assert_eq!(state.items()[2].name, "feature/b");
    }

    #[test]
    fn sort_by_column_click_same_column_toggles_direction() {
        let columns = vec![name_column()];
        let mut state = ListState::new(sample_branches());

        sort_by_column_click(&mut state, &columns, 0);
        assert!(state.sort_ascending());

        sort_by_column_click(&mut state, &columns, 0);
        assert!(!state.sort_ascending());
        assert_eq!(state.sort_column(), Some(0));
        // Descending: base still pinned first, then feature/b, feature/a.
        assert_eq!(state.items()[0].name, "main");
        assert_eq!(state.items()[1].name, "feature/b");
        assert_eq!(state.items()[2].name, "feature/a");

        sort_by_column_click(&mut state, &columns, 0);
        assert!(state.sort_ascending());
    }

    #[test]
    fn sort_by_column_click_different_column_resets_to_ascending() {
        let age_col = ColumnDef::<BranchInfo> {
            name: "Age",
            min_width: 5,
            wide_width: None,
            hide_below_width: None,
            compare: Some(|a, b| a.last_commit_date.cmp(&b.last_commit_date)),
        };
        let columns = vec![name_column(), age_col];
        let mut state = ListState::new(sample_branches());

        sort_by_column_click(&mut state, &columns, 0);
        sort_by_column_click(&mut state, &columns, 0); // now descending
        assert!(!state.sort_ascending());

        sort_by_column_click(&mut state, &columns, 1);
        assert_eq!(state.sort_column(), Some(1));
        assert!(state.sort_ascending());
    }

    // --- ListState basic tests ---

    #[test]
    fn new_state_has_correct_defaults() {
        let state = ListState::new(sample_branches());
        assert_eq!(state.cursor(), 0);
        assert_eq!(state.items().len(), 3);
        assert!(state.selected().iter().all(|&s| !s));
    }

    #[test]
    fn set_items_resets_selection() {
        let mut state = ListState::new(sample_branches());
        state.selected_mut()[1] = true;
        state.set_items(sample_branches());
        assert!(state.selected().iter().all(|&s| !s));
    }

    #[test]
    fn display_indices_returns_all_when_no_filter() {
        let state = ListState::new(sample_branches());
        let indices = state.display_indices();
        assert_eq!(indices.len(), 3);
    }

    #[test]
    fn display_indices_pinned_first() {
        let state = ListState::new(sample_branches());
        let indices = state.display_indices();
        // main (index 0) is pinned, should be first
        assert_eq!(indices[0], 0);
    }

    #[test]
    fn empty_state() {
        let state = ListState::<BranchInfo>::empty();
        assert_eq!(state.items().len(), 0);
        assert!(state.display_indices().is_empty());
        assert!(state.cursor_item().is_none());
        assert!(state.selected_indices().is_empty());
    }

    #[test]
    fn cursor_item_returns_correct_item() {
        let state = ListState::new(sample_branches());
        let item = state.cursor_item().unwrap();
        assert_eq!(item.name, "main");
    }

    #[test]
    fn selected_indices_returns_cursor_when_none_selected() {
        let state = ListState::new(sample_branches());
        let indices = state.selected_indices();
        assert_eq!(indices.len(), 1);
        assert_eq!(indices[0], 0); // cursor is at 0
    }

    #[test]
    fn selected_indices_returns_selected() {
        let mut state = ListState::new(sample_branches());
        state.selected_mut()[1] = true;
        state.selected_mut()[2] = true;
        let indices = state.selected_indices();
        assert_eq!(indices, vec![1, 2]);
    }

    // --- Navigation tests ---

    #[test]
    fn nav_down_moves_cursor() {
        let mut state = ListState::new(sample_branches());
        nav_down(&mut state);
        assert_eq!(state.table_state.selected(), Some(1));
    }

    #[test]
    fn nav_down_stops_at_end() {
        let mut state = ListState::new(sample_branches());
        nav_down(&mut state);
        nav_down(&mut state);
        nav_down(&mut state); // past end
        assert_eq!(state.table_state.selected(), Some(2)); // stays at last
    }

    #[test]
    fn nav_up_moves_cursor() {
        let mut state = ListState::new(sample_branches());
        nav_down(&mut state);
        nav_down(&mut state);
        nav_up(&mut state);
        assert_eq!(state.table_state.selected(), Some(1));
    }

    #[test]
    fn nav_up_stops_at_top() {
        let mut state = ListState::new(sample_branches());
        nav_up(&mut state);
        assert_eq!(state.table_state.selected(), Some(0));
    }

    #[test]
    fn nav_to_end() {
        let mut state = ListState::new(sample_branches());
        nav_end(&mut state);
        assert_eq!(state.table_state.selected(), Some(2));
    }

    #[test]
    fn nav_to_home() {
        let mut state = ListState::new(sample_branches());
        nav_end(&mut state);
        nav_home(&mut state);
        assert_eq!(state.table_state.selected(), Some(0));
    }

    #[test]
    fn nav_page_down_moves() {
        let mut state = ListState::new(sample_branches());
        nav_page_down(&mut state, 2);
        assert_eq!(state.table_state.selected(), Some(2));
    }

    #[test]
    fn nav_page_up_moves() {
        let mut state = ListState::new(sample_branches());
        nav_end(&mut state);
        nav_page_up(&mut state, 2);
        assert_eq!(state.table_state.selected(), Some(0));
    }

    #[test]
    fn nav_on_empty() {
        let mut state = ListState::<BranchInfo>::empty();
        nav_down(&mut state);
        nav_up(&mut state);
        nav_home(&mut state);
        nav_end(&mut state);
        nav_page_down(&mut state, 10);
        nav_page_up(&mut state, 10);
        // Should not panic
        assert!(state.table_state.selected().is_none() || state.table_state.selected() == Some(0));
    }

    // --- Selection tests ---

    #[test]
    fn select_toggle_on_non_pinned() {
        let mut state = ListState::new(sample_branches());
        nav_down(&mut state); // move to feature/a
        select_toggle(&mut state);
        assert!(state.selected()[1]); // feature/a selected
    }

    #[test]
    fn select_toggle_on_pinned_does_nothing() {
        let mut state = ListState::new(sample_branches());
        // cursor on main (pinned)
        select_toggle(&mut state);
        assert!(!state.selected()[0]); // main is pinned, not selectable
    }

    #[test]
    fn select_all_skips_pinned() {
        let mut state = ListState::new(sample_branches());
        select_all(&mut state);
        assert!(!state.selected()[0]); // main is pinned
        assert!(state.selected()[1]);
        assert!(state.selected()[2]);
    }

    #[test]
    fn deselect_all_clears() {
        let mut state = ListState::new(sample_branches());
        select_all(&mut state);
        deselect_all(&mut state);
        assert!(state.selected().iter().all(|&s| !s));
    }

    #[test]
    fn invert_selection_works() {
        let mut state = ListState::new(sample_branches());
        state.selected_mut()[1] = true;
        invert_selection(&mut state);
        assert!(!state.selected()[0]); // pinned stays unselected
        assert!(!state.selected()[1]); // was selected, now not
        assert!(state.selected()[2]); // was not selected, now selected
    }

    #[test]
    fn select_merged_works() {
        let mut state = ListState::new(sample_branches());
        select_merged(&mut state);
        assert!(!state.selected()[0]); // main (pinned)
        assert!(!state.selected()[1]); // unmerged
        assert!(state.selected()[2]); // merged
    }

    // --- Sorting tests ---

    #[test]
    fn sort_by_name_ascending() {
        let columns = vec![ColumnDef::<BranchInfo> {
            name: "Name",
            min_width: 10,
            wide_width: None,
            hide_below_width: None,
            compare: Some(|a, b| a.name.cmp(&b.name)),
        }];
        let mut state = ListState::new(sample_branches());
        state.set_sort(Some(0), true);
        apply_sort(&mut state, &columns);
        // main is pinned (first), then feature/a, feature/b
        assert_eq!(state.items()[0].name, "main");
        assert_eq!(state.items()[1].name, "feature/a");
        assert_eq!(state.items()[2].name, "feature/b");
    }

    #[test]
    fn sort_by_name_descending() {
        let columns = vec![ColumnDef::<BranchInfo> {
            name: "Name",
            min_width: 10,
            wide_width: None,
            hide_below_width: None,
            compare: Some(|a, b| a.name.cmp(&b.name)),
        }];
        let mut state = ListState::new(sample_branches());
        state.set_sort(Some(0), false);
        apply_sort(&mut state, &columns);
        assert_eq!(state.items()[0].name, "main"); // pinned first
        assert_eq!(state.items()[1].name, "feature/b");
        assert_eq!(state.items()[2].name, "feature/a");
    }

    #[test]
    fn sort_keeps_base_first_when_items_not_pre_sorted() {
        let mut branches = sample_branches();
        branches.rotate_left(1); // move main to the end

        let columns = vec![ColumnDef::<BranchInfo> {
            name: "Name",
            min_width: 10,
            wide_width: None,
            hide_below_width: None,
            compare: Some(|a, b| a.name.cmp(&b.name)),
        }];
        let mut state = ListState::new(branches);
        state.set_sort(Some(0), true);
        apply_sort(&mut state, &columns);
        assert_eq!(state.items()[0].name, "main");
    }

    #[test]
    fn sort_descending_keeps_base_first() {
        let columns = vec![ColumnDef::<BranchInfo> {
            name: "Name",
            min_width: 10,
            wide_width: None,
            hide_below_width: None,
            compare: Some(|a, b| a.name.cmp(&b.name)),
        }];
        let mut state = ListState::new(sample_branches());
        state.set_sort(Some(0), false);
        apply_sort(&mut state, &columns);
        assert_eq!(state.items()[0].name, "main");
        assert_eq!(state.items()[1].name, "feature/b");
        assert_eq!(state.items()[2].name, "feature/a");
    }

    #[test]
    fn cycle_sort_column_works() {
        let columns = vec![
            ColumnDef::<BranchInfo> {
                name: "Name",
                min_width: 10,
                wide_width: None,
                hide_below_width: None,
                compare: Some(|a, b| a.name.cmp(&b.name)),
            },
            ColumnDef::<BranchInfo> {
                name: "Unsortable",
                min_width: 5,
                wide_width: None,
                hide_below_width: None,
                compare: None,
            },
            ColumnDef::<BranchInfo> {
                name: "Age",
                min_width: 5,
                wide_width: None,
                hide_below_width: None,
                compare: Some(|a, b| a.last_commit_date.cmp(&b.last_commit_date)),
            },
        ];
        let mut state = ListState::new(sample_branches());

        // None → first sortable (0)
        cycle_sort_column(&mut state, &columns);
        assert_eq!(state.sort_column(), Some(0));

        // 0 → skip 1 (unsortable) → 2
        cycle_sort_column(&mut state, &columns);
        assert_eq!(state.sort_column(), Some(2));

        // 2 → None (wrap)
        cycle_sort_column(&mut state, &columns);
        assert_eq!(state.sort_column(), None);
    }

    #[test]
    fn toggle_sort_direction_works() {
        let mut state = ListState::new(sample_branches());
        assert!(state.sort_ascending());

        // No-op when sort_column is None
        toggle_sort_direction(&mut state);
        assert!(state.sort_ascending());

        // Toggles when a sort column is active
        state.set_sort(Some(0), true);
        toggle_sort_direction(&mut state);
        assert!(!state.sort_ascending());
        toggle_sort_direction(&mut state);
        assert!(state.sort_ascending());
    }

    // --- Filter integration tests ---

    #[test]
    fn search_filters_display() {
        let mut state = ListState::new(sample_branches());
        state.set_search_query("feature".to_string());
        assert_eq!(state.display_indices().len(), 2); // feature/a, feature/b
    }

    #[test]
    fn filter_query_status_merged() {
        let mut state = ListState::new(sample_branches());
        state.set_filter_query("merge:merged".to_string());
        // After rebuilding, only merged items + pinned should show
        let visible: Vec<&str> = state
            .display_indices()
            .iter()
            .map(|&i| state.items()[i].name.as_str())
            .collect();
        assert!(visible.contains(&"main")); // pinned
        assert!(visible.contains(&"feature/b")); // merged
        assert!(!visible.contains(&"feature/a")); // unmerged
    }

    #[test]
    fn search_and_filter_combine() {
        let mut state = ListState::new(sample_branches());
        state.set_search_query("feature".to_string());
        state.set_filter_query("merge:merged".to_string());
        // Only feature/b matches both
        let visible: Vec<&str> = state
            .display_indices()
            .iter()
            .map(|&i| state.items()[i].name.as_str())
            .collect();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0], "feature/b");
    }

    #[test]
    fn set_cursor_updates_table_state() {
        let mut state = ListState::new(sample_branches());
        state.set_cursor(2);
        assert_eq!(state.cursor(), 2);
        // feature/b is at display index 2
        assert_eq!(state.table_state.selected(), Some(2));
    }
}
