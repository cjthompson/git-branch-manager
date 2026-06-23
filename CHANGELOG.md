# Changelog

## 2026-06-23

### Fixes
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
