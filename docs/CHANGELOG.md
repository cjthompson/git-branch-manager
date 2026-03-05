# Changelog

## 2026-03-05

### Fixes
- Delete local + remote hangs on "deleting remote branches..." stage (#bug, #git)
- Column sizing — status min-width, PR/AB autosize, branch name ellipsis (#ui, #layout)

### Tasks
- Add filter system with discoverable menu and composable search bar tokens (#ux, #filtering)
- Show task-specific loading messages during startup (#ux, #loading)
- Defer initial branch loading to after TUI renders (#performance, #ux)
- Git operation optimizations - remove dead code, cache --list mode, git2 checkout (#performance, #git, #cleanup)
- Batch branch deletion - delete multiple branches in single git commands (#performance, #git)
- Apply shortcut key color to all shortcut keys (#ui, #theme)
- Apply theme color to shortcut keys shown in the bottom status bar (#ui, #status-bar, #theme)
- Rows that have a checked state should have a different background color (#ui, #selection)
- Change chore prefix color from yellow to brown/amber for light theme contrast (#ui, #theme)
- Actions menu: disable "Delete local + remote" when there is no remote branch (#ui, #menu)
- Add shortcut keys to each item in the Actions menu (#ui, #menu)
- Actions menu missing shortcut keys for merge, squash, rebase, create worktree (#ui, #menu)
- Actions menu: use same row cursor symbol and color scheme as branch list (#ui, #menu)
- Color shortcut key letters using the base branch theme color (#ui, #menu, #theme)
- Disabled Actions menu items should not color their shortcut key letter (#ui, #menu, #theme)

## 2026-03-04

### Fixes
- Rename config path from git-bm to git-branch-manager (#config)
- Confirmation popup with list of branches, cuts off the list, no scroll (#ui)
- After confirming an operation, the UI freezes (#ui, #async)
- When scrolling back up, pinned branches disappear from the list (#ui, #scroll)

### Tasks
- Move merge status symbols to the right of the status text (#ui, #symbols)
- Left mouse click on status bar words executes that command (#ui, #mouse)
- Right-click on branch row brings up context menu (#ui, #mouse)
- Left-click on branch row toggles checkbox (#ui, #mouse)
- Scroll wheel moves cursor in branch list (#ui, #mouse)
- Enable mouse support (#ui, #mouse)
- Add auto-fetch on launch setting row to settings modal (#ui, #settings, #config)
- Add default sort column + direction setting rows to settings modal (#ui, #settings, #config)
- Add theme setting row to settings modal (#ui, #settings, #config)
- Add symbol set setting row to settings modal (#ui, #settings, #config)
- Create settings modal skeleton (#ui, #settings)
- Add sort_column, sort_asc, auto_fetch fields to Config and persist/restore them (#config, #settings)
- Persist theme to config and load on startup (#config, #settings)
- Persist symbol set to config and load on startup (#config, #settings)
- Color PR column by status and add open-in-browser menu item (#ui, #github, #pr)
- For branches that are both ahead and behind, add force push option to ENTER menu (#ui, #menu)
- For branches that are behind, add pull option to ENTER menu (#ui, #menu)
- For branches that are ahead, add push option to ENTER menu (#ui, #menu)
- Allow cursor on base and current branches with ENTER menu but no checkbox or delete (#ui, #navigation)
- Add symbol for squash-merged status and move status symbols to far right (#ui, #symbols)
- Display progress of git operations (#ui, #async)
