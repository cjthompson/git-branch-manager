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

## Task #016: Make list_tags() non-blocking

**Problem:** `tags::list_tags()` was called synchronously on the main thread in 4 separate
locations (branch list `t` key, results return to tags, remote branches `t` key x2). Each
call opened a fresh `Repository` and iterated all tags. On repos with hundreds of tags,
this caused 100–500ms freezes.

**Fix:** Added `TagLoad` struct, `tag_load_rx` channel, `tag_loading` flag, `load_tags()`
helper, and `drain_tag_load_rx()` to the event loop. All 4 call sites now use `load_tags()`
which spawns a background thread. Added a "Loading tags..." screen to `tag_list.rs` that
displays while the background load is in progress.

**Files changed:**
- `src/app.rs` — Added `TagLoad` struct, `tag_load_rx`/`tag_loading` fields, `load_tags()`
  method, `drain_tag_load_rx()` in event loop; replaced 4 synchronous call sites
- `src/ui/tag_list.rs` — Added loading screen when `tag_loading` is true

## Task #017: Add timeout and cancellation to background git fetch

**Problem:** `fetch_sync()` used `Command::output()` which blocks indefinitely. If the
remote is unreachable (flaky WiFi, broken SSH), the background thread hangs forever and
the "Fetching remote branches..." toast never clears.

**Fix:** Replaced `Command::output()` with `Command::spawn()` + a polling loop with a
30-second deadline. On timeout, the child process is killed and `false` is returned.
Also improved the fetch completion handler in the event loop to distinguish success from
failure — a failed/timed-out fetch no longer sets `remote_fetched = true`.

**Files changed:**
- `src/git/operations.rs` — Rewrote `fetch_sync()` with spawn + timeout
- `src/app.rs` — Updated fetch completion handler to check success/failure

## Task #018: Add stdin(Stdio::null()) to gh CLI command

**Problem:** The `gh pr list` command in `github.rs` didn't pipe stdin to `/dev/null`. If
`gh` needs authentication, it could prompt on stdin and hang the background thread forever.

**Fix:** Added `.stdin(std::process::Stdio::null())` to the `gh` Command, matching the
pattern used by `git_cmd()` for git CLI calls.

**Files changed:**
- `src/git/github.rs` — Added `stdin(Stdio::null())` to gh Command

## Task #019: Cap squash channel drain per frame and use HashMap for O(1) lookup

**Problem:** `drain_squash_rx()` and `drain_remote_squash_rx()` consumed all available
channel items in a single frame with O(N) linear search per item. When many cache hits
arrive simultaneously, this could cause O(N*M) work blocking the next redraw.

**Fix:** Capped both drain loops to process at most 32 items per frame iteration. Replaced
`Vec::iter_mut().find()` linear search with a pre-built `HashMap<String, usize>` for O(1)
branch lookup by name.

**Files changed:**
- `src/app.rs` — Rewrote `drain_squash_rx()` and `drain_remote_squash_rx()` with per-frame
  cap and HashMap index lookup

## Task #020: Reduce event loop poll timeout from 250ms to 50ms

**Problem:** The event loop used a 250ms poll timeout, meaning background channel results
(squash detection, fetch completion, operation results) could sit unprocessed for up to
250ms before being rendered. This created a perceptible lag ceiling for UI updates.

**Fix:** Reduced `event::poll` timeout from 250ms to 50ms. This makes background results
appear ~5x faster while having negligible CPU impact (the poll syscall is cheap).

**Files changed:**
- `src/app.rs` — Changed `Duration::from_millis(250)` to `Duration::from_millis(50)`

## Task #021: Skip redundant remote fetch when auto_fetch already ran on startup

**Problem:** When `auto_fetch` is enabled in config, the startup thread runs `git fetch`
before loading branches. But when the user opens the Remote Branches tab, it checks
`remote_fetched` (which is `false`) and runs `git fetch` again — a redundant network
round-trip.

**Fix:** Added `did_fetch: bool` field to `InitialLoad` struct. The startup thread sets
it to `true` when `auto_fetch` is configured. `drain_load_rx()` propagates this to
`self.remote_fetched` so `open_remote_branches_view()` skips the second fetch. Refreshes
(via `refresh_branches()`) always set `did_fetch: false`.

**Files changed:**
- `src/app.rs` — Added `did_fetch` to `InitialLoad`, set `remote_fetched` in
  `drain_load_rx()` when `did_fetch` is true, set `did_fetch: false` in refresh path
- `src/main.rs` — Set `did_fetch: auto_fetch` in startup load

## Task #022: Respect auto_fetch setting in Remote Branches view

**Problem:** `open_remote_branches_view()` always spawned a background `git fetch` on first
open, regardless of the `auto_fetch` config setting. With auto_fetch disabled, the user
expects no network activity unless explicitly requested.

**Fix:** Gated the background fetch in `open_remote_branches_view()` on
`self.config.auto_fetch == Some(true)`. When auto_fetch is off, the Remote Branches view
only shows locally known remote tracking refs without fetching.

**Files changed:**
- `src/app.rs` — Added `auto_fetch` check to fetch condition in `open_remote_branches_view()`
