# Phase 3: UI Rendering Layer

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build all UI rendering — the generic list renderer that works for any view, all overlay UIs, status bar, tab bar, and toast system. After this phase, every visual component can be rendered to a ratatui Frame.

**Architecture:** A single `render_list_view()` function renders any `ListState<T>` using its `ColumnDef` definitions. Overlay UIs (help, menu, confirm, etc.) are standalone rendering functions. The tab bar and status bar are shared components used by all views.

**Tech Stack:** Rust, ratatui 0.30, crossterm 0.29

**Prerequisites:** Phase 1 (types, git) and Phase 2 (view framework) must be complete.

**Reference:** Current `src/ui/` directory. The generic list renderer replaces the 3 duplicated renderers (`branch_list.rs`, `remote_branch_list.rs`, `worktree_list.rs`).

---

### Task 1: Shared UI Utilities

**Files:**
- Create: `src/ui/mod.rs`
- Create: `src/ui/shared.rs`
- Update: `src/lib.rs`

- [ ] **Step 1: Create `src/ui/mod.rs`**

```rust
pub mod confirm;
pub mod executing;
pub mod filter_ui;
pub mod help;
pub mod list_render;
pub mod menu;
pub mod render;
pub mod results;
pub mod settings;
pub mod shared;
pub mod status_bar;
pub mod tab_bar;
pub mod toast;
```

- [ ] **Step 2: Implement shared utilities in `src/ui/shared.rs`**

Port from current `src/ui/shared.rs`:
- `prefix_style(name: &str, theme: &Theme) -> Style` — color branch name prefixes (feat/, fix/, chore/, etc.)
- `age_style(date: &DateTime<Utc>, theme: &Theme) -> Style` — color by age (green < 7d, yellow < 30d, orange < 90d, red > 90d)
- `truncate(s: &str, max_width: usize) -> String` — ellipsis truncation
- `centered_rect(width_pct: u16, height: u16, area: Rect) -> Rect` — centered overlay rectangle

- [ ] **Step 3: Add `pub mod ui;` to `src/lib.rs`**

- [ ] **Step 4: Verify it compiles, commit**

```bash
git add src/ui/ src/lib.rs
git commit -m "feat: add UI module scaffold and shared utilities"
```

---

### Task 2: Tab Bar Rendering

**Files:**
- Create: `src/ui/tab_bar.rs`

- [ ] **Step 1: Implement tab bar**

```rust
use crate::theme::Theme;
use crate::view::ViewId;
use ratatui::text::{Line, Span};
use ratatui::style::{Modifier, Style};

/// Builds the tab bar Line for the block title.
/// Shows all 4 tabs; active tab is highlighted.
pub fn tab_bar_line(active: ViewId, theme: &Theme) -> Line<'static> {
    let mut spans = Vec::new();

    for (i, view_id) in ViewId::ALL.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" │ ", Style::default().fg(theme.dim_fg())));
        }

        let label = view_id.label();
        if *view_id == active {
            spans.push(Span::styled(
                label.to_string(),
                theme.title.add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::styled(label.to_string(), theme.secondary_text));
        }
    }

    Line::from(spans)
}
```

- [ ] **Step 2: Verify it compiles, commit**

```bash
git add src/ui/tab_bar.rs
git commit -m "feat: add tab bar rendering for all 4 views"
```

---

### Task 3: Status Bar Rendering

**Files:**
- Create: `src/ui/status_bar.rs`

- [ ] **Step 1: Implement status bar**

Port from current `src/ui/shared.rs::render_status_bar()`. The status bar:
- Parses `[key]label` patterns from a format string
- Renders keys in bold/colored, labels in normal text
- Returns clickable regions `Vec<(u16, u16, KeyCode)>` for mouse support
- Shows active filter tokens

```rust
use crossterm::event::KeyCode;
use ratatui::prelude::*;
use crate::theme::Theme;

pub struct StatusBarItem {
    pub x_start: u16,
    pub x_end: u16,
    pub key: KeyCode,
}

pub fn render_status_bar(
    frame: &mut Frame,
    area: Rect,
    text: &str,
    theme: &Theme,
) -> Vec<StatusBarItem> {
    // Implementation: parse [key]label patterns, render as spans,
    // track x positions for clickable items
    // ... (port from current shared.rs render_status_bar)
    Vec::new() // placeholder — full implementation ports existing logic
}
```

- [ ] **Step 2: Verify it compiles, commit**

```bash
git add src/ui/status_bar.rs
git commit -m "feat: add status bar rendering with clickable shortcuts"
```

---

### Task 4: Generic List Renderer

**Files:**
- Create: `src/ui/list_render.rs`

This is the **key rendering component** — a single function that renders any `ListState<T>` with its column definitions. Replaces the 3 duplicated renderers.

- [ ] **Step 1: Implement generic list renderer**

```rust
use crate::symbols::SymbolSet;
use crate::theme::Theme;
use crate::types::*;
use crate::view::column::ColumnDef;
use crate::view::list_state::ListState;
use crate::view::ViewItem;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Row, Table, TableState};

/// Render context passed to per-view cell rendering callbacks
pub struct CellContext<'a> {
    pub theme: &'a Theme,
    pub symbols: &'a SymbolSet,
    pub width: u16,
    pub compact: bool,
}

/// Renders any list view generically.
///
/// `render_row` is the view-specific callback that turns an item into a Row.
/// This keeps the generic renderer from needing to know about BranchInfo vs TagInfo.
pub fn render_list_view<T: ViewItem>(
    frame: &mut Frame,
    area: Rect,
    state: &mut ListState<T>,
    columns: &[ColumnDef<T>],
    render_row: fn(&T, usize, bool, bool, &CellContext) -> Row<'static>,
    theme: &Theme,
    symbols: &SymbolSet,
) {
    let width = area.width as usize;
    let compact = width < 120;

    // Build header row
    let sort_arrow = if state.sort_ascending() { "▲" } else { "▼" };
    let header_cells: Vec<Cell> = columns
        .iter()
        .enumerate()
        .filter(|(_, col)| {
            col.hide_below_width.map_or(true, |threshold| area.width >= threshold)
        })
        .map(|(i, col)| {
            let label = if state.sort_column() == Some(i) {
                format!("{}{}", col.name, sort_arrow)
            } else {
                col.name.to_string()
            };
            Cell::from(label).style(theme.header)
        })
        .collect();

    let header = Row::new(header_cells).height(1);

    // Determine visible columns for width calculation
    let visible_columns: Vec<&ColumnDef<T>> = columns
        .iter()
        .filter(|col| col.hide_below_width.map_or(true, |t| area.width >= t))
        .collect();

    // Build column widths (proportional)
    let widths: Vec<Constraint> = visible_columns
        .iter()
        .map(|col| Constraint::Min(col.min_width))
        .collect();

    let ctx = CellContext {
        theme,
        symbols,
        width: area.width,
        compact,
    };

    // Build rows from display indices
    let rows: Vec<Row> = state
        .display_indices()
        .iter()
        .enumerate()
        .map(|(display_pos, &raw_idx)| {
            let item = &state.items()[raw_idx];
            let is_selected = state.selected()[raw_idx];
            let is_cursor = state.table_state().selected() == Some(display_pos);
            render_row(item, raw_idx, is_selected, is_cursor, &ctx)
        })
        .collect();

    let table = Table::new(rows, &widths)
        .header(header)
        .highlight_style(theme.cursor);

    frame.render_stateful_widget(table, area, state.table_state_mut());

    // Track header column positions for mouse click sorting
    // (store in state.header_columns for mouse handler to use)
}
```

- [ ] **Step 2: Verify it compiles, commit**

```bash
git add src/ui/list_render.rs
git commit -m "feat: add generic list renderer for all views"
```

---

### Task 5: Help Overlay

**Files:**
- Create: `src/ui/help.rs`

- [ ] **Step 1: Implement help overlay**

Port from current `src/ui/help.rs`. Two-column layout of keybindings. Uses `centered_rect()` from shared utilities. Renders over the current view with `Clear` widget underneath.

The help content should be context-aware — showing keys relevant to the active view. Common keys (nav, selection, sort, filter, tab) are shown for all views. View-specific keys are shown based on `ViewId`.

- [ ] **Step 2: Verify it compiles, commit**

```bash
git add src/ui/help.rs
git commit -m "feat: add help overlay with context-aware keybinding reference"
```

---

### Task 6: Context Menu Overlay

**Files:**
- Create: `src/ui/menu.rs`

- [ ] **Step 1: Implement context menu**

Port from current `src/ui/menu.rs`. Centered overlay showing:
- List of `MenuItem` structs (label, shortcut key, enabled flag)
- Disabled items grayed out with shortcut key still colored
- Keyboard nav: j/k, number shortcuts, Enter to execute
- Mouse: click to select
- Esc/q to close

```rust
#[derive(Debug, Clone)]
pub struct MenuItem {
    pub label: String,
    pub shortcut: Option<char>,
    pub action: crate::types::BranchAction,
    pub enabled: bool,
}
```

- [ ] **Step 2: Verify it compiles, commit**

```bash
git add src/ui/menu.rs
git commit -m "feat: add context menu overlay with disabled item support"
```

---

### Task 7: Confirmation Dialog

**Files:**
- Create: `src/ui/confirm.rs`

- [ ] **Step 1: Implement confirmation dialog**

Port from current `src/ui/confirm.rs`. Shows:
- Action description
- List of affected items
- `y`/`Enter` to confirm, `n`/`Esc` to cancel

- [ ] **Step 2: Verify it compiles, commit**

```bash
git add src/ui/confirm.rs
git commit -m "feat: add confirmation dialog for destructive operations"
```

---

### Task 8: Executing & Results Views

**Files:**
- Create: `src/ui/executing.rs`
- Create: `src/ui/results.rs`

- [ ] **Step 1: Implement executing view**

Port from current `src/ui/executing.rs`. Shows operation name + per-item progress.

- [ ] **Step 2: Implement results view**

Port from current `src/ui/results.rs`. Shows success/failure per item. Any key returns to previous view.

- [ ] **Step 3: Verify it compiles, commit**

```bash
git add src/ui/executing.rs src/ui/results.rs
git commit -m "feat: add executing progress and results views"
```

---

### Task 9: Settings Panel

**Files:**
- Create: `src/ui/settings.rs`

- [ ] **Step 1: Implement settings panel**

Port from current `src/ui/settings.rs`. Overlay with:
- Symbol set (cycle with ←/→)
- Theme (cycle with ←/→)
- Sort column, sort direction
- Auto-fetch toggle
- Load worktrees on launch toggle
- j/k navigation, Esc to save and close

- [ ] **Step 2: Verify it compiles, commit**

```bash
git add src/ui/settings.rs
git commit -m "feat: add settings panel overlay"
```

---

### Task 10: Filter Builder UI

**Files:**
- Create: `src/ui/filter_ui.rs`

- [ ] **Step 1: Implement generic filter builder**

This replaces the 3 duplicated filter dialogs. A single `draw_filter()` function takes a `&[FilterTokenDef]` and the current filter query, rendering the appropriate toggles.

```rust
use crate::theme::Theme;
use crate::view::filter::FilterTokenDef;
use ratatui::prelude::*;

/// Renders the filter builder overlay.
/// Shows only the tokens applicable to the current view.
pub fn draw_filter(
    frame: &mut Frame,
    filter_tokens: &[FilterTokenDef],
    current_query: &str,
    theme: &Theme,
) {
    // For each token: show key, label, and whether it's active in the query
    // Active tokens highlighted in green
    // Uses FilterSet::has_token() to check active state
}
```

- [ ] **Step 2: Verify it compiles, commit**

```bash
git add src/ui/filter_ui.rs
git commit -m "feat: add generic filter builder UI driven by FilterTokenDef"
```

---

### Task 11: Toast System

**Files:**
- Create: `src/ui/toast.rs`

- [ ] **Step 1: Implement toast notifications**

```rust
use chrono::{DateTime, Utc};
use ratatui::prelude::*;
use crate::theme::Theme;

pub struct Toast {
    pub message: String,
    pub expires: DateTime<Utc>,
}

impl Toast {
    pub fn new(message: String, duration_secs: i64) -> Self {
        Self {
            message,
            expires: Utc::now() + chrono::Duration::seconds(duration_secs),
        }
    }

    pub fn is_expired(&self) -> bool {
        Utc::now() > self.expires
    }
}

pub fn draw_toast(frame: &mut Frame, toast: &Toast, theme: &Theme) {
    // Render in bottom-right corner as a small bordered box
}
```

- [ ] **Step 2: Verify it compiles, commit**

```bash
git add src/ui/toast.rs
git commit -m "feat: add toast notification system"
```

---

### Task 12: Top-Level Render Dispatcher

**Files:**
- Create: `src/ui/render.rs`

- [ ] **Step 1: Implement render dispatcher**

```rust
/// Top-level draw function called by the event loop.
/// Dispatches to the appropriate view renderer + overlay.
pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    // Layout: main area + status bar
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(area);

    let main_area = chunks[0];
    let status_area = chunks[1];

    // Render active view's list
    match app.active_view {
        ViewId::Branches => { /* render branches list */ }
        ViewId::Remotes => { /* render remotes list */ }
        ViewId::Tags => { /* render tags list */ }
        ViewId::Worktrees => { /* render worktrees list */ }
    }

    // Render overlay if present
    match &app.overlay {
        Some(Overlay::Help) => { /* render help */ }
        Some(Overlay::Menu { cursor }) => { /* render menu */ }
        Some(Overlay::Confirm { action, items }) => { /* render confirm */ }
        Some(Overlay::Executing { label }) => { /* render executing */ }
        Some(Overlay::Results { results, return_view }) => { /* render results */ }
        Some(Overlay::Settings { cursor }) => { /* render settings */ }
        Some(Overlay::Filter) => { /* render filter builder */ }
        None => {}
    }

    // Render toast if present
    // Render search bar if active
    // Render status bar
}
```

- [ ] **Step 2: Verify it compiles, commit**

```bash
git add src/ui/render.rs
git commit -m "feat: add top-level render dispatcher"
```

---

### Task 13: Phase 3 Cleanup

- [ ] **Step 1: Run full test suite**

Run: `cargo test`

- [ ] **Step 2: Run clippy**

Run: `cargo clippy -- -D warnings`

- [ ] **Step 3: Commit**

```bash
git commit -m "chore: Phase 3 complete — all UI rendering components"
```
