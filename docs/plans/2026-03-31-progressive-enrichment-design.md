# Progressive Enrichment Design

**Date:** 2026-03-31
**Branch:** fix/ui-responsiveness

## Problem

Switching to the Remote Branches or Worktrees tab blocks the UI for 5–9 seconds while expensive per-item operations complete before anything is rendered. Profiling data:

- Remote branches (221 branches): `graph_descendant_of` = 4756ms (21ms/branch), `graph_ahead_behind` = 4261ms (19ms/branch). Total: ~9s.
- Worktrees (23 worktrees): `status_and_age` parallelized but total wall time = ~5.4s.

## Goal

Both views become interactive immediately after tab switch. Enrichment data (merge status, ahead/behind, working tree status) streams in per-item in the background, updating the UI progressively. Sorting re-applies when enrichment completes, with a brief toast notification.

## Approach: Phase-1 / Phase-2 Split (Option A)

Each view has two background phases:
- **Phase 1**: cheap enumeration only — no git graph traversal, no subprocess per item. Sends a single batch immediately. View becomes interactive.
- **Phase 2**: per-item enrichment streamed one result at a time via a dedicated channel. App applies updates in-place each tick.

## New Types (`types.rs`)

```rust
pub struct RemoteEnrichResult {
    pub full_ref: String,
    pub merge_status: MergeStatus,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
}

pub struct WorktreeEnrichResult {
    pub index: usize,
    pub wt_status: WorkingTreeStatus,
    pub age_date: DateTime<Utc>,
}
```

## Changed Data Structures

### `RemoteBranchInfo` phase-1 defaults
`list_remote_branches_phase1` drops `graph_descendant_of` and `graph_ahead_behind`. All branches arrive with `merge_status: Unmerged`, `ahead: None`, `behind: None`. These are filled in by phase 2.

### `WorktreeInfo` phase-1 defaults
`list_worktrees` / `build_worktree` drops `status_and_age`. Worktrees arrive with `wt_status` defaulting to clean and `age_date` defaulting to `Utc::now()`. These are filled in by phase 2.

### `App` new/changed fields
```rust
// New
pub remote_enrich_rx: Option<Receiver<RemoteEnrichResult>>,
pub toast: Option<String>,
pub toast_expires: Option<Instant>,

// Changed type
pub worktree_enrich_rx: Option<Receiver<WorktreeEnrichResult>>,  // was Receiver<WorktreeEnrich>
```

The existing `WorktreeEnrich` struct in `app.rs` is removed.

## Background Thread Changes

### `populate_remote_branches`
- Phase 1: enumerate refs, extract name/remote/date/has_local. No graph ops. Sends `RemoteLoad` immediately.
- Phase 2 (`spawn_remote_enrich`): new function, spawned after phase 1 arrives in `drain_remote_load_rx`. Iterates branches, calls `graph_descendant_of` + `graph_ahead_behind` per branch, sends one `RemoteEnrichResult`. Channel stays open until all branches processed.

### Squash candidates
Squash checker spawns after phase 1 with all unmerged branches as candidates (same as today). Phase 2 may later mark some as Merged — squash results for those are harmless redundant updates.

### `spawn_worktree_load`
Phase 1: parse `git worktree list --porcelain` only. No `status_and_age`. Sends `WorktreeLoad` immediately.

### `spawn_worktree_status_enrich` (new)
Spawned after phase 1 arrives. Iterates worktrees by index, calls `status_and_age` per worktree, sends one `WorktreeEnrichResult { index, wt_status, age_date }`.

### Existing `spawn_worktree_enrich` (branch/PR enrichment)
Unchanged in behavior — still runs after phase 1, still a single-batch send. Rename to `spawn_worktree_branch_enrich` to distinguish from the new status enrich.

## Drain Functions

### `drain_remote_enrich_rx` (new)
- Drains up to 32 items per tick (matches squash drain pattern).
- Matches by `full_ref`, updates `remote_branches[idx]` in-place.
- On disconnect: re-applies current sort, sets toast `"Sort updated"` for 2 seconds.

### `drain_worktree_enrich_rx` (changed)
- Was: receive one `WorktreeEnrich` containing full vec, replace `self.worktrees`.
- Now: drain up to 32 `WorktreeEnrichResult` per tick, update `worktrees[index]` in-place.
- On disconnect: re-applies current sort, sets toast `"Sort updated"` for 2 seconds.

## Sorting Behavior

Default sort (date) is stable from phase 1 — no jumping during enrichment. If user has selected a sort column that depends on enriched data (ahead, behind, status), sort runs on whatever data exists at that moment. When enrichment finishes, sort re-applies automatically and a toast notifies the user.

## Toast System

### `App` fields
```rust
pub toast: Option<String>,
pub toast_expires: Option<Instant>,
```

Set via a helper:
```rust
fn set_toast(&mut self, msg: impl Into<String>, duration: Duration) {
    self.toast = Some(msg.into());
    self.toast_expires = Some(Instant::now() + duration);
}
```

Cleared in the `run` loop each tick before drawing:
```rust
if self.toast_expires.map_or(false, |e| Instant::now() >= e) {
    self.toast = None;
    self.toast_expires = None;
}
```

### `draw_toast` (new, `ui/shared.rs`)
Reads `app.toast`. If `Some`, renders a small bordered box in the bottom-right corner above the status bar, using existing `app.theme.toast_text` / `app.theme.toast_border` styles.

### Migration of existing fetch toast
`remote_branch_list.rs` currently hardcodes the "Fetching remote branches…" toast based on `app.remote_loading`. This is replaced by setting `app.toast` when the fetch starts and clearing it when done, then calling `draw_toast` from the render path.

### Toast call sites in render
- `remote_branch_list.rs`: call `draw_toast` at end of `draw`
- `worktree_list.rs`: call `draw_toast` at end of `draw`
- (Toast is view-specific overlay, not global — only shown in the two views that use enrichment)

## Files Changed

| File | Change |
|------|--------|
| `src/types.rs` | Add `RemoteEnrichResult`, `WorktreeEnrichResult` |
| `src/git/branch.rs` | Remove graph ops from `list_remote_branches_phase1`; add `enrich_remote_branches` fn |
| `src/git/worktree.rs` | Remove `status_and_age` from `build_worktree`; add `enrich_worktrees` fn |
| `src/app.rs` | New fields; new drain fns; spawn enrich after phase-1 drains; set_toast helper; toast expiry in run loop; remove `WorktreeEnrich` struct |
| `src/ui/shared.rs` | Add `draw_toast` |
| `src/ui/remote_branch_list.rs` | Replace hardcoded fetch toast with `draw_toast`; add enrich progress to status bar |
| `src/ui/worktree_list.rs` | Add `draw_toast`; add enrich progress to status bar |

## Testing

- Existing integration tests should pass unchanged (they don't test timing or UI).
- Manual: switch tabs, verify list appears instantly, watch enrichment columns fill in, confirm sort toast appears.
- Remove `GBM_TIMING` instrumentation added during profiling once implementation is complete.
