# Worktrees View Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a `Worktrees` view that lists all active git worktrees with branch, age, ahead/behind, PR, and status columns, and supports removing worktrees via a context menu.

**Architecture:** Two-phase load: phase 1 (fast parse via `git worktree list --porcelain` + per-worktree `git status --porcelain` + age computation) runs on first open (or at startup if `load_worktrees_on_launch = true`). Phase 2 (branch enrichment — joins `BranchInfo` and `PrMap` to fill in a/b, merge status, PR) runs in the background after phase 1 completes. Results arrive via `mpsc` channels and are drained in the event loop.

**Tech Stack:** Rust, ratatui, crossterm, git CLI via `std::process::Command`, existing `mpsc` channel patterns from `remote_branches`/`tags` loaders.

---

### Task 1: Add `WorktreeInfo` and message types to `types.rs`

**Files:**
- Modify: `src/types.rs`

**Step 1: Add the struct and two new `BranchAction` variants**

Add after the `RemoteBranchInfo` impl block in `src/types.rs`:

```rust
/// All information about a single git worktree.
#[derive(Debug, Clone)]
pub struct WorktreeInfo {
    /// Absolute path to this worktree directory.
    pub path: std::path::PathBuf,
    /// Checked-out branch name, or `None` if detached HEAD.
    pub branch: Option<String>,
    /// True for the main (primary) worktree.
    pub is_main: bool,
    /// Short (7-char) HEAD commit SHA.
    pub commit_hash: String,
    /// Working tree status (staged/unstaged/untracked).
    pub wt_status: WorkingTreeStatus,
    /// Age date: newest mtime of dirty files if dirty, else HEAD commit date.
    pub age_date: chrono::DateTime<chrono::Utc>,
    // Fields below are populated by phase 2 (branch enrichment):
    pub merge_status: MergeStatus,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
    pub pr: Option<crate::git::github::PrStatus>,
}

impl WorktreeInfo {
    /// True for the main worktree (pinned to top).
    pub fn is_pinned(&self) -> bool {
        self.is_main
    }

    pub fn age_display(&self) -> String {
        format_age(&self.age_date)
    }

    pub fn age_short(&self) -> String {
        format_age_short(&self.age_date)
    }
}
```

Add two variants to `BranchAction` after `CheckoutRemote`:

```rust
WorktreeRemove,
WorktreeForceRemove,
```

Add their labels in `BranchAction::label()`:

```rust
BranchAction::WorktreeRemove => "Remove worktree",
BranchAction::WorktreeForceRemove => "Force remove worktree",
```

**Step 2: Build to verify types compile**

```sh
cargo build 2>&1 | head -30
```

Expected: compile errors only about `PrStatus` if it's not yet pub-re-exported — fix if needed, or accept "unused variant" warnings.

**Step 3: Commit**

```sh
git add src/types.rs
git commit -m "feat: add WorktreeInfo struct and WorktreeRemove/ForceRemove actions"
```

---

### Task 2: Add `load_worktrees_on_launch` to config

**Files:**
- Modify: `src/config.rs`
- Modify: `src/ui/settings.rs`
- Modify: `src/app.rs` (settings key handler)

**Step 1: Add the config field**

In `src/config.rs`, add after `auto_fetch`:

```rust
#[serde(default)]
pub load_worktrees_on_launch: Option<bool>,
```

**Step 2: Add the row to the settings UI**

In `src/ui/settings.rs`, add `load_worktrees_on_launch` display. Currently there are 5 rows (indices 0–4). Add a 6th:

```rust
let load_worktrees_display = if app.config.load_worktrees_on_launch == Some(true) {
    "on".to_string()
} else {
    "off".to_string()
};
```

Add to `rows`:

```rust
("Load worktrees on launch", load_worktrees_display),
```

Update the `height` from `13` to `14` (one more row):

```rust
let height = 14u16.min(area.height);
```

**Step 3: Add key handler for the new setting row**

In `src/app.rs::handle_settings_key`, the bounds check currently uses `.min(4)` for the cursor. Change to `.min(5)`:

```rust
*cursor = (*cursor + 1).min(5); // 6 rows (index 0..=5)
```

Add `cursor == 5` branches in both `KeyCode::Right` and `KeyCode::Left` handlers:

```rust
} else if cursor == 5 {
    self.config.load_worktrees_on_launch =
        Some(self.config.load_worktrees_on_launch != Some(true));
    self.config.save();
}
```

Add the same toggle in the `KeyCode::Char(' ')` handler:

```rust
} else if cursor == 5 {
    self.config.load_worktrees_on_launch =
        Some(self.config.load_worktrees_on_launch != Some(true));
    self.config.save();
}
```

**Step 4: Build**

```sh
cargo build 2>&1 | head -30
```

Expected: clean build.

**Step 5: Commit**

```sh
git add src/config.rs src/ui/settings.rs src/app.rs
git commit -m "feat: add load_worktrees_on_launch config setting"
```

---

### Task 3: Create `src/git/worktree.rs` — phase 1 loader

**Files:**
- Create: `src/git/worktree.rs`
- Modify: `src/git/mod.rs`
- Modify: `src/lib.rs`

**Step 3a: Write `src/git/worktree.rs`**

```rust
use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::{DateTime, TimeZone, Utc};

use crate::types::{MergeStatus, WorkingTreeStatus, WorktreeInfo};

/// Run a git command in `dir`, return stdout as String.
fn git_out(dir: &Path, args: &[&str]) -> String {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// Parse `git worktree list --porcelain` output into a list of raw fields.
/// Each worktree block is separated by a blank line and contains:
///   worktree <path>
///   HEAD <sha>
///   branch refs/heads/<name>   -- OR --
///   detached
fn parse_porcelain(output: &str) -> Vec<WorktreeInfo> {
    let mut result = Vec::new();
    let mut path: Option<PathBuf> = None;
    let mut sha = String::new();
    let mut branch: Option<String> = None;
    let mut is_main = true; // first block is always main

    for line in output.lines() {
        if line.is_empty() {
            if let Some(p) = path.take() {
                let wt = build_worktree(p, sha.clone(), branch.take(), is_main);
                result.push(wt);
                is_main = false;
                sha.clear();
            }
        } else if let Some(rest) = line.strip_prefix("worktree ") {
            path = Some(PathBuf::from(rest));
        } else if let Some(rest) = line.strip_prefix("HEAD ") {
            sha = rest.chars().take(7).collect();
        } else if let Some(rest) = line.strip_prefix("branch refs/heads/") {
            branch = Some(rest.to_string());
        }
        // "detached" line — branch stays None
    }

    // flush last block (no trailing blank line in some git versions)
    if let Some(p) = path {
        let wt = build_worktree(p, sha, branch, is_main);
        result.push(wt);
    }

    result
}

fn build_worktree(
    path: PathBuf,
    commit_hash: String,
    branch: Option<String>,
    is_main: bool,
) -> WorktreeInfo {
    let (wt_status, age_date) = status_and_age(&path);

    WorktreeInfo {
        path,
        branch,
        is_main,
        commit_hash,
        wt_status,
        age_date,
        merge_status: MergeStatus::Unmerged,
        ahead: None,
        behind: None,
        pr: None,
    }
}

/// Compute working tree status and age for a worktree directory.
///
/// Runs `git status --porcelain` in `dir`. If dirty, walks the listed file
/// paths to find the newest mtime. If clean, reads HEAD commit date via
/// `git log -1 --format=%ct HEAD`.
fn status_and_age(dir: &Path) -> (WorkingTreeStatus, DateTime<Utc>) {
    let status_out = git_out(dir, &["status", "--porcelain"]);

    let mut has_staged = false;
    let mut has_unstaged = false;
    let mut has_untracked = false;
    let mut dirty_paths: Vec<PathBuf> = Vec::new();

    for line in status_out.lines() {
        if line.len() < 3 {
            continue;
        }
        let xy = &line[..2];
        let file = line[3..].trim();
        // XY format: index status (X) then worktree status (Y)
        let x = xy.chars().next().unwrap_or(' ');
        let y = xy.chars().nth(1).unwrap_or(' ');

        if x == '?' && y == '?' {
            has_untracked = true;
            dirty_paths.push(dir.join(file));
        } else {
            if x != ' ' && x != '?' {
                has_staged = true;
            }
            if y != ' ' && y != '?' {
                has_unstaged = true;
                dirty_paths.push(dir.join(file));
            } else if x != ' ' && x != '?' {
                dirty_paths.push(dir.join(file));
            }
        }
    }

    let wt_status = WorkingTreeStatus {
        has_staged,
        has_unstaged,
        has_untracked,
    };

    let age_date = if wt_status.is_clean() {
        // Clean: use HEAD commit date
        head_commit_date(dir)
    } else {
        // Dirty: newest mtime of changed files
        newest_mtime(&dirty_paths).unwrap_or_else(|| head_commit_date(dir))
    };

    (wt_status, age_date)
}

fn head_commit_date(dir: &Path) -> DateTime<Utc> {
    let out = git_out(dir, &["log", "-1", "--format=%ct", "HEAD"]);
    out.trim()
        .parse::<i64>()
        .ok()
        .and_then(|ts| Utc.timestamp_opt(ts, 0).single())
        .unwrap_or_else(Utc::now)
}

fn newest_mtime(paths: &[PathBuf]) -> Option<DateTime<Utc>> {
    paths
        .iter()
        .filter_map(|p| {
            std::fs::metadata(p)
                .ok()
                .and_then(|m| m.modified().ok())
                .map(|t| DateTime::<Utc>::from(t))
        })
        .max()
}

/// List all worktrees for the repo rooted at `repo_path`.
/// Returns a `Vec<WorktreeInfo>` with phase-1 data only (no a/b, PR, merge status).
pub fn list_worktrees(repo_path: &Path) -> Vec<WorktreeInfo> {
    let out = git_out(repo_path, &["worktree", "list", "--porcelain"]);
    parse_porcelain(&out)
}
```

**Step 3b: Expose the module**

In `src/git/mod.rs`, add:

```rust
pub mod worktree;
```

In `src/lib.rs`, verify `pub mod git;` is already present (it is). The `worktree` module will be accessible as `git_branch_manager::git::worktree`.

**Step 3c: Build**

```sh
cargo build 2>&1 | head -30
```

Expected: clean. Fix any import issues.

**Step 3d: Commit**

```sh
git add src/git/worktree.rs src/git/mod.rs
git commit -m "feat: add git/worktree.rs phase-1 loader"
```

---

### Task 4: Add worktree remove operations to `git/operations.rs`

**Files:**
- Modify: `src/git/operations.rs`

**Step 1: Add two functions at the end of the file**

```rust
/// Remove a worktree (`git worktree remove <path>`).
/// Fails if the worktree has uncommitted changes.
pub fn remove_worktree(repo_path: &Path, worktree_path: &Path) -> OperationResult {
    let path_str = worktree_path.to_string_lossy();
    match git_cmd(repo_path)
        .args(["worktree", "remove", &path_str])
        .output()
    {
        Ok(o) if o.status.success() => OperationResult {
            branch_name: path_str.to_string(),
            action: crate::types::BranchAction::WorktreeRemove,
            success: true,
            message: format!("Removed worktree at {}", path_str),
        },
        Ok(o) => OperationResult {
            branch_name: path_str.to_string(),
            action: crate::types::BranchAction::WorktreeRemove,
            success: false,
            message: format!("Failed: {}", String::from_utf8_lossy(&o.stderr).trim()),
        },
        Err(e) => OperationResult {
            branch_name: path_str.to_string(),
            action: crate::types::BranchAction::WorktreeRemove,
            success: false,
            message: format!("Failed: {}", e),
        },
    }
}

/// Force-remove a worktree (`git worktree remove --force <path>`).
/// Works even if the worktree has uncommitted changes.
pub fn force_remove_worktree(repo_path: &Path, worktree_path: &Path) -> OperationResult {
    let path_str = worktree_path.to_string_lossy();
    match git_cmd(repo_path)
        .args(["worktree", "remove", "--force", &path_str])
        .output()
    {
        Ok(o) if o.status.success() => OperationResult {
            branch_name: path_str.to_string(),
            action: crate::types::BranchAction::WorktreeForceRemove,
            success: true,
            message: format!("Force-removed worktree at {}", path_str),
        },
        Ok(o) => OperationResult {
            branch_name: path_str.to_string(),
            action: crate::types::BranchAction::WorktreeForceRemove,
            success: false,
            message: format!("Failed: {}", String::from_utf8_lossy(&o.stderr).trim()),
        },
        Err(e) => OperationResult {
            branch_name: path_str.to_string(),
            action: crate::types::BranchAction::WorktreeForceRemove,
            success: false,
            message: format!("Failed: {}", e),
        },
    }
}
```

**Step 2: Build**

```sh
cargo build 2>&1 | head -30
```

**Step 3: Commit**

```sh
git add src/git/operations.rs
git commit -m "feat: add remove_worktree and force_remove_worktree operations"
```

---

### Task 5: Add `View::Worktrees` and App state fields

**Files:**
- Modify: `src/app.rs`

**Step 1: Add `View::Worktrees` to the `View` enum**

Locate:
```rust
pub enum View {
    ...
    RemoteFilter,
}
```

Add after `RemoteFilter`:
```rust
Worktrees,
```

**Step 2: Add `ResultsReturnView::Worktrees`**

Locate `ResultsReturnView` enum, add:
```rust
Worktrees,
```

**Step 3: Add worktree-related fields to `App`**

After the remote branches state block (around line 234), add:

```rust
// ── Worktrees state ──
pub worktrees: Vec<WorktreeInfo>,
pub worktree_cursor: usize,
pub worktree_table_state: TableState,
pub worktree_selected: Vec<bool>,
/// Receiver for phase-1 worktree load.
pub worktree_load_rx: Option<Receiver<WorktreeLoad>>,
/// Receiver for phase-2 branch enrichment.
pub worktree_enrich_rx: Option<Receiver<WorktreeEnrich>>,
/// True while phase-1 is in progress.
pub worktree_loading: bool,
```

**Step 4: Add `WorktreeLoad` and `WorktreeEnrich` payload structs**

Near the top of `app.rs` with the other payload structs:

```rust
pub(crate) struct WorktreeLoad {
    pub worktrees: Vec<WorktreeInfo>,
}

pub(crate) struct WorktreeEnrich {
    /// Updated worktrees with branch data filled in.
    pub worktrees: Vec<WorktreeInfo>,
}
```

**Step 5: Initialize the new fields in `App::new()`**

In the `Self { ... }` block, add:

```rust
worktrees: Vec::new(),
worktree_cursor: 0,
worktree_table_state: TableState::default(),
worktree_selected: Vec::new(),
worktree_load_rx: None,
worktree_enrich_rx: None,
worktree_loading: false,
```

**Step 6: Add the import for `WorktreeInfo`**

In the existing use block for types:
```rust
use git_branch_manager::types::{..., WorktreeInfo};
```

Also import the new worktree module:
```rust
use git_branch_manager::git::worktree;
```

**Step 7: Build**

```sh
cargo build 2>&1 | head -50
```

Fix any missing fields or import errors.

**Step 8: Commit**

```sh
git add src/app.rs
git commit -m "feat: add View::Worktrees, App worktree state fields, and payload types"
```

---

### Task 6: Add `open_worktrees_view()`, loaders, and event-loop drains

**Files:**
- Modify: `src/app.rs`

**Step 1: Add `open_worktrees_view()`**

After `open_remote_branches_view()`:

```rust
/// Open the Worktrees view.
///
/// Spawns a background thread to run phase-1 (parse + age).
/// Phase-2 enrichment is spawned automatically once phase-1 completes.
fn open_worktrees_view(&mut self) {
    self.spawn_worktree_load();
    self.view = View::Worktrees;
}

fn spawn_worktree_load(&mut self) {
    let repo_path = self.repo_path.clone();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let worktrees = worktree::list_worktrees(&repo_path);
        let _ = tx.send(WorktreeLoad { worktrees });
    });
    self.worktree_load_rx = Some(rx);
    self.worktree_loading = true;
}

fn spawn_worktree_enrich(&mut self) {
    let worktrees = self.worktrees.clone();
    let branches = self.branches.clone();
    let pr_map = self.pr_map.clone();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let enriched: Vec<WorktreeInfo> = worktrees
            .into_iter()
            .map(|mut wt| {
                if let Some(ref branch_name) = wt.branch {
                    if let Some(b) = branches.iter().find(|b| &b.name == branch_name) {
                        wt.merge_status = b.merge_status.clone();
                        wt.ahead = b.ahead;
                        wt.behind = b.behind;
                        wt.pr = pr_map.get(branch_name).map(|p| p.status.clone());
                    }
                }
                wt
            })
            .collect();
        let _ = tx.send(WorktreeEnrich { worktrees: enriched });
    });
    self.worktree_enrich_rx = Some(rx);
}
```

**Step 2: Add drain functions**

After `drain_remote_load_rx()`:

```rust
fn drain_worktree_load_rx(&mut self) {
    use std::sync::mpsc::TryRecvError;

    let Some(rx) = &self.worktree_load_rx else { return };

    match rx.try_recv() {
        Ok(load) => {
            self.worktree_load_rx = None;
            self.worktree_loading = false;
            let len = load.worktrees.len();
            self.worktrees = load.worktrees;
            self.worktree_selected = vec![false; len];
            self.worktree_cursor = 0;
            self.worktree_table_state = TableState::default().with_selected(
                if len == 0 { None } else { Some(0) },
            );
            // Spawn phase-2 enrichment now that phase-1 is done
            self.spawn_worktree_enrich();
        }
        Err(TryRecvError::Empty) => {}
        Err(TryRecvError::Disconnected) => {
            self.worktree_load_rx = None;
            self.worktree_loading = false;
        }
    }
}

fn drain_worktree_enrich_rx(&mut self) {
    use std::sync::mpsc::TryRecvError;

    let Some(rx) = &self.worktree_enrich_rx else { return };

    match rx.try_recv() {
        Ok(enrich) => {
            self.worktree_enrich_rx = None;
            self.worktrees = enrich.worktrees;
        }
        Err(TryRecvError::Empty) => {}
        Err(TryRecvError::Disconnected) => {
            self.worktree_enrich_rx = None;
        }
    }
}
```

**Step 3: Wire drains into the event loop**

In `App::run()`, add two calls alongside the other drains:

```rust
self.drain_worktree_load_rx();
self.drain_worktree_enrich_rx();
```

**Step 4: Handle `load_worktrees_on_launch` in `drain_load_rx()`**

At the end of the `Ok(load)` arm in `drain_load_rx()`, after spawning the PR loader:

```rust
// If configured, pre-load worktrees in the background
if self.config.load_worktrees_on_launch == Some(true) {
    self.spawn_worktree_load();
}
```

**Step 5: Build**

```sh
cargo build 2>&1 | head -50
```

**Step 6: Commit**

```sh
git add src/app.rs
git commit -m "feat: add worktree loader, enrichment, and event-loop drains"
```

---

### Task 7: Add key handler for the Worktrees view

**Files:**
- Modify: `src/app.rs`

**Step 1: Add `handle_event` dispatch**

In `handle_event`, in the `match &self.view` block, add:

```rust
View::Worktrees => self.handle_worktrees_key(key.code),
```

Also add scroll support in the mouse scroll arms:

```rust
} else if self.view == View::Worktrees {
    self.handle_worktrees_key(KeyCode::Down); // or Up
}
```

**Step 2: Add `handle_worktrees_key()`**

```rust
fn handle_worktrees_key(&mut self, code: KeyCode) {
    let len = self.worktrees.len();

    match code {
        KeyCode::Char('j') | KeyCode::Down => {
            if len > 0 && self.worktree_cursor + 1 < len {
                self.worktree_cursor += 1;
                self.worktree_table_state.select(Some(self.worktree_cursor));
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if self.worktree_cursor > 0 {
                self.worktree_cursor -= 1;
                self.worktree_table_state.select(Some(self.worktree_cursor));
            }
        }
        KeyCode::PageDown => {
            let new = (self.worktree_cursor + 20).min(len.saturating_sub(1));
            self.worktree_cursor = new;
            self.worktree_table_state.select(Some(new));
        }
        KeyCode::PageUp => {
            self.worktree_cursor = self.worktree_cursor.saturating_sub(20);
            self.worktree_table_state.select(Some(self.worktree_cursor));
        }
        KeyCode::Enter => {
            self.view = View::Menu { cursor: 0 };
        }
        KeyCode::Char('d') => {
            if !self.worktrees.is_empty() {
                let wt = &self.worktrees[self.worktree_cursor];
                if wt.wt_status.is_clean() {
                    self.results_return_view = ResultsReturnView::Worktrees;
                    self.view = View::Confirm {
                        action: BranchAction::WorktreeRemove,
                    };
                }
            }
        }
        KeyCode::Char('D') => {
            if !self.worktrees.is_empty() {
                self.results_return_view = ResultsReturnView::Worktrees;
                self.view = View::Confirm {
                    action: BranchAction::WorktreeForceRemove,
                };
            }
        }
        KeyCode::Tab | KeyCode::Char('w') => {
            self.view = View::BranchList;
        }
        KeyCode::Char('r') => {
            self.open_remote_branches_view();
        }
        KeyCode::Char('t') => {
            self.tag_cursor = 0;
            self.tag_search_query.clear();
            self.tag_search_active = false;
            self.tag_sort_by_name = false;
            self.load_tags();
        }
        KeyCode::Char('?') => {
            self.view = View::Help;
        }
        KeyCode::Char('T') => {
            self.theme = self.theme.next();
            let mut config = git_branch_manager::config::Config::load();
            config.theme = Some(self.theme.name.to_string());
            config.save();
        }
        KeyCode::Char('Y') => {
            self.symbols = crate::ui::symbols::next(self.symbols);
            let mut config = git_branch_manager::config::Config::load();
            config.symbols = Some(crate::ui::symbols::name(self.symbols).to_string());
            config.save();
        }
        KeyCode::Esc | KeyCode::Char('q') => {
            self.view = View::BranchList;
        }
        _ => {}
    }
}
```

**Step 3: Add worktrees shortcut (`w`) to branch list key handler**

In `handle_branch_list_key`, after `KeyCode::Char('r')`:

```rust
KeyCode::Char('w') => {
    self.open_worktrees_view();
}
```

Also add `w` in `handle_tags_key` and `handle_remote_branches_key` for consistent tab navigation.

**Step 4: Handle `WorktreeRemove` and `WorktreeForceRemove` in `execute_action_async()`**

After the `CheckoutRemote` block:

```rust
if action == BranchAction::WorktreeRemove || action == BranchAction::WorktreeForceRemove {
    if self.worktrees.is_empty() {
        return;
    }
    let wt_path = self.worktrees[self.worktree_cursor].path.clone();
    let repo_path = self.repo_path.clone();
    let force = action == BranchAction::WorktreeForceRemove;
    let cursor = self.worktree_cursor;
    self.spawn_op(label, move || {
        let result = if force {
            operations::force_remove_worktree(&repo_path, &wt_path)
        } else {
            operations::remove_worktree(&repo_path, &wt_path)
        };
        vec![result]
    });
    return;
}
```

**Step 5: Handle `ResultsReturnView::Worktrees` in `handle_results_key()`**

Add a branch:

```rust
ResultsReturnView::Worktrees => {
    self.results.clear();
    self.results_return_view = ResultsReturnView::BranchList;
    // Drop the removed worktree from the list instead of re-fetching
    // (the path is in the result message but we can just re-load for simplicity)
    self.open_worktrees_view();
}
```

Wait — the design says "drop from list in place". Do that instead. After the operation results come in (in `drain_op_rx`), we know which path was removed via `OperationResult.branch_name`. Handle it in `handle_results_key`:

```rust
ResultsReturnView::Worktrees => {
    // Drop successfully removed worktrees from the list in place
    for result in &self.results {
        if result.success
            && matches!(
                result.action,
                BranchAction::WorktreeRemove | BranchAction::WorktreeForceRemove
            )
        {
            let removed_path = &result.branch_name;
            self.worktrees.retain(|wt| wt.path.to_string_lossy() != *removed_path);
        }
    }
    self.results.clear();
    self.results_return_view = ResultsReturnView::BranchList;
    self.worktree_cursor = self.worktree_cursor.min(self.worktrees.len().saturating_sub(1));
    self.worktree_table_state.select(
        if self.worktrees.is_empty() { None } else { Some(self.worktree_cursor) },
    );
    self.view = View::Worktrees;
}
```

**Step 6: Handle `Confirm` overlay return for worktree actions**

In `handle_confirm_key`, the `Esc/n` arm already returns to `BranchList` by default. Add:

```rust
let is_worktree_action = matches!(
    &self.view,
    View::Confirm { action } if matches!(action,
        BranchAction::WorktreeRemove | BranchAction::WorktreeForceRemove
    )
);
```

And in the `Esc/n` arm, add:

```rust
} else if is_worktree_action {
    self.view = View::Worktrees;
}
```

Also in the `y` arm, add:

```rust
} else if is_worktree_action {
    self.results_return_view = ResultsReturnView::Worktrees;
}
```

**Step 7: Build**

```sh
cargo build 2>&1 | head -50
```

**Step 8: Commit**

```sh
git add src/app.rs
git commit -m "feat: add worktrees key handler, actions, and confirm/results wiring"
```

---

### Task 8: Build the worktree menu

**Files:**
- Modify: `src/app.rs`

The existing `build_menu_items()` and `handle_menu_key()` are branch-list-specific. The approach: add `build_worktree_menu_items()` and dispatch in `handle_menu_key()` based on current view.

**Step 1: Add `build_worktree_menu_items()`**

After `build_menu_items()`:

```rust
pub fn build_worktree_menu_items(&self) -> Vec<ui::menu::MenuItem> {
    if self.worktrees.is_empty() {
        return vec![];
    }
    let wt = &self.worktrees[self.worktree_cursor];
    let is_main = wt.is_main;
    let is_dirty = !wt.wt_status.is_clean();

    vec![
        ui::menu::MenuItem {
            label: "Remove worktree".into(),
            enabled: !is_main && !is_dirty,
            reason: if is_main {
                Some("main worktree".into())
            } else if is_dirty {
                Some("dirty".into())
            } else {
                None
            },
            shortcut: Some('d'),
        },
        ui::menu::MenuItem {
            label: "Force remove worktree".into(),
            enabled: !is_main,
            reason: if is_main {
                Some("main worktree".into())
            } else {
                None
            },
            shortcut: Some('D'),
        },
    ]
}
```

**Step 2: Dispatch in `handle_menu_key()`**

At the top of `handle_menu_key()`, detect which view owns the menu and use the appropriate items:

```rust
fn handle_menu_key(&mut self, code: KeyCode) {
    // Determine which item list to use based on return view
    let is_worktrees_menu = matches!(self.results_return_view, ResultsReturnView::Worktrees)
        || self.worktrees_menu_active; // see note below
```

Actually the cleanest approach: add a `menu_source: MenuSource` field to `App` that records which view opened the menu, set it in `handle_branch_list_key` and `handle_worktrees_key` when transitioning to `View::Menu`. Alternatively (simpler): check `self.view` history by storing which view was active before Menu.

**Simpler approach:** Add a `prev_view: View` field to `App`, set it whenever transitioning to `View::Menu { .. }`:

In `handle_branch_list_key` and `handle_worktrees_key` when setting `View::Menu`:
```rust
self.prev_view = View::BranchList; // or View::Worktrees
self.view = View::Menu { cursor: 0 };
```

Add `prev_view: View` to `App`:
```rust
pub prev_view: View,
```
Initialize as `View::BranchList` in `App::new()`.

Then in `handle_menu_key()`, at the top:

```rust
let items = if self.prev_view == View::Worktrees {
    self.build_worktree_menu_items()
} else {
    self.build_menu_items()
};
```

For `Esc` in `handle_menu_key`, return to `self.prev_view`:

```rust
KeyCode::Esc | KeyCode::Char('q') => {
    self.view = self.prev_view.clone();
}
```

For worktree menu item selection, map `Enter`/shortcut to actions:

```rust
// In the Enter arm, after determining menu_cursor:
if self.prev_view == View::Worktrees {
    let action = match menu_cursor {
        0 => BranchAction::WorktreeRemove,
        1 => BranchAction::WorktreeForceRemove,
        _ => return,
    };
    self.results_return_view = ResultsReturnView::Worktrees;
    self.view = View::Confirm { action };
    return;
}
// ... existing branch list menu logic
```

Similarly for `Char(ch)` shortcut dispatch.

**Step 3: Build**

```sh
cargo build 2>&1 | head -50
```

**Step 4: Commit**

```sh
git add src/app.rs
git commit -m "feat: add worktree context menu and prev_view tracking"
```

---

### Task 9: Create `src/ui/worktree_list.rs`

**Files:**
- Create: `src/ui/worktree_list.rs`
- Modify: `src/ui/mod.rs`
- Modify: `src/ui/render.rs`

**Step 1: Create the draw function**

Model closely after `src/ui/remote_branch_list.rs`. Key differences: columns are Path, Branch, Age, A/B, PR, Status.

```rust
use chrono::{DateTime, Utc};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};

use crate::app::App;
use git_branch_manager::git::github::PrStatus;
use git_branch_manager::types::MergeStatus;

fn age_style(date: &DateTime<Utc>) -> Style {
    let days = (Utc::now() - *date).num_days();
    if days < 7 {
        Style::new().fg(Color::Green)
    } else if days < 30 {
        Style::new().fg(Color::Yellow)
    } else if days < 90 {
        Style::new().fg(Color::Indexed(208))
    } else {
        Style::new().fg(Color::Red)
    }
}

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    app.terminal_rows = area.height;

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(area);

    let main_area = layout[0];
    let status_area = layout[1];

    let width = main_area.width as usize;
    let compact_age = width < 120;
    let hide_ab = width < 80;
    let short_status = width < 70;
    let hide_age = width < 60;

    let title = format!(
        "git-branch-manager \u{2014} worktrees (base: {})",
        app.base_branch
    );
    let block = Block::default()
        .title(title)
        .title_style(app.theme.title)
        .borders(Borders::ALL);

    // Build header
    let mut header_cells = vec![
        Cell::from("Path").style(app.theme.header),
        Cell::from("Branch").style(app.theme.header),
    ];
    if !hide_age {
        header_cells.push(Cell::from("Age").style(app.theme.header));
    }
    if !hide_ab {
        header_cells.push(Cell::from("A/B").style(app.theme.header));
        header_cells.push(Cell::from("PR").style(app.theme.header));
    }
    header_cells.push(Cell::from("Status").style(app.theme.header));
    let header = Row::new(header_cells).style(app.theme.header).height(1);

    // Build rows
    let rows: Vec<Row> = app
        .worktrees
        .iter()
        .enumerate()
        .map(|(i, wt)| {
            let is_cursor = i == app.worktree_cursor;
            let row_style = if is_cursor { app.theme.cursor } else { Style::default() };

            // Path: relative to repo root
            let rel_path = wt.path
                .strip_prefix(&app.repo_path)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| wt.path.to_string_lossy().to_string());
            let path_prefix = if wt.is_main { "[main] " } else { "" };
            let path_cell = Cell::from(format!("{}{}", path_prefix, rel_path)).style(row_style);

            // Branch
            let branch_str = wt.branch.as_deref().unwrap_or("(detached)");
            let branch_cell = Cell::from(branch_str).style(row_style);

            let mut cells = vec![path_cell, branch_cell];

            // Age
            if !hide_age {
                let age_str = if compact_age {
                    wt.age_short()
                } else {
                    wt.age_display()
                };
                cells.push(Cell::from(age_str).style(
                    if is_cursor { row_style } else { age_style(&wt.age_date) },
                ));
            }

            if !hide_ab {
                // A/B
                let ab_str = if wt.branch.is_some() {
                    match (wt.ahead, wt.behind) {
                        (Some(a), Some(b)) => format!("\u{2191}{} \u{2193}{}", a, b),
                        (Some(a), None) => format!("\u{2191}{}", a),
                        (None, Some(b)) => format!("\u{2193}{}", b),
                        (None, None) => String::new(),
                    }
                } else {
                    String::new()
                };
                cells.push(Cell::from(ab_str).style(row_style));

                // PR
                let pr_str = if wt.branch.is_some() {
                    match &wt.pr {
                        Some(PrStatus::Open(n)) => format!("#{}", n),
                        Some(PrStatus::Merged(n)) => format!("merged #{}", n),
                        Some(PrStatus::Closed(n)) => format!("closed #{}", n),
                        None => String::new(),
                    }
                } else {
                    String::new()
                };
                cells.push(Cell::from(pr_str).style(row_style));
            }

            // Status: merge status + wt_status
            let status_str = if wt.branch.is_some() {
                let merge = match wt.merge_status {
                    MergeStatus::Merged => if short_status { "merged" } else { "merged" },
                    MergeStatus::SquashMerged => if short_status { "squash" } else { "squash merged" },
                    MergeStatus::Unmerged => "",
                };
                let wt_s = wt.wt_status.summary();
                if merge.is_empty() {
                    wt_s
                } else if wt_s == "clean" {
                    merge.to_string()
                } else {
                    format!("{} {}", merge, wt_s)
                }
            } else {
                wt.wt_status.summary()
            };
            cells.push(Cell::from(status_str).style(row_style));

            Row::new(cells)
        })
        .collect();

    // Column widths
    let mut constraints = vec![
        Constraint::Min(20),  // path
        Constraint::Min(15),  // branch
    ];
    if !hide_age {
        constraints.push(Constraint::Length(if compact_age { 6 } else { 16 }));
    }
    if !hide_ab {
        constraints.push(Constraint::Length(10)); // a/b
        constraints.push(Constraint::Length(12)); // pr
    }
    constraints.push(Constraint::Min(12)); // status

    let loading_footer = if app.worktree_loading {
        " [loading...]"
    } else {
        ""
    };

    let table = Table::new(rows, constraints)
        .header(header)
        .block(block)
        .row_highlight_style(app.theme.cursor)
        .highlight_spacing(ratatui::widgets::HighlightSpacing::Never);

    frame.render_stateful_widget(table, main_area, &mut app.worktree_table_state);

    // Status bar
    let status_text = format!(
        "  ENTER menu  d remove  D force-remove  w branches  r remote  t tags  ?help{}",
        loading_footer
    );
    let status = Paragraph::new(status_text).style(app.theme.dim);
    frame.render_widget(status, status_area);
}
```

**Step 2: Expose the module**

In `src/ui/mod.rs`, add:

```rust
pub mod worktree_list;
```

**Step 3: Add match arm in `render.rs`**

Add to the imports:

```rust
use super::{..., worktree_list};
```

Add to `draw()`:

```rust
View::Worktrees => worktree_list::draw(frame, app),
```

Also handle `Confirm` overlay for worktree actions — render `worktree_list` as the background:

In the `Confirm` arm, add:

```rust
let is_worktree_action = matches!(
    action,
    git_branch_manager::types::BranchAction::WorktreeRemove
        | git_branch_manager::types::BranchAction::WorktreeForceRemove
);
// ... add to the if/else chain:
} else if is_worktree_action {
    worktree_list::draw(frame, app);
}
```

**Step 4: Build**

```sh
cargo build 2>&1 | head -50
```

Fix any import issues (PrStatus path, theme fields used, etc.).

**Step 5: Commit**

```sh
git add src/ui/worktree_list.rs src/ui/mod.rs src/ui/render.rs
git commit -m "feat: add worktree_list UI draw function"
```

---

### Task 10: Update `help.rs` and add `w` to tab navigation in other views

**Files:**
- Modify: `src/ui/help.rs`
- Modify: `src/app.rs`

**Step 1: Add worktrees entry to help text**

In `HELP_TEXT`, add after `r       Remote branches`:

```
w       Worktrees view
```

**Step 2: Add `w` shortcut to remaining views**

In `handle_tags_key`, after `KeyCode::Char('r')`:
```rust
KeyCode::Char('w') => {
    self.open_worktrees_view();
}
```

In `handle_remote_branches_key`, after `KeyCode::Char('t')`:
```rust
KeyCode::Char('w') => {
    self.open_worktrees_view();
}
```

**Step 3: Build and test**

```sh
cargo build 2>&1 | head -20
cargo run
```

Navigate: `w` from any view → Worktrees. Verify columns render. Navigate with j/k. Press Enter → context menu appears. Press `d` on a clean non-main worktree → confirm overlay → `y` → results → returns to Worktrees view with item removed.

**Step 4: Commit**

```sh
git add src/ui/help.rs src/app.rs
git commit -m "feat: add w shortcut to all views for Worktrees tab, update help"
```

---

### Task 11: Fix `confirm.rs` to not disable confirm for worktree actions

**Files:**
- Modify: `src/ui/confirm.rs`

`confirm.rs` has a list of actions that render as "single item, no selection" vs "batch". Check that `WorktreeRemove` and `WorktreeForceRemove` render correctly (they operate on a single worktree, not a multi-selection).

**Step 1: Review `confirm.rs`**

Read the file. Look for the match on `BranchAction` variants that controls confirm text. Add handling for `WorktreeRemove` and `WorktreeForceRemove` if needed (they should show the path of the worktree to be removed).

**Step 2: Build**

```sh
cargo build 2>&1 | head -20
```

**Step 3: Commit if changes were needed**

```sh
git add src/ui/confirm.rs
git commit -m "fix: handle WorktreeRemove/ForceRemove in confirm overlay"
```

---

### Task 12: Final verification

**Step 1: Full build and clippy**

```sh
cargo build && cargo clippy 2>&1 | head -40
```

Fix any warnings.

**Step 2: Manual smoke test**

```sh
cargo run
```

Verify:
1. `w` from BranchList opens Worktrees view
2. List shows path, branch (or `(detached)`), age, columns
3. Main worktree is pinned at top with `[main]` prefix
4. Phase-2 enrichment fills in a/b and status after a moment
5. Settings view shows "Load worktrees on launch" row, Space toggles
6. Toggle `load_worktrees_on_launch` on, quit and reopen — worktrees load without opening the view
7. `d` on dirty worktree is disabled in menu
8. `d` on clean non-main worktree → confirm → removes it from list in place
9. `D` force-removes dirty worktree after confirm

**Step 3: Commit any fixes, then final commit**

```sh
git add -p
git commit -m "fix: worktrees view smoke test fixes"
```
