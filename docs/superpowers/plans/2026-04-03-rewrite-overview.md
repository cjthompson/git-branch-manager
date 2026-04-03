# git-branch-manager Full Rewrite — Overview

> **For agentic workers:** This is the overview plan. Each phase has its own detailed plan file. Use superpowers:subagent-driven-development or superpowers:executing-plans to implement each phase in order.

**Goal:** Rewrite git-branch-manager from a ~4000-line monolithic `app.rs` + duplicated view code into a modular codebase where all 4 views share a generic framework for navigation, selection, sorting, filtering, rendering, and key handling.

**Architecture:** A generic `ListState<T>` + `ViewDef` trait system replaces the current copy-paste-per-view approach. Each view defines its columns, context menu actions, and filter tokens declaratively. A single generic renderer handles all 4 views. The git layer is preserved mostly as-is since it's already well-structured.

**Tech Stack:** Rust, ratatui 0.30, crossterm 0.29, git2 0.20, clap 4, chrono 0.4, serde/serde_json, toml

---

## Phase Summary

| Phase | Plan File | Goal | Produces |
|-------|-----------|------|----------|
| 1 | `phase1-types-git.md` | Core types + git operations | All data types, git ops, config, CLI. `cargo test` passes. |
| 2 | `phase2-view-framework.md` | Generic view framework | `ListState<T>`, `ViewDef` trait, columns, sort, filter, nav, selection. All 4 view definitions. Unit-testable without a terminal. |
| 3 | `phase3-ui-rendering.md` | UI rendering layer | Generic list renderer, all overlay UIs, tab bar, status bar, toast. Renderable to a test buffer. |
| 4 | `phase4-app-integration.md` | App shell + wiring | `App` struct, event loop, channel management, key/mouse dispatch, `main.rs`. Running application. |

Each phase builds on the previous. Each produces working, testable software independently.

---

## Target File Structure

```
src/
├── main.rs                    # Entry point, CLI, startup sequence
├── lib.rs                     # Crate root, re-exports for integration tests
├── cli.rs                     # clap derive struct (--base, --list, --symbols)
├── config.rs                  # Config struct, TOML load/save
├── types.rs                   # All shared data types and enums
├── symbols.rs                 # SymbolSet (ASCII, Unicode, Powerline)
├── theme.rs                   # Theme struct + 4 theme definitions
│
├── git/                       # Git operations layer (Phase 1)
│   ├── mod.rs
│   ├── branch.rs              # Branch listing, base detection, phase-1 load
│   ├── merge_detection.rs     # Regular merge + squash-merge detection
│   ├── squash_loader.rs       # Background squash-merge checker
│   ├── cache.rs               # Squash-merge result cache
│   ├── operations.rs          # All git command execution
│   ├── worktree.rs            # Worktree listing + enrichment
│   ├── tags.rs                # Tag listing + operations
│   ├── github.rs              # GitHub PR types + fetching
│   ├── pr_loader.rs           # Background PR loader
│   └── status.rs              # Working tree status detection
│
├── view/                      # Generic view framework (Phase 2)
│   ├── mod.rs                 # ViewId enum, ViewItem trait, ViewDef trait
│   ├── list_state.rs          # ListState<T> — cursor, selection, sort, filter
│   ├── column.rs              # ColumnDef<T>, ColumnRenderContext
│   ├── filter.rs              # FilterSet, FilterTokenDef, apply_filters()
│   ├── branches.rs            # BranchesViewDef: columns, menu, filter tokens
│   ├── remotes.rs             # RemotesViewDef
│   ├── tags.rs                # TagsViewDef
│   └── worktrees.rs           # WorktreesViewDef
│
├── ui/                        # UI rendering layer (Phase 3)
│   ├── mod.rs
│   ├── render.rs              # Top-level draw() dispatcher
│   ├── list_render.rs         # Generic list/table renderer for any view
│   ├── tab_bar.rs             # Tab bar in block title
│   ├── status_bar.rs          # Bottom status bar + clickable items
│   ├── help.rs                # Help overlay
│   ├── menu.rs                # Context menu overlay
│   ├── confirm.rs             # Confirmation dialog overlay
│   ├── executing.rs           # Operation progress screen
│   ├── results.rs             # Operation results screen
│   ├── settings.rs            # Settings panel overlay
│   ├── filter_ui.rs           # Filter builder overlay (generic)
│   ├── search.rs              # Inline search bar
│   ├── toast.rs               # Toast notification system
│   └── shared.rs              # prefix_style, age_style, truncate, centered_rect
│
└── app.rs                     # App struct + event loop + dispatch (Phase 4)
                               # (~500-800 lines vs current ~4000)

tests/
└── integration.rs             # Git operation integration tests (ported in Phase 1)
```

---

## Core Architectural Decisions

### 1. Generic View Framework

The central innovation. All 4 views share these abstractions:

```rust
// Every list item implements this trait
trait ViewItem: Clone {
    fn display_name(&self) -> &str;
    fn is_pinned(&self) -> bool;
    fn merge_status(&self) -> Option<&MergeStatus>;
    fn last_commit_date(&self) -> &DateTime<Utc>;
    fn ahead(&self) -> Option<u32> { None }
    fn behind(&self) -> Option<u32> { None }
    fn pr_info(&self) -> Option<&PrInfo> { None }
}

// Each view declares its columns, menus, filters
trait ViewDef {
    type Item: ViewItem;
    fn id(&self) -> ViewId;
    fn columns(&self) -> &[ColumnDef<Self::Item>];
    fn context_menu(&self, item: &Self::Item) -> Vec<MenuItem>;
    fn filter_tokens(&self) -> &[FilterTokenDef];
}

// Generic state for any list view
struct ListState<T: ViewItem> {
    items: Vec<T>,
    cursor: usize,
    selected: Vec<bool>,
    sort_column: Option<usize>,
    sort_ascending: bool,
    search_query: String,
    search_active: bool,
    filter_query: String,
    table_state: TableState,
    // ... loading state, header_columns for mouse clicks
}
```

Navigation, selection, sorting, and filtering are free functions on `&mut ListState<T>`:
```rust
fn nav_down<T: ViewItem>(state: &mut ListState<T>) { ... }
fn select_toggle<T: ViewItem>(state: &mut ListState<T>) { ... }
fn apply_sort<T: ViewItem>(state: &mut ListState<T>, columns: &[ColumnDef<T>]) { ... }
fn apply_filters<T: ViewItem>(state: &mut ListState<T>, tokens: &[FilterTokenDef]) { ... }
```

### 2. Action Model

Two tiers, enforced consistently across all views:
- **View-level keys** (`d`, `D`, `p`, etc.): Act on all selected items (or cursor if none selected)
- **Context menu** (`Enter` / right-click): Acts on single cursor item only

### 3. Column-Based Filtering

Filter tokens are derived from column definitions. Each `FilterTokenDef` knows which column it applies to. The filter builder UI only shows tokens for columns present in the active view.

### 4. Background Threading

Same mpsc channel pattern as current code. Channels are drained generically in the event loop rather than with per-channel methods.

### 5. Overlay Stack

Overlays (Help, Menu, Confirm, Executing, Results, Settings, Filter) are separate from the active view. The `App` tracks: `active_view: ViewId` + `overlay: Option<Overlay>`.

---

## Phase Dependencies

```
Phase 1 (Types + Git) ─────────────┐
                                    ├──→ Phase 3 (UI Rendering)
Phase 2 (View Framework) ──────────┤
                                    └──→ Phase 4 (App Integration)
```

Phases 1 and 2 are independent of each other and **can be built in parallel**. Phases 3 and 4 depend on both Phase 1 and Phase 2.

---

## Testing Strategy

| Phase | Test Type | What |
|-------|-----------|------|
| 1 | Integration | Git operations against temp repos (port existing 47 tests) |
| 1 | Unit | Format helpers, cache, config parsing |
| 2 | Unit | Navigation, selection, sorting, filtering on mock data |
| 3 | Unit | Rendering to ratatui `TestBackend` buffer |
| 4 | Integration | Full app startup, basic event handling |

---

## Detailed Phase Plans

- [Phase 1: Core Types & Git Layer](./phase1-types-git.md)
- [Phase 2: Generic View Framework](./phase2-view-framework.md)
- [Phase 3: UI Rendering Layer](./phase3-ui-rendering.md)
- [Phase 4: App Shell & Integration](./phase4-app-integration.md)
