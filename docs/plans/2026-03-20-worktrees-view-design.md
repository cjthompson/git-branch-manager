# Worktrees View Design

**Date:** 2026-03-20

## Overview

Add a new `Worktrees` view that lists all active git worktrees with branch, age, ahead/behind, PR, and status columns — matching the column set of the branch list and remote branches views. Supports removing worktrees via a context menu.

## Data Model

New `WorktreeInfo` struct in `types.rs`:

```rust
pub struct WorktreeInfo {
    pub path: PathBuf,                    // absolute path to the worktree dir
    pub branch: Option<String>,           // None if detached HEAD
    pub is_main: bool,                    // true for the primary worktree
    pub commit_hash: String,              // HEAD SHA (short)
    pub wt_status: WorkingTreeStatus,     // staged/unstaged/untracked (existing type)
    pub age_date: DateTime<Utc>,          // newest mtime of changed files if dirty, else HEAD commit date
    // joined from BranchInfo after branch data loads:
    pub merge_status: MergeStatus,        // defaults to Unmerged until enriched
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
    pub pr: Option<PrStatus>,
}
```

**Age semantics:** if `wt_status` is dirty, run `git status --porcelain` in the worktree directory, collect the listed file paths, take the newest `mtime`. If clean, use the HEAD commit date.

Detached HEAD worktrees show `(detached)` in the branch column; a/b, PR, and merge status columns are left empty.

## Loading Architecture

Two-phase load gated by a new `load_worktrees_on_launch: Option<bool>` config setting (default `false`, mirrors `auto_fetch`).

### Phase 1 — Fast parse
Runs `git worktree list --porcelain`, then for each worktree:
- `git status --porcelain` to build `WorkingTreeStatus` and collect dirty file paths
- `stat` the dirty file paths to find the newest mtime (age if dirty)
- `git log -1 --format=%ct HEAD` for HEAD commit date (age if clean)

Produces `Vec<WorktreeInfo>` with branch-join fields defaulted (`merge_status: Unmerged`, ahead/behind/pr: `None`).

### Phase 2 — Branch enrichment
Background thread joins the loaded `Vec<BranchInfo>` and `PrMap` to fill in merge_status, ahead, behind, pr for each worktree whose `branch` is found in the branch list. Sends a `WorktreeEnrich` payload back via `mpsc::Receiver<WorktreeEnrich>`.

### Timing
- `load_worktrees_on_launch: true` → Phase 1 runs in the startup background thread alongside the initial branch load; Phase 2 runs after both complete.
- `load_worktrees_on_launch: false` (default) → Phase 1 runs when the user first opens the Worktrees view; Phase 2 runs immediately after Phase 1 completes.

### New `App` fields
```rust
pub worktrees: Vec<WorktreeInfo>,
pub worktree_cursor: usize,
pub worktree_table_state: TableState,
pub worktree_selected: Vec<bool>,
pub worktree_load_rx: Option<Receiver<WorktreeLoad>>,     // phase 1
pub worktree_enrich_rx: Option<Receiver<WorktreeEnrich>>, // phase 2
pub worktree_loading: bool,
```

## View & UI

New `View::Worktrees` variant in the `View` enum.

### Column layout (responsive, matches other views)

| Column | Behavior |
|--------|----------|
| Path (relative to repo root) | always shown, clips at cell width |
| Branch | always shown, `(detached)` for detached HEAD |
| Age | full `"3 days ago"` ≥ 120 cols, short `"3d"` < 120 cols, hidden < 60 cols |
| A/B | hidden < 80 cols |
| PR | hidden < 80 cols |
| Status | full text ≥ 70 cols, abbreviated < 70 cols |

Main worktree row pinned to top (like base branch in branch list).

### Navigation
Tab cycles: `BranchList → RemoteBranches → Tags → Worktrees → BranchList`.

### Actions
Enter opens context menu (consistent with all other views). Shortcut keys match equivalent actions in other views:

- `d` → Remove worktree (`git worktree remove <path>`) — disabled with reason `"dirty"` if `wt_status` is not clean
- `D` → Force remove worktree (`git worktree remove --force <path>`) — always available, goes through confirm overlay

After a successful remove, the entry is dropped from `app.worktrees` in place (no re-fetch).

## New `BranchAction` Variants

```rust
BranchAction::WorktreeRemove,        // label: "Remove worktree"
BranchAction::WorktreeForceRemove,   // label: "Force remove worktree"
```

New functions in `git/operations.rs`:
- `remove_worktree(repo_path: &Path, worktree_path: &Path) -> OperationResult`
- `force_remove_worktree(repo_path: &Path, worktree_path: &Path) -> OperationResult`

## Settings

New field in `config.rs`:
```rust
pub load_worktrees_on_launch: Option<bool>,
```

Shown in the Settings view alongside `auto_fetch` as `"Load worktrees on launch"`. Toggle with Space, persisted to config file.

## File Checklist

- `src/types.rs` — add `WorktreeInfo`, `WorktreeLoad`, `WorktreeEnrich`, `BranchAction::WorktreeRemove/WorktreeForceRemove`
- `src/config.rs` — add `load_worktrees_on_launch` field
- `src/git/worktree.rs` — new module: `list_worktrees()` (phase 1 parse + age computation)
- `src/git/operations.rs` — `remove_worktree()`, `force_remove_worktree()`
- `src/git/mod.rs` — expose `worktree` module
- `src/app.rs` — new `App` fields, `View::Worktrees`, tab navigation, key handling, event loop drain for both receivers
- `src/ui/worktree_list.rs` — new: `draw()` for the Worktrees view
- `src/ui/render.rs` — add match arm for `View::Worktrees`
- `src/ui/mod.rs` — expose `worktree_list` module
- `src/ui/help.rs` — add Worktrees keybindings
- `src/ui/settings.rs` — add `load_worktrees_on_launch` toggle
