# Phase 2-4 Full Implementation Design

Date: 2026-03-04
Scope: All 21 remaining backlog tickets (BL-002 through BL-025)

## Implementation Phases

### Phase 1 — Parallel (minimal file overlap)

#### BL-009: Ahead/Behind Indicators

**Changes:**
- `types.rs`: Add `ahead: Option<u32>`, `behind: Option<u32>` to `BranchInfo`
- `git/branch.rs`: In `collect_branch_metadata`, compute via `repo.graph_ahead_behind(local_oid, remote_oid)` for tracked branches (git2 native)
- `ui/branch_list.rs`: Display `↑3 ↓1` or empty if 0/0

**Notes:** Only populated for `TrackingStatus::Tracked { gone: false }`. Zero values display as empty (not `↑0 ↓0`).

#### BL-012: Checkout Branch (with stash)

**Changes:**
- `types.rs`: Add `BranchAction::Checkout` variant
- `git/operations.rs`: Add `checkout_branch(repo_path, branch_name, stash: bool)` using git CLI
- `app.rs`: Keybinding `c` on cursor branch. If dirty tree → confirm "Stash and checkout?". If clean → confirm "Checkout?".
- `ui/help.rs`: Add `c` keybinding
- Flow: `git stash` (if dirty) → `git checkout <branch>` → `git stash pop` (if stashed). After: `refresh_branches()`

#### BL-004: Force Recheck

**Changes:**
- `git/cache.rs`: Add `BranchCache::clear(&self)` method (deletes cache file)
- `app.rs`: Keybinding `R`. Calls `cache.clear()` then `refresh_branches()`
- `ui/help.rs`: Add `R` keybinding

### Phase 2 — Sequential (heavy branch_list.rs refactor)

#### BL-017: Column Layout Redesign

**Changes:**
- `ui/branch_list.rs`: Replace `List` with ratatui `Table` widget
- Columns: checkbox | name | age | remote | ahead | behind | merge status
- Column widths: `Length(5)` checkbox, `Min(20)` name, `Length(14)` age, `Length(3)` remote, `Length(4)` ahead, `Length(4)` behind, `Length(14)` status
- Cursor via `Table::row_highlight_style(theme::CURSOR_STYLE)`

#### BL-016: Base Branch Pinned at Top

**Changes:**
- `ui/branch_list.rs`: Sort order: base first, current second (if different), then all others by date
- No checkbox for base/current rows
- Cursor movement skips pinned rows

#### BL-002: Status Bar with Progress

**Changes:**
- `app.rs`: Add `squash_checked: usize`, `squash_total: usize` fields. Increment `squash_checked` on each `SquashResult` received.
- `ui/branch_list.rs`: Status bar format: `branches: N | selected: N | merged: N | squash: N | [checking N/M...]`
- Background color fill proportional to `squash_checked / squash_total`

#### BL-005: Fetch/Prune

**Changes:**
- `git/operations.rs`: Add `fetch(repo_path)` and `fetch_prune(repo_path)` using git CLI
- `app.rs`: Keybinding `f` for fetch, `F` for fetch --prune. Shows "Fetching..." in status bar. After: `refresh_branches()`
- `ui/help.rs`: Add `f`/`F` keybindings

#### BL-015: ENTER Key Operations Menu

**Changes:**
- `app.rs`: Add `View::Menu { items, cursor }` variant. ENTER key opens menu.
- `ui/menu.rs`: New file. Inline popup positioned at cursor row right edge.
- Menu items: Checkout, Delete local, Delete local+remote, Merge, Rebase, Worktree
- Disabled items shown dimmed with reason. Arrow keys navigate, Enter selects, Esc dismisses.

#### BL-010: Delete Cursor Branch

**Changes:**
- `app.rs`: Keybinding `x`. Opens confirm overlay for single branch under cursor (not selection).
- `types.rs`: Add `BranchAction::DeleteCursor` or reuse `DeleteLocal` with different source
- Reuses existing delete logic

### Phase 3 — Enhanced Features

#### BL-022: Responsive Width

**Changes:**
- `ui/branch_list.rs`: Calculate available width, apply progressive trimming:
  1. Shorten ages: "1 week ago" → "1w"
  2. Trim branch names (configurable: start/end/middle, min length)
  3. Remove low-priority columns (PR#, remote, ahead/behind)
  4. Shorten status: "merged" → "m"
  5. Remove age column
- Width breakpoints from column constraints

#### BL-024: Column Sorting

**Changes:**
- `app.rs`: Add `sort_column: Option<usize>`, `sort_ascending: bool`. Keybinding `s` cycles column, `S` reverses.
- `ui/branch_list.rs`: Sort indicator arrow in column header. Pinned rows stay pinned.

#### BL-014: Symbol Sets

**Changes:**
- `ui/symbols.rs`: New file. `SymbolSet` struct with checkbox, cursor, arrows, status, separator fields. Three presets: `ASCII`, `UNICODE`, `POWERLINE`.
- `cli.rs`: Add `--symbols ascii|unicode|powerline` flag
- Auto-detect: check `TERM_PROGRAM` for kitty/wezterm/alacritty/iTerm2 → Powerline, else Unicode
- `config.rs`: New file. Persist symbol choice in `~/.config/git-bm/config.toml`

#### BL-008: Fast-Forward

**Changes:**
- `git/operations.rs`: Add `fast_forward(repo_path, branch_name)` → `git fetch origin <branch>:<branch>`
- Accessible from operations menu (BL-015). Batch on selected branches.

#### BL-011: Merge into Base

**Changes:**
- `git/operations.rs`: Add `merge_branch(repo_path, branch, base, squash: bool)`
- From operations menu. Confirm: "Merge <branch> into <base>? [regular/squash/cancel]"
- Requires clean working tree (offer stash). After: refresh branches.

#### BL-013: Rebase onto Base

**Changes:**
- `git/operations.rs`: Add `rebase_branch(repo_path, branch, base)`
- From operations menu. Requires clean working tree.
- If conflicts: exit TUI with message about resolving conflicts.

#### BL-007: Worktrees

**Changes:**
- `git/operations.rs`: Add `create_worktree(repo_path, branch, path)`
- From operations menu. Prompt for directory (default: `.worktrees/<sanitized-branch>`).
- Batch on selected branches.

#### BL-025: Tag Management

**Changes:**
- `app.rs`: Add `Tab` enum (`Branches`, `Tags`). Tab key switches.
- `ui/tag_list.rs`: New file. Tag list with name, date, commit, message columns.
- `git/tags.rs`: New file. List tags (git2), create/delete/push (git CLI).
- Operations: `t` create, `d` delete, `p` push.

### Phase 4 — Polish

#### BL-019: Branch Prefix Coloring

**Changes:**
- `ui/branch_list.rs`: Parse branch name at `/`. Apply color from map.
- Default: `fix/`=red, `feat/`=green, `chore/`=yellow, `hotfix/`=magenta
- Configurable in config.toml

#### BL-020: Age-Based Coloring

**Changes:**
- `ui/branch_list.rs`: Color age cell: <1w green, <1mo yellow, <3mo orange, >3mo red

#### BL-018: Selectable Themes

**Changes:**
- `ui/theme.rs`: `Theme` struct replaces all constants. 4 presets: dark (default), light, solarized, dracula.
- `app.rs`: Store current theme. Keybinding `T` cycles. Persisted in config.
- All `theme::CONSTANT` usages become `app.theme.field` or passed through render context.

#### BL-023: PR# Column

**Changes:**
- `git/github.rs`: New file. `gh auth token` → GitHub API `GET /repos/{owner}/{repo}/pulls`.
- Match `head.ref` to branch name. Background thread (like squash detection).
- `types.rs`: Add `pr_number: Option<u32>` to `BranchInfo`
- Graceful fallback: empty column if gh unavailable or unauthenticated.

## New Files Summary

```
src/
  config.rs          # Config file parsing (~/.config/git-bm/config.toml)
  git/
    github.rs        # GitHub API integration (PR numbers)
    tags.rs          # Tag listing and operations
  ui/
    menu.rs          # Inline popup operations menu
    symbols.rs       # Symbol set definitions (ASCII/Unicode/Powerline)
    tag_list.rs      # Tag management screen
```

## Key Architectural Decisions

1. **Table widget** over List for column layout — enables responsive width and sorting
2. **Stash-and-checkout** flow for dirty working trees — convenient for users
3. **Inline popup** for operations menu — positioned at cursor row
4. **Built-in theme presets** — no config file needed for themes
5. **`gh auth token`** for GitHub API — reuse user's existing auth, graceful fallback
6. **Auto-detect Powerline** via TERM_PROGRAM — fall back to Unicode
7. **Tab bar** for Branches/Tags — discoverable navigation
8. **Config file** at `~/.config/git-bm/config.toml` — optional, for persisting preferences
