# UI Responsiveness Review

Code review identified several areas where synchronous operations on the main thread
cause visible pauses in the TUI. This document tracks each fix applied.

## Task #014: Make refresh_branches() non-blocking

**Problem:** `refresh_branches()` ran `list_branches_phase1`, `detect_working_tree_status`,
and `BranchCache::load` synchronously on the main thread. On repos with 50+ branches or
large working trees, this caused 200ms–2s freezes after every operation (when dismissing
the Results screen) and on manual refresh (`R` key).

**Fix:** Converted `refresh_branches()` to spawn a background thread, reusing the existing
`load_rx` / `load_progress_rx` channel infrastructure and `drain_load_rx()` handler from
the initial startup load. The loading screen now displays "Refreshing branches..." while
work happens in the background. Also added missing state resets (`list_scroll_offset`,
`results`, `search_query`, `search_active`) to `drain_load_rx()` for parity with the
old synchronous path.

**Files changed:**
- `src/app.rs` — Rewrote `refresh_branches()` to spawn background thread; added state
  resets to `drain_load_rx()`
