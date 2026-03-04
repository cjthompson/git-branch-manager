# git-branch-manager — Backlog

Tracked features, improvements, and bugs. Organized by implementation tier — foundational items first.

## Status Legend

- **planned** — Accepted, not yet started
- **in-progress** — Actively being worked on
- **done** — Completed (move to Done section)
- **wontfix** — Rejected or deferred indefinitely

## Design Inspirations

- **pcu (package-check-updates)** — Selection list UX pattern: `◯`/`◉` checkboxes, `❯` cursor indicator, footer hint bar with symbols (`↑↓ navigate • space select • a all • i invert • ⏎ submit`). Color pattern: green `◉` selected / gray `◯` unselected, yellow `❯` cursor, bold white for key data / dim gray for secondary, color-coded severity (green=patch, yellow=minor, red=major), footer uses bold keys + dim descriptions. Analogous mapping for branches: green=merged, yellow=squash-merged, red=unmerged.
- **[lazygit](https://github.com/jesseduffield/lazygit)** — Color usage to make information consumable at a glance
- **[lazyworktree](https://github.com/chmouel/lazyworktree)** — Good use of powerline / font awesome symbols
- **[serie](https://github.com/lusingander/serie)** — Clean column layout with string trimming for a tidy table look; tag display (local + remote)
- **[gitui](https://github.com/gitui-org/gitui)** — Rust/ratatui async architecture reference; theme system via config files

## Ratatui Features to Leverage

- **Layout constraints** — Use `Layout` with `Constraint::Min`, `Constraint::Max`, `Constraint::Percentage`, `Constraint::Ratio` for responsive column widths (relevant to BL-017, BL-022)
- **Table widget** — Built-in `Table` with `Row`/`Cell` may be a better fit than manual `Span` assembly for the column layout redesign (BL-017)
- **Scrollbar widget** — Multiple scrollbar styles available for horizontal/vertical scrolling (BL-022)
- **Symbol sets** — ratatui has built-in `symbols::scrollbar`, `symbols::border`, `symbols::line` with multiple presets; can extend this pattern for our checkbox/status symbols (BL-014)
- **No extra crate needed for checkboxes** — Currently using hardcoded `[x]`/`[ ]` strings, which is sufficient. BL-014 will swap these for unicode/powerline variants without needing `tui-checkbox`.

---

## Backlog

### Tier 1 — Foundations

These have no dependencies and unlock the most downstream work.

#### BL-001: Async TUI launch with progressive branch loading
- **Status:** done
- **Priority:** critical
- **Blocks:** BL-002, BL-009
- **Description:** Launch directly into the TUI instead of blocking on data gathering. The TUI should be interactive immediately. Branch data should load in the background and the list should update progressively as information is collected (e.g., once a branch is determined to be squash-merged, that row updates live). This is a major architectural change from the current "gather everything then render" model.

#### BL-017: Column layout redesign
- **Status:** done
- **Priority:** critical
- **Blocks:** BL-014, BL-019, BL-020, BL-022, BL-023, BL-024
- **Description:** Redesign branch list columns using ratatui `Table` widget with `Layout` constraints:
  1. Checkbox
  2. Branch name (without `origin/` prefix)
  3. Relative age
  4. Has remote branch icon
  5. Ahead icon
  6. Behind icon
  7. Other status icons
  8. (Configurable) GitHub PR# associated with the branch
  9. Right-aligned: "unmerged", "merged", "squashed"

#### BL-021: Improved merge status and cursor colors
- **Status:** done
- **Priority:** critical
- **Blocks:** BL-018
- **Description:** Current merge status colors are too similar — make them more distinct (green=merged, yellow=squash-merged, red=unmerged per pcu pattern). Improve the cursor/selection bar to be more colorful and visible. Fix the current gray-on-light-gray contrast issue that makes text under the cursor unreadable. Bold white for key data, dim gray for secondary info.

#### BL-003: Branch status cache
- **Status:** done
- **Priority:** critical
- **Blocks:** BL-004
- **Description:** Write a cache file to `/tmp` to remember merge/squash-merge status. Merged and squash-merged branches are cached permanently (they cannot become "unmerged"). Unmerged branches are cached with the HEAD commit hash and timestamp — the cached value is used unless the branch has been modified. Upstream branch existence can be cached but must be refreshed after `git fetch` or prune.

#### BL-006: Working tree status display
- **Status:** done
- **Priority:** critical
- **Blocks:** BL-011, BL-012, BL-013
- **Description:** Show whether the working tree is clean, has staged changes, has unstaged changes, or has untracked files. If the working tree is not clean, disable git commands that require a clean working tree (e.g., checkout, rebase).

#### BL-016: Base branch pinned at top
- **Status:** done
- **Priority:** high
- **Blocks:** BL-024
- **Description:** Pin the base branch (and main branch if different) at the very top of the list in a distinct color. No checkbox next to base/main branches since they cannot be selected for operations.

### Tier 2 — Core Features

Depend on one Tier 1 item. Unlock Tier 3 work or deliver key functionality.

#### BL-002: Status bar with progress indicators
- **Status:** done
- **Priority:** high
- **Depends on:** BL-001
- **Description:** Add a status bar showing: branch count, selected count, squash-merged count, regular-merged count, and a status area displaying current activity ("Reading branches...", "Checking for merged branches...", "Checking for squash merges...", etc.). Use a background color fill in the status section to indicate percentage completed of the current task. Show `(x/y)` task counts.

#### BL-009: Ahead/behind indicators
- **Status:** done
- **Priority:** high
- **Depends on:** BL-001
- **Blocks:** BL-022
- **Description:** Show icons for branches that are ahead or behind their remote tracking branch. These statuses can be computed as part of the async branch loading pipeline.

#### BL-005: Git fetch and fetch --prune support
- **Status:** done
- **Priority:** high
- **Blocks:** BL-008
- **Description:** Support `git fetch` and `git fetch --prune` from within the TUI to ensure the remote branch list is up-to-date. Show feedback during the fetch operation.

#### BL-015: ENTER key for operations menu
- **Status:** done
- **Priority:** high
- **Description:** SPACE bar for select/unselect (already implemented). ENTER key should bring up a contextual menu of git operations for the branch under the cursor (delete, merge, checkout, rebase, create worktree, etc.).

#### BL-010: Delete branch under cursor
- **Status:** done
- **Priority:** high
- **Description:** Delete the branch under the cursor regardless of selection state, with a confirmation prompt. Currently deletion only works on selected branches.

#### BL-012: Checkout branch under cursor
- **Status:** done
- **Priority:** high
- **Depends on:** BL-006
- **Description:** Checkout the branch under the cursor. Show a confirmation prompt. Depends on working tree status to warn/block if the working tree is dirty.

#### BL-004: Force recheck command
- **Status:** done
- **Priority:** medium
- **Depends on:** BL-003
- **Description:** Add a keybinding/command to force recheck all branches, ignoring the cache.

### Tier 3 — Enhanced Features

Depend on Tier 2 items or multiple Tier 1 items.

#### BL-022: Responsive width handling
- **Status:** done
- **Priority:** high
- **Depends on:** BL-017, BL-009
- **Description:** Adjust display to terminal width. Progressive trimming in this order:
  1. Shorten times from "1 week ago" to "1w"
  2. Trim branch names (user-selectable: trim start, end, or middle; configurable minimum length)
  3. Remove least important status icons (ahead, behind, PR#, has remote)
  4. Shorten merge status to single letter: "u", "m", "s"
  5. Remove age column entirely
  6. Show horizontal scrollbar if possible
  User-selectable option to enable horizontal scrolling instead of trimming.
  Depends on column layout (defines what to trim) and ahead/behind indicators (included in trim order).

#### BL-024: Column sorting
- **Status:** done
- **Priority:** medium
- **Depends on:** BL-017, BL-016
- **Description:** Allow sorting the branch list by any column in ascending or descending order (branch name, age, merge status, ahead/behind, etc.). Toggling sort on a column cycles through ascending → descending → default. Show a sort indicator (arrow) in the column header for the active sort. Depends on column layout (defines sortable columns) and pinned base branch (pinned rows must stay pinned regardless of sort order).

#### BL-014: Symbol set selection (powerline / unicode / ASCII)
- **Status:** done
- **Priority:** medium
- **Depends on:** BL-017
- **Description:** User-selectable option for display symbols. Three tiers:
  - **ASCII**: `[x]`/`[ ]` checkboxes, `>` cursor, `*` indicators, `--` separators
  - **Unicode**: `◉`/`◯` checkboxes, `❯` cursor, `↑↓` arrows, `⏎` enter, `✓`/`✗` status, `•` separator
  - **Powerline / Font Awesome**: nerd font glyphs, powerline separators, icon-based status indicators
  Affects checkboxes, cursor, status icons, footer hint bar, separators. Depends on column layout to know which icons/symbols are needed. Reference: pcu update list style.

#### BL-008: Fast-forward update local branches
- **Status:** done
- **Priority:** medium
- **Depends on:** BL-005
- **Description:** Update all local branches to match their remote tracking branches if they can be fast-forwarded (e.g., `git fetch origin mybranch:mybranch`). Useful for keeping local branches current without checking them out. Depends on fetch so remote refs are current before attempting fast-forward.

#### BL-011: Merge branch under cursor
- **Status:** done
- **Priority:** medium
- **Depends on:** BL-006
- **Description:** Merge the branch under the cursor into a destination branch. Prompt for merge type (regular merge or squash merge) and destination branch (default to base branch). Depends on working tree status because merge requires a clean working tree.

#### BL-013: Rebase branch
- **Status:** done
- **Priority:** medium
- **Depends on:** BL-006
- **Description:** Rebase the branch under the cursor onto a target branch. Prompt for the target branch with the base branch as default. Depends on working tree status because rebase requires a clean working tree.

#### BL-007: Create worktrees for branches
- **Status:** done
- **Priority:** medium
- **Description:** Create a git worktree for the branch under the cursor, or for all selected branches. Prompt for the base directory with a default of `.worktrees/<branch-name-sanitized>` inside the repo.

#### BL-025: Tag management screen
- **Status:** done
- **Priority:** medium
- **Description:** A separate screen (not part of the branch list) for viewing and managing local and remote tags. List tags with columns for name, date, and associated commit. Support creating, deleting, and pushing tags. Accessible via a keybinding from the branch list (e.g., `t`). Inspired by serie's tag display.

### Tier 4 — Polish & Customization

Cosmetic, configuration, and nice-to-have features. Depend on Tier 1–3 items.

#### BL-019: Branch prefix coloring
- **Status:** done
- **Priority:** low
- **Depends on:** BL-017
- **Description:** Color branch name prefixes differently. Branches with `/` like `feat/thing` get one color for the prefix and another for the rest. Allow configuration to assign specific colors to prefixes (e.g., `fix/` = red, `feat/` = green, `chore/` = yellow). Depends on column layout for branch name rendering.

#### BL-020: Age-based coloring
- **Status:** done
- **Priority:** low
- **Depends on:** BL-017
- **Description:** Color the age column based on how old the branch is: less than a week, less than a month, less than a quarter, etc. with progressively "warmer" colors for older branches. Depends on column layout for age column rendering.

#### BL-018: Selectable color themes
- **Status:** planned
- **Priority:** low
- **Depends on:** BL-021
- **Description:** Support selectable color themes similar to Python Textual's built-in schemes. Allow users to switch between themes. Depends on having good base colors established first so themes build on solid defaults.

#### BL-023: GitHub PR# column
- **Status:** planned
- **Priority:** low
- **Depends on:** BL-017
- **Description:** Optionally show the GitHub PR number associated with each branch. User-configurable setting to enable/disable this column. Requires GitHub API integration or `gh` CLI.

#### BL-026: Branch search with `/`
- **Status:** done
- **Priority:** medium
- **Description:** Press `/` to open a search input that filters the branch list by name. Typing narrows the visible branches to those matching the query. ESC or empty input exits search mode and restores the full list. Search should be case-insensitive substring match.

#### BL-027: Sort state persistence
- **Status:** done
- **Priority:** medium
- **Description:** Sort column and ascending/descending state persists across branch list refreshes (after operations, fetch, etc.).

#### BL-028: Page scrolling (PgUp/PgDn)
- **Status:** done
- **Priority:** medium
- **Description:** PgUp/PgDn keys move cursor by ~20 rows in both branch list and tag views, respecting pinned rows and search filter.

#### BL-029: Mouse column sorting
- **Status:** done
- **Priority:** medium
- **Description:** Click column header names to sort by that column. Second click reverses sort order. Uses crossterm mouse events.

#### BL-030: Right-aligned age and status columns
- **Status:** done
- **Priority:** low
- **Description:** Age and status columns are right-aligned in both branch list and tag list Table widgets.

#### BL-031: Merge status symbols
- **Status:** done
- **Priority:** low
- **Description:** Status column shows nerd font/unicode/ASCII symbols for each merge state (✔ merged, ≈ squash-merged, ✘ unmerged) from the active symbol set.

---

## Dependency Graph

```
TIER 1 (foundations)          TIER 2 (core)              TIER 3 (enhanced)          TIER 4 (polish)

BL-001 (async TUI) ────────→ BL-002 (status bar)
                    ────────→ BL-009 (ahead/behind) ───→ BL-022 (responsive width)

BL-003 (cache) ────────────→ BL-004 (force recheck)

BL-005 (fetch/prune) ──────→ BL-008 (fast-forward)

BL-006 (working tree) ─────→ BL-012 (checkout)
                       ────→ BL-011 (merge)
                       ────→ BL-013 (rebase)

BL-017 (column layout) ───→ BL-014 (symbol sets)
                        ──→ BL-022 (responsive width)
                        ──→ BL-024 (sorting)
                                                         ──→ BL-019 (prefix coloring)
                                                         ──→ BL-020 (age coloring)
                                                         ──→ BL-023 (PR# column)

BL-016 (pinned base) ─────→ BL-024 (sorting)

BL-021 (base colors) ──────────────────────────────────→ BL-018 (themes)

Independent: BL-010 (delete cursor), BL-015 (ENTER menu), BL-007 (worktrees), BL-025 (tags)
```

---

## Done

<!-- Move completed items here with completion date -->
