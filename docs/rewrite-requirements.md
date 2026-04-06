# git-branch-manager: Full Rewrite Feature Requirements

## Context

This document captures the complete feature set of the current git-branch-manager TUI application, to serve as the requirements specification for a full rewrite. The goal is to preserve all existing functionality while producing a cleaner, more maintainable codebase.

The current app is ~4000-line `app.rs` monolith plus numerous supporting modules. The rewrite should keep the same architecture philosophy (async TUI, background enrichment) but with better separation of concerns.

---

## Rewrite Goals

### Primary Goal

Rewrite the codebase to keep the same features but produce a well-organized codebase that leverages shared abstractions and reusability to create a consistent interface across all views.

### Design Principles

1. **All 4 views are peers**: Local Branches, Remote Branches, Worktrees, and Tags must share the same core structures for display, column layout, sorting, row rendering, key handling, filtering, and selection. No view is "special" — they all inherit from common abstractions.

2. **Modularity and reuse over copy-paste**: The current codebase has significant duplication between views. The rewrite must eliminate this by extracting shared behavior into traits/generics that each view implements.

3. **Consistency**: All views must behave identically for common operations (navigation, selection, sorting, filtering, search, tab switching). View-specific behavior is layered on top.

4. **Testability**: New code prefers modularity and reuse that enables unit testing of individual components.

5. **Implementation freedom**: The rewrite can ignore the existing implementation completely. The current code is a reference for understanding *what* the feature does, not *how* it should be built.

### Action Model

Actions follow a consistent two-tier model across all views:

- **View-level shortcut keys** (e.g., `d`, `f`, `D`): Act on **all selected items** or the **entire repo**. These are bulk/global operations.
- **Context menu** (opened via `Enter` or right-click): Acts on the **single item under the cursor**. These are per-item operations.

Example: `p` at the view level pulls all selected branches. To pull just one branch, press `Enter` to open its context menu, then `p`.

---

## 1. Application Startup & CLI

### 1.1 CLI Flags
- `--base <branch>`: Override auto-detected base branch
- `--list`: Non-interactive branch list to stdout (no TUI)
- `--symbols <ascii|unicode|powerline>`: Override symbol set (new: not in current codebase)

### 1.2 Startup Sequence
- Open git repo from current working directory (error if not in a git repo)
- Auto-detect base branch (checks: main → master → develop → first branch)
- Phase-1 branch load runs synchronously before TUI starts (fast: git2 local data only)
- TUI renders immediately after phase-1
- Background threads start for:
  - Squash-merge detection (progressive, capped at 32/tick)
  - PR loading via `gh pr list` CLI
  - Auto-fetch (if configured)

### 1.3 Auto-Fetch on Startup
- Configurable toggle (`auto_fetch` in config)
- Runs `git fetch --all` in background before enrichment
- Skip redundant fetch if auto-fetch already ran

---

## 2. Data Model

### 2.1 BranchInfo (local branches)
- `name`: branch name
- `is_current`: bool (checked-out branch)
- `is_base`: bool (pinned at top)
- `tracking_status`: Tracked(gone: bool) | Local
- `ahead`: u32 (commits ahead of remote)
- `behind`: u32 (commits behind remote)
- `last_commit_date`: DateTime
- `merge_status`: MergeStatus

### 2.2 RemoteBranchInfo
- `full_ref`: e.g. `refs/remotes/origin/main`
- `remote`: e.g. `origin`
- `short_name`: e.g. `main`
- `has_local`: bool (local tracking branch exists)
- `is_base`: bool
- `last_commit_date`: DateTime
- `ahead`: u32
- `behind`: u32
- `pr_info`: Option<PrInfo>
- `merge_status`: MergeStatus

### 2.3 WorktreeInfo
- `path`: PathBuf
- `branch`: Option<String>
- `is_main`: bool (main worktree)
- `commit_hash`: String
- `working_tree_status`: WorkingTreeStatus
- `age_date`: Option<DateTime>
- `merge_status`: MergeStatus
- `pr_info`: Option<PrInfo>

### 2.4 TagInfo
- `name`: String
- `commit_hash`: String
- `date`: DateTime
- `message`: Option<String> (new: annotated tag message, not in current codebase)
- `is_annotated`: bool

### 2.5 Enums
- `MergeStatus`: Merged | SquashMerged | Unmerged | Pending
- `TrackingStatus`: Tracked { gone: bool } | Local
- `WorkingTreeStatus`: { staged: bool, unstaged: bool, untracked: bool }
- `PrStatus`: Draft | Open | Merged | Closed
- `PrInfo`: { number: u32, status: PrStatus }

---

## 3. Views & Tab Bar

A **view** is a full-screen panel with its own list, columns, keybindings, and state. The **tab bar** is the mechanism for selecting which view is active — it does not imply any hierarchy between views.

There are exactly 4 primary views, all peers. **All 4 views must share the same base behavior** — navigation, selection, sorting, filtering, search, column rendering, and key handling are implemented once and reused. View-specific behavior (columns, operations, context menu items) is layered on top of this shared foundation.

| Tab | Direct key | Description |
|-----|-----------|-------------|
| Local Branches | view-specific toggle key | Default view; local branch list |
| Remote Branches | view-specific toggle key | Remote refs; lazy-loaded on first visit |
| Worktrees | view-specific toggle key | Git worktrees; optionally preloaded |
| Tags | view-specific toggle key | Git tags; lazy-loaded on first visit |

Each view is fully independent: its own cursor, selection state, sort column/direction, search query, filter set, and background loading state. Switching tabs preserves all per-view state.

### Direct-access key convention
Each view has a **toggle key** that works from any other primary view:
- Pressing the toggle key of the *current* view returns to Local Branches (acts as "go back").
- Pressing the toggle key of a *different* view switches to that view.
- The exact key assignments are implementation-defined (currently: `r` for Remote, `w` for Worktrees, `t` for Tags from Branches; each non-branch view uses its own letter to return to Branches).

### Tab Bar
- Rendered as part of the top Block title
- All 4 tabs displayed; active tab highlighted
- `Tab` / `Shift+Tab` cycles forward/backward through all 4 views in a fixed order
- Fixed cycle order: Local Branches → Remote Branches → Tags → Worktrees → (back to Local Branches)
- **All 4 views participate in the Tab cycle** (current codebase only cycles 3; Tags must be added)

### Overlay views (not tabs)
These render *on top of* the active primary view and do not appear in the tab bar:
- `Help` — keybinding reference overlay
- `Menu` — per-item context menu (for single-item actions on the cursor row)
- `Confirm` — destructive action confirmation dialog
- `Executing` — operation in-progress screen
- `Results` — operation outcome screen (any key → return)
- `Settings` — configuration panel
- `Filter` — composable filter builder

---

## 4. Common Behavior (all 4 primary views)

All behavior described in this section is implemented **once** in shared code and reused by all 4 views. Views do not duplicate this logic.

### 4.0 Global Keys
These keys work identically from every primary view:

| Key | Action |
|-----|--------|
| `Tab` / `Shift+Tab` | Cycle to next / previous tab |
| `f` | Fetch all remotes (`git fetch --all`) |
| `F` | Fetch all remotes + prune (`git fetch --all --prune`) |
| `T` | Cycle color theme |
| `Y` | Cycle symbol set |
| `,` | Open settings |
| `?` | Open help overlay |
| `q` | Quit |

### 4.1 Column Layout
Every list has:
1. **Checkbox** column — `[ ]` / `[x]` (ASCII), `◯`/`◉` (Unicode), icons (Powerline)
2. **View-specific columns** (see per-view sections)
3. **A/B column** — ahead/behind counts (`↑3 ↓2`) (omitted if not applicable to the view)
4. **Age column** — relative age (`2d`, `3w`, `1y`)
5. **Status column** — merge status icon + working tree status (where applicable)

Column headers are clickable to sort by that column. All 4 views support clickable column headers consistently.

### 4.2 Navigation
- `j`/`↓`: move cursor down
- `k`/`↑`: move cursor up
- `PageDown`/`PageUp`: scroll by page
- `Home`/`End`: jump to first/last item

### 4.3 Selection (multi-select)
- `Space`: toggle current item selected/unselected
- `a`: select all visible items
- `n`: deselect all
- `i`: invert selection
- `m`: select all merged + squash-merged items (views with merge status only)

### 4.4 Sorting
- `s`: cycle sort column (cycles through the columns defined for the active view)
- `S`: toggle ascending/descending order
- Mouse click on column header: sort by that column
- Sort state persisted to config file
- **All 4 views use the same sorting mechanism** — only the set of sortable columns differs per view

### 4.5 Context Menu (Enter / Right-click)
- `Enter`: open context menu for item under cursor
- Right-click: open context menu for the clicked row
- Menu shows available actions **for the single item under the cursor**
- Context menu actions operate on **one item only** (the cursor row), never on the selection
- Disabled items shown grayed-out with shortcut key still colored
- Escape or `q` to close menu

### 4.6 View-Level Action Keys
- View-level shortcut keys (e.g., `d`, `D`, `p`) act on **all selected items**
- If no items are selected, they act on the item under the cursor
- These are bulk/global operations (delete selected, push selected, etc.)

### 4.7 Mouse Support
- Scroll wheel: navigate list
- Left-click: move cursor to clicked row
- Left-click checkbox column: toggle selection
- Left-click column header: sort by that column
- Left-click status bar shortcut: trigger that action
- Right-click: open context menu for that row

### 4.8 Search / Filter
- `/`: toggle inline search bar (live substring filter on names)
- `\`: open filter builder UI (composable token filters)
- Filter persists while view is active, cleared on `Esc`
- **Filters correspond to columns**: each view offers filter tokens for the columns it displays. Filter tokens for columns that don't exist in a view are not shown/ignored.
- Common filter tokens available across views (where the column exists):
  - `status:merged` / `status:squash` / `status:unmerged` — filter by merge status
  - `age:<7d` / `age:<30d` / `age:>30d` / `age:>90d` — filter by age
- View-specific filter tokens are defined in each view's section

---

## 5. Local Branches View (Tab 1)

### 5.1 Columns
1. Checkbox
2. Branch name (with prefix color coding, ellipsis if truncated)
3. Remote indicator (tracking remote name, or `local`)
4. Ahead/Behind counts
5. Age
6. Status (merge status icon + working tree flags)
7. PR number (if GitHub PR associated)

### 5.2 Special Rows
- **Base branch**: pinned at top, distinct styling, non-deletable
- **Current branch**: highlighted with distinct color/symbol

### 5.3 Merge Status Colors
- Merged: green ✔
- Squash-merged: yellow ≈
- Unmerged: red ✘
- Pending (still loading): gray …

### 5.4 View-Level Keybindings (act on selection)
| Key | Action |
|-----|--------|
| `d` | Delete selected branches (local only) |
| `D` | Delete selected branches (local + remote) |
| `p` | Push selected branches to remote |
| `R` | Clear squash-merge cache + refresh |

Plus all common keys from §4 (navigation, selection, sorting, search, filter, tab, theme, symbols, settings, help, quit, fetch).

### 5.5 Context Menu Actions (act on cursor row only)
- Checkout
- Delete Local
- Delete Local + Remote
- Merge into current (regular)
- Merge into current (squash)
- Rebase onto base
- Push to remote
- Force push (--force-with-lease)
- Pull (fast-forward)
- Fast-forward (update without checkout)
- Create Worktree
- (if PR exists) Open PR in browser

### 5.6 Filter Tokens
- `status:merged` / `status:squash` / `status:unmerged` — filter by merge status
- `pr:yes` / `pr:no` — filter by PR association
- `sync:ahead` / `sync:behind` — filter by sync status
- `age:<7d` / `age:<30d` / `age:>30d` / `age:>90d` — filter by age
- Text search on branch name (via `/`)

### 5.7 Branch Name Prefix Colors
Prefix groups with distinct colors (e.g. `feat/`, `fix/`, `chore/`, `release/`):
- `feat/` or `feature/`: blue/cyan
- `fix/` or `bugfix/`: red/orange
- `chore/`: amber
- `release/`: green
- `hotfix/`: magenta
- Others: default

---

## 6. Tags View (Tab 3)

Tags is a primary view and full peer of the other three tabs. It shares the same base view infrastructure (navigation, selection, sorting, filtering, search, context menu) as all other views. Lazy-loaded on first visit.

### 6.1 Columns
Every row follows the same structure as other views:
1. Checkbox
2. Tag name (with prefix color coding)
3. Commit hash (short)
4. Age
5. Message (annotated tag message, truncated) (new: not in current codebase)

A/B (ahead/behind) and merge status columns are not applicable to tags and are omitted.

### 6.2 Loading
- Tags are lazy-loaded the first time the Tags tab is activated
- Progress indicator shown during load
- Sort and filter state persists between tab switches

### 6.3 View-Level Keybindings (act on selection)
| Key | Action |
|-----|--------|
| `d` | Delete selected tags (local) |
| `D` | Delete selected tags (local + remote) |
| `p` | Push selected tags to remote |

Plus all common keys from §4 (navigation, selection, sorting, search, filter, tab, theme, symbols, settings, help, quit, fetch).

### 6.4 Context Menu Actions (act on cursor row only)
- Delete local tag
- Delete local + remote tag
- Push tag to remote

### 6.5 Filter Tokens
- `age:<7d` / `age:<30d` / `age:>30d` / `age:>90d` — filter by age
- Text search on tag name (via `/`)

---

## 7. Remote Branches View (Tab 2)

### 7.1 Loading
- Lazy-loaded on first navigation to the view
- Background fetch of remote data on view activation
- Progress shown during load

### 7.2 Columns
1. Checkbox
2. Remote/branch name (e.g. `origin/main`)
3. Local tracking indicator (has local branch or not)
4. Ahead/Behind counts
5. Age
6. Status (merge status + PR)
7. PR number

### 7.3 View-Level Keybindings (act on selection)
| Key | Action |
|-----|--------|
| `d` | Delete selected remote branches |

Plus all common keys from §4 (navigation, selection, sorting, search, filter, tab, theme, symbols, settings, help, quit, fetch).

### 7.4 Context Menu Actions (act on cursor row only)
- Checkout (create local tracking branch)
- Delete remote branch
- Delete remote + local
- Fetch remote
- Pull remote into local
- Merge remote into current branch
- Cherry-pick tip of remote
- Open PR in browser

### 7.5 Filter Tokens
- `status:merged` / `status:squash` / `status:unmerged` — filter by merge status
- `pr:yes` / `pr:no` — filter by PR association
- `sync:ahead` / `sync:behind` — filter by sync status
- `age:<7d` / `age:<30d` / `age:>30d` / `age:>90d` — filter by age
- Text search on branch name (via `/`)

---

## 8. Worktrees View (Tab 4)

Worktree branches are already visible in the Local Branches view. This view focuses on **worktree-specific operations** — actions that apply to worktrees as filesystem entities, not to their underlying branches.

### 8.1 Loading
- Optionally preloaded on startup (`load_worktrees_on_launch` config)
- Otherwise lazy-loaded on first navigation

### 8.2 Columns
1. Checkbox
2. Path (relative to repo root)
3. Branch name
4. Main worktree indicator
5. Working tree status (clean/dirty flags)
6. Age
7. Merge status

### 8.3 View-Level Keybindings (act on selection)
| Key | Action |
|-----|--------|
| `d` | Remove selected worktrees (clean only) |
| `D` | Force remove selected worktrees |

Plus all common keys from §4 (navigation, selection, sorting, search, filter, tab, theme, symbols, settings, help, quit, fetch).

### 8.4 Context Menu Actions (act on cursor row only)
- Remove worktree (fails if dirty)
- Force remove worktree

### 8.5 Filter Tokens
- `status:merged` / `status:squash` / `status:unmerged` — filter by merge status
- `age:<7d` / `age:<30d` / `age:>30d` / `age:>90d` — filter by age
- Text search on path/branch name (via `/`)

---

## 9. Help Overlay

- Triggered by `?`
- Two-column layout of all keybindings
- Escape or `?` to close
- Renders over the current view (doesn't navigate away)

---

## 10. Context Menu

- Centered overlay
- List of actions for current item
- Disabled items shown grayed with shortcut still visible
- Keyboard navigation: `j`/`k`, number shortcuts
- Mouse: click to select
- `Esc`/`q`: close

---

## 11. Confirmation Dialog

- Triggered before any destructive operation
- Shows action description + list of affected items
- `y`/`Enter`: confirm
- `n`/`Esc`: cancel

---

## 12. Executing View

- Full-screen progress display during operations
- Shows current operation name
- Per-item progress updates as operations complete

---

## 13. Results View

- Full-screen results after operations complete
- Shows success/failure per item with messages
- Any key → return to branch list + refresh

---

## 14. Settings View

### 14.1 Options
| Setting | Values | Description |
|---------|--------|-------------|
| `theme` | dark / light / solarized / dracula | Color theme |
| `symbols` | ascii / unicode / powerline | Symbol set |
| `sort_column` | name / age / ahead / behind / status | Default sort |
| `sort_asc` | true / false | Sort direction |
| `auto_fetch` | true / false | Fetch on startup |
| `load_worktrees_on_launch` | true / false | Preload worktrees |

### 14.2 Persistence
- Config file: `~/.config/git-branch-manager/config.toml`
- Changes saved immediately on toggle
- Theme/symbol changes applied live

---

## 15. Filter / Search System

### 15.1 Inline Search (`/`)
- Appears as a text bar at the bottom
- Case-insensitive substring match on item names
- Live filtering as you type
- `Esc`: clear and close

### 15.2 Filter Builder (`\`)
- Menu-based composable filters
- **Filters correspond to columns**: each view's filter builder shows tokens for the columns present in that view. Tokens for columns that don't exist in the active view are not shown.
- Filters are composable (AND logic)
- Active filters shown in status bar

### 15.3 Filter Tokens (full set)

| Token | Values | Applicable views | Description |
|-------|--------|-----------------|-------------|
| `status:` | merged / squash / unmerged | Local, Remote, Worktrees | Filter by merge status |
| `pr:` | yes / no | Local, Remote | Filter by PR association |
| `sync:` | ahead / behind | Local, Remote | Filter by sync status |
| `age:` | <7d / <30d / >30d / >90d | All 4 views | Filter by last commit age |

- Quick-toggle keys in filter menu: `m` (merged), `s` (squash), `u` (unmerged), `p` (PR), `P` (no PR), `a` (ahead), `b` (behind), `1`-`4` (age presets)
- Keys for inapplicable filters are not shown in a given view's filter builder

---

## 16. Background Threading Architecture

### 16.1 Channels (mpsc)
| Channel | Purpose |
|---------|---------|
| `load_rx` | Initial branch list results |
| `load_progress_rx` | Progress messages during load |
| `squash_rx` | Progressive squash-merge results |
| `remote_load_rx` | Remote branch list results |
| `remote_enrich_rx` | Remote ahead/behind + merge status |
| `remote_fetch_rx` | Fetch completion |
| `pr_rx` | PR info from `gh` CLI |
| `tag_load_rx` | Tag list results |
| `worktree_load_rx` | Worktree list results |
| `worktree_enrich_rx` | Worktree enrichment results |
| `op_rx` | Git operation results |
| `progress_rx` | Per-item progress during ops |

### 16.2 Enrichment Phases
- **Phase 1** (sync, before TUI): basic metadata, regular merge detection
- **Phase 2** (background): squash-merge, ahead/behind, PR info, worktree status

### 16.3 Squash-Merge Cache
- Persistent JSON cache at `/tmp/`
- Keyed by branch name + HEAD commit hash
- `R` key clears cache and forces re-check

---

## 17. Git Operations

### 17.1 Local Branch Operations
- `checkout_branch(name, auto_stash)`: stash if dirty, checkout, pop stash
- `delete_local(name)`: `git branch -D`
- `merge_branch(name, squash)`: merge or squash-merge with conflict detection
- `rebase_branch(name)`: rebase onto base, abort on conflict
- `push_branch(name)`: push to tracking remote
- `force_push_branch(name)`: `--force-with-lease`
- `pull_branch(name)`: fast-forward pull
- `fast_forward(name)`: update without checkout

### 17.2 Remote Operations
- `fetch()`: `git fetch --all`
- `fetch_prune()`: `git fetch --all --prune`
- `fetch_remote(remote)`: fetch specific remote
- `delete_remotes_batch(refs)`: batch `git push --delete` with fallback
- `checkout_remote_branch(full_ref)`: create local tracking branch
- `pull_remote(remote, branch)`: pull into local tracking
- `merge_remote_into_current(full_ref)`: merge remote ref into current HEAD
- `cherry_pick_remote(full_ref)`: cherry-pick tip commit

### 17.3 Tag Operations
- `list_tags()`: list sorted by date
- `delete_tag(name)`: delete local tag
- `delete_tags_batch(names)`: batch local delete
- `delete_remote_tags_batch(names)`: batch remote delete
- `push_tag(name)`: push to remote

### 17.4 Worktree Operations
- `create_worktree(branch)`: create under `.worktrees/<sanitized-name>/`
- `remove_worktree(path)`: clean remove (fails if dirty)
- `force_remove_worktree(path)`: `--force`

---

## 18. Themes

Four color themes, each defining styles for:
- Merged / SquashMerged / Unmerged status
- Current branch
- Base branch
- Cursor highlight
- Column headers
- Selected items (checkbox)
- Disabled menu items
- Search bar
- Status bar shortcuts
- Age coloring (warmer = older)
- Branch prefix colors

| Theme | Description |
|-------|-------------|
| Dark | Default dark background |
| Light | Light background |
| Solarized | Solarized palette |
| Dracula | Dracula palette |

---

## 19. Symbol Sets

Three symbol sets:

| Slot | ASCII | Unicode | Powerline |
|------|-------|---------|-----------|
| Checked | `[x]` | `◉` | nerd icon |
| Unchecked | `[ ]` | `◯` | nerd icon |
| Cursor | `>` | `❯` | nerd icon |
| Ahead | `+` | `↑` | nerd icon |
| Behind | `-` | `↓` | nerd icon |
| Current | `*` | `●` | nerd icon |
| Merged | `v` | `✔` | nerd icon |
| SquashMerged | `~` | `≈` | nerd icon |
| Unmerged | `x` | `✘` | nerd icon |

---

## 20. Performance & Responsiveness

- 50ms event loop poll interval
- TUI renders immediately after phase-1 (no waiting for squash detection)
- Squash-merge results applied progressively (cap 32 per tick)
- Remote fetch has 30s timeout to prevent hangs
- `stdin(Stdio::null())` on git CLI calls to prevent credential prompts from blocking
- O(1) branch lookup via HashMap
- Optional timing instrumentation: `GBM_TIMING=1` env var → `key_timing.log`

---

## 21. Status Bar

- Bottom bar showing contextual keybinding hints
- Shortcuts are clickable (mouse support)
- Active filters shown as tokens
- Toast notifications (temporary messages, auto-expire)
- Progress messages during background loading

---

## 22. Toast / Notification System

- Short-lived status messages (e.g. "Fetching...", "Cache cleared")
- Automatically expire after N seconds
- Appear in the status bar area

---

## 23. Error Handling

- Git operation failures shown in Results view
- Network/fetch errors shown as toast
- Not-in-git-repo error exits with message before TUI starts
- Operations that partially fail show per-item status

---

## 24. Squash-Merge Detection Algorithm

For each unmerged branch candidate:
```
ancestor = git merge-base <base> <branch>
temp     = git commit-tree <branch>^{tree} -p <ancestor> -m _
result   = git cherry <base> <temp>
```
A `-` prefix = squash content exists in base → SquashMerged
A `+` prefix = not in base → Unmerged

Uses git CLI (not git2) because git2 lacks `commit-tree` and `cherry`.

---

## Verification Plan

Before implementation:
1. Run the current app and verify each section above matches observed behavior
2. Walk through keybindings systematically (each key in each view)
3. Test filter tokens
4. Test context menu actions
5. Verify column layout in each view
6. Verify theme/symbol cycling
7. Verify settings persistence
8. Verify background loading indicators

Automation option: Use iTerm2 AppleScript + `expect`/`tmux send-keys` to drive the TUI programmatically for verification.

---

## Implementation Status (as of 2026-04-03)

Code analysis confirms ~95% of requirements are implemented in the current codebase. The following items are gaps or changes from current behavior:

### Structural Changes for Rewrite

1. **Tags not in Tab/Shift+Tab cycle** (§3): Current code cycles 3 views; Tags must be added as Tab 3 in the fixed cycle order.

2. **Action model consistency** (§4.5, §4.6): Current code mixes single-item and multi-item actions at the view level. Rewrite must enforce: context menu = single cursor item, view-level keys = selection/bulk.

3. **Shared view infrastructure** (§3, §4): Current views duplicate navigation, selection, sorting, filtering, and rendering logic. Rewrite must extract into shared abstractions that all 4 views inherit from.

4. **Column-based filter system** (§15): Current filter builder varies ad-hoc per view. Rewrite should derive available filters from the columns defined for each view.

### New Features (not in current codebase)

1. **`--symbols` CLI flag** (§1.1): Override symbol set from command line.
2. **TagInfo `message` field** (§2.4): Display annotated tag messages in the tag list.
3. **Right-click context menu** (§4.5): Open context menu on right-click.
4. **Tags view context menu** (§6.4): Tags currently use direct keys only; rewrite adds Enter/right-click context menu.

### Preserved Features

All other sections — data models, git operations, themes, symbol sets, mouse support, background threading, settings persistence, PR integration, squash-merge cache, toast system, and all existing keybindings — are fully implemented in current code and must be preserved.
