# Phase 2: Generic View Framework

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the shared abstraction layer that all 4 views inherit from. This eliminates the duplication that defines the current codebase. After this phase, navigation, selection, sorting, filtering, and column definitions work generically for any item type.

**Architecture:** A `ViewItem` trait describes what any list item can provide. A `ListState<T>` struct holds all per-view state (cursor, selection, sort, filter, search). A `ViewDef` trait declares columns, context menu items, and filter tokens per view. Free functions operate on `&mut ListState<T>` for navigation, selection, sorting, and filtering.

**Tech Stack:** Rust, ratatui 0.30 (for `TableState` and `Cell`/`Span` types), chrono 0.4

**Prerequisites:** Phase 1 must be complete (types.rs, all data model structs).

**Reference:** Current duplication patterns in `app.rs` (3x ListNav implementations, 3x sort functions, 3x filter functions). See overview plan for the design rationale.

---

### Task 1: ViewItem Trait & ViewId Enum

**Files:**
- Create: `src/view/mod.rs`
- Update: `src/lib.rs`

- [ ] **Step 1: Create `src/view/mod.rs` with core trait**

```rust
pub mod branches;
pub mod column;
pub mod filter;
pub mod list_state;
pub mod remotes;
pub mod tags;
pub mod worktrees;

use crate::types::*;
use chrono::{DateTime, Utc};

/// Identifies which primary view is active
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewId {
    Branches,
    Remotes,
    Tags,
    Worktrees,
}

impl ViewId {
    /// Fixed tab cycle order: Branches → Remotes → Tags → Worktrees
    pub fn next(self) -> Self {
        match self {
            Self::Branches => Self::Remotes,
            Self::Remotes => Self::Tags,
            Self::Tags => Self::Worktrees,
            Self::Worktrees => Self::Branches,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Branches => Self::Worktrees,
            Self::Remotes => Self::Branches,
            Self::Tags => Self::Remotes,
            Self::Worktrees => Self::Tags,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Branches => "Branches",
            Self::Remotes => "Remote",
            Self::Tags => "Tags",
            Self::Worktrees => "Worktrees",
        }
    }

    /// All 4 views in tab order
    pub const ALL: [ViewId; 4] = [Self::Branches, Self::Remotes, Self::Tags, Self::Worktrees];
}

/// Trait implemented by every list item type (BranchInfo, RemoteBranchInfo, etc.)
/// Provides the common interface the generic framework needs.
pub trait ViewItem: Clone {
    fn display_name(&self) -> &str;
    fn is_pinned(&self) -> bool;
    fn merge_status(&self) -> Option<&MergeStatus> { None }
    fn last_commit_date(&self) -> &DateTime<Utc>;
    fn ahead(&self) -> Option<u32> { None }
    fn behind(&self) -> Option<u32> { None }
    fn pr_info(&self) -> Option<&PrInfo> { None }
    fn is_current(&self) -> bool { false }
}
```

- [ ] **Step 2: Implement ViewItem for all 4 data types**

Add to `src/view/mod.rs`:

```rust
impl ViewItem for BranchInfo {
    fn display_name(&self) -> &str { &self.name }
    fn is_pinned(&self) -> bool { self.is_base || self.is_current }
    fn merge_status(&self) -> Option<&MergeStatus> { Some(&self.merge_status) }
    fn last_commit_date(&self) -> &DateTime<Utc> { &self.last_commit_date }
    fn ahead(&self) -> Option<u32> { self.ahead }
    fn behind(&self) -> Option<u32> { self.behind }
    fn is_current(&self) -> bool { self.is_current }
}

impl ViewItem for RemoteBranchInfo {
    fn display_name(&self) -> &str { &self.full_ref }
    fn is_pinned(&self) -> bool { self.is_base }
    fn merge_status(&self) -> Option<&MergeStatus> { Some(&self.merge_status) }
    fn last_commit_date(&self) -> &DateTime<Utc> { &self.last_commit_date }
    fn ahead(&self) -> Option<u32> { self.ahead }
    fn behind(&self) -> Option<u32> { self.behind }
}

impl ViewItem for TagInfo {
    fn display_name(&self) -> &str { &self.name }
    fn is_pinned(&self) -> bool { false }
    fn last_commit_date(&self) -> &DateTime<Utc> { &self.date }
}

impl ViewItem for WorktreeInfo {
    fn display_name(&self) -> &str {
        self.branch.as_deref().unwrap_or("[detached]")
    }
    fn is_pinned(&self) -> bool { self.is_main }
    fn merge_status(&self) -> Option<&MergeStatus> { Some(&self.merge_status) }
    fn last_commit_date(&self) -> &DateTime<Utc> { &self.age_date }
    fn ahead(&self) -> Option<u32> { self.ahead }
    fn behind(&self) -> Option<u32> { self.behind }
}
```

- [ ] **Step 3: Add `pub mod view;` to `src/lib.rs`**

- [ ] **Step 4: Create stub files for submodules**

Create empty stubs for: `src/view/list_state.rs`, `src/view/column.rs`, `src/view/filter.rs`, `src/view/branches.rs`, `src/view/remotes.rs`, `src/view/tags.rs`, `src/view/worktrees.rs`

- [ ] **Step 5: Verify it compiles**

Run: `cargo build`
Expected: Compiles.

- [ ] **Step 6: Commit**

```bash
git add src/view/ src/lib.rs
git commit -m "feat: add ViewId enum and ViewItem trait with impls for all 4 data types"
```

---

### Task 2: ColumnDef System

**Files:**
- Modify: `src/view/column.rs`
- Test: `src/view/column.rs` (inline tests)

- [ ] **Step 1: Write tests for column definitions**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;
    use chrono::Utc;
    use std::cmp::Ordering;

    fn sample_branch(name: &str, days_ago: i64) -> BranchInfo {
        BranchInfo {
            name: name.to_string(),
            is_current: false,
            is_base: false,
            tracking: TrackingStatus::Local,
            ahead: None,
            behind: None,
            last_commit_date: Utc::now() - chrono::Duration::days(days_ago),
            merge_status: MergeStatus::Unmerged,
        }
    }

    #[test]
    fn sort_by_name() {
        let a = sample_branch("alpha", 1);
        let b = sample_branch("beta", 1);
        let col = ColumnDef::<BranchInfo> {
            name: "Name",
            min_width: 10,
            hide_below_width: None,
            compare: Some(|a, b| a.name.cmp(&b.name)),
        };
        let cmp_fn = col.compare.unwrap();
        assert_eq!(cmp_fn(&a, &b), Ordering::Less);
    }

    #[test]
    fn sort_by_age() {
        let older = sample_branch("old", 10);
        let newer = sample_branch("new", 1);
        let col = ColumnDef::<BranchInfo> {
            name: "Age",
            min_width: 5,
            hide_below_width: Some(60),
            compare: Some(|a, b| a.last_commit_date.cmp(&b.last_commit_date)),
        };
        let cmp_fn = col.compare.unwrap();
        assert_eq!(cmp_fn(&older, &newer), Ordering::Less);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib column`
Expected: FAIL — ColumnDef not defined.

- [ ] **Step 3: Implement ColumnDef**

Write into `src/view/column.rs`:

```rust
use super::ViewItem;
use std::cmp::Ordering;

/// Defines a single column in a view's table layout.
/// The compare function is used for sorting; None means not sortable.
pub struct ColumnDef<T: ViewItem> {
    pub name: &'static str,
    pub min_width: u16,
    /// Hide this column when terminal width is below this threshold
    pub hide_below_width: Option<u16>,
    /// Comparison function for sorting. None = column is not sortable.
    pub compare: Option<fn(&T, &T) -> Ordering>,
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib column`
Expected: All tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/view/column.rs
git commit -m "feat: add ColumnDef generic column definition system"
```

---

### Task 3: ListState Generic Struct

**Files:**
- Modify: `src/view/list_state.rs`
- Test: `src/view/list_state.rs` (inline tests)

- [ ] **Step 1: Write tests for ListState**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;
    use chrono::Utc;

    fn sample_branches() -> Vec<BranchInfo> {
        vec![
            BranchInfo {
                name: "main".into(), is_current: false, is_base: true,
                tracking: TrackingStatus::Local, ahead: None, behind: None,
                last_commit_date: Utc::now(), merge_status: MergeStatus::Unmerged,
            },
            BranchInfo {
                name: "feature/a".into(), is_current: false, is_base: false,
                tracking: TrackingStatus::Local, ahead: None, behind: None,
                last_commit_date: Utc::now() - chrono::Duration::days(1),
                merge_status: MergeStatus::Unmerged,
            },
            BranchInfo {
                name: "feature/b".into(), is_current: false, is_base: false,
                tracking: TrackingStatus::Local, ahead: None, behind: None,
                last_commit_date: Utc::now() - chrono::Duration::days(2),
                merge_status: MergeStatus::Merged,
            },
        ]
    }

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
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib list_state`
Expected: FAIL.

- [ ] **Step 3: Implement ListState**

Write into `src/view/list_state.rs`:

```rust
use super::ViewItem;
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
        Self {
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
        }
    }

    pub fn empty() -> Self {
        Self::new(Vec::new())
    }

    // --- Accessors ---

    pub fn items(&self) -> &[T] { &self.items }
    pub fn items_mut(&mut self) -> &mut Vec<T> { &mut self.items }
    pub fn cursor(&self) -> usize { self.cursor }
    pub fn selected(&self) -> &[bool] { &self.selected }
    pub fn selected_mut(&mut self) -> &mut Vec<bool> { &mut self.selected }
    pub fn table_state(&self) -> &TableState { &self.table_state }
    pub fn table_state_mut(&mut self) -> &mut TableState { &mut self.table_state }
    pub fn sort_column(&self) -> Option<usize> { self.sort_column }
    pub fn sort_ascending(&self) -> bool { self.sort_ascending }
    pub fn search_query(&self) -> &str { &self.search_query }
    pub fn search_active(&self) -> bool { self.search_active }
    pub fn filter_query(&self) -> &str { &self.filter_query }
    pub fn display_indices(&self) -> &[usize] { &self.display_indices }

    // --- Mutators ---

    pub fn set_items(&mut self, items: Vec<T>) {
        let len = items.len();
        self.items = items;
        self.selected = vec![false; len];
        self.cursor = 0;
        if len > 0 {
            self.table_state.select(Some(0));
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

        let mut pinned = Vec::new();
        let mut non_pinned = Vec::new();

        for (i, item) in self.items.iter().enumerate() {
            // Apply search filter
            if !search_lower.is_empty()
                && !item.display_name().to_lowercase().contains(&search_lower)
            {
                continue;
            }

            if item.is_pinned() {
                pinned.push(i);
            } else {
                non_pinned.push(i);
            }
        }

        pinned.extend(non_pinned);
        self.display_indices = pinned;
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
        let selected: Vec<usize> = self.selected
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib list_state`
Expected: All tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/view/list_state.rs
git commit -m "feat: add generic ListState<T> with cursor, selection, sort, filter state"
```

---

### Task 4: Navigation Functions

**Files:**
- Modify: `src/view/list_state.rs` (add nav functions)
- Test: `src/view/list_state.rs` (add tests)

- [ ] **Step 1: Write navigation tests**

Add to the tests module in `src/view/list_state.rs`:

```rust
    #[test]
    fn nav_down_moves_cursor() {
        let mut state = ListState::new(sample_branches());
        nav_down(&mut state);
        assert_eq!(state.table_state.selected(), Some(1));
    }

    #[test]
    fn nav_down_wraps_at_end() {
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib list_state`
Expected: FAIL — nav functions not defined.

- [ ] **Step 3: Implement navigation functions**

Add to `src/view/list_state.rs` (outside the `impl` block, as free functions):

```rust
/// Move cursor down one row in the display list
pub fn nav_down<T: ViewItem>(state: &mut ListState<T>) {
    let len = state.display_indices.len();
    if len == 0 { return; }
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
    if len == 0 { return; }
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
    if state.display_indices.is_empty() { return; }
    state.table_state.select(Some(0));
    if let Some(&raw_idx) = state.display_indices.first() {
        state.cursor = raw_idx;
    }
}

/// Jump to last item
pub fn nav_end<T: ViewItem>(state: &mut ListState<T>) {
    let len = state.display_indices.len();
    if len == 0 { return; }
    state.table_state.select(Some(len - 1));
    if let Some(&raw_idx) = state.display_indices.last() {
        state.cursor = raw_idx;
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib list_state`
Expected: All tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/view/list_state.rs
git commit -m "feat: add generic navigation functions (up, down, page, home, end)"
```

---

### Task 5: Selection Functions

**Files:**
- Modify: `src/view/list_state.rs`
- Test: inline

- [ ] **Step 1: Write selection tests**

Add to tests:

```rust
    #[test]
    fn select_toggle() {
        let mut state = ListState::new(sample_branches());
        super::select_toggle(&mut state);
        // Pinned items (main) are not selectable
        assert!(!state.selected()[0]); // main is pinned
    }

    #[test]
    fn select_all_skips_pinned() {
        let mut state = ListState::new(sample_branches());
        super::select_all(&mut state);
        assert!(!state.selected()[0]); // main is pinned
        assert!(state.selected()[1]);
        assert!(state.selected()[2]);
    }

    #[test]
    fn deselect_all() {
        let mut state = ListState::new(sample_branches());
        super::select_all(&mut state);
        super::deselect_all(&mut state);
        assert!(state.selected().iter().all(|&s| !s));
    }

    #[test]
    fn invert_selection() {
        let mut state = ListState::new(sample_branches());
        state.selected_mut()[1] = true;
        super::invert_selection(&mut state);
        assert!(!state.selected()[0]); // pinned stays unselected
        assert!(!state.selected()[1]); // was selected, now not
        assert!(state.selected()[2]); // was not selected, now selected
    }

    #[test]
    fn select_merged() {
        let mut state = ListState::new(sample_branches());
        super::select_merged(&mut state);
        assert!(!state.selected()[0]); // main (pinned)
        assert!(!state.selected()[1]); // unmerged
        assert!(state.selected()[2]); // merged
    }
```

- [ ] **Step 2: Implement selection functions**

Add to `src/view/list_state.rs`:

```rust
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
        if state.items[i].is_pinned() { continue; }
        if let Some(status) = state.items[i].merge_status() {
            if matches!(status, MergeStatus::Merged | MergeStatus::SquashMerged) {
                state.selected[i] = true;
            }
        }
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test --lib list_state`
Expected: All tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/view/list_state.rs
git commit -m "feat: add generic selection functions (toggle, all, none, invert, merged)"
```

---

### Task 6: Generic Sorting

**Files:**
- Modify: `src/view/list_state.rs`
- Test: inline

- [ ] **Step 1: Write sorting tests**

```rust
    #[test]
    fn sort_by_name_ascending() {
        let columns = vec![
            ColumnDef::<BranchInfo> {
                name: "Name",
                min_width: 10,
                hide_below_width: None,
                compare: Some(|a, b| a.name.cmp(&b.name)),
            },
        ];
        let mut state = ListState::new(sample_branches());
        state.set_sort(Some(0), true);
        super::apply_sort(&mut state, &columns);
        // main is pinned (first), then feature/a, feature/b
        assert_eq!(state.items()[0].name, "main");
        assert_eq!(state.items()[1].name, "feature/a");
        assert_eq!(state.items()[2].name, "feature/b");
    }

    #[test]
    fn sort_by_name_descending() {
        let columns = vec![
            ColumnDef::<BranchInfo> {
                name: "Name",
                min_width: 10,
                hide_below_width: None,
                compare: Some(|a, b| a.name.cmp(&b.name)),
            },
        ];
        let mut state = ListState::new(sample_branches());
        state.set_sort(Some(0), false);
        super::apply_sort(&mut state, &columns);
        assert_eq!(state.items()[0].name, "main"); // pinned first
        assert_eq!(state.items()[1].name, "feature/b");
        assert_eq!(state.items()[2].name, "feature/a");
    }

    #[test]
    fn cycle_sort_column() {
        let mut state = ListState::new(sample_branches());
        let num_cols = 3;
        super::cycle_sort_column(&mut state, num_cols);
        assert_eq!(state.sort_column(), Some(0));
        super::cycle_sort_column(&mut state, num_cols);
        assert_eq!(state.sort_column(), Some(1));
        super::cycle_sort_column(&mut state, num_cols);
        assert_eq!(state.sort_column(), Some(2));
        super::cycle_sort_column(&mut state, num_cols);
        assert_eq!(state.sort_column(), None); // wraps to None
    }
```

- [ ] **Step 2: Implement sorting**

Add to `src/view/list_state.rs` (needs `use super::column::ColumnDef;` at top):

```rust
use super::column::ColumnDef;

/// Apply sort based on current sort_column and sort_ascending.
/// Pinned items always stay at the top, only non-pinned items are sorted.
pub fn apply_sort<T: ViewItem>(state: &mut ListState<T>, columns: &[ColumnDef<T>]) {
    let Some(col_idx) = state.sort_column else { return };
    let Some(column) = columns.get(col_idx) else { return };
    let Some(compare) = column.compare else { return };

    let asc = state.sort_ascending;

    // Find where pinned items end
    let pin_count = state.items.iter().take_while(|item| item.is_pinned()).count();
    let sortable = &mut state.items[pin_count..];

    sortable.sort_by(|a, b| {
        let ord = compare(a, b);
        if asc { ord } else { ord.reverse() }
    });

    // Reset selection and cursor
    state.selected = vec![false; state.items.len()];
    state.cursor = 0;
    state.table_state.select(if state.items.is_empty() { None } else { Some(0) });
    state.rebuild_display_indices();
}

/// Cycle to next sort column (None → 0 → 1 → ... → None)
pub fn cycle_sort_column<T: ViewItem>(state: &mut ListState<T>, num_columns: usize) {
    state.sort_column = match state.sort_column {
        None => Some(0),
        Some(c) if c + 1 < num_columns => Some(c + 1),
        Some(_) => None,
    };
}

/// Toggle sort direction
pub fn toggle_sort_direction<T: ViewItem>(state: &mut ListState<T>) {
    state.sort_ascending = !state.sort_ascending;
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test --lib list_state`
Expected: All tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/view/list_state.rs
git commit -m "feat: add generic sorting with pinned-first preservation"
```

---

### Task 7: FilterSet & Filter Tokens

**Files:**
- Modify: `src/view/filter.rs`
- Test: `src/view/filter.rs` (inline tests)

- [ ] **Step 1: Write filter tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::MergeStatus;

    #[test]
    fn parse_empty_query() {
        let fs = FilterSet::parse("");
        assert!(fs.statuses.is_empty());
        assert!(!fs.pr_yes);
        assert!(!fs.sync_ahead);
        assert!(fs.text.is_empty());
    }

    #[test]
    fn parse_status_merged() {
        let fs = FilterSet::parse("status:merged");
        assert_eq!(fs.statuses, vec![MergeStatus::Merged]);
    }

    #[test]
    fn parse_multiple_tokens() {
        let fs = FilterSet::parse("status:merged pr:yes age:<7d");
        assert_eq!(fs.statuses, vec![MergeStatus::Merged]);
        assert!(fs.pr_yes);
        assert!(fs.age_newer_secs.is_some());
    }

    #[test]
    fn parse_age_newer() {
        let fs = FilterSet::parse("age:<30d");
        assert_eq!(fs.age_newer_secs, Some(30 * 86400));
    }

    #[test]
    fn parse_age_older() {
        let fs = FilterSet::parse("age:>90d");
        assert_eq!(fs.age_older_secs, Some(90 * 86400));
    }

    #[test]
    fn toggle_token_adds() {
        let result = FilterSet::toggle_token("", "status:merged");
        assert_eq!(result, "status:merged");
    }

    #[test]
    fn toggle_token_removes() {
        let result = FilterSet::toggle_token("status:merged pr:yes", "status:merged");
        assert_eq!(result, "pr:yes");
    }

    #[test]
    fn has_token() {
        assert!(FilterSet::has_token("status:merged pr:yes", "status:merged"));
        assert!(!FilterSet::has_token("status:merged pr:yes", "status:squash"));
    }
}
```

- [ ] **Step 2: Implement FilterSet**

Write into `src/view/filter.rs`:

```rust
use crate::types::MergeStatus;

/// Parsed filter state from a query string
#[derive(Debug, Default, Clone)]
pub struct FilterSet {
    pub statuses: Vec<MergeStatus>,
    pub pr_yes: bool,
    pub pr_no: bool,
    pub sync_ahead: bool,
    pub sync_behind: bool,
    pub age_newer_secs: Option<i64>,
    pub age_older_secs: Option<i64>,
    pub text: String,
}

/// Defines which filter tokens are available in a view
#[derive(Debug, Clone)]
pub struct FilterTokenDef {
    pub key: char,
    pub label: &'static str,
    pub token: &'static str,
}

impl FilterSet {
    pub fn parse(query: &str) -> Self {
        let mut fs = Self::default();
        let mut text_parts = Vec::new();

        for token in query.split_whitespace() {
            match token {
                "status:merged" => fs.statuses.push(MergeStatus::Merged),
                "status:squash" => fs.statuses.push(MergeStatus::SquashMerged),
                "status:unmerged" => fs.statuses.push(MergeStatus::Unmerged),
                "pr:yes" => fs.pr_yes = true,
                "pr:no" => fs.pr_no = true,
                "sync:ahead" => fs.sync_ahead = true,
                "sync:behind" => fs.sync_behind = true,
                t if t.starts_with("age:<") => {
                    fs.age_newer_secs = parse_age_secs(&t[5..]);
                }
                t if t.starts_with("age:>") => {
                    fs.age_older_secs = parse_age_secs(&t[5..]);
                }
                other => text_parts.push(other),
            }
        }

        fs.text = text_parts.join(" ");
        fs
    }

    pub fn is_empty(&self) -> bool {
        self.statuses.is_empty()
            && !self.pr_yes
            && !self.pr_no
            && !self.sync_ahead
            && !self.sync_behind
            && self.age_newer_secs.is_none()
            && self.age_older_secs.is_none()
            && self.text.is_empty()
    }

    pub fn toggle_token(query: &str, token: &str) -> String {
        if Self::has_token(query, token) {
            query
                .split_whitespace()
                .filter(|&t| t != token)
                .collect::<Vec<_>>()
                .join(" ")
        } else {
            let mut result = query.to_string();
            if !result.is_empty() { result.push(' '); }
            result.push_str(token);
            result
        }
    }

    pub fn has_token(query: &str, token: &str) -> bool {
        query.split_whitespace().any(|t| t == token)
    }
}

fn parse_age_secs(s: &str) -> Option<i64> {
    let (num_str, multiplier) = if let Some(n) = s.strip_suffix('d') {
        (n, 86400i64)
    } else if let Some(n) = s.strip_suffix('w') {
        (n, 7 * 86400)
    } else if let Some(n) = s.strip_suffix('m') {
        (n, 30 * 86400)
    } else if let Some(n) = s.strip_suffix('y') {
        (n, 365 * 86400)
    } else {
        return None;
    };
    num_str.parse::<i64>().ok().map(|n| n * multiplier)
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test --lib filter`
Expected: All tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/view/filter.rs
git commit -m "feat: add FilterSet parsing, toggle, and FilterTokenDef"
```

---

### Task 8: Integrate Filtering into ListState

**Files:**
- Modify: `src/view/list_state.rs` (add filter-aware display index rebuild)
- Test: inline

- [ ] **Step 1: Write filter integration tests**

```rust
    #[test]
    fn search_filters_display() {
        let mut state = ListState::new(sample_branches());
        state.set_search_query("feature".to_string());
        assert_eq!(state.display_indices().len(), 2); // feature/a, feature/b
    }

    #[test]
    fn filter_query_status_merged() {
        let mut state = ListState::new(sample_branches());
        state.set_filter_query("status:merged".to_string());
        // After rebuilding, only merged items + pinned should show
        let visible: Vec<&str> = state.display_indices().iter()
            .map(|&i| state.items()[i].name.as_str())
            .collect();
        assert!(visible.contains(&"main")); // pinned
        assert!(visible.contains(&"feature/b")); // merged
        assert!(!visible.contains(&"feature/a")); // unmerged
    }
```

- [ ] **Step 2: Update `rebuild_display_indices` to use FilterSet**

Update the method in `ListState`:

```rust
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
            if !item.is_pinned() && !filter.is_empty() {
                if !self.matches_filter(item, &filter) {
                    continue;
                }
            }

            if item.is_pinned() {
                pinned.push(i);
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
            if age_secs > newer_secs { return false; }
        }
        if let Some(older_secs) = filter.age_older_secs {
            let age_secs = chrono::Utc::now()
                .signed_duration_since(*item.last_commit_date())
                .num_seconds();
            if age_secs < older_secs { return false; }
        }

        // Ahead/behind filters
        if filter.sync_ahead && item.ahead().unwrap_or(0) == 0 { return false; }
        if filter.sync_behind && item.behind().unwrap_or(0) == 0 { return false; }

        // PR filters
        if filter.pr_yes && item.pr_info().is_none() { return false; }
        if filter.pr_no && item.pr_info().is_some() { return false; }

        true
    }
```

- [ ] **Step 3: Run tests**

Run: `cargo test --lib list_state`
Expected: All tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/view/list_state.rs
git commit -m "feat: integrate FilterSet into ListState display index rebuild"
```

---

### Task 9: Theme System

**Files:**
- Create: `src/theme.rs`
- Update: `src/lib.rs`
- Test: `src/theme.rs` (inline tests)

- [ ] **Step 1: Write theme tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_cycle() {
        let t = Theme::dark();
        let t2 = t.next();
        assert_eq!(t2.name, "light");
        let t3 = t2.next();
        assert_eq!(t3.name, "solarized");
        let t4 = t3.next();
        assert_eq!(t4.name, "dracula");
        let t5 = t4.next();
        assert_eq!(t5.name, "dark");
    }

    #[test]
    fn theme_from_name() {
        let t = Theme::from_name("dracula");
        assert_eq!(t.name, "dracula");
        let t = Theme::from_name("invalid");
        assert_eq!(t.name, "dark"); // default
    }
}
```

- [ ] **Step 2: Implement Theme**

Write `src/theme.rs` — port from current `ui/theme.rs` with the same 4 themes and all style fields. The struct has a `name` field and a `next()` method that cycles through dark→light→solarized→dracula.

(Full implementation: port the existing Theme struct with all ~30 style fields from the current `src/ui/theme.rs`, keeping the same color values.)

- [ ] **Step 3: Run tests**

Run: `cargo test --lib theme`
Expected: All tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/theme.rs src/lib.rs
git commit -m "feat: add Theme system with 4 color themes and cycling"
```

---

### Task 10: Symbol Sets

**Files:**
- Create: `src/symbols.rs`
- Update: `src/lib.rs`
- Test: `src/symbols.rs` (inline tests)

- [ ] **Step 1: Write symbol tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbol_cycle() {
        let s = SymbolSet::ascii();
        assert_eq!(s.next().checkbox_on, "◉");
        assert_eq!(s.next().next().checkbox_on, "\u{f058}"); // powerline
        assert_eq!(s.next().next().next().checkbox_on, "[x]"); // back to ascii
    }

    #[test]
    fn from_name() {
        assert_eq!(SymbolSet::from_name("ascii").checkbox_on, "[x]");
        assert_eq!(SymbolSet::from_name("unicode").checkbox_on, "◉");
    }
}
```

- [ ] **Step 2: Implement SymbolSet**

Write `src/symbols.rs` — port from current `ui/symbols.rs` with all 3 variants and the `detect()` auto-detection function.

- [ ] **Step 3: Run tests, commit**

```bash
git add src/symbols.rs src/lib.rs
git commit -m "feat: add SymbolSet system (ASCII, Unicode, Powerline) with detection"
```

---

### Task 11: Per-View Definitions (Branches)

**Files:**
- Modify: `src/view/branches.rs`
- Test: inline

- [ ] **Step 1: Write tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_correct_column_count() {
        let view = BranchesViewDef;
        assert!(view.columns().len() >= 5);
    }

    #[test]
    fn name_column_is_sortable() {
        let view = BranchesViewDef;
        let name_col = &view.columns()[0];
        assert!(name_col.compare.is_some());
    }

    #[test]
    fn filter_tokens_include_status() {
        let view = BranchesViewDef;
        let tokens = view.filter_tokens();
        assert!(tokens.iter().any(|t| t.token == "status:merged"));
    }
}
```

- [ ] **Step 2: Implement BranchesViewDef**

```rust
use crate::types::BranchInfo;
use super::column::ColumnDef;
use super::filter::FilterTokenDef;

pub struct BranchesViewDef;

impl BranchesViewDef {
    pub fn columns(&self) -> Vec<ColumnDef<BranchInfo>> {
        vec![
            ColumnDef {
                name: "Branch",
                min_width: 15,
                hide_below_width: None,
                compare: Some(|a, b| a.name.cmp(&b.name)),
            },
            ColumnDef {
                name: "Remote",
                min_width: 8,
                hide_below_width: Some(80),
                compare: None,
            },
            ColumnDef {
                name: "A/B",
                min_width: 8,
                hide_below_width: Some(80),
                compare: Some(|a, b| a.ahead.unwrap_or(0).cmp(&b.ahead.unwrap_or(0))),
            },
            ColumnDef {
                name: "Age",
                min_width: 5,
                hide_below_width: Some(60),
                compare: Some(|a, b| a.last_commit_date.cmp(&b.last_commit_date)),
            },
            ColumnDef {
                name: "Status",
                min_width: 3,
                hide_below_width: None,
                compare: Some(|a, b| {
                    let rank = |s: &crate::types::MergeStatus| match s {
                        crate::types::MergeStatus::Merged => 0,
                        crate::types::MergeStatus::SquashMerged => 1,
                        crate::types::MergeStatus::Unmerged => 2,
                        crate::types::MergeStatus::Pending => 3,
                    };
                    rank(&a.merge_status).cmp(&rank(&b.merge_status))
                }),
            },
        ]
    }

    pub fn filter_tokens(&self) -> Vec<FilterTokenDef> {
        vec![
            FilterTokenDef { key: 'm', label: "Merged", token: "status:merged" },
            FilterTokenDef { key: 's', label: "Squash-merged", token: "status:squash" },
            FilterTokenDef { key: 'u', label: "Unmerged", token: "status:unmerged" },
            FilterTokenDef { key: 'p', label: "Has PR", token: "pr:yes" },
            FilterTokenDef { key: 'P', label: "No PR", token: "pr:no" },
            FilterTokenDef { key: 'a', label: "Ahead", token: "sync:ahead" },
            FilterTokenDef { key: 'b', label: "Behind", token: "sync:behind" },
            FilterTokenDef { key: '1', label: "<7 days", token: "age:<7d" },
            FilterTokenDef { key: '2', label: "<30 days", token: "age:<30d" },
            FilterTokenDef { key: '3', label: ">30 days", token: "age:>30d" },
            FilterTokenDef { key: '4', label: ">90 days", token: "age:>90d" },
        ]
    }
}
```

- [ ] **Step 3: Run tests, commit**

- [ ] **Step 4: Repeat for Remotes, Tags, Worktrees view definitions**

Create similar `RemotesViewDef`, `TagsViewDef`, `WorktreesViewDef` in their respective files, each with appropriate columns and filter tokens per the requirements document.

- [ ] **Step 5: Commit all view definitions**

```bash
git add src/view/
git commit -m "feat: add all 4 view definitions (columns, filter tokens)"
```

---

### Task 12: Full Phase 2 Test Suite & Cleanup

- [ ] **Step 1: Run all tests**

Run: `cargo test`
Expected: All Phase 1 + Phase 2 tests pass.

- [ ] **Step 2: Run clippy**

Run: `cargo clippy -- -D warnings`

- [ ] **Step 3: Commit**

```bash
git commit -m "chore: Phase 2 complete — generic view framework with all 4 view definitions"
```
