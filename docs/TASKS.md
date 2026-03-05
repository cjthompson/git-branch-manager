# Tasks

---

## task: apply shortcut key color to all shortcut keys
**Date:** 2026-03-05 13:15 | **Priority:** medium | **Tags:** #ui #theme
**Status:** completed (2026-03-05 13:30)

### Requirements
- Find every UI location that renders shortcut key hints in `[x]word` format — including the confirm modal (`[y]es [n]o`), help overlay, and any other popups
- In each location, split the `[x]` token into three spans: `[` (normal style), `x` (theme.title fg color), `]` (normal style) — matching the pattern already used in the status bar and Actions menu
- The key letter color should use `theme.title` fg (the same accent color used everywhere else)

---

## task: apply theme color to shortcut keys shown in the bottom status bar
**Date:** 2026-03-05 12:45 | **Priority:** medium | **Tags:** #ui #status-bar #theme
**Status:** completed (2026-03-05 13:00)

### Requirements
- In the bottom status bar, shortcut key labels (e.g. `d delete`, `space select`, `enter menu`) should have the key letter/symbol colored using `theme.title` — the same accent color used for menu shortcut letters
- Non-key text (the command word) stays in the existing status bar style
- The key portion and label portion are already rendered as separate spans or can be split — apply `theme.title` fg to the key span only

---

## task: disabled Actions menu items should not color their shortcut key letter
**Date:** 2026-03-05 12:35 | **Priority:** medium | **Tags:** #ui #menu #theme
**Status:** completed (2026-03-05 12:40)

### Requirements
- In `src/ui/menu.rs`, when building the shortcut span for a disabled item, use `item_style` (the dim style) instead of `theme.title` for the shortcut letter span
- Only enabled items get the `theme.title` accent on the shortcut letter

---

## task: color shortcut key letters using the base branch theme color
**Date:** 2026-03-05 12:15 | **Priority:** medium | **Tags:** #ui #menu #theme
**Status:** completed (2026-03-05 12:30)

### Requirements
- In the Actions menu, shortcut key labels are rendered as `[x]` — the brackets should remain in the default text color, but the letter `x` should be colored using the same style as the base branch (`theme.current_branch` or whichever style is used for the pinned base branch row)
- This means splitting `[x]` into three spans: `[` (default), `x` (theme color), `]` (default)
- Identify which theme field is used for the base branch color and use that same field

---

## task: Actions menu missing shortcut keys for merge, squash, rebase, create worktree
**Date:** 2026-03-05 11:50 | **Priority:** medium | **Tags:** #ui #menu
**Status:** completed (2026-03-05 12:00)

### Requirements
- Add shortcut keys for the four items currently missing them:
  - `m` = Merge into base
  - `s` = Squash merge into base
  - `r` = Rebase onto base
  - `w` = Create worktree

---

## task: actions menu: use same row cursor symbol and color scheme as branch list
**Date:** 2026-03-05 11:45 | **Priority:** medium | **Tags:** #ui #menu
**Status:** completed (2026-03-05 12:00)

### Requirements
- The Actions menu cursor/highlight should use the same cursor symbol (`app.symbols.cursor_prefix`) and cursor style (`theme.cursor`) as the branch list row cursor
- Currently the menu likely uses a plain `>` or block highlight — replace with the project's configured symbol and color

---

## task: add shortcut keys to each item in the Actions menu
**Date:** 2026-03-05 11:30 | **Priority:** medium | **Tags:** #ui #menu
**Status:** completed (2026-03-05 12:00)

### Requirements
- Each item in the ENTER key Actions menu should have a single-letter shortcut key displayed next to it
- Pressing the shortcut key immediately executes that action (same as navigating to it and pressing Enter)
- Shortcut keys should be shown in the menu item label, e.g. `[d] Delete local`
- Disabled items' shortcut keys should not be active
- Suggested shortcuts (adjust if conflicts exist in the codebase): `d` = Delete local, `D` = Delete local + remote, `p` = Push, `P` = Force push, `l` = Pull, `f` = Fast-forward, `o` = Open PR in browser, `c` = Checkout

---

## task: actions menu: disable "Delete local + remote" when there is no remote branch
**Date:** 2026-03-05 11:00 | **Priority:** medium | **Tags:** #ui #menu
**Status:** completed (2026-03-05 11:20)

### Requirements
- In the ENTER key actions menu, the "Delete local + remote" option should be disabled (not selectable, visually dimmed) when the branch has no remote tracking branch
- A branch has no remote when `branch.tracking_status == TrackingStatus::Local` (or equivalent — no upstream set)
- Disabled items should be skipped when navigating with ↑/↓ (cursor jumps over them)
- Disabled items should be rendered with a dim style so users can see they exist but are unavailable

---

## fix: column sizing — status min-width, PR/AB autosize, branch name ellipsis
**Date:** 2026-03-05 10:30 | **Priority:** high | **Tags:** #ui #layout
**Status:** completed (2026-03-05 11:00)

### Requirements
- **Status column**: set a minimum width so `squash-merged ●` always fits without truncation; compute the minimum as the length of the longest possible status string plus symbol (e.g. `"squash-merged ●"`) and use that as the column constraint minimum
- **PR column**: autosize to the widest PR value in the current branch list (e.g. `"#2424"` = 5 chars); use `Constraint::Length(max_pr_width)` where `max_pr_width` is computed from the data, with a minimum of the header label width
- **A/B column**: autosize to the widest ahead/behind value in the current branch list (e.g. `"109"` = 3 chars, formatted as `"ahead/behind"` or just ahead count); use `Constraint::Length(max_ab_width)` computed from data, with a minimum of the header label width
- **Branch name column**: when the branch name (including prefix marker) is longer than the available column width, trim from the right and append an ellipsis — use `…` (U+2026) for unicode/powerline symbol sets and `...` for ascii

---

## task: change chore prefix color from yellow to brown/amber for light theme contrast
**Date:** 2026-03-05 10:15 | **Priority:** medium | **Tags:** #ui #theme
**Status:** completed (2026-03-05 10:20)

### Requirements
- In `src/ui/branch_list.rs`, in the `prefix_style` function, change `"chore" => Some(Style::new().fg(Color::Yellow))` to `"chore" => Some(Style::new().fg(Color::Indexed(130)))`
- This applies globally (all themes), as `Indexed(130)` brown/amber reads well on dark backgrounds too and avoids the clash with light theme's checked-row background (`Indexed(229)`)

---

## task: rows that have a checked state should have a different background color
**Date:** 2026-03-04 15:30 | **Priority:** medium | **Tags:** #ui #selection
**Status:** completed (2026-03-05 10:00)

### Requirements
- In the branch list, rows where the branch is selected (checked state, i.e., in `app.selected_branches`) should render with a distinct background color
- The selected-row background should be visually distinct from both the normal row background and the cursor-highlight background
- Apply the background color to the entire row span, not just the checkbox cell
- Add a `checked_row` field to the `Theme` struct in `src/ui/theme.rs` with these per-theme values:
  - dark: `Style::new().bg(Color::Indexed(236))`
  - light: `Style::new().bg(Color::Indexed(229))`
  - solarized: `Style::new().bg(Color::Indexed(22))`
  - dracula: `Style::new().bg(Color::Indexed(22))`
- In `src/ui/branch_list.rs`, when building each row's line spans, if the branch is in `app.selected_branches`, patch the background of all spans in that row to use `theme.checked_row.bg`

---

## task: move merge status symbols to the right of the status text
**Date:** 2026-03-04 15:00 | **Priority:** medium | **Tags:** #ui #symbols
**Status:** completed (2026-03-04 15:10)

### Requirements
- In the status column of the branch list, the symbol should appear AFTER the text, not before it
- Short format: `"m ✔"` → `"✔ m"` becomes `"m ✔"` (symbol at end): change `"{symbol} m"` to `"m {symbol}"`
- Long format: `"✔ merged"` → `"merged ✔"`: change `"{symbol} merged"` to `"merged {symbol}"` (and same for squash-merged and unmerged)
- In `src/ui/branch_list.rs` around lines 321-349, swap the order in all 6 format strings (3 statuses × 2 formats)
- The cell is already right-aligned (`Alignment::Right`) so the symbol will naturally sit against the right edge

---

## task: left mouse click on status bar words executes that command
**Date:** 2026-03-04 13:00 | **Priority:** medium | **Tags:** #ui #mouse
**Status:** completed (2026-03-04 14:30)
**Depends on:** "enable mouse support"

### Requirements
- In the bottom status bar, detect left-click on clickable words (e.g., "fetch", "help", "quit", their keybinding labels)
- Store the x-ranges of each clickable status bar item during render (similar to how `header_columns` stores column header x-ranges for sort clicks)
- On left-click in the status bar row, look up which item was clicked and execute the corresponding action (same as pressing the keyboard shortcut for that command)

---

## task: right-click on branch row brings up context menu
**Date:** 2026-03-04 13:00 | **Priority:** medium | **Tags:** #ui #mouse
**Status:** completed (2026-03-04 14:30)
**Depends on:** "enable mouse support"

### Requirements
- On right-click (`MouseEventKind::Down(MouseButton::Right)`) on any branch row, set the cursor to that row and open `View::Menu { cursor: 0 }` (same as pressing Enter)
- Reuse the existing `build_menu_items` and `View::Menu` logic — no new menu code needed
- Works on both pinned rows (base/current branch) and regular rows

---

## task: left-click on branch row toggles checkbox
**Date:** 2026-03-04 13:00 | **Priority:** medium | **Tags:** #ui #mouse
**Status:** completed (2026-03-04 14:30)
**Depends on:** "enable mouse support"

### Requirements
- On left-click (`MouseEventKind::Down(MouseButton::Left)`) on a branch row (but NOT on the header row or status bar), move the cursor to that row and toggle its selected state (same as pressing Space)
- Do not toggle selection on pinned rows (base branch, current branch) — they have no checkbox
- Clicking an already-selected row deselects it

---

## task: scroll wheel moves cursor in branch list
**Date:** 2026-03-04 13:00 | **Priority:** medium | **Tags:** #ui #mouse
**Status:** completed (2026-03-04 14:30)
**Depends on:** "enable mouse support"

### Requirements
- `MouseEventKind::ScrollDown` moves the cursor down one row (same as pressing `↓`)
- `MouseEventKind::ScrollUp` moves the cursor up one row (same as pressing `↑`)
- Scrolling respects the existing scroll offset and pinned row logic

---

## task: enable mouse support
**Date:** 2026-03-04 13:00 | **Priority:** medium | **Tags:** #ui #mouse
**Status:** completed (2026-03-04 14:30)
**Depends on:** none

### Requirements
- Enable crossterm mouse capture on startup: call `crossterm::execute!(stdout, crossterm::event::EnableMouseCapture)` after entering raw mode
- Disable mouse capture on exit: call `crossterm::execute!(stdout, crossterm::event::DisableMouseCapture)` in the cleanup/restore path (same place terminal is restored)
- Mouse events are already partially handled in `app.rs` (`handle_mouse_click` exists for column header sort clicks) — verify crossterm mouse events are being received and routed; if not, ensure `crossterm::event::Event::Mouse` is matched in the event loop

---

## task: add auto-fetch on launch setting row to settings modal
**Date:** 2026-03-04 12:00 | **Priority:** medium | **Tags:** #ui #settings #config
**Status:** completed (2026-03-04 14:30)
**Depends on:** "add sort_column/sort_asc/auto_fetch fields to Config", "create settings modal skeleton"

### Requirements
- Add an Auto-fetch row to the settings modal (navigable with ↑/↓)
- `←`/`→` or `Space` toggles between `on` and `off`
- Saving the modal writes `auto_fetch` to config via `config.save()`
- On app launch in `main.rs`, if `config.auto_fetch == true`, trigger a `git fetch` before displaying branches (reuse the existing fetch operation from `git/operations.rs`)

---

## task: add default sort column + direction setting rows to settings modal
**Date:** 2026-03-04 12:00 | **Priority:** medium | **Tags:** #ui #settings #config
**Status:** completed (2026-03-04 14:30)
**Depends on:** "add sort_column/sort_asc/auto_fetch fields to Config", "create settings modal skeleton"

### Requirements
- Add two rows to the settings modal: "Default sort column" and "Default sort direction"
- Sort column cycles through: `none` → `name` → `age` → `ahead` → `behind` → `status` → `none`
- Sort direction toggles: `ascending` / `descending`
- Saving the modal writes `sort_column` and `sort_asc` to config via `config.save()`
- On app launch in `main.rs`, if `config.sort_column` is set, apply it as the initial sort (call `app.apply_sort()` after `App::new`)

---

## task: add theme setting row to settings modal
**Date:** 2026-03-04 12:00 | **Priority:** medium | **Tags:** #ui #settings #config
**Status:** completed (2026-03-04 14:30)
**Depends on:** "persist theme to config and load on startup", "create settings modal skeleton"

### Requirements
- Add a Theme row to the settings modal (navigable with ↑/↓)
- `←`/`→` cycles through available themes (same cycle as the existing `T` keybinding)
- Changes apply live (immediate re-render) so the user sees a preview
- Saving the modal writes `theme` to config via `config.save()`

---

## task: add symbol set setting row to settings modal
**Date:** 2026-03-04 12:00 | **Priority:** medium | **Tags:** #ui #settings #config
**Status:** completed (2026-03-04 14:30)
**Depends on:** "persist symbol set to config and load on startup", "create settings modal skeleton"

### Requirements
- Add a Symbol set row to the settings modal (navigable with ↑/↓)
- `←`/`→` cycles: `unicode` → `powerline` → `ascii` → `unicode`
- Changes apply live (immediate re-render) so the user sees a preview
- Saving the modal writes `symbols` to config via `config.save()`

---

## task: create settings modal skeleton
**Date:** 2026-03-04 12:00 | **Priority:** medium | **Tags:** #ui #settings
**Status:** completed (2026-03-04 12:30)
**Depends on:** none

### Requirements
- Add `View::Settings { cursor: usize }` variant to the `View` enum in `app.rs`
- Press `,` (comma) from the branch list to open `View::Settings { cursor: 0 }`
- Add `handle_settings_key` in `app.rs`: `↑`/`↓` move cursor, `Esc` closes back to `View::BranchList`
- Create `src/ui/settings.rs` with a `draw(frame: &mut Frame, app: &App)` function — renders a centered overlay using `ratatui::widgets::Clear` + `Block` with title "Settings", body is empty for now (rows will be added by later tasks)
- Register in `src/ui/mod.rs` and add a match arm in `src/ui/render.rs`
- Add `,  settings` to the keybinding list in `src/ui/help.rs`

---

## task: add sort_column, sort_asc, auto_fetch fields to Config and persist/restore them
**Date:** 2026-03-04 12:00 | **Priority:** medium | **Tags:** #config #settings
**Status:** completed (2026-03-04 13:00)
**Depends on:** "rename config path from git-bm to git-branch-manager"

### Requirements
- Add `sort_column: Option<String>` (values: `"name"`, `"age"`, `"ahead"`, `"behind"`, `"status"`), `sort_asc: Option<bool>`, and `auto_fetch: Option<bool>` fields to `Config` in `src/config.rs` with `#[serde(default)]`
- In `app.rs` `App::new`, read `config.sort_column` and `config.sort_asc` and apply them as the initial sort state
- In `app.rs`, whenever the user changes sort (column header click, sort keybindings), write the new values back to config via `config.save()`

---

## task: persist theme to config and load on startup
**Date:** 2026-03-04 12:00 | **Priority:** medium | **Tags:** #config #settings
**Status:** completed (2026-03-04 13:00)
**Depends on:** "rename config path from git-bm to git-branch-manager"

### Requirements
- `Config.theme` field already exists in `src/config.rs` — it is not currently read on startup
- In `app.rs` `App::new`, read `config.theme` and use it to select the initial `Theme` (via `Theme::from_name` or equivalent)
- The existing `T` keybinding already writes `config.theme` on change — verify this is working correctly and fix if not

---

## task: persist symbol set to config and load on startup
**Date:** 2026-03-04 12:00 | **Priority:** medium | **Tags:** #config #settings
**Status:** completed (2026-03-04 13:00)
**Depends on:** "rename config path from git-bm to git-branch-manager"

### Requirements
- `Config.symbols` field already exists in `src/config.rs` — it is not currently read on startup
- In `app.rs` `App::new`, read `config.symbols` and use it to select the initial `SymbolSet` (via `symbols::from_name`) instead of always calling `symbols::detect()`
- Add a keybinding to cycle the symbol set (suggest `Y`) — on change, write `config.symbols` and call `config.save()`
- Add `Y  symbols` to the keybinding list in `src/ui/help.rs`

---

## fix: rename config path from git-bm to git-branch-manager
**Date:** 2026-03-04 12:00 | **Priority:** high | **Tags:** #config
**Status:** completed (2026-03-04 12:30)
**Depends on:** none

### Requirements
- In `src/config.rs` `config_path()`, change `.join("git-bm")` to `.join("git-branch-manager")`
- If `~/.config/git-bm/config.toml` exists on disk, migrate it: in `Config::load()`, check the old path first and if found, copy it to the new path and delete the old file

---

## task: color PR column by status and add open-in-browser menu item
**Date:** 2026-03-04 10:48 | **Priority:** medium | **Tags:** #ui #github #pr
**Status:** completed (2026-03-04 11:30)

### Requirements
- Color the PR# column based on PR status: draft (gray/dim), open (green), merged (purple), closed (red)
- Fetch PR status alongside PR number during the async GitHub loading
- Add an "Open PR in browser" menu item in the ENTER key operations menu for branches that have an associated PR
- Use `gh pr view --web` or equivalent to open the PR in the default browser

---

## task: for branches that are both ahead and behind, add force push option to ENTER menu
**Date:** 2026-03-04 10:45 | **Priority:** medium | **Tags:** #ui #menu
**Status:** completed (2026-03-04 10:55)

### Requirements
- When a branch is both ahead AND behind its remote tracking branch, add a "Force Push" option to the ENTER key operations menu
- The force push option should only appear when both ahead > 0 and behind > 0
- Force push should execute `git push --force-with-lease` for the selected branch
- Show a confirmation prompt before executing force push (it's destructive)

---

## task: for branches that are behind, add pull option to ENTER menu
**Date:** 2026-03-04 10:44 | **Priority:** medium | **Tags:** #ui #menu
**Status:** completed (2026-03-04 10:52)

### Requirements
- When a branch is behind its remote tracking branch, add a "Pull" option to the ENTER key operations menu
- The pull option should only appear for branches that have a behind count > 0
- Pull should execute `git pull` for the selected branch (or `git fetch origin branch:branch` if not currently checked out)

---

## task: for branches that are ahead, add push option to ENTER menu
**Date:** 2026-03-04 10:44 | **Priority:** medium | **Tags:** #ui #menu
**Status:** completed (2026-03-04 10:50)

### Requirements
- When a branch is ahead of its remote tracking branch, add a "Push" option to the ENTER key operations menu
- The push option should only appear for branches that have an ahead count > 0
- Push should execute `git push` for the selected branch

---

## task: allow cursor on base and current branches with ENTER menu but no checkbox or delete
**Date:** 2026-03-04 10:44 | **Priority:** medium | **Tags:** #ui #navigation
**Status:** completed (2026-03-04 10:48)

### Requirements
- Allow the cursor to move onto pinned rows (base branch, current branch) — currently they are skipped
- Do NOT show a checkbox on pinned rows (they remain unselectable for bulk operations)
- The ENTER key should open the operations menu on pinned rows
- Disable/hide the "Delete" option in the ENTER menu when on a base or current branch

---

## task: add symbol for squash-merged status and move status symbols to far right
**Date:** 2026-03-04 10:41 | **Priority:** medium | **Tags:** #ui #symbols
**Status:** completed (2026-03-04 10:43)

### Requirements
- Add a distinct symbol for `squash merged` status (e.g., `≈` in unicode set) alongside existing merged (`✔`) and unmerged (`✘`) symbols
- Move the status symbols column to the far right of the branch list table
- Ensure all three symbol sets (ASCII, Unicode, Powerline) have a squash-merged symbol

---

## fix: confirmation popup with list of branches, cuts off the list, no scroll
**Date:** 2026-03-04 03:30 | **Priority:** high | **Tags:** #ui
**Status:** completed (2026-03-04 04:15)

### Requirements
- Put the confirmation text at the TOP of the popup
- If the list of branches exceeds the popup height, show `...N more` at the bottom

---

## fix: after confirming an operation, the UI freezes
**Date:** 2026-03-04 03:30 | **Priority:** high | **Tags:** #ui #async
**Status:** completed (2026-03-04 04:18)

### Requirements
- All git operations should run async so that the UI never freezes during execution

---

## task: display progress of git operations
**Date:** 2026-03-04 03:30 | **Priority:** medium | **Tags:** #ui #async
**Status:** completed (2026-03-04 04:22)

### Requirements
- All operations should display progress of completion (e.g., progress bar) to show work is being done
- Progress display should have a cancel option

---

## fix: when scrolling back up, pinned branches disappear from the list
**Date:** 2026-03-04 03:30 | **Priority:** high | **Tags:** #ui #scroll
**Status:** completed (2026-03-04 04:26)

### Requirements
- After scrolling down and back up, pinned branches (base branch, current branch) must still appear at the top of the list
- Pinned rows should always remain visible regardless of scroll position
