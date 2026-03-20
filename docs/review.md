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

## Task #015: Make populate_remote_branches() non-blocking

**Problem:** `populate_remote_branches()` ran `list_remote_branches_phase1`, cache loading,
and squash candidate resolution synchronously on the main thread. This caused 100–500ms
freezes when pressing `r` to open Remote Branches and when the background fetch completed.

**Fix:** Added `RemoteLoad` struct and `remote_load_rx` channel. Converted
`populate_remote_branches()` to spawn a background thread. Added `drain_remote_load_rx()`
to the event loop to receive results and apply them. The existing `remote_loading` toast
("Fetching remote branches...") now shows during both the fetch and the local ref
enumeration. Fixed fetch completion logic to avoid briefly flashing `remote_loading = false`
before the reload starts.

**Files changed:**
- `src/app.rs` — Added `RemoteLoad` struct, `remote_load_rx` field, rewrote
  `populate_remote_branches()`, added `drain_remote_load_rx()` to event loop
