# Local/Remote Merge Status Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extend `MergeStatus` with four new variants that distinguish whether a branch is merged into the local base branch, the remote tracking ref (`origin/<base>`), or both.

**Architecture:** Add `LocalMerged`, `RemoteMerged`, `LocalSquashMerged`, `RemoteSquashMerged` to the enum. Detection builds two reachable sets (local + remote) and maps branch OIDs to the appropriate variant. Rendering adds italic style and a directional arrow suffix symbol.

**Tech Stack:** Rust, git2, ratatui, existing symbol/theme/view infrastructure.

## Global Constraints

- `cargo build` must pass after every task.
- `cargo test` must pass after every task.
- No new dependencies.
- Column `min_width` → 5, `wide_width` → 16 (was 4 and 15).
- Italic modifier applied to all four new variants in cells rendering.
- New cache string keys use underscore separators consistent with existing `"squash_merged"`.

---

## File Map

| File | Change |
|------|--------|
| `src/types.rs` | Add 4 enum variants |
| `src/symbols.rs` | Add `status_local_suffix` + `status_remote_suffix` fields |
| `src/git/merge_detection.rs` | Replace "prefer remote" fix with dual-set detection; add `BaseReachable` struct |
| `src/git/squash_loader.rs` | Replace `effective_base_branch` with dual-pass squash detection |
| `src/git/cache.rs` | Add serialization for 4 new variants |
| `src/ui/cells.rs` | Add 4 match arms with italic + arrow suffix |
| `src/view/column.rs` | Widen merge column; extend sort ranks |
| `src/view/filter.rs` | Add 4 new filter tokens |

---

## Task 1: Add enum variants and cache serialization

**Files:**
- Modify: `src/types.rs`
- Modify: `src/git/cache.rs`

**Context:** `MergeStatus` is `#[derive(Serialize, Deserialize)]` in `types.rs` lines 8-14. Cache serializes status to strings in `BranchCache::insert` (lines ~359-383) and deserializes in `BranchCache::lookup` (lines ~280-290). The four new variants represent partial-sync states and are cached tied to commit hash (like `Unmerged` today — volatile, invalidated when commit changes).

- [ ] **Step 1: Add 4 variants to MergeStatus**

In `src/types.rs`, extend the enum:
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MergeStatus {
    Merged,
    SquashMerged,
    LocalMerged,         // merged into local base, not yet in origin/<base>
    RemoteMerged,        // merged into origin/<base>, local base not fast-forwarded
    LocalSquashMerged,   // squash-merged into local base only
    RemoteSquashMerged,  // squash-merged into origin/<base> only
    Unmerged,
    Pending,
}
```

- [ ] **Step 2: Update cache insert to handle new variants**

In `src/git/cache.rs`, find the `match status` block in `BranchCache::insert` and extend it:
```rust
let status_str = match status {
    MergeStatus::Merged => "merged",
    MergeStatus::SquashMerged => "squash_merged",
    MergeStatus::LocalMerged => "local_merged",
    MergeStatus::RemoteMerged => "remote_merged",
    MergeStatus::LocalSquashMerged => "local_squash_merged",
    MergeStatus::RemoteSquashMerged => "remote_squash_merged",
    MergeStatus::Unmerged => "unmerged",
    MergeStatus::Pending => {
        return; // never cache Pending
    }
};
```

- [ ] **Step 3: Update cache lookup to deserialize new variants**

In `BranchCache::lookup`, find the `match entry.merge_status.as_str()` block and extend it:
```rust
let status = match entry.merge_status.as_str() {
    "merged" => MergeStatus::Merged,
    "squash_merged" => MergeStatus::SquashMerged,
    "local_merged" => MergeStatus::LocalMerged,
    "remote_merged" => MergeStatus::RemoteMerged,
    "local_squash_merged" => MergeStatus::LocalSquashMerged,
    "remote_squash_merged" => MergeStatus::RemoteSquashMerged,
    "unmerged" => MergeStatus::Unmerged,
    _ => {
        self.record_miss();
        return None;
    }
};
```

- [ ] **Step 4: Fix exhaustiveness errors — stub new arms everywhere**

Run `cargo build 2>&1 | grep "non-exhaustive"` to find every match that needs new arms. Add stub arms returning/doing the same as `Merged` or `Unmerged` (whichever is closest). These will be replaced in later tasks. Example pattern:
```rust
MergeStatus::LocalMerged | MergeStatus::RemoteMerged => /* same as Merged for now */,
MergeStatus::LocalSquashMerged | MergeStatus::RemoteSquashMerged => /* same as SquashMerged for now */,
```

- [ ] **Step 5: Build and test**

```bash
cargo build 2>&1
cargo test 2>&1
```
Expected: clean build, all tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/types.rs src/git/cache.rs
git commit -m "feat: add LocalMerged/RemoteMerged/LocalSquashMerged/RemoteSquashMerged variants"
```

---

## Task 2: Add directional suffix symbols

**Files:**
- Modify: `src/symbols.rs`

**Context:** `SymbolSet` struct has fields `status_merged`, `status_squash_merged`, `status_unmerged`. Three sets: `ascii()`, `unicode()`, `powerline()`. The new fields `status_local_suffix` and `status_remote_suffix` provide the `↑`/`↓` arrow modifiers rendered after the base symbol.

- [ ] **Step 1: Add two fields to the SymbolSet struct**

In `src/symbols.rs`, add after `status_unmerged`:
```rust
pub struct SymbolSet {
    pub status_merged: &'static str,
    pub status_squash_merged: &'static str,
    pub status_unmerged: &'static str,
    pub status_local_suffix: &'static str,   // appended to base symbol for LocalMerged/LocalSquashMerged
    pub status_remote_suffix: &'static str,  // appended to base symbol for RemoteMerged/RemoteSquashMerged
    // ... remaining fields unchanged
}
```

- [ ] **Step 2: Fill in values for each symbol set**

`ascii()`:
```rust
status_local_suffix: "^",
status_remote_suffix: "v",
```

`unicode()`:
```rust
status_local_suffix: "\u{2191}",  // ↑ upwards arrow
status_remote_suffix: "\u{2193}", // ↓ downwards arrow
```

`powerline()`:
```rust
status_local_suffix: "\u{2191}",  // ↑ (same as unicode — no nerd font equivalent needed)
status_remote_suffix: "\u{2193}", // ↓
```

- [ ] **Step 3: Build**

```bash
cargo build 2>&1
```
Expected: clean build (the struct fields are added; no tests check these yet).

- [ ] **Step 4: Commit**

```bash
git add src/symbols.rs
git commit -m "feat: add status_local_suffix/status_remote_suffix to SymbolSet"
```

---

## Task 3: Dual-ref merge detection

**Files:**
- Modify: `src/git/merge_detection.rs`
- Modify: `src/git/squash_loader.rs`

**Context:** This task replaces the incorrect "prefer remote" partial fix (added earlier in this branch) with the correct dual-check approach. The current partial fix added `resolve_base_oid` (prefers `origin/<base>`) and `effective_base_branch` (same idea for CLI) — both need to be replaced.

### 3a: Regular merge detection

The current `detect_merged_branches` (line ~29) returns `HashSet<git2::Oid>` (local reachable set). We replace `resolve_base_oid` with a `BaseReachable` struct that holds both sets and knows how to map an OID to a `MergeStatus`.

- [ ] **Step 1: Replace `resolve_base_oid` with `BaseReachable` struct**

Remove the `resolve_base_oid` function and add:
```rust
/// Holds the reachable sets for both the local base branch and its remote tracking ref.
/// Used to determine whether a branch is merged, and if only into one side.
pub struct BaseReachable {
    pub local: HashSet<git2::Oid>,
    pub remote: HashSet<git2::Oid>,
}

impl BaseReachable {
    /// Returns the appropriate MergeStatus for a branch OID.
    /// When no remote tracking ref exists, local is treated as authoritative (returns Merged).
    pub fn regular_merge_status(&self, oid: git2::Oid) -> Option<MergeStatus> {
        let in_local = self.local.contains(&oid);
        let in_remote = self.remote.contains(&oid);
        let has_remote = !self.remote.is_empty();
        match (in_local, in_remote, has_remote) {
            (true, true, _) => Some(MergeStatus::Merged),
            (true, false, false) => Some(MergeStatus::Merged), // no remote — local is truth
            (false, true, _) => Some(MergeStatus::RemoteMerged),
            (true, false, true) => Some(MergeStatus::LocalMerged),
            (false, false, _) => None,
        }
    }
}

fn build_reachable_from_ref(repo: &Repository, base_branch: &str) -> HashSet<git2::Oid> {
    let oid = match repo
        .find_branch(base_branch, git2::BranchType::Local)
        .ok()
        .and_then(|b| b.get().target())
    {
        Some(oid) => oid,
        None => return HashSet::new(),
    };
    revwalk_from_oid(repo, oid)
}

fn build_reachable_from_remote_ref(repo: &Repository, base_branch: &str) -> HashSet<git2::Oid> {
    let remote_name = format!("origin/{base_branch}");
    let oid = match repo
        .find_branch(&remote_name, git2::BranchType::Remote)
        .ok()
        .and_then(|b| b.get().target())
    {
        Some(oid) => oid,
        None => return HashSet::new(),
    };
    revwalk_from_oid(repo, oid)
}

fn revwalk_from_oid(repo: &Repository, oid: git2::Oid) -> HashSet<git2::Oid> {
    let mut revwalk = match repo.revwalk() {
        Ok(r) => r,
        Err(_) => return HashSet::new(),
    };
    let _ = revwalk.set_sorting(git2::Sort::NONE);
    let _ = revwalk.push(oid);
    let mut set = HashSet::new();
    for oid_result in &mut revwalk {
        if let Ok(oid) = oid_result {
            set.insert(oid);
        }
    }
    set
}
```

- [ ] **Step 2: Update `detect_merged_branches` to use `BaseReachable`**

Replace the current `find_branch(...BranchType::Local)` lookup and single revwalk with:
```rust
let local_reachable = build_reachable_from_ref(repo, base_branch);
let remote_reachable = build_reachable_from_remote_ref(repo, base_branch);

if local_reachable.is_empty() && remote_reachable.is_empty() {
    span.record("base_lookup_result", "find_branch_error");
    return Err(anyhow::anyhow!("base branch not found: {base_branch}"));
}
span.record("base_lookup_result", "success");

let base_reachable = BaseReachable { local: local_reachable, remote: remote_reachable };
```

Then replace the per-branch loop's status assignment:
```rust
for (i, branch_oid) in &candidates {
    if let Some(status) = base_reachable.regular_merge_status(*branch_oid) {
        match status {
            MergeStatus::Merged => merged_count += 1,
            _ => merged_count += 1, // count all "merged-somewhere" as merged for stats
        }
        branches[*i].merge_status = status;
    } else {
        unmerged_count += 1;
    }
}
```

Change the return type from `HashSet<git2::Oid>` to `BaseReachable`:
```rust
pub fn detect_merged_branches(
    repo: &Repository,
    base_branch: &str,
    branches: &mut [BranchInfo],
) -> anyhow::Result<BaseReachable> {
```

- [ ] **Step 3: Update `build_reachable_set_from_repo` and `build_reachable_set`**

Replace the single-ref lookup in `build_reachable_set_from_repo` with:
```rust
pub fn build_reachable_set_from_repo(repo: &Repository, base_branch: &str) -> BaseReachable {
    let local = build_reachable_from_ref(repo, base_branch);
    let remote = build_reachable_from_remote_ref(repo, base_branch);
    BaseReachable { local, remote }
}

pub fn build_reachable_set(repo_path: &Path, base_branch: &str) -> BaseReachable {
    let repo = match git2::Repository::open(repo_path) {
        Ok(r) => r,
        Err(_) => return BaseReachable { local: HashSet::new(), remote: HashSet::new() },
    };
    build_reachable_set_from_repo(&repo, base_branch)
}
```

- [ ] **Step 4: Update `apply_merge_statuses` to accept `&BaseReachable`**

```rust
pub fn apply_merge_statuses(
    repo: &Repository,
    branches: &mut [BranchInfo],
    base_reachable: &BaseReachable,
) {
    if base_reachable.local.is_empty() && base_reachable.remote.is_empty() {
        return;
    }
    for branch in branches.iter_mut() {
        if branch.is_base || branch.is_current {
            continue;
        }
        if let Ok(b) = repo.find_branch(&branch.name, git2::BranchType::Local) {
            if let Some(oid) = b.get().target() {
                if let Some(status) = base_reachable.regular_merge_status(oid) {
                    branch.merge_status = status;
                }
            }
        }
    }
}
```

- [ ] **Step 5: Fix callers in `app.rs`**

Search for all uses of the old return type. The returned `BaseReachable` is used for squash candidate computation — callers that previously used the `HashSet` directly need to use `base_reachable.local` or the union. Find the squash candidate collection in `app.rs` (look for code that calls `detect_merged_branches` and uses the result to build `squash_candidates`). Change it to use `base_reachable.local` for merge-base precomputation (the local set is sufficient for finding the common ancestor).

```bash
grep -n "detect_merged_branches\|build_reachable_set\|apply_merge_statuses" src/app.rs
```

For any code that did `let reachable = detect_merged_branches(...)` and then used `reachable` as a `HashSet`, change to `let base_reachable = detect_merged_branches(...)` and use `base_reachable.local` where a single `HashSet` was previously expected.

### 3b: Squash detection

- [ ] **Step 6: Replace `effective_base_branch` in `squash_loader.rs` with dual-pass detection**

Remove the `effective_base_branch` function and the `let effective_base = ...` call. Instead, after the cache check and before calling `is_squash_merged`, call it twice:

```rust
// Remove: let effective_base = effective_base_branch(&repo_path, &base_branch);

// In the thread loop, replace the single is_squash_merged call with:
let local_squash = is_squash_merged(
    &repo_path,
    &base_branch,
    branch_name,
    Some(commit_hash),
    merge_base.as_deref(),
);
let remote_base = format!("origin/{base_branch}");
let remote_squash = is_squash_merged(
    &repo_path,
    &remote_base,
    branch_name,
    Some(commit_hash),
    None, // don't reuse local merge_base — remote base may differ
);

let status = match (local_squash, remote_squash) {
    (true, true) => MergeStatus::SquashMerged,
    (false, true) => MergeStatus::RemoteSquashMerged,
    (true, false) => MergeStatus::LocalSquashMerged,
    (false, false) => MergeStatus::Unmerged,
};
let is_squash = !matches!(status, MergeStatus::Unmerged);
```

Update the `SquashResult` send and cache insert to use `status` directly instead of the `is_squash` bool where possible. If `SquashResult` only carries `is_squash_merged: bool`, change it in `types.rs` to carry `status: MergeStatus` for the full picture:

In `src/types.rs`, update `SquashResult`:
```rust
pub struct SquashResult {
    pub branch_name: String,
    pub status: MergeStatus, // replaces is_squash_merged: bool
}
```

Then update `app.rs` where `SquashResult` is consumed — instead of `if result.is_squash_merged { MergeStatus::SquashMerged } else { ... }`, use `result.status` directly.

Remove the `use std::path::Path` and `use std::process::Command` imports added by the previous partial fix if they're no longer needed after removing `effective_base_branch`.

- [ ] **Step 7: Build and test**

```bash
cargo build 2>&1
cargo test 2>&1
```
Expected: clean build, all tests pass.

- [ ] **Step 8: Commit**

```bash
git add src/git/merge_detection.rs src/git/squash_loader.rs src/types.rs src/app.rs
git commit -m "feat: dual-ref merge detection for local vs remote base branch"
```

---

## Task 4: Rendering — cells, column widths, sort ranks, filter tokens

**Files:**
- Modify: `src/ui/cells.rs`
- Modify: `src/view/column.rs`
- Modify: `src/view/filter.rs`

### 4a: cells.rs — new match arms with italic

- [ ] **Step 1: Add 4 match arms to `merge_status_parts`**

Replace any stub arms from Task 1 with real implementations. The full match block becomes:

```rust
let (full, short, style) = match status {
    MergeStatus::Merged => (
        format!("merged {}", symbols.status_merged),
        format!("m {}", symbols.status_merged),
        theme.merged,
    ),
    MergeStatus::SquashMerged => (
        format!("squash-merged {}", symbols.status_squash_merged),
        format!("sm {}", symbols.status_squash_merged),
        theme.squash_merged,
    ),
    MergeStatus::LocalMerged => (
        format!("local-merged {}{}", symbols.status_merged, symbols.status_local_suffix),
        format!("lm {}{}", symbols.status_merged, symbols.status_local_suffix),
        theme.merged.add_modifier(Modifier::ITALIC),
    ),
    MergeStatus::RemoteMerged => (
        format!("remote-merged {}{}", symbols.status_merged, symbols.status_remote_suffix),
        format!("rm {}{}", symbols.status_merged, symbols.status_remote_suffix),
        theme.merged.add_modifier(Modifier::ITALIC),
    ),
    MergeStatus::LocalSquashMerged => (
        format!("local-squash {}{}", symbols.status_squash_merged, symbols.status_local_suffix),
        format!("ls {}{}", symbols.status_squash_merged, symbols.status_local_suffix),
        theme.squash_merged.add_modifier(Modifier::ITALIC),
    ),
    MergeStatus::RemoteSquashMerged => (
        format!("remote-squash {}{}", symbols.status_squash_merged, symbols.status_remote_suffix),
        format!("rs {}{}", symbols.status_squash_merged, symbols.status_remote_suffix),
        theme.squash_merged.add_modifier(Modifier::ITALIC),
    ),
    MergeStatus::Unmerged => (
        format!("unmerged {}", symbols.status_unmerged),
        format!("u {}", symbols.status_unmerged),
        theme.unmerged,
    ),
    MergeStatus::Pending => (
        "pending \u{2026}".to_string(),
        "\u{2026}".to_string(),
        theme.dim,
    ),
};
```

Add `use ratatui::style::Modifier;` at the top of cells.rs if not already imported.

- [ ] **Step 2: Update tests in cells.rs**

Find the existing tests for `merge_status_parts` (around lines 208-285). Add tests for all 4 new variants. Pattern:
```rust
#[test]
fn merge_status_local_remote_variants() {
    let ctx = make_test_ctx(); // use the same helper as existing tests
    let w = Some(20);
    assert_eq!(
        merge_status_parts(&MergeStatus::RemoteMerged, &ctx, w).0,
        "remote-merged +v" // ASCII symbols: status_merged="+", status_remote_suffix="v"
    );
    assert_eq!(
        merge_status_parts(&MergeStatus::LocalMerged, &ctx, w).0,
        "local-merged +^"
    );
    assert_eq!(
        merge_status_parts(&MergeStatus::RemoteSquashMerged, &ctx, w).0,
        "remote-squash ~v"
    );
    assert_eq!(
        merge_status_parts(&MergeStatus::LocalSquashMerged, &ctx, w).0,
        "local-squash ~^"
    );
    // abbreviated forms
    assert_eq!(merge_status_parts(&MergeStatus::RemoteMerged, &ctx, Some(5)).0, "rm +v");
    assert_eq!(merge_status_parts(&MergeStatus::LocalMerged, &ctx, Some(5)).0, "lm +^");
}
```

### 4b: column.rs — widths and sort ranks

- [ ] **Step 3: Widen merge column**

In `src/view/column.rs`, update `merge_status_column`:
```rust
pub fn merge_status_column<T: ViewItem>(name: &'static str) -> ColumnDef<T> {
    ColumnDef {
        name,
        min_width: 5,       // was 4 — "rm +v" is 5 chars
        wide_width: Some(16), // was 15 — "remote-merged +v" is 16 chars
        hide_below_width: None,
        compare: Some(merge_status_cmp),
    }
}
```

- [ ] **Step 4: Update sort ranks**

```rust
pub fn merge_status_rank(status: &MergeStatus) -> u8 {
    match status {
        MergeStatus::Merged => 0,
        MergeStatus::SquashMerged => 1,
        MergeStatus::RemoteMerged => 2,
        MergeStatus::LocalMerged => 3,
        MergeStatus::RemoteSquashMerged => 4,
        MergeStatus::LocalSquashMerged => 5,
        MergeStatus::Unmerged => 6,
        MergeStatus::Pending => 7,
    }
}
```

- [ ] **Step 5: Update column tests in column.rs**

Find `merge_status_rank_correct_values` test and extend it:
```rust
assert_eq!(merge_status_rank(&MergeStatus::Merged), 0);
assert_eq!(merge_status_rank(&MergeStatus::SquashMerged), 1);
assert_eq!(merge_status_rank(&MergeStatus::RemoteMerged), 2);
assert_eq!(merge_status_rank(&MergeStatus::LocalMerged), 3);
assert_eq!(merge_status_rank(&MergeStatus::RemoteSquashMerged), 4);
assert_eq!(merge_status_rank(&MergeStatus::LocalSquashMerged), 5);
assert_eq!(merge_status_rank(&MergeStatus::Unmerged), 6);
assert_eq!(merge_status_rank(&MergeStatus::Pending), 7);
```

### 4c: filter.rs — new tokens

- [ ] **Step 6: Add 4 filter tokens and parse arms**

In `merge_tokens()`, add after the existing squash token:
```rust
FilterTokenDef { key: 'r', label: "Remote-merged", token: "merge:remote-merged" },
FilterTokenDef { key: 'l', label: "Local-merged",  token: "merge:local-merged"  },
// Remote-squash and local-squash omitted for now — rare enough that the text filter covers it
```

(If you want all 4, add `merge:remote-squash` and `merge:local-squash` as well with keys `R` and `L`.)

In `FilterSet::parse`, add the new cases:
```rust
"merge:remote-merged" => fs.statuses.push(MergeStatus::RemoteMerged),
"merge:local-merged"  => fs.statuses.push(MergeStatus::LocalMerged),
"merge:remote-squash" => fs.statuses.push(MergeStatus::RemoteSquashMerged),
"merge:local-squash"  => fs.statuses.push(MergeStatus::LocalSquashMerged),
```

- [ ] **Step 7: Build and test**

```bash
cargo build 2>&1
cargo test 2>&1
```
Expected: clean build, all tests pass (including the new cells and column tests).

- [ ] **Step 8: Commit**

```bash
git add src/ui/cells.rs src/view/column.rs src/view/filter.rs
git commit -m "feat: render local/remote merge status variants with italic + directional arrows"
```

---

## Verification

1. `cargo build` — clean
2. `cargo test` — all pass
3. Run in `~/workspace/martech/hightouch-sync` **without** pulling local `main` first. `feature/sfdc-events-model` should show `remote-merged ✔↓` (italic) — it's merged in `origin/main` but local `main` is behind.
4. After `git pull` on local `main`, the status should update to `merged ✔` (non-italic).
5. Verify sort order: remote/local-merged branches sort between SquashMerged and Unmerged.
6. Verify filter: typing `merge:remote-merged` in the filter bar shows only remotely-merged branches.
