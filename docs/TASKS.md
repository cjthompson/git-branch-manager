# Tasks

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
