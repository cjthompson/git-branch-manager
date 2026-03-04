# Phase 2-4 Full Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Implement all 21 remaining backlog tickets to complete git-branch-manager with full feature set.

**Architecture:** Incremental enhancement of the existing ratatui TUI. Phase 1 adds data/operations without touching UI layout. Phase 2 rewrites the branch list to use Table widget, adds status bar, operations menu. Phase 3 adds responsive layout, symbols, sorting, git operations. Phase 4 adds cosmetic polish and GitHub integration.

**Tech Stack:** Rust, ratatui 0.30, git2 0.20, crossterm 0.29, clap 4, serde, toml (new dep for config)

---

## Phase 1 — Independent Features

### Task 1: BL-009 — Ahead/Behind Indicators

**Files:**
- Modify: `src/types.rs` (add fields to BranchInfo)
- Modify: `src/git/branch.rs` (compute ahead/behind in collect_branch_metadata)
- Modify: `src/ui/branch_list.rs` (display ahead/behind)
- Test: `tests/integration.rs` (add ahead/behind test)

**Step 1: Write the failing test**

Add to `tests/integration.rs`:

```rust
#[test]
fn test_ahead_behind_indicators() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Set up a "remote" by cloning
    let remote_dir = tmpdir.path().join("remote");
    run_git(dir, &["clone", "--bare", ".", remote_dir.to_str().unwrap()]);
    run_git(dir, &["remote", "add", "origin", remote_dir.to_str().unwrap()]);
    run_git(dir, &["fetch", "origin"]);
    run_git(dir, &["branch", "--set-upstream-to=origin/main", "main"]);

    // Create a feature branch, push it, then add a local commit
    run_git(dir, &["checkout", "-b", "feature-ahead"]);
    let f = dir.join("ahead.txt");
    std::fs::write(&f, "ahead\n").unwrap();
    run_git(dir, &["add", "ahead.txt"]);
    run_git(dir, &["commit", "-m", "Ahead commit"]);
    run_git(dir, &["push", "origin", "feature-ahead"]);

    // Add another local commit (ahead by 1)
    let f2 = dir.join("ahead2.txt");
    std::fs::write(&f2, "ahead2\n").unwrap();
    run_git(dir, &["add", "ahead2.txt"]);
    run_git(dir, &["commit", "-m", "Another ahead commit"]);

    run_git(dir, &["checkout", "main"]);

    let repo = git2::Repository::open(dir).unwrap();
    let branches = branch::list_branches_phase1(&repo, "main").unwrap();

    let feature = branches.iter().find(|b| b.name == "feature-ahead").unwrap();
    assert_eq!(feature.ahead, Some(1), "should be 1 ahead");
    assert_eq!(feature.behind, Some(0), "should be 0 behind");
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test test_ahead_behind -- --nocapture`
Expected: FAIL — `BranchInfo` has no `ahead`/`behind` fields

**Step 3: Add ahead/behind fields to BranchInfo**

In `src/types.rs`, add to `BranchInfo` struct:

```rust
pub ahead: Option<u32>,
pub behind: Option<u32>,
```

In `src/git/branch.rs`, in `collect_branch_metadata`, after computing `tracking`, compute ahead/behind:

```rust
let (ahead, behind) = match &tracking {
    TrackingStatus::Tracked { gone: false, .. } => {
        let branch_oid = commit.id();
        if let Ok(upstream) = branch.upstream() {
            if let Ok(upstream_commit) = upstream.get().peel_to_commit() {
                let upstream_oid = upstream_commit.id();
                match repo.graph_ahead_behind(branch_oid, upstream_oid) {
                    Ok((a, b)) => (Some(a as u32), Some(b as u32)),
                    Err(_) => (None, None),
                }
            } else {
                (None, None)
            }
        } else {
            (None, None)
        }
    }
    _ => (None, None),
};
```

Add `ahead, behind` to the `BranchInfo` push.

**Step 4: Run test to verify it passes**

Run: `cargo test test_ahead_behind -- --nocapture`
Expected: PASS

**Step 5: Update branch_list.rs to display ahead/behind**

In `src/ui/branch_list.rs`, add spans after the tracking text:

```rust
let ahead_behind = match (branch.ahead, branch.behind) {
    (Some(a), Some(b)) if a > 0 || b > 0 => {
        let mut parts = Vec::new();
        if a > 0 { parts.push(format!("↑{}", a)); }
        if b > 0 { parts.push(format!("↓{}", b)); }
        parts.join("")
    }
    _ => String::new(),
};
```

Add `Span::styled(ahead_behind, theme::SECONDARY_TEXT)` to the line.

**Step 6: Run full test suite**

Run: `cargo test`
Expected: All tests pass

**Step 7: Commit**

```bash
git add src/types.rs src/git/branch.rs src/ui/branch_list.rs tests/integration.rs
git commit -m "BL-009: Add ahead/behind indicators for tracked branches"
```

---

### Task 2: BL-004 — Force Recheck Command

**Files:**
- Modify: `src/git/cache.rs` (add clear method)
- Modify: `src/app.rs` (add R keybinding)
- Modify: `src/ui/help.rs` (add R to help text)

**Step 1: Add clear method to BranchCache**

In `src/git/cache.rs`, add method to `impl BranchCache`:

```rust
/// Delete the cache file and clear in-memory entries.
pub fn clear(&mut self) {
    self.entries.clear();
    let _ = std::fs::remove_file(&self.path);
}
```

**Step 2: Add R keybinding to app.rs**

In `src/app.rs`, in `handle_branch_list_key`, add case:

```rust
KeyCode::Char('R') => {
    let mut cache = cache::BranchCache::load(&self.repo_path);
    cache.clear();
    self.refresh_branches();
}
```

**Step 3: Update help.rs**

Add to `HELP_TEXT`:

```
R       Force recheck (clear cache)
```

**Step 4: Build and run tests**

Run: `cargo build && cargo test`
Expected: All pass

**Step 5: Commit**

```bash
git add src/git/cache.rs src/app.rs src/ui/help.rs
git commit -m "BL-004: Add force recheck command (R key clears cache)"
```

---

### Task 3: BL-012 — Checkout Branch with Stash

**Files:**
- Modify: `src/types.rs` (add Checkout action)
- Modify: `src/git/operations.rs` (add checkout + stash functions)
- Modify: `src/app.rs` (add c keybinding, handle checkout action)
- Modify: `src/ui/help.rs` (add c keybinding)
- Test: `tests/integration.rs` (add checkout test)

**Step 1: Write the failing test**

Add to `tests/integration.rs`:

```rust
#[test]
fn test_checkout_branch() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Create a feature branch
    run_git(dir, &["branch", "feature-checkout"]);

    // Checkout feature branch
    let result = operations::checkout_branch(dir, "feature-checkout", false);
    assert!(result.success, "checkout should succeed: {}", result.message);

    // Verify HEAD points to feature-checkout
    let repo = git2::Repository::open(dir).unwrap();
    let head = repo.head().unwrap();
    let branch_name = head.shorthand().unwrap();
    assert_eq!(branch_name, "feature-checkout");
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test test_checkout_branch -- --nocapture`
Expected: FAIL — `checkout_branch` doesn't exist

**Step 3: Add Checkout variant to BranchAction**

In `src/types.rs`:

```rust
pub enum BranchAction {
    DeleteLocal,
    DeleteLocalAndRemote,
    Checkout,
}

impl BranchAction {
    pub fn label(&self) -> &'static str {
        match self {
            BranchAction::DeleteLocal => "Delete local",
            BranchAction::DeleteLocalAndRemote => "Delete local + remote",
            BranchAction::Checkout => "Checkout",
        }
    }
}
```

**Step 4: Add checkout_branch to operations.rs**

In `src/git/operations.rs`:

```rust
/// Checkout a branch, optionally stashing and unstashing.
pub fn checkout_branch(repo_path: &Path, branch_name: &str, stash: bool) -> OperationResult {
    if stash {
        let stash_output = Command::new("git")
            .current_dir(repo_path)
            .args(["stash", "push", "-m", "git-bm auto-stash"])
            .output();
        if let Ok(output) = &stash_output {
            if !output.status.success() {
                return OperationResult {
                    branch_name: branch_name.to_string(),
                    action: BranchAction::Checkout,
                    success: false,
                    message: format!("Stash failed: {}", String::from_utf8_lossy(&output.stderr).trim()),
                };
            }
        }
    }

    let checkout = Command::new("git")
        .current_dir(repo_path)
        .args(["checkout", branch_name])
        .output();

    let result = match checkout {
        Ok(output) if output.status.success() => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::Checkout,
            success: true,
            message: format!("Checked out {}", branch_name),
        },
        Ok(output) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::Checkout,
            success: false,
            message: format!("Checkout failed: {}", String::from_utf8_lossy(&output.stderr).trim()),
        },
        Err(e) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::Checkout,
            success: false,
            message: format!("Failed to run git: {}", e),
        },
    };

    if stash && result.success {
        let _ = Command::new("git")
            .current_dir(repo_path)
            .args(["stash", "pop"])
            .output();
    }

    result
}
```

**Step 5: Run test to verify it passes**

Run: `cargo test test_checkout_branch -- --nocapture`
Expected: PASS

**Step 6: Wire checkout into app.rs**

In `handle_branch_list_key`, add:

```rust
KeyCode::Char('c') => {
    let branch = &self.branches[self.cursor];
    if !branch.is_current && !branch.is_base {
        self.view = View::Confirm {
            action: BranchAction::Checkout,
        };
    }
}
```

In `execute_action`, add the Checkout arm. Checkout operates on the cursor branch (not selection):

```rust
BranchAction::Checkout => {
    let branch_name = &self.branches[self.cursor].name;
    let needs_stash = !self.working_tree_status.is_clean();
    let result = operations::checkout_branch(&self.repo_path, branch_name, needs_stash);
    self.results.push(result);
}
```

**Step 7: Update help.rs**

Add to `HELP_TEXT`:

```
c       Checkout cursor branch
```

**Step 8: Run full test suite**

Run: `cargo test`
Expected: All pass

**Step 9: Commit**

```bash
git add src/types.rs src/git/operations.rs src/app.rs src/ui/help.rs tests/integration.rs
git commit -m "BL-012: Checkout branch under cursor with auto-stash"
```

---

## Phase 2 — Column Layout & Core Features

### Task 4: BL-017 — Column Layout Redesign (Table Widget)

**Files:**
- Modify: `src/ui/branch_list.rs` (major rewrite — List → Table)

This is the biggest single change. Replace the manual `List` + `Span` assembly with ratatui `Table`.

**Step 1: Rewrite branch_list.rs to use Table**

Replace the entire items/list construction with:

```rust
use ratatui::widgets::{Cell, Row, Table};

// Column widths
let widths = [
    Constraint::Length(5),   // checkbox "  [x]"
    Constraint::Min(20),     // branch name
    Constraint::Length(14),  // age
    Constraint::Length(6),   // ahead/behind
    Constraint::Length(14),  // merge status
];

let header = Row::new(vec![
    Cell::from(""),
    Cell::from("Branch").style(theme::TITLE_STYLE),
    Cell::from("Age").style(theme::TITLE_STYLE),
    Cell::from("↑↓").style(theme::TITLE_STYLE),
    Cell::from("Status").style(theme::TITLE_STYLE),
]);

let rows: Vec<Row> = app.branches.iter().enumerate().map(|(i, branch)| {
    let is_selected = app.selected[i];

    let checkbox = if branch.is_base || branch.is_current {
        "    ".to_string()
    } else if is_selected {
        " [x]".to_string()
    } else {
        " [ ]".to_string()
    };
    let checkbox_style = if is_selected { theme::SELECTED_STYLE } else { theme::SECONDARY_TEXT };

    let name = format!("{}{}", if branch.is_current { "* " } else { "  " }, branch.name);
    let name_style = if branch.is_current {
        theme::CURRENT_BRANCH_STYLE
    } else if is_selected {
        theme::SELECTED_STYLE
    } else {
        theme::PRIMARY_TEXT
    };

    let age = branch.age_display();

    let ab = match (branch.ahead, branch.behind) {
        (Some(a), Some(b)) if a > 0 || b > 0 => {
            let mut p = Vec::new();
            if a > 0 { p.push(format!("↑{}", a)); }
            if b > 0 { p.push(format!("↓{}", b)); }
            p.join("")
        }
        _ => String::new(),
    };

    let (status_text, status_style) = match branch.merge_status {
        MergeStatus::Merged => ("merged", theme::MERGED_STYLE),
        MergeStatus::SquashMerged => ("squash-merged", theme::SQUASH_MERGED_STYLE),
        MergeStatus::Unmerged => ("unmerged", theme::UNMERGED_STYLE),
    };

    Row::new(vec![
        Cell::from(checkbox).style(checkbox_style),
        Cell::from(name).style(name_style),
        Cell::from(age).style(theme::SECONDARY_TEXT),
        Cell::from(ab).style(theme::SECONDARY_TEXT),
        Cell::from(status_text).style(status_style),
    ])
}).collect();

let table = Table::new(rows, widths)
    .header(header)
    .block(block)
    .row_highlight_style(theme::CURSOR_STYLE);
```

Use `TableState` for cursor/scroll management instead of manual offset calculation. Store `table_state: TableState` in `App`.

**Step 2: Add TableState to App**

In `src/app.rs`, add `use ratatui::widgets::TableState;` and add field:

```rust
pub table_state: TableState,
```

Initialize in `new()`: `table_state: TableState::default().with_selected(0)`.

Update cursor movement to sync `table_state.select(Some(self.cursor))`.

**Step 3: Render with StatefulWidget**

In `branch_list.rs`, use `frame.render_stateful_widget(table, main_area, &mut app.table_state)` instead of `frame.render_widget`. This requires `app: &mut App` — update the draw signature.

Note: This cascades to `render.rs` and all draw functions needing `&mut App`. Update signatures accordingly.

**Step 4: Build and test**

Run: `cargo build && cargo test`
Expected: All pass (no logic change, just rendering change)

**Step 5: Commit**

```bash
git add src/ui/branch_list.rs src/app.rs src/ui/render.rs src/ui/confirm.rs src/ui/help.rs src/ui/results.rs
git commit -m "BL-017: Rewrite branch list with Table widget and column layout"
```

---

### Task 5: BL-016 — Base Branch Pinned at Top

**Files:**
- Modify: `src/ui/branch_list.rs` (sort pinned rows first)
- Modify: `src/app.rs` (cursor skip logic)

**Step 1: Sort branches with pinned at top**

In `branch_list.rs` (or in `App::refresh_branches` / `App::new`), sort:

```rust
// After loading branches, re-sort to pin base + current at top
self.branches.sort_by(|a, b| {
    let pin_a = if a.is_base { 0 } else if a.is_current { 1 } else { 2 };
    let pin_b = if b.is_base { 0 } else if b.is_current { 1 } else { 2 };
    pin_a.cmp(&pin_b).then(b.last_commit_date.cmp(&a.last_commit_date))
});
```

**Step 2: Cursor skip logic**

In `handle_branch_list_key`, modify up/down movement to skip pinned rows:

```rust
KeyCode::Char('j') | KeyCode::Down => {
    let mut next = self.cursor + 1;
    while next < len && (self.branches[next].is_base || self.branches[next].is_current) {
        next += 1;
    }
    if next < len {
        self.cursor = next;
    }
}
```

Similarly for up movement. Initialize cursor to first non-pinned row.

**Step 3: Build and test**

Run: `cargo build && cargo test`

**Step 4: Commit**

```bash
git add src/app.rs src/ui/branch_list.rs
git commit -m "BL-016: Pin base and current branch at top of list"
```

---

### Task 6: BL-002 — Status Bar with Progress

**Files:**
- Modify: `src/app.rs` (add squash progress tracking)
- Modify: `src/ui/branch_list.rs` (rich status bar)

**Step 1: Add progress tracking to App**

In `src/app.rs`, add fields:

```rust
pub squash_checked: usize,
pub squash_total: usize,
```

Initialize `squash_total` to `candidates.len()` in `new()` and `refresh_branches()`. Increment `squash_checked` in `drain_squash_rx` on each `Ok(result)`.

**Step 2: Update status bar in branch_list.rs**

Replace the simple status bar with:

```rust
let merged_count = app.branches.iter().filter(|b| b.merge_status == MergeStatus::Merged).count();
let squash_count = app.branches.iter().filter(|b| b.merge_status == MergeStatus::SquashMerged).count();

let progress = if app.squash_total > 0 && app.squash_checked < app.squash_total {
    format!(" | checking {}/{}", app.squash_checked, app.squash_total)
} else {
    String::new()
};

let status_text = format!(
    " {} branches | {} selected | {} merged | {} squashed{}  — [d]el [D]el+remote [c]heckout [f]etch [?]help [q]uit",
    total, selected_count, merged_count, squash_count, progress
);
```

For the progress fill, split the status bar into two spans: one with a green background proportional to progress, one with the default background.

**Step 3: Build and test**

Run: `cargo build && cargo test`

**Step 4: Commit**

```bash
git add src/app.rs src/ui/branch_list.rs
git commit -m "BL-002: Status bar with branch counts and squash-check progress"
```

---

### Task 7: BL-005 — Fetch and Fetch --Prune

**Files:**
- Modify: `src/git/operations.rs` (add fetch functions)
- Modify: `src/app.rs` (add f/F keybindings)
- Modify: `src/ui/help.rs` (add f/F)

**Step 1: Add fetch operations**

In `src/git/operations.rs`:

```rust
/// Run git fetch in the given repo.
pub fn fetch(repo_path: &Path) -> OperationResult {
    run_fetch(repo_path, false)
}

/// Run git fetch --prune in the given repo.
pub fn fetch_prune(repo_path: &Path) -> OperationResult {
    run_fetch(repo_path, true)
}

fn run_fetch(repo_path: &Path, prune: bool) -> OperationResult {
    let mut args = vec!["fetch"];
    if prune {
        args.push("--prune");
    }
    match Command::new("git").current_dir(repo_path).args(&args).output() {
        Ok(output) if output.status.success() => OperationResult {
            branch_name: String::new(),
            action: BranchAction::DeleteLocal, // reuse, or add a Fetch variant
            success: true,
            message: if prune { "Fetched with prune".into() } else { "Fetched".into() },
        },
        Ok(output) => OperationResult {
            branch_name: String::new(),
            action: BranchAction::DeleteLocal,
            success: false,
            message: format!("Fetch failed: {}", String::from_utf8_lossy(&output.stderr).trim()),
        },
        Err(e) => OperationResult {
            branch_name: String::new(),
            action: BranchAction::DeleteLocal,
            success: false,
            message: format!("Failed to run git: {}", e),
        },
    }
}
```

Note: Consider adding `BranchAction::Fetch` and `BranchAction::FetchPrune` variants to `types.rs` instead of reusing `DeleteLocal`. This keeps the results screen accurate.

**Step 2: Wire into app.rs**

```rust
KeyCode::Char('f') => {
    let result = operations::fetch(&self.repo_path);
    self.results.push(result);
    self.view = View::Results;
}
KeyCode::Char('F') => {
    let result = operations::fetch_prune(&self.repo_path);
    self.results.push(result);
    self.view = View::Results;
}
```

**Step 3: Update help.rs**

```
f       Fetch
F       Fetch + prune
```

**Step 4: Build and test**

Run: `cargo build && cargo test`

**Step 5: Commit**

```bash
git add src/types.rs src/git/operations.rs src/app.rs src/ui/help.rs
git commit -m "BL-005: Add git fetch and fetch --prune support"
```

---

### Task 8: BL-010 — Delete Branch Under Cursor

**Files:**
- Modify: `src/app.rs` (add x keybinding, cursor-based delete)
- Modify: `src/ui/help.rs` (add x)
- Modify: `src/ui/confirm.rs` (handle single-branch confirm for cursor)

**Step 1: Add x keybinding**

In `app.rs` `handle_branch_list_key`:

```rust
KeyCode::Char('x') => {
    let branch = &self.branches[self.cursor];
    if !branch.is_base && !branch.is_current {
        self.view = View::Confirm {
            action: BranchAction::DeleteLocal,
        };
    }
}
```

**Step 2: Update execute_action for cursor-based delete**

Modify `execute_action` to handle the case where no branches are selected but the cursor branch should be deleted. When `action == DeleteLocal` and no selection, use cursor branch:

```rust
let target_branches: Vec<String> = if self.has_selection() {
    self.selected_branch_names().iter().map(|s| s.to_string()).collect()
} else {
    vec![self.branches[self.cursor].name.clone()]
};
```

**Step 3: Update confirm.rs to show cursor branch when no selection**

In `confirm.rs`, check if selection exists; if not, show cursor branch name.

**Step 4: Update help.rs**

```
x       Delete cursor branch
```

**Step 5: Build and test**

Run: `cargo build && cargo test`

**Step 6: Commit**

```bash
git add src/app.rs src/ui/help.rs src/ui/confirm.rs
git commit -m "BL-010: Delete branch under cursor (x key)"
```

---

### Task 9: BL-015 — ENTER Key Operations Menu

**Files:**
- Create: `src/ui/menu.rs`
- Modify: `src/ui/mod.rs` (register menu module)
- Modify: `src/app.rs` (add Menu view, ENTER key, menu navigation)
- Modify: `src/ui/render.rs` (render menu overlay)

**Step 1: Create menu.rs**

```rust
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem};

use crate::app::App;
use super::theme;

pub struct MenuItem {
    pub label: String,
    pub enabled: bool,
    pub reason: Option<String>, // why disabled
}

pub fn draw(frame: &mut Frame, app: &App, items: &[MenuItem], menu_cursor: usize) {
    let area = frame.area();

    // Position the menu near the cursor row
    let menu_width = 35u16;
    let menu_height = (items.len() as u16 + 2).min(area.height);

    // Anchor to cursor row position (right side of screen)
    let cursor_screen_row = app.cursor.saturating_sub(app.list_scroll_offset) as u16 + 2; // +2 for border+header
    let y = cursor_screen_row.min(area.height.saturating_sub(menu_height));
    let x = area.width.saturating_sub(menu_width + 2);

    let rect = Rect::new(x, y, menu_width, menu_height);

    let list_items: Vec<ListItem> = items.iter().enumerate().map(|(i, item)| {
        let prefix = if i == menu_cursor { "▸ " } else { "  " };
        let style = if item.enabled {
            if i == menu_cursor { theme::PRIMARY_TEXT } else { Style::default() }
        } else {
            theme::SECONDARY_TEXT
        };
        let text = if let Some(reason) = &item.reason {
            format!("{}{} ({})", prefix, item.label, reason)
        } else {
            format!("{}{}", prefix, item.label)
        };
        ListItem::new(text).style(style)
    }).collect();

    let block = Block::default()
        .title("Actions")
        .title_style(theme::TITLE_STYLE)
        .borders(Borders::ALL);

    let list = List::new(list_items).block(block);

    frame.render_widget(Clear, rect);
    frame.render_widget(list, rect);
}
```

**Step 2: Add Menu view variant**

In `app.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum View {
    BranchList,
    Confirm { action: BranchAction },
    Results,
    Help,
    Menu { cursor: usize },
}
```

**Step 3: Build menu items based on context**

In `app.rs`, add method:

```rust
pub fn build_menu_items(&self) -> Vec<menu::MenuItem> {
    let branch = &self.branches[self.cursor];
    let dirty = !self.working_tree_status.is_clean();

    vec![
        MenuItem { label: "Checkout".into(), enabled: !branch.is_current && !dirty, reason: if dirty { Some("dirty tree".into()) } else if branch.is_current { Some("current".into()) } else { None } },
        MenuItem { label: "Delete local".into(), enabled: !branch.is_base && !branch.is_current, reason: if branch.is_base { Some("base".into()) } else if branch.is_current { Some("current".into()) } else { None } },
        MenuItem { label: "Delete local + remote".into(), enabled: !branch.is_base && !branch.is_current, reason: None },
        // More items added in Phase 3 (merge, rebase, worktree)
    ]
}
```

**Step 4: Wire ENTER key and menu navigation**

```rust
KeyCode::Enter => {
    if !self.branches[self.cursor].is_base {
        self.view = View::Menu { cursor: 0 };
    }
}
```

Add `handle_menu_key` method:

```rust
fn handle_menu_key(&mut self, code: KeyCode) {
    let View::Menu { cursor: ref mut menu_cursor } = self.view else { return };
    let items = self.build_menu_items();
    match code {
        KeyCode::Char('j') | KeyCode::Down => {
            if *menu_cursor + 1 < items.len() { *menu_cursor += 1; }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if *menu_cursor > 0 { *menu_cursor -= 1; }
        }
        KeyCode::Enter => {
            let item = &items[*menu_cursor];
            if item.enabled {
                // Map menu item to action, transition to Confirm
                let action = match menu_cursor {
                    0 => BranchAction::Checkout,
                    1 => BranchAction::DeleteLocal,
                    2 => BranchAction::DeleteLocalAndRemote,
                    _ => return,
                };
                self.view = View::Confirm { action };
            }
        }
        KeyCode::Esc | KeyCode::Char('q') => {
            self.view = View::BranchList;
        }
        _ => {}
    }
}
```

**Step 5: Update render.rs**

```rust
View::Menu { .. } => {
    branch_list::draw(frame, app);
    let items = app.build_menu_items();
    let View::Menu { cursor } = &app.view else { unreachable!() };
    menu::draw(frame, app, &items, *cursor);
}
```

**Step 6: Register module in ui/mod.rs**

Add `pub mod menu;`

**Step 7: Build and test**

Run: `cargo build && cargo test`

**Step 8: Commit**

```bash
git add src/ui/menu.rs src/ui/mod.rs src/app.rs src/ui/render.rs
git commit -m "BL-015: Add ENTER key inline operations menu"
```

---

## Phase 3 — Enhanced Features

### Task 10: BL-014 — Symbol Set Selection

**Files:**
- Create: `src/ui/symbols.rs`
- Create: `src/config.rs`
- Modify: `src/cli.rs` (add --symbols flag)
- Modify: `src/app.rs` (store active symbol set)
- Modify: `src/ui/branch_list.rs` (use symbols from set)
- Modify: `src/ui/mod.rs` (register symbols module)
- Modify: `src/lib.rs` (register config module)
- Modify: `Cargo.toml` (add toml dep)

**Step 1: Add toml dependency**

In `Cargo.toml`, add: `toml = "0.8"`

**Step 2: Create symbols.rs**

```rust
#[derive(Debug, Clone)]
pub struct SymbolSet {
    pub checkbox_on: &'static str,
    pub checkbox_off: &'static str,
    pub cursor: &'static str,
    pub arrow_up: &'static str,
    pub arrow_down: &'static str,
    pub status_merged: &'static str,
    pub status_squash: &'static str,
    pub status_unmerged: &'static str,
    pub current_branch: &'static str,
    pub separator: &'static str,
}

pub const ASCII: SymbolSet = SymbolSet {
    checkbox_on: "[x]",
    checkbox_off: "[ ]",
    cursor: ">",
    arrow_up: "+",
    arrow_down: "-",
    status_merged: "merged",
    status_squash: "squash",
    status_unmerged: "unmerged",
    current_branch: "*",
    separator: "--",
};

pub const UNICODE: SymbolSet = SymbolSet {
    checkbox_on: "◉",
    checkbox_off: "◯",
    cursor: "❯",
    arrow_up: "↑",
    arrow_down: "↓",
    status_merged: "✓ merged",
    status_squash: "≈ squashed",
    status_unmerged: "✗ unmerged",
    current_branch: "●",
    separator: "•",
};

pub const POWERLINE: SymbolSet = SymbolSet {
    checkbox_on: "\u{f046}",  //  (nf-fa-check_square_o)
    checkbox_off: "\u{f096}", //  (nf-fa-square_o)
    cursor: "\u{e0b1}",       //  (powerline right arrow)
    arrow_up: "\u{f062}",     //  (nf-fa-arrow_up)
    arrow_down: "\u{f063}",   //  (nf-fa-arrow_down)
    status_merged: "\u{f00c} merged",   //  merged
    status_squash: "\u{f0e8} squashed", //  squashed
    status_unmerged: "\u{f00d} unmerged", //  unmerged
    current_branch: "\u{e0a0}", //  (git branch symbol)
    separator: "\u{e0b1}",
};

/// Auto-detect the best symbol set based on terminal.
pub fn detect() -> &'static SymbolSet {
    let term = std::env::var("TERM_PROGRAM").unwrap_or_default();
    match term.as_str() {
        "iTerm.app" | "WezTerm" | "kitty" | "Alacritty" => &POWERLINE,
        _ => &UNICODE,
    }
}

/// Parse a symbol set name from CLI/config.
pub fn from_name(name: &str) -> &'static SymbolSet {
    match name {
        "ascii" => &ASCII,
        "unicode" => &UNICODE,
        "powerline" => &POWERLINE,
        _ => detect(),
    }
}
```

**Step 3: Create config.rs**

```rust
use std::path::PathBuf;
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub symbols: Option<String>,
    #[serde(default)]
    pub theme: Option<String>,
    #[serde(default)]
    pub trim_strategy: Option<String>,
    #[serde(default)]
    pub trim_min_length: Option<usize>,
}

impl Config {
    pub fn load() -> Self {
        let path = config_path();
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let path = config_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(s) = toml::to_string_pretty(self) {
            let _ = std::fs::write(&path, s);
        }
    }
}

fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("git-bm")
        .join("config.toml")
}
```

Note: Add `dirs = "6"` to `Cargo.toml` for `dirs::config_dir()`.

**Step 4: Add --symbols CLI flag**

In `src/cli.rs`:

```rust
/// Symbol set: ascii, unicode, powerline (default: auto-detect)
#[arg(long)]
pub symbols: Option<String>,
```

**Step 5: Wire symbols into App and branch_list.rs**

Store the active `SymbolSet` reference in `App` and use it in `branch_list.rs` instead of hardcoded strings.

**Step 6: Build and test**

Run: `cargo build && cargo test`

**Step 7: Commit**

```bash
git add src/ui/symbols.rs src/config.rs src/ui/mod.rs src/lib.rs src/cli.rs src/app.rs src/ui/branch_list.rs Cargo.toml Cargo.lock
git commit -m "BL-014: Symbol set selection (ASCII/Unicode/Powerline) with auto-detect"
```

---

### Task 11: BL-022 — Responsive Width

**Files:**
- Modify: `src/ui/branch_list.rs` (responsive column widths and trimming)
- Modify: `src/types.rs` (add compact age_display)

**Step 1: Add compact age display**

In `src/types.rs`, add method to `BranchInfo`:

```rust
pub fn age_short(&self) -> String {
    let duration = Utc::now() - self.last_commit_date;
    let seconds = duration.num_seconds();
    if seconds < 60 { "now".into() }
    else if seconds < 3600 { format!("{}m", duration.num_minutes()) }
    else if seconds < 86400 { format!("{}h", duration.num_hours()) }
    else if seconds < 604800 { format!("{}d", duration.num_days()) }
    else if seconds < 2_592_000 { format!("{}w", duration.num_weeks()) }
    else if seconds < 31_536_000 { format!("{}mo", duration.num_days() / 30) }
    else { format!("{}y", duration.num_days() / 365) }
}
```

**Step 2: Implement progressive trimming**

In `branch_list.rs`, before building rows, calculate the available width and determine which columns to show and how to format them:

```rust
let width = main_area.width as usize;

enum WidthMode {
    Full,       // > 120 chars: all columns, full text
    Compact,    // 80-120: short ages, trimmed names
    Narrow,     // 60-80: drop ahead/behind, short status
    Minimal,    // < 60: name + status only
}

let mode = if width > 120 { WidthMode::Full }
    else if width > 80 { WidthMode::Compact }
    else if width > 60 { WidthMode::Narrow }
    else { WidthMode::Minimal };
```

Adjust column widths and content based on mode. Trim branch names using the user's configured strategy (from `Config`).

```rust
fn trim_name(name: &str, max_len: usize, strategy: &str) -> String {
    if name.len() <= max_len { return name.to_string(); }
    match strategy {
        "start" => format!("…{}", &name[name.len() - max_len + 1..]),
        "middle" => {
            let half = (max_len - 1) / 2;
            format!("{}…{}", &name[..half], &name[name.len() - half..])
        }
        _ => format!("{}…", &name[..max_len - 1]),  // "end" default
    }
}
```

**Step 3: Build and test**

Run: `cargo build && cargo test`

**Step 4: Commit**

```bash
git add src/ui/branch_list.rs src/types.rs
git commit -m "BL-022: Responsive width handling with progressive trimming"
```

---

### Task 12: BL-024 — Column Sorting

**Files:**
- Modify: `src/app.rs` (add sort state and s/S keybindings)
- Modify: `src/ui/branch_list.rs` (sort indicators, apply sort)

**Step 1: Add sort state to App**

```rust
pub sort_column: Option<usize>,
pub sort_ascending: bool,
```

**Step 2: Add sort keybindings**

```rust
KeyCode::Char('s') => {
    self.sort_column = Some(match self.sort_column {
        Some(c) => (c + 1) % 4, // cycle: name, age, ahead, status
        None => 0,
    });
    self.sort_ascending = true;
    self.apply_sort();
}
KeyCode::Char('S') => {
    self.sort_ascending = !self.sort_ascending;
    self.apply_sort();
}
```

**Step 3: Implement apply_sort**

```rust
fn apply_sort(&mut self) {
    let col = match self.sort_column {
        Some(c) => c,
        None => return,
    };
    let asc = self.sort_ascending;

    // Partition: pinned rows first, then sortable rows
    let pin_count = self.branches.iter().take_while(|b| b.is_base || b.is_current).count();
    let sortable = &mut self.branches[pin_count..];

    sortable.sort_by(|a, b| {
        let ord = match col {
            0 => a.name.cmp(&b.name),
            1 => a.last_commit_date.cmp(&b.last_commit_date),
            2 => a.ahead.unwrap_or(0).cmp(&b.ahead.unwrap_or(0)),
            3 => {
                let rank = |s: &MergeStatus| match s {
                    MergeStatus::Merged => 0,
                    MergeStatus::SquashMerged => 1,
                    MergeStatus::Unmerged => 2,
                };
                rank(&a.merge_status).cmp(&rank(&b.merge_status))
            }
            _ => std::cmp::Ordering::Equal,
        };
        if asc { ord } else { ord.reverse() }
    });

    // Rebuild selected vec to match new order
    self.selected = vec![false; self.branches.len()];
}
```

**Step 4: Show sort indicator in header**

In `branch_list.rs`, when building the header row, append `▲` or `▼` to the active column name.

**Step 5: Update help.rs**

```
s       Cycle sort column
S       Reverse sort order
```

**Step 6: Build and test**

Run: `cargo build && cargo test`

**Step 7: Commit**

```bash
git add src/app.rs src/ui/branch_list.rs src/ui/help.rs
git commit -m "BL-024: Column sorting with s/S keybindings"
```

---

### Task 13: BL-008 — Fast-Forward Update

**Files:**
- Modify: `src/git/operations.rs` (add fast_forward function)
- Modify: `src/types.rs` (add FastForward action variant)
- Modify: `src/app.rs` (wire into menu)
- Modify: `src/ui/menu.rs` (add menu item)

**Step 1: Add FastForward operation**

In `types.rs`, add `BranchAction::FastForward`.

In `operations.rs`:

```rust
pub fn fast_forward(repo_path: &Path, branch_name: &str) -> OperationResult {
    match Command::new("git")
        .current_dir(repo_path)
        .args(["fetch", "origin", &format!("{}:{}", branch_name, branch_name)])
        .output()
    {
        Ok(output) if output.status.success() => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::FastForward,
            success: true,
            message: "Fast-forwarded to remote".to_string(),
        },
        Ok(output) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::FastForward,
            success: false,
            message: format!("Cannot fast-forward: {}", String::from_utf8_lossy(&output.stderr).trim()),
        },
        Err(e) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::FastForward,
            success: false,
            message: format!("Failed to run git: {}", e),
        },
    }
}
```

**Step 2: Add to operations menu and execute_action**

**Step 3: Build and test**

Run: `cargo build && cargo test`

**Step 4: Commit**

```bash
git add src/types.rs src/git/operations.rs src/app.rs src/ui/menu.rs
git commit -m "BL-008: Fast-forward local branches to remote"
```

---

### Task 14: BL-011 — Merge Branch into Base

**Files:**
- Modify: `src/git/operations.rs` (add merge function)
- Modify: `src/types.rs` (add Merge/SquashMerge action variants)
- Modify: `src/app.rs` (wire merge from menu, stash handling)

**Step 1: Add merge operation**

In `operations.rs`:

```rust
pub fn merge_branch(repo_path: &Path, branch_name: &str, base: &str, squash: bool, stash: bool) -> Vec<OperationResult> {
    let mut results = Vec::new();

    if stash {
        let stash_out = Command::new("git").current_dir(repo_path).args(["stash", "push", "-m", "git-bm auto-stash"]).output();
        if let Ok(o) = &stash_out {
            if !o.status.success() {
                results.push(OperationResult { branch_name: branch_name.to_string(), action: BranchAction::Merge, success: false, message: "Stash failed".into() });
                return results;
            }
        }
    }

    // Checkout base
    let co = Command::new("git").current_dir(repo_path).args(["checkout", base]).output();
    if let Ok(o) = &co {
        if !o.status.success() {
            results.push(OperationResult { branch_name: branch_name.to_string(), action: BranchAction::Merge, success: false, message: format!("Checkout {} failed", base) });
            return results;
        }
    }

    // Merge
    let mut merge_args = vec!["merge"];
    if squash { merge_args.push("--squash"); }
    merge_args.push(branch_name);

    let merge_out = Command::new("git").current_dir(repo_path).args(&merge_args).output();
    match merge_out {
        Ok(o) if o.status.success() => {
            if squash {
                // Need to commit after squash merge
                let _ = Command::new("git").current_dir(repo_path).args(["commit", "-m", &format!("Squash merge {}", branch_name)]).output();
            }
            results.push(OperationResult { branch_name: branch_name.to_string(), action: BranchAction::Merge, success: true, message: if squash { "Squash merged".into() } else { "Merged".into() } });
        }
        Ok(o) => {
            results.push(OperationResult { branch_name: branch_name.to_string(), action: BranchAction::Merge, success: false, message: format!("Merge failed: {}", String::from_utf8_lossy(&o.stderr).trim()) });
        }
        Err(e) => {
            results.push(OperationResult { branch_name: branch_name.to_string(), action: BranchAction::Merge, success: false, message: format!("Failed: {}", e) });
        }
    }

    if stash {
        let _ = Command::new("git").current_dir(repo_path).args(["stash", "pop"]).output();
    }

    results
}
```

**Step 2: Add Merge variant to BranchAction, wire into menu**

**Step 3: Build and test**

**Step 4: Commit**

```bash
git commit -m "BL-011: Merge branch into base (regular or squash)"
```

---

### Task 15: BL-013 — Rebase onto Base

**Files:**
- Modify: `src/git/operations.rs` (add rebase function)
- Modify: `src/types.rs` (add Rebase action variant)

**Step 1: Add rebase operation**

```rust
pub fn rebase_branch(repo_path: &Path, branch_name: &str, base: &str, stash: bool) -> Vec<OperationResult> {
    let mut results = Vec::new();

    if stash {
        let _ = Command::new("git").current_dir(repo_path).args(["stash", "push", "-m", "git-bm auto-stash"]).output();
    }

    // Checkout the branch
    let co = Command::new("git").current_dir(repo_path).args(["checkout", branch_name]).output();
    if let Ok(o) = &co {
        if !o.status.success() {
            results.push(OperationResult { branch_name: branch_name.to_string(), action: BranchAction::Rebase, success: false, message: "Checkout failed".into() });
            return results;
        }
    }

    // Rebase
    let rebase = Command::new("git").current_dir(repo_path).args(["rebase", base]).output();
    match rebase {
        Ok(o) if o.status.success() => {
            results.push(OperationResult { branch_name: branch_name.to_string(), action: BranchAction::Rebase, success: true, message: format!("Rebased onto {}", base) });
        }
        Ok(o) => {
            // Abort the rebase on failure
            let _ = Command::new("git").current_dir(repo_path).args(["rebase", "--abort"]).output();
            results.push(OperationResult { branch_name: branch_name.to_string(), action: BranchAction::Rebase, success: false, message: format!("Rebase conflicts: {}", String::from_utf8_lossy(&o.stderr).trim()) });
        }
        Err(e) => {
            results.push(OperationResult { branch_name: branch_name.to_string(), action: BranchAction::Rebase, success: false, message: format!("Failed: {}", e) });
        }
    }

    if stash {
        let _ = Command::new("git").current_dir(repo_path).args(["stash", "pop"]).output();
    }

    results
}
```

**Step 2: Wire into menu, build, test, commit**

```bash
git commit -m "BL-013: Rebase branch onto base"
```

---

### Task 16: BL-007 — Create Worktrees

**Files:**
- Modify: `src/git/operations.rs` (add create_worktree function)
- Modify: `src/types.rs` (add Worktree action variant)

**Step 1: Add worktree operation**

```rust
pub fn create_worktree(repo_path: &Path, branch_name: &str, worktree_path: Option<&str>) -> OperationResult {
    let sanitized = branch_name.replace('/', "-");
    let default_path = repo_path.join(".worktrees").join(&sanitized);
    let path = worktree_path
        .map(|p| std::path::PathBuf::from(p))
        .unwrap_or(default_path);

    match Command::new("git")
        .current_dir(repo_path)
        .args(["worktree", "add", path.to_str().unwrap_or(""), branch_name])
        .output()
    {
        Ok(o) if o.status.success() => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::Worktree,
            success: true,
            message: format!("Worktree at {}", path.display()),
        },
        Ok(o) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::Worktree,
            success: false,
            message: format!("Failed: {}", String::from_utf8_lossy(&o.stderr).trim()),
        },
        Err(e) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::Worktree,
            success: false,
            message: format!("Failed: {}", e),
        },
    }
}
```

**Step 2: Wire into menu, build, test, commit**

```bash
git commit -m "BL-007: Create worktrees for branches"
```

---

### Task 17: BL-025 — Tag Management Screen

**Files:**
- Create: `src/git/tags.rs`
- Create: `src/ui/tag_list.rs`
- Modify: `src/git/mod.rs` (register tags module)
- Modify: `src/ui/mod.rs` (register tag_list module)
- Modify: `src/app.rs` (add Tab enum, tag state, Tab key)
- Modify: `src/types.rs` (add TagInfo struct)
- Modify: `src/ui/render.rs` (dispatch to tag_list)
- Modify: `src/ui/branch_list.rs` (render tab bar)

**Step 1: Add TagInfo to types.rs**

```rust
#[derive(Debug, Clone)]
pub struct TagInfo {
    pub name: String,
    pub date: DateTime<Utc>,
    pub commit_hash: String,
    pub message: Option<String>, // for annotated tags
    pub is_remote: bool,
}
```

**Step 2: Create git/tags.rs**

```rust
use anyhow::Result;
use chrono::DateTime;
use git2::Repository;

use crate::types::TagInfo;

pub fn list_tags(repo: &Repository) -> Result<Vec<TagInfo>> {
    let mut tags = Vec::new();
    repo.tag_foreach(|oid, name| {
        let name = String::from_utf8_lossy(name)
            .strip_prefix("refs/tags/")
            .unwrap_or(&String::from_utf8_lossy(name))
            .to_string();

        if let Ok(obj) = repo.find_object(oid, None) {
            let (date, message) = if let Ok(tag) = obj.as_tag() {
                let d = tag.tagger().map(|t| DateTime::from_timestamp(t.when().seconds(), 0).unwrap_or_default()).unwrap_or_default();
                (d, tag.message().map(|m| m.to_string()))
            } else if let Ok(commit) = obj.peel_to_commit() {
                let d = DateTime::from_timestamp(commit.committer().when().seconds(), 0).unwrap_or_default();
                (d, None)
            } else {
                (chrono::Utc::now(), None)
            };

            tags.push(TagInfo {
                name,
                date,
                commit_hash: oid.to_string()[..8].to_string(),
                message,
                is_remote: false,
            });
        }
        true
    })?;
    tags.sort_by(|a, b| b.date.cmp(&a.date));
    Ok(tags)
}
```

**Step 3: Add Tab enum and tag state to App**

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tab {
    Branches,
    Tags,
}

// In App struct:
pub tab: Tab,
pub tags: Vec<TagInfo>,
pub tag_cursor: usize,
```

**Step 4: Create ui/tag_list.rs**

Similar layout to branch_list but with tag-specific columns (name, date, commit, message). Uses Table widget.

**Step 5: Wire Tab key**

In event handling, `KeyCode::Tab` switches between `Tab::Branches` and `Tab::Tags`. When switching to Tags, call `list_tags`. In render.rs, dispatch based on tab.

**Step 6: Add tab bar to branch_list.rs and tag_list.rs**

Render a horizontal bar at the top: `[Branches] Tags` or `Branches [Tags]` with the active tab highlighted.

**Step 7: Add tag operations (create, delete, push)**

Keybindings in tag view: `t` create, `d` delete, `p` push. Each shells out to git CLI.

**Step 8: Build and test**

Run: `cargo build && cargo test`

**Step 9: Commit**

```bash
git add src/git/tags.rs src/ui/tag_list.rs src/git/mod.rs src/ui/mod.rs src/app.rs src/types.rs src/ui/render.rs src/ui/branch_list.rs
git commit -m "BL-025: Tag management screen with tab navigation"
```

---

## Phase 4 — Polish & Customization

### Task 18: BL-019 — Branch Prefix Coloring

**Files:**
- Modify: `src/ui/branch_list.rs` (color branch name by prefix)

**Step 1: Add prefix color map**

```rust
fn prefix_color(name: &str) -> Option<Color> {
    if let Some(prefix) = name.split('/').next() {
        match prefix {
            "fix" | "bugfix" | "hotfix" => Some(Color::Red),
            "feat" | "feature" => Some(Color::Green),
            "chore" => Some(Color::Yellow),
            "docs" | "doc" => Some(Color::Blue),
            "refactor" => Some(Color::Magenta),
            "test" | "tests" => Some(Color::Cyan),
            _ => None,
        }
    } else {
        None
    }
}
```

**Step 2: Apply in branch name cell**

When rendering the branch name cell, if the name contains `/`, split into prefix and rest, color the prefix with the mapped color.

**Step 3: Build, test, commit**

```bash
git commit -m "BL-019: Color branch name prefixes by convention"
```

---

### Task 19: BL-020 — Age-Based Coloring

**Files:**
- Modify: `src/ui/branch_list.rs` (color age cell by age)

**Step 1: Add age color function**

```rust
fn age_color(date: &DateTime<Utc>) -> Color {
    let days = (Utc::now() - *date).num_days();
    if days < 7 { Color::Green }
    else if days < 30 { Color::Yellow }
    else if days < 90 { Color::Indexed(208) } // orange
    else { Color::Red }
}
```

**Step 2: Apply to age cell style**

Replace `theme::SECONDARY_TEXT` on the age cell with `Style::new().fg(age_color(&branch.last_commit_date))`.

**Step 3: Build, test, commit**

```bash
git commit -m "BL-020: Age-based coloring for branch age column"
```

---

### Task 20: BL-018 — Selectable Color Themes

**Files:**
- Modify: `src/ui/theme.rs` (Theme struct with presets)
- Modify: `src/app.rs` (store current theme, T key to cycle)
- Modify: `src/config.rs` (persist theme choice)
- Modify: all `ui/*.rs` files (use theme from app context instead of constants)

**Step 1: Create Theme struct**

```rust
#[derive(Debug, Clone)]
pub struct Theme {
    pub name: &'static str,
    pub merged: Style,
    pub squash_merged: Style,
    pub unmerged: Style,
    pub primary_text: Style,
    pub secondary_text: Style,
    pub cursor: Style,
    pub cursor_prefix: Style,
    pub selected: Style,
    pub current_branch: Style,
    pub error: Style,
    pub dim: Style,
    pub status_bar: Style,
    pub title: Style,
}
```

**Step 2: Define presets**

```rust
pub const DARK: Theme = Theme { name: "dark", merged: Style::new().fg(Color::Green).add_modifier(Modifier::BOLD), /* ... */ };
pub const LIGHT: Theme = Theme { name: "light", /* lighter colors */ };
pub const SOLARIZED: Theme = Theme { name: "solarized", /* solarized palette */ };
pub const DRACULA: Theme = Theme { name: "dracula", /* dracula palette */ };

pub const ALL_THEMES: &[&Theme] = &[&DARK, &LIGHT, &SOLARIZED, &DRACULA];
```

**Step 3: Store theme in App, add T keybinding**

```rust
pub theme: &'static Theme,

KeyCode::Char('T') => {
    let idx = ALL_THEMES.iter().position(|t| t.name == self.theme.name).unwrap_or(0);
    self.theme = ALL_THEMES[(idx + 1) % ALL_THEMES.len()];
    // Save to config
    let mut config = Config::load();
    config.theme = Some(self.theme.name.to_string());
    config.save();
}
```

**Step 4: Replace all theme:: constant references**

All `ui/*.rs` draw functions need access to the theme. Pass it through the draw calls or add to a render context struct.

**Step 5: Build, test, commit**

```bash
git commit -m "BL-018: Selectable color themes (dark, light, solarized, dracula)"
```

---

### Task 21: BL-023 — GitHub PR# Column

**Files:**
- Create: `src/git/github.rs`
- Modify: `src/git/mod.rs` (register github module)
- Modify: `src/types.rs` (add pr_number to BranchInfo)
- Modify: `src/app.rs` (spawn background PR loader)
- Modify: `src/ui/branch_list.rs` (add PR# column)
- Modify: `Cargo.toml` (add ureq dep for HTTP)

**Step 1: Add ureq dependency**

In `Cargo.toml`: `ureq = { version = "3", features = ["json"] }`

**Step 2: Create git/github.rs**

```rust
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::sync::mpsc::{self, Receiver};
use std::thread;

pub struct PrInfo {
    pub branch_name: String,
    pub pr_number: u32,
}

/// Get GitHub auth token via `gh auth token`. Returns None if gh not installed or not authed.
fn get_gh_token() -> Option<String> {
    Command::new("gh")
        .args(["auth", "token"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

/// Get GitHub repo owner/name from remote URL.
fn get_repo_slug(repo_path: &Path) -> Option<String> {
    let output = Command::new("git")
        .current_dir(repo_path)
        .args(["remote", "get-url", "origin"])
        .output()
        .ok()?;
    if !output.status.success() { return None; }
    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // Parse git@github.com:owner/repo.git or https://github.com/owner/repo.git
    if let Some(rest) = url.strip_prefix("git@github.com:") {
        Some(rest.strip_suffix(".git").unwrap_or(rest).to_string())
    } else if let Some(rest) = url.strip_prefix("https://github.com/") {
        Some(rest.strip_suffix(".git").unwrap_or(rest).to_string())
    } else {
        None
    }
}

/// Spawn a background thread that fetches open PRs and matches them to branches.
pub fn spawn_pr_loader(repo_path: &Path, branch_names: Vec<String>) -> Option<Receiver<PrInfo>> {
    let token = get_gh_token()?;
    let slug = get_repo_slug(repo_path)?;
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        let url = format!("https://api.github.com/repos/{}/pulls?state=open&per_page=100", slug);
        let response = ureq::get(&url)
            .header("Authorization", &format!("Bearer {}", token))
            .header("Accept", "application/vnd.github.v3+json")
            .header("User-Agent", "git-branch-manager")
            .call();

        if let Ok(resp) = response {
            if let Ok(body) = resp.body_mut().read_to_string() {
                if let Ok(prs) = serde_json::from_str::<Vec<serde_json::Value>>(&body) {
                    let pr_map: HashMap<String, u32> = prs.iter().filter_map(|pr| {
                        let number = pr["number"].as_u64()? as u32;
                        let head_ref = pr["head"]["ref"].as_str()?.to_string();
                        Some((head_ref, number))
                    }).collect();

                    for name in &branch_names {
                        if let Some(&number) = pr_map.get(name) {
                            let _ = tx.send(PrInfo { branch_name: name.clone(), pr_number: number });
                        }
                    }
                }
            }
        }
    });

    Some(rx)
}
```

**Step 3: Add pr_number field to BranchInfo**

In `types.rs`: `pub pr_number: Option<u32>,`

**Step 4: Wire into App**

Similar to `squash_rx`, add `pr_rx: Option<Receiver<PrInfo>>`. Drain in event loop. Spawn in `main.rs` after TUI setup.

**Step 5: Add PR# column to branch_list.rs**

Add a column after merge status that shows `#123` if `pr_number` is set.

**Step 6: Build and test**

Run: `cargo build && cargo test`

**Step 7: Commit**

```bash
git add src/git/github.rs src/git/mod.rs src/types.rs src/app.rs src/main.rs src/ui/branch_list.rs Cargo.toml Cargo.lock
git commit -m "BL-023: GitHub PR# column with async loading via gh auth"
```

---

## Summary of New Dependencies

```toml
[dependencies]
toml = "0.8"    # config file parsing (BL-014)
dirs = "6"      # config dir detection (BL-014)
ureq = { version = "3", features = ["json"] }  # GitHub API (BL-023)
```

## Implementation Order with Parallelization Notes

**Can run as parallel agents (Phase 1):**
- Task 1 (BL-009): primarily touches branch.rs, types.rs
- Task 2 (BL-004): primarily touches cache.rs
- Task 3 (BL-012): primarily touches operations.rs

All three touch app.rs and help.rs — merge conflicts possible. Run in worktree isolation.

**Must be sequential (Phase 2):**
- Tasks 4-9: each builds on the previous, all touch branch_list.rs and app.rs

**Can partially parallelize (Phase 3):**
- Tasks 13-16 (operations) can run in parallel if in separate worktrees
- Tasks 10-12 (UI features) must be sequential

**Can partially parallelize (Phase 4):**
- Tasks 18-19 (coloring) can run in parallel
- Task 20 (themes) must be last in Phase 4 (changes all style references)
- Task 21 (PR#) is independent
