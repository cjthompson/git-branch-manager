# Graph Actions-Menu Availability Correction Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace blanket `is_current`/`is_base` menu gating with per-operation capability evaluators, so the Graph `ENTER` menu (and the identical Branches/Remotes/Worktrees `ENTER` menu, since they share `build_menu_items`) exposes every Git operation that is actually safe for a branch's real state — current-and-clean, checked out in a linked worktree, no upstream, dirty, or base-worktree — with an accurate per-row reason when an operation is withheld, and dispatches merge/squash against the correct worktree instead of always running from `App.repo_path`.

**Architecture:** A new pure-function module, `src/git/capability.rs`, evaluates one `BranchAction` at a time against small, already-loaded facts (`BranchInfo`, an optional matching `WorktreeInfo`, tracking/ahead/behind, `has_configured_remote`). `src/app.rs`'s `build_branch_menu_for` (and the Branches-view bulk key handlers that currently duplicate `is_pinned()` gating) call these evaluators instead of inlining `!branch.is_current` checks. `MenuItem.enabled`/`MenuItem.reason` (`src/ui/menu.rs:12-22`) already have the right shape — this plan changes what feeds them, not the row type. Merge/squash dispatch gains a worktree-resolution step so `git checkout <base>` runs in the base's own worktree path, not unconditionally in `App.repo_path`.

**Tech Stack:** Rust, git2, `std::process::Command` git CLI shell-outs, existing `App`, `JobQueue`, `WorktreeInfo`/`BranchInfo` types, `cargo test` integration fixtures in `tests/integration.rs`.

**Spec:** Project-tasks database task #094, "Correct Graph Actions-menu availability for current branches and worktrees" (fix, high priority, tags `#ui #git #worktree`). This plan carries the durable design/implementation detail; the task's five verbatim requirements are referenced by number (Req 1–5) throughout.

## Context

[`docs/git-operations-inventory.md`](../../git-operations-inventory.md) documents that the Graph `ENTER` menu is **ref-driven**: it resolves each ref attached to the selected commit back to the authoritative `BranchInfo`/`RemoteBranchInfo`/`TagInfo` row and reuses the exact same menu builder the Branches/Remotes/Tags/Worktrees views use on their own `ENTER` key (`docs/git-operations-inventory.md:25-28`, confirmed in code — see Design). Its "Main findings" section (lines 172–194) calls out that several action labels describe broader Git operations than the app actually performs (Pull is FF-only, Merge/Rebase target only the configured base), which is directly relevant to Req 3's "actual … divergence … prerequisites" language.

The inventory does **not** cover the specific bug #094 fixes: today, `build_branch_menu_for` (`src/app.rs:2552-2758`) gates almost every action — Checkout, Delete local, Delete local+remote, Force-delete local, Fast-forward, Merge, Squash merge, Rebase, Create worktree — behind a single `!branch.is_current` (sometimes combined with `!branch.is_base`) check. `is_current` means only "this is the branch checked out in the worktree the app process itself is running from" (`src/git/branch.rs:779`, `current_branch` comes from that process's own `repo.head()`). It says nothing about:

- a branch checked out in a **different, linked** worktree (Req 1/2) — the menu will happily offer Checkout/Delete/Force-delete/Create-worktree for such a branch today, and the underlying git command fails at runtime instead of being disabled up front;
- whether Push/Pull/Fast-forward/Force-push/Rebase are actually possible given real upstream/divergence state (Req 3) — e.g. Rebase is blanket-disabled on the current branch even though rebasing your own checked-out branch onto base is the single most common case, while `git::operations::rebase_branch` (`src/git/operations.rs:533-579`) already checks out `branch_name` itself, so a true current-and-clean branch works fine;
- which worktree Merge/Squash should actually run in (Req 4) — `merge_branch`/`rebase_branch` always `git checkout <base>`/`git checkout <branch_name>` inside whatever single `repo_path` was passed in, and both `App` (`src/app.rs:2997`, `:3491` etc.) and `JobQueue` (`src/job_queue.rs:146,209`) only ever hold **one** `repo_path`, fixed to wherever the TUI process itself is running. There is currently **no per-worktree dispatch path at all** — flagged in Open Questions/Risks below, since this is new plumbing, not a rewire of existing capability.

A second, independent copy of the same blanket rule exists for bulk multi-select operations on the Branches view: `get_selected_branch_names` (`src/app.rs:3238-3240`) filters targets with `(!b.is_pinned())` where `is_pinned() = is_base || is_current` (`src/types.rs:260-263`), used by `delete_selected_branches` (`:3027`) and `push_selected_branches` (`:3198`). This is the "per-view key handler" duplicate gating Req 1 also needs corrected for parity, even though `handle_branches_key` (`:1155-1177`) itself only exposes `d`/`D`/`p`/fetch/tab-switch keys directly — Checkout/Fast-forward/Merge/Squash/Rebase/Create-worktree are reachable **only** through the shared `ENTER` menu (`open_context_menu` → `build_menu_items` → `build_branch_menu_for`, `src/app.rs:2382-2414`), so fixing the menu builder fixes those for every view that calls it.

The codebase already has two reusable primitives worth building on instead of reinventing (see Design):

- `git::worktree::branches_checked_out_in_worktrees(repo_path) -> HashSet<String>` (`src/git/worktree.rs:307`), already used by the Graph loader to populate `GraphRef.has_linked_worktree` (`src/git/graph.rs:124,1216`, consumed only for a rendering glyph today, `src/ui/graph_render.rs:692`).
- The already-loaded `self.worktrees.items(): &[WorktreeInfo]` App state, which `build_delete_preflight` (`src/app.rs:3049-3108`) already scans (no extra git shell-out) to build `DeleteRisk::CheckedOut { branch, worktree, is_main }` (`src/ui/confirm.rs:20-31`) for the destructive-delete confirmation flow. This exact "does some `WorktreeInfo` have `branch == Some(name)`" lookup is the pattern capability evaluators should reuse for Req 2's worktree exclusions, rather than re-shelling to `git worktree list`.

## Goal

Every `BranchAction` row in the ref-driven menu (`build_branch_menu_for`, `build_remote_menu_for`, `build_worktree_menu`) is driven by a small, independently testable, per-operation capability evaluator instead of a single blanket `is_current`/`is_base` flag. Each evaluator returns availability plus (when unavailable) a short, stable reason string compatible with the existing `MenuItem.reason: Option<String>` field. Checkout/local-delete/remote-delete/force-delete/create-worktree are unavailable specifically when the branch is checked out in *some* worktree (main or linked) other than being merely `is_base`; Push/Pull/Fast-forward/Force-push/Rebase are gated on real upstream/divergence/cleanliness/worktree facts, not on `is_current` alone; Merge/Squash additionally require that the **destination** (base) worktree be resolved and clean, and execute in that worktree's path rather than `App.repo_path`.

## Non-goals

- Changing the Graph's ref-driven action model to a commit-driven one (arbitrary-commit diff/cherry-pick/branch-at-commit/tag-at-commit/detached-worktree actions). Tracked as a gap in `docs/git-operations-inventory.md:179-181`; out of scope here.
- Adding an arbitrary-ref target model (reset, ref movement, reflog recovery, tag retargeting, refspec-level fetch/push). See `docs/git-operations-inventory.md:139-154,182-184`.
- History editing or working-tree authoring (amend, interactive rebase, staging, commit creation, stash management as a first-class feature, conflict continuation, bisect). See `docs/git-operations-inventory.md:65-79,185-187`.
- Making Merge/Rebase/Pull target an arbitrary user-chosen destination instead of the configured base branch, or making Push/Force-push support arbitrary refspecs. That's the "Partial" gap called out at `docs/git-operations-inventory.md:96-99,112-117,188-191`; this plan only makes the *existing* fixed-destination operations correctly gated and worktree-aware, it does not widen their targets.
- Worktree lifecycle completeness (move/lock/prune/repair/detached-at-commit creation) — `docs/git-operations-inventory.md:167-171,192-194`.
- Reworking `FailureCause`/`OperationResult`/the Confirm-Results modal shell introduced by the 2026-09-13 modal redesign plan; this plan only adds new preflight *evaluators* that feed the existing `MenuItem`/`Overlay::Confirm` types, and (for Req 4) a worktree-selection step before dispatch.

## Design

### Where capabilities live

New module `src/git/capability.rs`, added to `src/git/mod.rs` (alongside `branch`, `operations`, `worktree`) and re-exported transitively via `lib.rs`'s existing `pub mod git;`. Pure functions only — no I/O, no `Command`, no `git2::Repository` — so they take already-loaded facts and are unit-testable without a repo fixture:

```rust
// src/git/capability.rs

/// Snapshot of the facts an evaluator needs about the branch's own worktree
/// state. `None` means "not checked out in any worktree the app has loaded".
pub struct WorktreePresence<'a> {
    pub path: &'a Path,
    pub is_main: bool,
    pub is_clean: bool,
}

/// Availability plus, when unavailable, a short stable reason matching the
/// existing `MenuItem.reason` convention ("current", "base", "no remote", …).
pub struct Capability {
    pub available: bool,
    pub reason: Option<&'static str>,
}

impl Capability {
    pub fn yes() -> Self { Self { available: true, reason: None } }
    pub fn no(reason: &'static str) -> Self { Self { available: false, reason: Some(reason) } }
}

pub fn can_checkout_local(branch: &BranchInfo, worktree: Option<WorktreePresence>) -> Capability;
pub fn can_create_worktree(branch: &BranchInfo, worktree: Option<WorktreePresence>) -> Capability;
pub fn can_delete_local(branch: &BranchInfo, worktree: Option<WorktreePresence>) -> Capability;
pub fn can_delete_remote(branch: &BranchInfo, worktree: Option<WorktreePresence>, has_remote: bool) -> Capability;
pub fn can_force_delete_local(branch: &BranchInfo, worktree: Option<WorktreePresence>) -> Capability;
pub fn can_delete_local_and_remote(branch: &BranchInfo, worktree: Option<WorktreePresence>, has_remote: bool) -> Capability;
pub fn can_push(branch: &BranchInfo, has_configured_remote: bool, is_clean: bool) -> Capability;
pub fn can_pull(branch: &BranchInfo, worktree: Option<WorktreePresence>, has_remote: bool) -> Capability;
pub fn can_fast_forward(branch: &BranchInfo, worktree: Option<WorktreePresence>, has_remote: bool) -> Capability;
pub fn can_force_push(branch: &BranchInfo, has_remote: bool) -> Capability;
pub fn can_rebase(branch: &BranchInfo, worktree: Option<WorktreePresence>, is_clean: bool) -> Capability;
pub fn can_merge_into_base(branch: &BranchInfo, base_worktree: Option<WorktreePresence>) -> Capability;
pub fn can_squash_into_base(branch: &BranchInfo, base_worktree: Option<WorktreePresence>) -> Capability;
```

Rationale for the split:

- **Checkout / local-delete / remote-delete / force-delete / create-worktree** (Req 2) all share one exclusion: unavailable when `worktree.is_some()` — i.e. the branch is checked out in *any* worktree, main or linked — **except** that `is_base` (protected base branch) and the existing `is_current` reason keep their own distinct, more specific reason strings so the UI doesn't regress to a generic "checked out" message for the common current-branch case. Concretely: check `is_base` first (`"base"`), then `is_current` (`"current"`), then `worktree.is_some()` (`"checked out in worktree"`), in that order, matching today's existing reason precedence in `build_branch_menu_for`.
- **Push** (Req 3) never needs a checkout — `push_branch` (`src/git/operations.rs:397-423`) pushes the local ref directly via `git push --set-upstream origin <branch>` regardless of what's checked out where. So `can_push` must **not** consult worktree state at all; it only needs `has_configured_remote` and — per Req 3's explicit example — cleanliness when the branch has no upstream and is the current branch (a dirty current-and-unpushed branch can still push its committed history; only the "no upstream, use current HEAD" framing implies "clean" as a readability/scope guard from the task text, see Open Questions).
- **Fast-forward / Pull (non-current path)** (Req 3) both resolve to `git fetch origin <branch>:<branch>` (`fast_forward`, `src/git/operations.rs:332-359`; `pull_branch` delegates to it when not current, `:391-393`). Modern git refuses to fetch into a ref that is checked out anywhere (including the running worktree's own current branch). So both need the **same** worktree exclusion as Req 2's group, which today's `enabled: !branch.is_current && has_remote` only half-covers (it misses "checked out in a different linked worktree").
- **Pull (current-branch path)** runs `git pull --ff-only` in place (`pull_branch`, `:368-390`) — no worktree exclusion needed, only `has_remote` and `is_behind`.
- **Force-push** needs `has_remote`/ahead/behind divergence only, same as today, but should not depend on `is_current` (force-push, like push, doesn't require checkout).
- **Rebase** (Req 3) does `git checkout <branch_name>` then `git rebase <base>` (`rebase_branch`, `src/git/operations.rs:533-579`) inside the dispatch's single `repo_path`. It is safe when: the branch is not base, and either (a) it is the current branch and clean (checkout of the branch you're already on is a no-op, so this is exactly Req 3's "a clean current branch may expose … rebase even when it has no upstream"), or (b) it is not checked out in any worktree at all. It remains unsafe when checked out in a **different** worktree (the checkout step would collide with the same "refuse to check out a branch that's already checked out elsewhere" git guard used for Checkout).
- **Merge / Squash into base** (Req 4) need the **base** branch's worktree, not the source branch's: `merge_branch` (`src/git/operations.rs:459-530`) checks out `base` first. So capability must resolve `base_worktree: Option<WorktreePresence>` (the worktree currently on the configured base branch, if any) and require it to be `None` (base isn't checked out elsewhere so the *running* worktree can check it out) or resolved-and-clean (see Design → dispatch below for what "resolved" means once dispatch also changes worktrees). The source branch itself has no checkout requirement for Merge (only the base needs checking out), matching current git behavior; Squash follows the same base-worktree rule with the same `--squash` distinction already in `merge_branch`.

Existing reuse — do **not** reinvent:

- Resolve `Option<WorktreePresence>` for a branch by scanning `self.worktrees.items()` for `worktree.branch.as_deref() == Some(name)`, exactly the lookup `build_delete_preflight` already performs at `src/app.rs:3072-3076`. Wrap this as a small private helper in `app.rs`, e.g. `fn worktree_presence_for<'a>(&'a self, branch: &str) -> Option<capability::WorktreePresence<'a>>`, rather than shelling out via `worktree::try_worktree_for_branch`/`try_other_worktree_for_branch` (those exist for *runtime* classification of a command failure in `git/operations.rs:99-124`, not for menu-build-time preflight — reuse them only if `self.worktrees` isn't populated yet, e.g. before the background worktree loader completes; see Open Questions).
- `MenuItem { enabled, reason, .. }` (`src/ui/menu.rs:12-22`) is already exactly the row shape Req 1 wants — do not add new fields to it. Capability evaluators produce `Capability { available, reason }`; `build_branch_menu_for` maps `Capability` → `(enabled, reason.map(str::to_owned))`.
- `requires_final_confirmation` (`src/app.rs:3959-3966`) and the two-stage `Overlay::Confirm` destructive flow already exist and are untouched by this plan — only the *availability* computation upstream of opening a confirm changes, not the confirm/destructive-review flow itself.
- `DeletePreflight`/`DeleteRisk::CheckedOut`/`DeleteRisk::DirtyWorktree` (`src/ui/confirm.rs:14-32`, built by `build_delete_preflight`, `src/app.rs:3049-3108`) already surface the same "checked out"/"dirty" facts for the destructive-delete review screen. This plan does not change that screen; it only makes the *menu row* upstream of it (`Delete local`, `Force-delete local`) correctly disabled before the user ever reaches that screen for a checked-out-elsewhere branch — today the menu allows opening it, and the preflight only warns after the fact.

### Ref-driven menu construction

`build_branch_menu_for(&self, branch: &BranchInfo) -> Vec<MenuItem>` (`src/app.rs:2552-2758`) keeps its current shape (one `Vec<MenuItem>` literal) but replaces each `enabled: …, reason: …` pair with a call into `capability::can_*`, e.g.:

```rust
let checkout_cap = capability::can_checkout_local(branch, self.worktree_presence_for(&branch.name));
MenuItem {
    label: "Checkout".into(),
    enabled: checkout_cap.available,
    reason: checkout_cap.reason.map(str::to_owned),
    shortcut: Some('c'),
    action: BranchAction::Checkout,
    target: branch.name.clone(),
    remote: None,
},
```

`build_graph_menu_for` (`src/app.rs:2449-2490`) is untouched — it already just resolves the ref back to the authoritative `BranchInfo`/`RemoteBranchInfo`/`TagInfo` and calls `build_branch_menu_for`/`build_remote_menu_for`/`build_tag_menu_for`, so fixing those three builders fixes Graph `ENTER` for local branches, remote branches, and (unaffected) tags simultaneously — this is the "same ref-driven menu, multiple callers" property the inventory doc calls out at line 25-28. `build_worktree_menu` (`src/app.rs:2911-2990`) already does per-operation gating with real facts (`is_main`, `is_dirty`, `is_detached`, `is_base`, `has_remote`) and is a good model for the new branch evaluators' style, though it is not itself in scope for #094 (Worktrees-view rows aren't part of the blanket-`is_current` bug).

### Per-view key handler parity

`get_selected_branch_names` (`src/app.rs:3238-3240`) currently filters with `!b.is_pinned()` (`is_base || is_current`, `src/types.rs:260-263`) for both `delete_selected_branches` (`:3027`, used by `d`/`D`) and `push_selected_branches` (`:3198`, used by `p`). Replace the single blanket filter with action-specific capability checks so bulk multi-select parity matches the menu:

```rust
fn get_selected_branch_names_for(&self, action_check: impl Fn(&BranchInfo) -> bool) -> Vec<String> {
    list_state::collect_targets(&self.branches, |b| action_check(b).then(|| b.name.clone()))
}
```

`delete_selected_branches(false)` uses `can_delete_local`, `delete_selected_branches(true)` uses `can_delete_local_and_remote`, `push_selected_branches` uses `can_push`. This is the "matching gating parity" step Req 1 implies by pairing menu availability with dispatchable per-view keys.

### Worktree-aware merge/squash dispatch (Req 4)

Today `execute_menu_action` (`src/app.rs:2992-3023`) routes `Merge`/`SquashMerge`/`Rebase` through `open_confirm_with_remote` → `JobQueue::enqueue` → `JobQueue::start` (`src/job_queue.rs:195-234`), which always clones the **single** `self.repo_path` the queue was constructed with (`JobQueue::new(repo_path, base_branch)`, `:156`) and passes it unchanged into `execute_action_with_remote` → `operations::merge_branch`/`rebase_branch` (`src/job_queue.rs:714-735`). There is no per-job worktree override anywhere in `JobQueue`. This plan adds one:

1. `ActionJob` (wherever it's defined in `job_queue.rs`, alongside `targets`/`remote`/`return_view`) gains an optional `dispatch_path: Option<PathBuf>`, defaulting to `None` (meaning "use the queue's own `repo_path`", preserving every existing action's behavior).
2. `JobQueue::start` uses `job.dispatch_path.clone().unwrap_or_else(|| self.repo_path.clone())` instead of unconditionally cloning `self.repo_path`.
3. `App`, when building the `Merge`/`SquashMerge` confirm choice, resolves the base branch's worktree the same way as `worktree_presence_for` (scan `self.worktrees.items()` for `branch == Some(base_branch)`); if found, that worktree's `path` becomes `dispatch_path`. `Rebase`'s dispatch path is resolved from the **target branch's own** worktree (if checked out anywhere) or falls back to `self.repo_path` for the current-and-clean case.
4. `merge_branch`/`rebase_branch` themselves are unchanged — they already take `repo_path: &Path` as a parameter; only the *caller* now supplies the resolved worktree path instead of always `self.repo_path`.

This is genuinely new plumbing, not a rewire of an existing-but-unused capability — flagged explicitly in Open Questions/Risks.

## Implementation steps

- [ ] **Step 1: Add the capability module.**

  Create `src/git/capability.rs` with `WorktreePresence`, `Capability`, and the twelve `can_*` function signatures listed in Design. Add `pub mod capability;` to `src/git/mod.rs:1-14` (alphabetical slot, after `cache` and before `cherry_loader`, matching the existing sort order).

- [ ] **Step 2: Implement each evaluator with a focused unit test.**

  In `src/git/capability.rs`'s `#[cfg(test)] mod tests`, add one test per evaluator covering: the happy path, the `is_base` exclusion, the `is_current` exclusion (where applicable), and the "checked out in a different worktree" exclusion (where applicable) — e.g. `can_rebase_allows_clean_current_branch_without_upstream`, `can_rebase_blocks_branch_checked_out_in_other_worktree`, `can_merge_into_base_blocks_when_base_worktree_dirty`, `can_push_ignores_worktree_state`. Build `BranchInfo`/`WorktreePresence` fixtures inline with `..Default::default()`-style helpers local to the test module (do not add `Default` to the production `BranchInfo` if it doesn't already derive it — check first).

- [ ] **Step 3: Add `App::worktree_presence_for` and `App::worktree_presence_for_branch_or_base` helpers.**

  In `src/app.rs`, near `build_delete_preflight` (`:3049`), add a private helper that performs the same `self.worktrees.items().iter().find(|w| w.branch.as_deref() == Some(name))` lookup (reusing the exact pattern at `:3072-3076`) and maps the found `WorktreeInfo` into `capability::WorktreePresence { path: &wt.path, is_main: wt.is_main, is_clean: wt.wt_status.is_clean() }`.

- [ ] **Step 4: Refactor `build_branch_menu_for` to call the evaluators.**

  Replace each `enabled`/`reason` pair in `src/app.rs:2567-2757` (Checkout, Delete local, Delete local+remote, Force-delete local, Fast-forward, Push, Force push, Pull, Merge into base, Squash merge into base, Rebase onto base, Create worktree) with the matching `capability::can_*` call plus `self.worktree_presence_for(...)`/`self.has_configured_remote`/`is_ahead`/`is_behind` inputs, per Design. Leave `Open PR in browser`'s `has_pr` gating untouched — it isn't part of #094.

- [ ] **Step 5: Refactor bulk multi-select key handlers for parity.**

  Update `get_selected_branch_names` (`src/app.rs:3238-3240`), `delete_selected_branches` (`:3027-3046`), and `push_selected_branches` (`:3198-3201`) to use the per-action capability predicates instead of `!b.is_pinned()`, per Design → "Per-view key handler parity".

- [ ] **Step 6: Add the `dispatch_path` plumbing to `JobQueue`.**

  In `src/job_queue.rs`: add `dispatch_path: Option<PathBuf>` to `ActionJob`'s definition; update `JobQueue::start` (`:208-234`) to resolve `let repo_path = job.dispatch_path.clone().unwrap_or_else(|| self.repo_path.clone());`; update every existing `ActionJob { .. }` construction site (`enqueue`/callers) to set `dispatch_path: None` so non-merge/rebase/squash actions are unaffected. Grep `ActionJob {` and `ActionJob::` across `job_queue.rs` and `app.rs` to find every construction site before editing.

- [ ] **Step 7: Resolve and pass the worktree-aware dispatch path from `App`.**

  In `execute_menu_action` (`src/app.rs:2992-3023`) and wherever `Merge`/`SquashMerge`/`Rebase` confirm choices are built (the `open_confirm_with_remote` call for those actions), resolve `dispatch_path` per Design → "Worktree-aware merge/squash dispatch" and thread it into the eventual `JobQueue::enqueue` call. Update `JobQueue::enqueue`'s signature (or add an `enqueue_with_dispatch_path` variant) as needed — check its current signature in `job_queue.rs` before deciding whether to add a parameter or an overload, to avoid breaking the many existing non-worktree-aware call sites.

- [ ] **Step 8: Update `ui/help.rs` and status-bar text if needed.**

  Grep `src/ui/help.rs` and `src/ui/status_bar.rs` for any prose implying "unavailable on the current branch" (none found as of this plan's authoring — `help.rs` is a key/label table with no such gating prose) and correct only if the audit finds stale text.

- [ ] **Step 9: Add the Req 5 regression tests in `tests/integration.rs`.**

  Using `setup_test_repo()` (`tests/integration.rs:54`) and the linked-worktree pattern already established at `test_delete_local_classifies_primary_and_linked_worktree_failures` (`:1546-1581`) and `test_dirty_linked_worktree_reports_all_changes_and_remains_recoverable` (`:1586-1625`), add scenarios for: **current** (branch is the running worktree's HEAD, clean), **linked-worktree** (branch checked out via `git worktree add <path> <branch>` elsewhere), **no-upstream** (local branch with `TrackingStatus::Local`), **dirty** (uncommitted changes in the branch's own worktree), and **base-worktree** (the configured base branch itself checked out in a linked worktree, exercising Merge/Squash gating). Each test builds the relevant `BranchInfo`/`WorktreeInfo` (or drives `App`'s real loaders against the temp repo) and asserts `MenuItem.enabled`/`reason` from `build_branch_menu_for`/`build_graph_menu_for`, plus — for at least one enabled action per scenario — that dispatching it through `execute_menu_action`/`JobQueue` does not attempt an unsafe checkout. Note for the executor: the task's literal "commit 11d7ca1 in the P008 worktree" traces to a real commit in *this* repository (`11d7ca1cc2d4bb15b1d9c4ce484d1c990d0914c5`, "docs: update changelog for unavailable actions") observed during interactive use, not a fixture hash — no P008 worktree currently exists in this repo's `git worktree list`, so the regression test must construct an equivalent temp-repo scenario (a commit reachable from a branch checked out in a linked worktree) rather than reference that literal hash. Name the primary test something traceable, e.g. `graph_enter_p008_style_linked_worktree_exposes_safe_operations_only`.

- [ ] **Step 10: Run the full verification matrix.**

  Run: `cargo build && cargo test && cargo clippy`. Also run `cargo fmt --all -- --check` if the repo's CI expects it (check for a `rustfmt.toml`/CI workflow first — this plan's Verification section below is the authoritative list).

## Files to Modify

- `src/git/mod.rs` — add `pub mod capability;`.
- `src/git/capability.rs` (new) — `WorktreePresence`, `Capability`, and the twelve `can_*` evaluators plus their unit tests.
- `src/app.rs` — add `worktree_presence_for` helper near `build_delete_preflight` (`:3049`); refactor `build_branch_menu_for` (`:2552-2758`) to call evaluators; refactor `get_selected_branch_names`/`delete_selected_branches`/`push_selected_branches` (`:3027-3046,3198-3201,3238-3240`) for bulk-key parity; resolve and thread `dispatch_path` in `execute_menu_action` (`:2992-3023`) for Merge/SquashMerge/Rebase.
- `src/job_queue.rs` — add `dispatch_path: Option<PathBuf>` to `ActionJob`; use it in `JobQueue::start` (`:208-234`) instead of unconditionally cloning `self.repo_path`; update all existing `ActionJob` construction sites.
- `src/ui/help.rs`, `src/ui/status_bar.rs` — audit only; edit if stale "unavailable on current branch" prose is found (none identified during this plan's research).
- `tests/integration.rs` — add the Req 5 regression tests (current, linked-worktree, no-upstream, dirty, base-worktree scenarios) near the existing worktree-classification tests (`:1546-1625`).

## Verification

- `cargo build` — must compile with the new `capability` module wired in.
- `cargo test` — full suite, including the new `git::capability` unit tests and the new `tests/integration.rs` regression scenarios (Req 5).
- `cargo clippy` — no new warnings.
- Manual, via `cargo run` inside a repo with a linked worktree (`git worktree add ../gbm-p008 <some-branch>`): open the Graph view, move to a commit whose local-branch ref is checked out in that linked worktree, press `Enter`, and confirm Checkout/Delete local/Delete local+remote/Force-delete local/Create worktree are disabled with a worktree-specific reason, while Push/Pull/Fast-forward/Force-push (where upstream/divergence allow) remain available — this is the practical stand-in for "commit 11d7ca1 in the P008 worktree" noted in Step 9.
- Manual: repeat for a clean, no-upstream current branch and confirm Push and Rebase are enabled (Req 3's explicit example), a dirty branch (confirm the dirty-aware rows show a reason instead of dispatching an unsafe checkout), and a base branch checked out in a linked worktree (confirm Merge/Squash on some other branch are gated on that base worktree's cleanliness, and — if enabled — actually check out and merge in the base's own worktree path, not `App.repo_path`). Capture the resulting enabled/reason matrix in the PR description for reviewer cross-check against Req 1–4.

## Open questions / risks

- **No existing per-worktree dispatch path.** As found during research, `JobQueue`/`execute_action_with_remote`/`merge_branch`/`rebase_branch` currently operate against exactly one `repo_path` for the process's entire lifetime (`src/job_queue.rs:146,156,209`). Req 4 ("execute against the correct worktree instead of checking out a branch already checked out elsewhere") requires genuinely new plumbing (the `dispatch_path` addition in Step 6/7), not a rewire of a dormant capability. This is the highest-risk/most novel part of the plan and should be built and tested before the capability-evaluator refactor is considered complete, since a capability that says "available" but still dispatches into the wrong worktree would be a regression, not a fix.
- **"Clean" as a gating condition for Push/Rebase.** Req 3 says "a clean current branch may expose push and rebase even when it has no upstream," but `push_branch` doesn't care about working-tree cleanliness at all (it only pushes committed refs), and `rebase_branch`'s dispatch already auto-stashes dirty changes via `needs_stash` (`src/job_queue.rs:220-222`, computed fresh at job-start time from `status::detect_working_tree_status`). It's unclear whether "clean" should be a **hard gate** in the capability evaluator (simpler, more conservative, but blocks a workflow the auto-stash mechanism already supports) or whether the evaluator should allow a dirty current branch through and let the existing auto-stash carry it. This plan's evaluator signatures accept `is_clean: bool` so either policy is a one-line change, but the executor needs a decision before Step 2's tests are written — recommend asking the user/task owner rather than guessing.
- **How "the destination/base worktree" is resolved when multiple worktrees could match.** `self.worktrees.items()` should have at most one entry per branch (each branch can only be checked out in one worktree at a time, enforced by git itself), so ambiguity shouldn't arise in practice — but if `self.worktrees` hasn't finished its background load yet (`worktree::enrich_worktrees`, streamed async per `App`'s data-flow architecture) when the menu is built, `worktree_presence_for` will return `None` (branch looks "not checked out anywhere") even if it actually is, and a stale-data race could let an unsafe operation through. Recommend gating menu-availability computation on `self.worktrees.loading == false` for the affected actions, or falling back to the live `worktree::try_worktree_for_branch` shell-out (already used for runtime classification) when the cache looks incomplete — flagging this rather than picking one silently, since it trades a possible false "unavailable" (safe but annoying) against an extra shell-out on every menu build (slower but always correct).
- **Force-push safety heuristics beyond ahead/behind.** The plan keeps `can_force_push`'s gating equivalent to today's ahead-and-behind check (`git push --force-with-lease` already provides the safety net at the git level). #094 doesn't ask for anything beyond that, but if the task owner wants an additional heuristic (e.g. warn when the remote has commits from another author), that's new scope not covered here.
- **Bulk multi-select UX when capability reasons differ per branch.** Today `d`/`D`/`p` on the Branches view silently drop pinned branches from the target list with no feedback. Making the filter capability-aware doesn't change that silent-drop behavior — it only makes the *predicate* correct. If the task owner wants per-branch reasons surfaced for bulk operations too, that's a follow-up, not part of #094's stated requirements.
