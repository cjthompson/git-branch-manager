# Changelog

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
