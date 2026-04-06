# Progressive Enrichment Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make the Remote Branches and Worktrees tabs interactive immediately on switch, with merge status / ahead-behind / working-tree-status streaming in per-item in the background.

**Architecture:** Split each view's load into Phase 1 (cheap enumeration, renders immediately) and Phase 2 (per-item graph traversal / subprocess, streamed via a dedicated channel). A generic toast system replaces the three hardcoded toast blocks. Sort re-applies automatically when enrichment finishes and shows a brief toast.

**Tech Stack:** Rust, ratatui, git2, std::sync::mpsc

---

## Background / Key Facts

- Build command: `export PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH" && cargo build`
- Tests: `cargo test`
- Profiling data: remote branches phase1 currently takes ~9s (21ms/branch × 221 branches); worktree load takes ~5.4s (parallelised `git status` per worktree).
- The existing `GBM_TIMING` instrumentation added during profiling should be **removed** in Task 1.
- Design doc: `docs/plans/2026-03-31-progressive-enrichment-design.md`

---

### Task 1: Remove GBM_TIMING instrumentation

**Files:**
- Modify: `src/git/branch.rs` (inside `list_remote_branches_phase1`)
- Modify: `src/git/worktree.rs` (inside `list_worktrees`)

**Step 1: Remove timing from `list_remote_branches_phase1`**

Strip everything added during profiling: the `use std::time::Instant;`, `timing` bool, `t_total`, `t_merge_total`, `t_ab_total` accumulators, the per-branch `Instant::now()` calls, and the final `eprintln!` block. Leave the rest of the function unchanged.

**Step 2: Remove timing from `list_worktrees`**

Revert `list_worktrees` to its original one-liner body:

```rust
pub fn list_worktrees(repo_path: &Path) -> Vec<WorktreeInfo> {
    let out = git_out(repo_path, &["worktree", "list", "--porcelain"]);
    parse_porcelain(&out)
}
```

**Step 3: Build**

```
cargo build
```
Expected: no errors, 3 pre-existing dead-code warnings only.

**Step 4: Commit**

```bash
git add src/git/branch.rs src/git/worktree.rs
git commit -m "chore: remove GBM_TIMING profiling instrumentation"
```

---

### Task 2: Add `RemoteEnrichResult` and `WorktreeEnrichResult` to `types.rs`

**Files:**
- Modify: `src/types.rs`

`RemoteEnrichResult` and `WorktreeEnrichResult` need to be in `types.rs` (which is re-exported via `lib.rs`) so both `app.rs` and `git/` modules can use them without circular imports.

**Step 1: Add the two new structs**

Find the block of existing structs near the top of `src/types.rs` (after `use` statements). Add after the existing `SquashResult` struct:

```rust
/// Per-item result sent from the remote-branch enrichment background thread.
#[derive(Debug, Clone)]
pub struct RemoteEnrichResult {
    /// Identifies which branch to update (matches `RemoteBranchInfo::full_ref`).
    pub full_ref: String,
    pub merge_status: MergeStatus,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
}

/// Per-item result sent from the worktree status-enrichment background thread.
#[derive(Debug, Clone)]
pub struct WorktreeEnrichResult {
    /// Index into `App::worktrees` (position in the phase-1 list).
    pub index: usize,
    pub wt_status: WorkingTreeStatus,
    pub age_date: chrono::DateTime<chrono::Utc>,
}
```

**Step 2: Build**

```
cargo build
```
Expected: no new errors.

**Step 3: Commit**

```bash
git add src/types.rs
git commit -m "feat: add RemoteEnrichResult and WorktreeEnrichResult types"
```

---

### Task 3: Split `list_remote_branches_phase1` — remove graph ops

**Files:**
- Modify: `src/git/branch.rs`

**Step 1: Strip graph ops from `list_remote_branches_phase1`**

Remove the `graph_descendant_of` call and the `graph_ahead_behind` call from the per-branch loop. Replace with hardcoded defaults:

```rust
let merge_status = MergeStatus::Unmerged;
let ahead = None;
let behind = None;
```

The `base_oid` computation at the top of the function can stay — it is still used by `list_branches_phase1` (a different function in the same file). Actually, `base_oid` is only used for the two calls you're removing — so remove it too. Also remove the `local_names` HashSet, since `has_local` detection via `local_names.contains(...)` still needs to stay. Wait — `has_local` uses `local_names`, keep that. Only remove `base_oid`.

Concretely, after your edit the function should:
- Still build `local_names`
- NOT compute `base_oid`
- Loop over remote branches, extract `full_ref`, `remote`, `short_name`, `is_base`, `last_commit_date`, `has_local`
- Set `merge_status = MergeStatus::Unmerged`, `ahead = None`, `behind = None`
- Push to `branches`
- Sort by date
- Return

**Step 2: Build**

```
cargo build
```
Expected: no errors. The compiler will tell you if `base_oid` is referenced anywhere else.

**Step 3: Commit**

```bash
git add src/git/branch.rs
git commit -m "feat: strip graph ops from list_remote_branches_phase1 (phase-1 now list-only)"
```

---

### Task 4: Add `enrich_remote_branches` function to `git/branch.rs`

**Files:**
- Modify: `src/git/branch.rs`

This function runs in a background thread. It opens the repo, iterates remote branches by `full_ref`, computes `graph_descendant_of` + `graph_ahead_behind` per branch, and sends one `RemoteEnrichResult` per branch. The channel closing naturally signals completion.

**Step 1: Add the function**

Add at the bottom of `src/git/branch.rs`:

```rust
/// Enrich remote branches with merge status and ahead/behind counts.
/// Called in a background thread after phase-1 load completes.
/// Sends one `RemoteEnrichResult` per branch; channel close signals completion.
pub fn enrich_remote_branches(
    repo_path: &std::path::Path,
    base_branch: &str,
    branches: Vec<crate::types::RemoteBranchInfo>,
    tx: std::sync::mpsc::Sender<crate::types::RemoteEnrichResult>,
) {
    use git2::BranchType;
    use crate::types::RemoteEnrichResult;

    let Ok(repo) = git2::Repository::open(repo_path) else { return };

    let base_oid = repo
        .find_branch(base_branch, BranchType::Local)
        .ok()
        .and_then(|b| b.get().peel_to_commit().ok())
        .map(|c| c.id());

    for branch in &branches {
        let refname = format!("refs/remotes/{}", branch.full_ref);
        let commit_id = match repo
            .find_reference(&refname)
            .ok()
            .and_then(|r| r.peel_to_commit().ok())
            .map(|c| c.id())
        {
            Some(id) => id,
            None => continue,
        };

        let merge_status = if let Some(base) = base_oid {
            if repo.graph_descendant_of(base, commit_id).unwrap_or(false) {
                crate::types::MergeStatus::Merged
            } else {
                crate::types::MergeStatus::Unmerged
            }
        } else {
            crate::types::MergeStatus::Unmerged
        };

        let (ahead, behind) = if let Some(base) = base_oid {
            repo.graph_ahead_behind(commit_id, base)
                .map(|(a, b)| (Some(a as u32), Some(b as u32)))
                .unwrap_or((None, None))
        } else {
            (None, None)
        };

        if tx.send(RemoteEnrichResult {
            full_ref: branch.full_ref.clone(),
            merge_status,
            ahead,
            behind,
        }).is_err() {
            return; // receiver dropped (user navigated away)
        }
    }
}
```

**Step 2: Build**

```
cargo build
```
Expected: no errors.

**Step 3: Commit**

```bash
git add src/git/branch.rs
git commit -m "feat: add enrich_remote_branches background enrichment function"
```

---

### Task 5: Split `list_worktrees` / `build_worktree` — remove `status_and_age` from phase 1

**Files:**
- Modify: `src/git/worktree.rs`

**Step 1: Strip `status_and_age` from `build_worktree`**

Replace the call to `status_and_age` with defaults:

```rust
fn build_worktree(
    path: PathBuf,
    commit_hash: String,
    branch: Option<String>,
    is_main: bool,
) -> WorktreeInfo {
    WorktreeInfo {
        path,
        branch,
        is_main,
        commit_hash,
        wt_status: WorkingTreeStatus::clean(),
        age_date: chrono::Utc::now(),
        merge_status: MergeStatus::Unmerged,
        ahead: None,
        behind: None,
        pr: None,
    }
}
```

The `status_and_age`, `head_commit_date`, and `newest_mtime` functions stay in the file — they will be used by the new enrichment function.

**Step 2: Build**

```
cargo build
```
Expected: no errors. `status_and_age` and helpers may now be flagged as unused — that's fine, they'll be used in Task 6.

**Step 3: Commit**

```bash
git add src/git/worktree.rs
git commit -m "feat: strip status_and_age from worktree phase-1 load"
```

---

### Task 6: Add `enrich_worktrees` function to `git/worktree.rs`

**Files:**
- Modify: `src/git/worktree.rs`

**Step 1: Add the function**

Add at the bottom of `src/git/worktree.rs`:

```rust
/// Enrich worktrees with working-tree status and accurate age date.
/// Called in a background thread after phase-1 load completes.
/// Sends one `WorktreeEnrichResult` per worktree; channel close signals completion.
pub fn enrich_worktrees(
    worktrees: Vec<crate::types::WorktreeInfo>,
    tx: std::sync::mpsc::Sender<crate::types::WorktreeEnrichResult>,
) {
    use crate::types::WorktreeEnrichResult;

    for (index, wt) in worktrees.iter().enumerate() {
        let (wt_status, age_date) = status_and_age(&wt.path);
        if tx.send(WorktreeEnrichResult { index, wt_status, age_date }).is_err() {
            return; // receiver dropped
        }
    }
}
```

**Step 2: Build**

```
cargo build
```
Expected: no errors, no new warnings.

**Step 3: Commit**

```bash
git add src/git/worktree.rs
git commit -m "feat: add enrich_worktrees background enrichment function"
```

---

### Task 7: Add toast fields and `set_toast` to `App`; add enrichment receiver fields

**Files:**
- Modify: `src/app.rs`

This task adds the new `App` fields and the `set_toast` helper. It does NOT yet wire up the enrichment threads — that's Task 8.

**Step 1: Add new fields to the `App` struct**

In the `App` struct definition (around line 194), add after the `worktree_enrich_rx` field:

```rust
/// Per-item enrichment results for remote branches (merge status, ahead/behind).
pub remote_enrich_rx: Option<Receiver<RemoteEnrichResult>>,
/// Per-item enrichment results for worktrees (wt_status, age_date).
/// Replaces the old batch WorktreeEnrich receiver.
// NOTE: type changes from Option<Receiver<WorktreeEnrich>> to Option<Receiver<WorktreeEnrichResult>>
```

Change the existing `worktree_enrich_rx` field type from `Option<Receiver<WorktreeEnrich>>` to `Option<Receiver<WorktreeEnrichResult>>`.

Add toast fields anywhere in the struct (near the timing fields at the bottom is fine):
```rust
/// Current toast message, if any.
pub toast: Option<String>,
/// When the current toast expires.
pub toast_expires: Option<Instant>,
```

**Step 2: Update the `use` line at the top of `app.rs`**

Add `RemoteEnrichResult` and `WorktreeEnrichResult` to the existing `use git_branch_manager::types::...` import.

**Step 3: Update `App::new` initialiser**

Add to the `Self { ... }` block:
```rust
remote_enrich_rx: None,
toast: None,
toast_expires: None,
```

Change `worktree_enrich_rx: None` — the field type changed but the init value stays `None`, so no code change needed there.

**Step 4: Remove the `WorktreeEnrich` struct**

Find and delete (around line 173):
```rust
pub(crate) struct WorktreeEnrich {
    pub worktrees: Vec<WorktreeInfo>,
}
```

**Step 5: Add `set_toast` helper method**

Add to `impl App` (near other small helpers):
```rust
pub fn set_toast(&mut self, msg: impl Into<String>, duration: Duration) {
    self.toast = Some(msg.into());
    self.toast_expires = Some(Instant::now() + duration);
}
```

**Step 6: Add toast expiry check to the `run` loop**

In `pub fn run`, just before `terminal.draw(...)`, add:
```rust
// Expire toast
if self.toast_expires.map_or(false, |e| Instant::now() >= e) {
    self.toast = None;
    self.toast_expires = None;
}
```

**Step 7: Add `remote_enrich_rx` to `is_any_loading`**

Find `fn is_any_loading` and add:
```rust
|| self.remote_enrich_rx.is_some()
```

**Step 8: Build**

```
cargo build
```
Expected: errors about `WorktreeEnrich` still referenced in `drain_worktree_enrich_rx` and `spawn_worktree_enrich` — that's expected and will be fixed in Task 8.

**Step 9: Commit** (after Task 8 fixes the errors — skip this commit until Task 8 is done)

---

### Task 8: Wire up enrichment threads in `app.rs`

**Files:**
- Modify: `src/app.rs`

**Step 1: Add `spawn_remote_enrich`**

Add new method to `impl App`:

```rust
fn spawn_remote_enrich(&mut self) {
    let repo_path = self.repo_path.clone();
    let base_branch = self.base_branch.clone();
    let branches = self.remote_branches.clone();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        branch::enrich_remote_branches(&repo_path, &base_branch, branches, tx);
    });
    self.remote_enrich_rx = Some(rx);
}
```

**Step 2: Add `spawn_worktree_status_enrich`**

```rust
fn spawn_worktree_status_enrich(&mut self) {
    let worktrees = self.worktrees.clone();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        worktree::enrich_worktrees(worktrees, tx);
    });
    self.worktree_enrich_rx = Some(rx);
}
```

**Step 3: Rename `spawn_worktree_enrich` → `spawn_worktree_branch_enrich`**

Find the existing `fn spawn_worktree_enrich` and rename it to `spawn_worktree_branch_enrich`. Update its call site in `drain_worktree_load_rx` from `self.spawn_worktree_enrich()` to `self.spawn_worktree_branch_enrich()`.

**Step 4: Update `drain_remote_load_rx` to spawn remote enrich**

After `self.remote_loading = false;` and the existing state reset code in `drain_remote_load_rx`, call:
```rust
self.spawn_remote_enrich();
```

Also update the squash candidates setup: since all branches now arrive as `Unmerged`, the existing candidate filter (`b.merge_status == MergeStatus::Unmerged && !b.is_base`) already works correctly — no change needed there.

**Step 5: Replace `drain_worktree_enrich_rx`**

The existing function receives a single `WorktreeEnrich` batch and replaces the whole vec. Replace it entirely with:

```rust
fn drain_worktree_enrich_rx(&mut self) {
    use std::sync::mpsc::TryRecvError;
    let Some(rx) = &self.worktree_enrich_rx else { return };

    let mut drained = 0;
    let done = loop {
        if drained >= 32 { break false; }
        match rx.try_recv() {
            Ok(result) => {
                drained += 1;
                if result.index < self.worktrees.len() {
                    self.worktrees[result.index].wt_status = result.wt_status;
                    self.worktrees[result.index].age_date = result.age_date;
                }
            }
            Err(TryRecvError::Empty) => break false,
            Err(TryRecvError::Disconnected) => break true,
        }
    };

    if done {
        self.worktree_enrich_rx = None;
        self.apply_worktree_sort();
        self.set_toast("Sort updated", Duration::from_secs(2));
    }
}
```

**Step 6: Add `drain_remote_enrich_rx`**

Add new method after `drain_remote_load_rx`:

```rust
fn drain_remote_enrich_rx(&mut self) {
    use std::sync::mpsc::TryRecvError;
    let Some(rx) = &self.remote_enrich_rx else { return };

    // Build full_ref → index map once per drain
    let index_map: HashMap<String, usize> = self
        .remote_branches
        .iter()
        .enumerate()
        .map(|(i, b)| (b.full_ref.clone(), i))
        .collect();

    let mut drained = 0;
    let done = loop {
        if drained >= 32 { break false; }
        match rx.try_recv() {
            Ok(result) => {
                drained += 1;
                if let Some(&idx) = index_map.get(&result.full_ref) {
                    self.remote_branches[idx].merge_status = result.merge_status;
                    self.remote_branches[idx].ahead = result.ahead;
                    self.remote_branches[idx].behind = result.behind;
                }
            }
            Err(TryRecvError::Empty) => break false,
            Err(TryRecvError::Disconnected) => break true,
        }
    };

    if done {
        self.remote_enrich_rx = None;
        self.apply_remote_sort();
        self.set_toast("Sort updated", Duration::from_secs(2));
    }
}
```

**Step 7: Call `drain_remote_enrich_rx` in the `run` loop**

In `pub fn run`, add `self.drain_remote_enrich_rx();` alongside the other drain calls.

**Step 8: Update `drain_worktree_load_rx` to spawn status enrich**

After `self.spawn_worktree_branch_enrich();` in `drain_worktree_load_rx`, also call:
```rust
self.spawn_worktree_status_enrich();
```

**Step 9: Build**

```
cargo build
```
Expected: clean build.

**Step 10: Run tests**

```
cargo test
```
Expected: all pass.

**Step 11: Commit**

```bash
git add src/app.rs
git commit -m "feat: wire progressive enrichment threads for remote branches and worktrees"
```

---

### Task 9: Generalise the toast UI

**Files:**
- Modify: `src/ui/shared.rs`
- Modify: `src/ui/remote_branch_list.rs`
- Modify: `src/ui/worktree_list.rs`

Both views currently have identical 12-line hardcoded toast blocks. Replace all three with a shared helper.

**Step 1: Add `draw_toast` to `src/ui/shared.rs`**

Add at the bottom of the file:

```rust
/// Render a toast notification in the bottom-right corner of `area` if `app.toast` is set.
/// Uses `app.theme.toast_text` and `app.theme.toast_border` for styling.
pub fn draw_toast(frame: &mut ratatui::Frame, area: ratatui::layout::Rect, app: &crate::app::App) {
    use ratatui::widgets::{Block, Borders, Clear, Paragraph};
    use ratatui::layout::Alignment;

    let Some(ref msg) = app.toast else { return };

    let padded = format!(" {} ", msg);
    let toast_width = padded.len() as u16 + 2; // +2 for border
    let toast_height: u16 = 3;
    let x = area.width.saturating_sub(toast_width).saturating_sub(1);
    let y = area.height.saturating_sub(toast_height).saturating_sub(2);
    let toast_area = ratatui::layout::Rect::new(x, y, toast_width, toast_height);

    let toast = Paragraph::new(padded.as_str())
        .style(app.theme.toast_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(app.theme.toast_border),
        )
        .alignment(Alignment::Center);
    frame.render_widget(Clear, toast_area);
    frame.render_widget(toast, toast_area);
}
```

**Step 2: Replace hardcoded fetch toast in `remote_branch_list.rs`**

Find and delete the existing 15-line block:
```rust
// Toast overlay while fetching remote branches
if app.remote_loading {
    let msg = " Fetching remote branches… ";
    ...
    frame.render_widget(toast, toast_area);
}
```

Replace with:
```rust
super::shared::draw_toast(frame, area, app);
```

**Step 3: Set `app.toast` when fetch starts/ends**

In `app.rs`, find `open_remote_branches_view` where the fetch thread is spawned. After `self.remote_loading = true;`, call:
```rust
self.set_toast("Fetching remote branches\u{2026}", Duration::from_secs(60));
```
(60 seconds is a ceiling — the toast will be replaced by "Sort updated" when enrichment finishes, or cleared when the fetch completes.)

In the `run` loop's `remote_fetch_rx` drain block (around line 514), when fetch completes successfully, clear the fetch toast (it will be replaced by the enrich toast later):
```rust
self.toast = None;
self.toast_expires = None;
```

**Step 4: Replace hardcoded loading toast in `worktree_list.rs`**

Find and delete the existing "Loading worktrees…" toast block:
```rust
if app.worktree_loading {
    let msg = " Loading worktrees… ";
    ...
    frame.render_widget(toast, toast_area);
}
```

Replace with:
```rust
super::shared::draw_toast(frame, area, app);
```

**Step 5: Set `app.toast` when worktree load starts**

In `app.rs`, in `spawn_worktree_load`, after `self.worktree_loading = true;`, call:
```rust
self.set_toast("Loading worktrees\u{2026}", Duration::from_secs(30));
```

In `drain_worktree_load_rx`, when load completes (`self.worktree_loading = false;`), clear the loading toast:
```rust
self.toast = None;
self.toast_expires = None;
```

**Step 6: Build**

```
cargo build
```
Expected: clean build.

**Step 7: Run tests**

```
cargo test
```
Expected: all pass.

**Step 8: Commit**

```bash
git add src/ui/shared.rs src/ui/remote_branch_list.rs src/ui/worktree_list.rs src/app.rs
git commit -m "feat: generalise toast into shared draw_toast helper"
```

---

### Task 10: Add enrich progress to status bars

**Files:**
- Modify: `src/app.rs`
- Modify: `src/ui/remote_branch_list.rs`
- Modify: `src/ui/worktree_list.rs`

**Step 1: Add enrich progress counters to `App`**

Add to the `App` struct:
```rust
pub remote_enrich_checked: usize,
pub remote_enrich_total: usize,
pub worktree_enrich_checked: usize,
pub worktree_enrich_total: usize,
```

Initialise all four to `0` in `App::new`.

**Step 2: Set totals when enrichment spawns**

In `spawn_remote_enrich`:
```rust
self.remote_enrich_checked = 0;
self.remote_enrich_total = self.remote_branches.len();
```

In `spawn_worktree_status_enrich`:
```rust
self.worktree_enrich_checked = 0;
self.worktree_enrich_total = self.worktrees.len();
```

**Step 3: Increment checked counter in drain functions**

In `drain_remote_enrich_rx`, after each successful `Ok(result)`:
```rust
self.remote_enrich_checked += 1;
```

In `drain_worktree_enrich_rx`, after each successful `Ok(result)`:
```rust
self.worktree_enrich_checked += 1;
```

**Step 4: Show progress in remote branches status bar**

In `remote_branch_list.rs`, find the `progress` string computation (currently shows squash check progress). Add a second progress string for enrich, and concatenate:

```rust
let enrich_progress = if app.remote_enrich_total > 0
    && app.remote_enrich_checked < app.remote_enrich_total
{
    format!(" | enriching {}/{}", app.remote_enrich_checked, app.remote_enrich_total)
} else {
    String::new()
};
```

Append `enrich_progress` to `status_text` alongside the existing `progress`.

**Step 5: Show progress in worktree status bar**

In `worktree_list.rs`, add to the status text:
```rust
let enrich_progress = if app.worktree_enrich_total > 0
    && app.worktree_enrich_checked < app.worktree_enrich_total
{
    format!(" | enriching {}/{}", app.worktree_enrich_checked, app.worktree_enrich_total)
} else {
    String::new()
};
```

Include `enrich_progress` in the `status_text` format strings for both width variants.

**Step 6: Build**

```
cargo build
```
Expected: clean build.

**Step 7: Run tests**

```
cargo test
```
Expected: all pass.

**Step 8: Manual smoke test**

```
cargo run
```
- Press Tab → Remote Branches view appears instantly with branch list but all showing `Unmerged` / no A/B
- Status bar shows `| enriching 0/221` (or similar)
- Columns fill in progressively
- When enrichment completes, status bar progress disappears and "Sort updated" toast flashes briefly
- Press Tab → Worktrees view appears instantly with branch names but no wt_status
- Status bar shows `| enriching 0/23`
- Rows fill in progressively
- "Sort updated" toast appears when done

**Step 9: Commit**

```bash
git add src/app.rs src/ui/remote_branch_list.rs src/ui/worktree_list.rs
git commit -m "feat: show per-item enrichment progress in status bars"
```

---

### Task 11: Final cleanup and verification

**Files:**
- Check: `src/git/branch.rs`, `src/git/worktree.rs`, `src/app.rs`

**Step 1: Check for dead code warnings**

```
cargo build 2>&1 | grep warning
```

Investigate any new `dead_code` warnings introduced by this work. The three pre-existing warnings (`age_style`, `truncate`, `ahead_behind`) are fine to leave.

**Step 2: Run full test suite**

```
cargo test
```
Expected: all pass.

**Step 3: Run clippy**

```
cargo clippy
```
Fix any new clippy lints introduced by this work.

**Step 4: Commit any cleanup**

```bash
git add -p
git commit -m "chore: cleanup after progressive enrichment implementation"
```

---

## Summary of Commits (expected order)

1. `chore: remove GBM_TIMING profiling instrumentation`
2. `feat: add RemoteEnrichResult and WorktreeEnrichResult types`
3. `feat: strip graph ops from list_remote_branches_phase1`
4. `feat: add enrich_remote_branches background enrichment function`
5. `feat: strip status_and_age from worktree phase-1 load`
6. `feat: add enrich_worktrees background enrichment function`
7. `feat: wire progressive enrichment threads for remote branches and worktrees` (Tasks 7+8 combined)
8. `feat: generalise toast into shared draw_toast helper`
9. `feat: show per-item enrichment progress in status bars`
10. `chore: cleanup after progressive enrichment implementation`
