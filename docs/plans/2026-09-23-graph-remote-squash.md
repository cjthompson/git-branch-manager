# Graph view: detect squash commits on `origin/<base>`

## Context
In `~/workspace/gusto-pro-growth-project-plans`, PR #6 squash-merged
`ct/docs-audit-directory-index-rule` into `origin/main` as `d3821f4`. The
Branches view correctly reports `remote-squash`, but the Graph view shows no
squash marker on `d3821f4` and no source-branch ref beside it.

Local `main` has diverged (`b612802` ahead, `d3821f4` behind), so `d3821f4`
exists only on `origin/main`.

## Root cause
`compute_possible_squash_updates` in `src/git/graph.rs:523` only considers
commits whose `commit.branch` is the **local** base branch
(`GraphRefKind::LocalBranch && name == base_branch`) in three places:
- `base_tip` lookup (line ~535)
- `BaseCommit` patch-job selection (line ~552)
- the final `updates` filter (line ~674)

A commit that lives only on `origin/<base>` is never hashed as a base commit,
so it can't match a branch tip's patch-id. The Branches view uses
`squash_loader.rs`, which checks `origin/<base>` too — hence the mismatch.

## Fix
1. Treat commits whose `commit.branch` is `RemoteBranch` named
   `origin/<base_branch>` as base commits too, in job selection and in the
   `updates` filter. Extract a helper, e.g. `is_base_lane(commit, base)`, so
   all three sites share it.
2. Keep `base_tip` for `displayed_branch_relation`, but also skip a branch tip
   as `RegularlyMerged` if it is an ancestor of `origin/<base>`. That avoids
   marking already-merged tips as candidates when only the remote contains them.
3. If there's no local base ref at all, fall back to the `origin/<base>` tip
   for `base_tip` rather than returning early.
4. Optional, for consistency with Branches view: mark the squash as remote-only
   when the matched base commit is not reachable from the local base. Rendering
   in `src/ui/graph_render.rs` (~193, ~622) would add the
   `status_remote_suffix` symbol. Confirm with the user before doing this.

Confirm first how `commit.branch` gets assigned to `origin/main`-only commits
(the lane logic around `graph.rs:301-320`, `__gbm_remote_base__`). The helper
must match whatever name/kind those commits actually carry.

## Tests (TDD — write first, see them fail)
In `tests/integration.rs`, next to the existing squash graph tests
(around `LocalSquashMerged`, e.g. line 4474):
- repo with a bare `origin`; branch `feature` off `main`; squash `feature`
  onto `origin/main` via a clone and push; add one unrelated local commit to
  `main`; fetch.
- assert `load_graph_with_squash_annotations` marks the `origin/main` squash
  commit `is_possible_squash_merge` with `feature` in its sources.
- regression: the existing local-squash graph tests still pass.

## Branching
`~/dev` repo: branch `ct/graph-remote-squash` cut from the default branch after
`git fetch`, in `./.claude/worktrees/ct-graph-remote-squash`. No PR unless asked.

## Verification
- `cargo test` (full suite) and `cargo clippy`.
- `cargo run -- --repo ~/workspace/gusto-pro-growth-project-plans`, Graph tab:
  `d3821f4` shows the squash symbol and `ct/docs-audit-directory-index-rule`.

## Tasks

Task records for `project-tasks` plan `P001`, project
`github.com-personal/cjthompson/git-branch-manager`. To re-create them, run
one `task add` per record with `--plan-id <P001's global id>` (from
`task-db plan get --seq P001`). `Depends on` uses the `T` keys below, since
`#NNN` numbers are reassigned on re-creation. Map each `T` key to its new
`#NNN` before passing `--dep`. `Requirements` go in as one `--req` each.
`Current ID` is the task's number in the database as of 2026-09-23. Each
`Anchor` is the slug of its record's heading; pass the raw heading as
`--anchor` and the helper slugifies it.

---

### T1 — Create worktree and branch

- **Current ID:** `#035`
- **Type:** `task`
- **Priority:** `medium`
- **Anchor:** `t1-create-worktree-and-branch`
- **Title:** Create worktree and branch for implementation
- **Depends on:** none
- **Requirements:**
  - `git fetch`, then resolve the default branch with
    `git symbolic-ref --short refs/remotes/origin/HEAD`
  - branch `ct/graph-remote-squash` cut from that ref
  - worktree at `./.claude/worktrees/ct-graph-remote-squash`
  - `.claude/worktrees/` ignored via `.gitignore` or `.git/info/exclude`

---

### T2 — Confirm branch assignment

- **Current ID:** `#031`
- **Type:** `task`
- **Priority:** `high`
- **Anchor:** `t2-confirm-branch-assignment`
- **Title:** Confirm commit.branch assignment for origin/<base>-only commits
- **Depends on:** `T1`
- **Requirements:**
  - read the lane logic in `src/git/graph.rs` around lines 301-320
    (`__gbm_remote_base__`)
  - record the exact `GraphRefKind` and name carried by a commit that is
    only on `origin/<base>`
  - no code changes

---

### T3 — Write failing remote-squash test

- **Current ID:** `#034`
- **Type:** `task`
- **Priority:** `high`
- **Anchor:** `t3-write-failing-remote-squash-test`
- **Title:** Write TDD test for remote-only squash detection
- **Depends on:** `T1`
- **Requirements:**
  - add the test to `tests/integration.rs` next to the existing squash
    graph tests (around line 4474)
  - fixture: bare `origin`; `feature` branched off `main`; `feature`
    squashed onto `origin/main` from a second clone and pushed; one
    unrelated local commit on `main`; `git fetch`
  - assert `load_graph_with_squash_annotations` sets
    `is_possible_squash_merge` on the `origin/main` squash commit, with
    `feature` in `possible_squash_merge_sources`
  - run it and confirm it fails before any fix

---

### T4 — Extract is_base_lane helper

- **Current ID:** `#032`
- **Type:** `task`
- **Priority:** `high`
- **Anchor:** `t4-extract-is-base-lane-helper`
- **Title:** Extract is_base_lane() helper for base commit detection
- **Depends on:** `T2`, `T3`
- **Requirements:**
  - add `is_base_lane(commit, base)` in `src/git/graph.rs`
  - true for the local base branch, and for the `origin/<base>` lane using
    the kind/name confirmed in `T2`
  - replace the three inline checks in `compute_possible_squash_updates`:
    `base_tip` lookup, `BaseCommit` job selection, final `updates` filter

---

### T5 — Check origin base in squash updates

- **Current ID:** `#033`
- **Type:** `task`
- **Priority:** `high`
- **Anchor:** `t5-check-origin-base-in-squash-updates`
- **Title:** Update compute_possible_squash_updates to check origin/<base>
- **Depends on:** `T4`
- **Requirements:**
  - commits on `origin/<base>` become `BaseCommit` patch jobs and receive
    enrichment updates
  - a branch tip that is an ancestor of `origin/<base>` is treated as
    `RegularlyMerged`, not a squash candidate
  - with no local base ref, fall back to the `origin/<base>` tip for
    `base_tip` instead of returning early
  - the `T3` test passes

---

### T6 — Run tests and clippy

- **Current ID:** `#036`
- **Type:** `task`
- **Priority:** `high`
- **Anchor:** `t6-run-tests-and-clippy`
- **Title:** Run tests and clippy verification
- **Depends on:** `T5`
- **Requirements:**
  - `cargo test` passes in full, including the existing local-squash graph
    tests
  - `cargo clippy` reports no new warnings

---

### T7 — Verify Graph view on d3821f4

- **Current ID:** `#037`
- **Type:** `task`
- **Priority:** `high`
- **Anchor:** `t7-verify-graph-view-on-d3821f4`
- **Title:** Test Graph view displays squash on d3821f4
- **Depends on:** `T6`
- **Requirements:**
  - `cargo run -- --repo ~/workspace/gusto-pro-growth-project-plans`
  - Graph tab: `d3821f4` shows the squash symbol, with
    `ct/docs-audit-directory-index-rule` as its source
  - only valid while local `main` in that repo still lacks `d3821f4`

---

### T8 — Optional remote-only marker

- **Current ID:** `#038`
- **Type:** `task`
- **Priority:** `low`
- **Anchor:** `t8-optional-remote-marker`
- **Title:** Optional: add remote-only marker to Graph rendering
- **Depends on:** `T7`
- **Requirements:**
  - get the user's approval before starting
  - mark a squash as remote-only when the matched base commit is not
    reachable from the local base
  - render `status_remote_suffix` in `src/ui/graph_render.rs` (around
    lines 193 and 622), matching the Branches view's `remote-squash`

---
