# git-branch-manager: Full Rewrite Feature Requirements

## Context

This document captures the complete feature set of the current git-branch-manager TUI application, to serve as the requirements specification for a full rewrite. The goal is to preserve all existing functionality while producing a cleaner, more maintainable codebase.

The current app is ~4000-line `app.rs` monolith plus numerous supporting modules. The rewrite should keep the same architecture philosophy (async TUI, background enrichment) but with better separation of concerns.

---

## Phase 1: Feature Inventory (for review)

This list needs step-by-step verification against the running application before implementation begins.

---

## 1. Application Startup & CLI

### 1.1 CLI Flags
- `--base <branch>`: Override auto-detected base branch
- `--list`: Non-interactive branch list to stdout (no TUI)
- `--symbols <ascii|unicode|powerline>`: Override symbol set

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
- `message`: Option<String>

### 2.5 Enums
- `MergeStatus`: Merged | SquashMerged | Unmerged | Pending
- `TrackingStatus`: Tracked { gone: bool } | Local
- `WorkingTreeStatus`: { staged: bool, unstaged: bool, untracked: bool }
- `PrStatus`: Draft | Open | Merged | Closed
- `PrInfo`: { number: u32, status: PrStatus }

---

## 3. Views & Tab Bar

A **view** is a full-screen panel with its own list, columns, keybindings, and state. The **tab bar** is the mechanism for selecting which view is active — it does not imply any hierarchy between views.

There are exactly 4 primary views, all peers:

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
- Fixed cycle order: Local Branches → Tags → Remote Branches → Worktrees → (back to Local Branches)

### Overlay views (not tabs)
These render *on top of* the active primary view and do not appear in the tab bar:
- `Help` — keybinding reference overlay
- `Menu` — per-item context menu
- `Confirm` — destructive action confirmation dialog
- `Executing` — operation in-progress screen
- `Results` — operation outcome screen (any key → return)
- `Settings` — configuration panel
- `Filter` — composable filter builder

---

## 4. Common Behavior (all 4 primary views)

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
3. **A/B column** — ahead/behind counts (`↑3 ↓2`)
4. **Age column** — relative age (`2d`, `3w`, `1y`)
5. **Status column** — merge status icon + working tree status

Column headers are clickable to sort by that column.

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
- `m`: select all merged + squash-merged branches (branches view only)

### 4.4 Sorting
- `s`: cycle sort column
- `S`: toggle ascending/descending order
- Mouse click on column header: sort by that column
- Sort state persisted to config file

### 4.5 Context Menu (Enter)
- `Enter`: open context menu for item under cursor
- Menu shows available actions for the item type
- Disabled items shown grayed-out with shortcut key still colored
- Escape or `q` to close menu

### 4.6 Mouse Support
- Scroll wheel: navigate list
- Left-click: move cursor to clicked row
- Left-click checkbox column: toggle selection
- Left-click column header: sort by that column
- Left-click status bar shortcut: trigger that action
- Right-click: open context menu for that row

### 4.7 Search / Filter
- `/`: toggle inline search bar (live substring filter on names)
- `\`: open filter builder UI (composable token filters)
- Filter persists while view is active, cleared on `Esc`

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

### 5.4 Keybindings
| Key | Action |
|-----|--------|
| `j`/`↓` | Move cursor down |
| `k`/`↑` | Move cursor up |
| `PageDown`/`PageUp` | Scroll by page |
| `Home`/`End` | Jump to first/last |
| `Space` | Toggle selection |
| `a`/`n`/`i`/`m` | Select all / none / invert / merged |
| `d` | Delete selected branches (local only) |
| `D` | Delete selected branches (local + remote) |
| `c` | Checkout branch under cursor |
| `x` | Quick-delete branch under cursor |
| `f` | Fetch all remotes |
| `F` | Fetch all remotes + prune |
| `R` | Clear squash-merge cache + refresh |
| `T` | Cycle theme |
| `Y` | Cycle symbol set |
| `,` | Open settings |
| `Tab`/`Shift+Tab` | Cycle tabs |
| `?` | Show help overlay |
| `q` | Quit |

### 5.5 Context Menu Actions
- Delete Local
- Delete Local + Remote
- Checkout
- Merge into current (regular)
- Merge into current (squash)
- Rebase onto base
- Push to remote
- Force push (--force-with-lease)
- Pull (fast-forward)
- Fast-forward (update without checkout)
- Fetch remote
- Fetch + Prune
- Create Worktree
- (if PR exists) Open PR in browser

### 5.6 Branch Name Prefix Colors
Prefix groups with distinct colors (e.g. `feat/`, `fix/`, `chore/`, `release/`):
- `feat/` or `feature/`: blue/cyan
- `fix/` or `bugfix/`: red/orange
- `chore/`: amber
- `release/`: green
- `hotfix/`: magenta
- Others: default

---

## 6. Tags View (Tab 4)

Tags is a primary view and full peer of the other three tabs. It is lazy-loaded on first visit.

### 6.1 Columns
Every row follows the same structure as other views:
1. Checkbox
2. Tag name (with prefix color coding)
3. Commit hash (short)
4. Age
5. Status / message (truncated)

A/B (ahead/behind) column is not applicable to tags and is omitted.

### 6.2 Loading
- Tags are lazy-loaded the first time the Tags tab is activated
- Progress indicator shown during load
- Sort and filter state persists between tab switches

### 6.3 Keybindings
| Key | Action |
|-----|--------|
| `j`/`↓` | Move cursor down |
| `k`/`↑` | Move cursor up |
| `PageDown`/`PageUp` | Scroll by page |
| `Home`/`End` | Jump to first/last |
| `Space` | Toggle selection |
| `a` | Select all |
| `n` | Deselect all |
| `i` | Invert selection |
| `d` | Delete selected tags (local) |
| `D` | Delete selected tags (local + remote) |
| `p` | Push tag under cursor to remote |
| `f` | Fetch all remotes |
| `F` | Fetch all remotes + prune |
| `/` | Inline search |
| `\` | Filter builder |
| `s` | Toggle sort (name vs date) |
| `Tab`/`Shift+Tab` | Cycle to next/previous tab |
| `?` | Help overlay |
| `q` | Quit |

### 6.4 Context Menu Actions
- Delete local tag
- Delete local + remote tag
- Push tag to remote

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

### 7.3 Keybindings
| Key | Action |
|-----|--------|
| `j`/`↓` | Move cursor down |
| `k`/`↑` | Move cursor up |
| `Space` | Toggle selection |
| `a`/`n`/`i` | Select all / none / invert |
| `c` | Checkout as local tracking branch |
| `d` | Delete selected remote branches |
| `f` | Fetch all remotes |
| `F` | Fetch all remotes + prune |
| `p` | Pull into local tracking branch |
| `m` | Merge remote branch into current local |
| `y` | Cherry-pick tip commit of remote branch |
| `v` | Open PR in browser (`gh pr view`) |
| `Tab`/`Shift+Tab` | Cycle tabs |
| `?` | Help overlay |
| `q` | Quit |

### 7.4 Context Menu Actions
- Delete remote branch
- Delete remote + local
- Checkout (create tracking branch)
- Fetch remote
- Pull remote into local
- Merge remote into current branch
- Cherry-pick tip of remote
- Open PR in browser

---

## 8. Worktrees View (Tab 3)

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

### 8.3 Keybindings
| Key | Action |
|-----|--------|
| `j`/`↓` | Move cursor down |
| `k`/`↑` | Move cursor up |
| `Space` | Toggle selection |
| `a`/`n`/`i` | Select all / none / invert |
| `d` | Remove selected worktrees (clean only) |
| `D` | Force remove selected worktrees |
| `c` | Checkout branch associated with worktree |
| `f` | Fetch all remotes |
| `F` | Fetch all remotes + prune |
| `Tab`/`Shift+Tab` | Cycle tabs |
| `?` | Help overlay |
| `q` | Quit |

### 8.4 Context Menu Actions
- Remove worktree (fails if dirty)
- Force remove worktree
- Checkout branch

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
- Filter tokens:

| Token | Values | Description |
|-------|--------|-------------|
| `status:` | merged / squash / unmerged | Filter by merge status |
| `pr:` | yes / no | Filter by PR association |
| `sync:` | ahead / behind | Filter by sync status |
| `age:` | <7d / <30d / >30d / >90d | Filter by last commit age |

- Quick-toggle keys in filter menu: `m` (merged), `s` (squash), `u` (unmerged), `p` (PR), `P` (no PR), `a` (ahead), `b` (behind), `1`-`4` (age presets)
- Filters are composable (AND logic)
- Active filters shown in status bar

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
