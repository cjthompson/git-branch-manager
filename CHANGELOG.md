# Changelog

## 2026-07-14

### Tasks
- Upgrade git2 crate to 0.21.0 (#deps)
- Stretchy-column priority in the responsive table layout is now matched by column name ("Branch"/"Name"/"Path") instead of position, so a future column reorder can't silently break which column gets priority width; also adds a tracking-link symbol to SymbolSet for a follow-up task (#ui)
- A/B column now colors ahead and behind counts differently (green/yellow) instead of a single shared color, across all four themes (#ui)
- Worktree details' "Changed Files" section now itemizes staged files (previously only counted via a has_staged flag, not listed individually); a file both staged and further edited shows as two separate entries (#ui, #worktrees)
- Responsive column width now demotes Age, then Merge, then A/B+PR one tier at a time as the terminal narrows, instead of flipping every column to its compact form at once (#ui, #responsive-width)

## 2026-07-10

### Fixes
- Deleting multiple remote branches in the Remotes view now shows the same per-item progress bar as local branch deletion, via a shared delete_remotes_with_progress helper

### Tasks
- Details view: worktrees with modified or untracked files now show a "Changed Files" section listing each path with its modified/untracked kind
- Parallelize squash-merge detection loop, capped at 4 worker threads pulling from a shared queue, with cache reads/writes retained by a single owner thread

## 2026-07-07

### Tasks
- Change error text color in Results overlay from dark gray to a brighter color (e.g. red)

## 2026-07-06

### Fixes
- Default sort column/direction settings now take effect at runtime: applied at startup to both Branches and Remotes views, applied live when edited in the Settings overlay, and applied to the CLI dump path (`--branches`/`--remotes`). Audit of all six Settings rows confirmed Symbol set, Theme, Auto-fetch, and Load worktrees were already correctly wired (#settings, #sort, #branches)
- Sort mode is preserved across post-operation reloads (fetch, delete, push, etc.) on Remotes/Tags/Worktrees views; previously `refresh_after_operation` rebuilt the `ListState` via `ListState::empty()` which discarded `sort_column`/`sort_ascending`. Branches was already correct (#fetch, #sort, #state)
- Default sort now applies to all four views (Branches, Remotes, Tags, Worktrees), each tracked independently in both Config and runtime `ListState`. Settings overlay shows one merged sort row per view (e.g. "Branches sort: age (asc)"). Existing configs with the legacy top-level `sort_column`/`sort_asc` are migrated automatically to the Branches and Remotes per-view fields (#settings, #sort, #tags, #worktrees)

## 2026-06-29

### Fixes
- Push is now available for any local branch when a remote is configured, not just tracked branches; uses `--set-upstream` to create the tracking ref automatically (#branches, #push)
- Remote branches list now updates immediately after "delete local + remote" without requiring a fetch (#ui, #remotes)
- Results and Confirm modals now resize dynamically to fit content — long messages and key hints no longer cut off (#ui, #modal)
- Remote branches now inherit squash-merge status from local branch detection — squash-merged branches no longer show as unmerged in the Remotes view (#ui, #remotes, #merge-detection)

## 2026-06-23

### Fixes
- Diagnostics: cache-accuracy audit now verifies every local branch (running the real squash check per non-reachable branch) instead of only cached ones, and reports verified/skipped counts with reasons rather than a misleading "verified" tally (#diagnostics, #cache)
- Filter modal: keep open on selection; add ESC hint; rename section to Merge Status (#ui, #filter)
- Rename merge filter syntax `status:` to `merge:` (e.g. `merge:merged`) for consistency with the renamed Merge column (#ui, #filter)
- Add abbreviated versions for Merge and Status columns (responsive like Age); rename merge column to "Merge" for consistency across views (#ui, #columns, #responsive)
- Change Remote column in Branches view to show indicator symbol instead of branch name (#ui, #columns)
- Make Path column have priority over Branch column in Worktrees view (#ui, #columns, #worktrees)

### Tasks
- Add Diagnostics modal (F2) with cache-accuracy verification (#diagnostics, #cache)

## 2026-06-09

### Tasks
- Remotes view: widen Age when space allows and fall back to compact ages when the resolved cell is too narrow

## 2026-06-08

### Tasks
- Worktrees view: Path column shows the end of the path, and Branch gets more room with left-truncated ellipsis display when too narrow

## 2026-06-06

### Tasks
- Extract generic confirm_selected helper (collect_targets + open_confirm) (#refactor, #dryness)
- Reduce active-view sort dispatch repetition via generic list_state helpers (#refactor, #dryness)
- Extract branch-like summary logic from status bar (#refactor, #dryness)
- Extract shared cell renderers (age, status, ahead/behind, PR) (#refactor, #dryness)
- Extract shared filter token groups (#refactor, #dryness)
- Extract shared column comparators and builders (#refactor, #dryness)
