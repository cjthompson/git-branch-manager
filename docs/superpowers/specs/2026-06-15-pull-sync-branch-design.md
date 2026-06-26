# Pull = sync a single branch to its `origin/` counterpart

**Date:** 2026-06-15
**Status:** Design — approved for planning

## Context

Pressing Enter on a branch and choosing **Pull** should make *that one branch*
match its `origin/` counterpart, if possible — no other branches are touched.
There is no global pull command and none is planned.

Today this is broken:

- `app.rs` (≈ line 2277) calls `operations::pull_branch(repo_path, name, false, …)`
  with `is_current` **hardcoded to `false`**. So every Pull takes the
  `fast_forward` path → `git fetch origin <X>:<X>`, which Git refuses for a
  branch that is checked out in any worktree:
  `fatal: refusing to fetch into branch 'refs/heads/main' checked out at '…'`.
- There is no handling for a **diverged** branch (local commits not on origin).
  The user hit both at once: `main` checked out in another worktree *and* one
  commit ahead of origin.

The fix is to make Pull state-aware: detect the branch's relationship to
`origin/<X>` and where (if anywhere) it is checked out, then run the correct
command — falling back to an explicit reconciliation choice when the histories
have diverged.

## Goal

A Pull that:

1. Fast-forwards when it can (a pure pointer move), regardless of whether the
   branch is checked out.
2. On divergence, asks the user to **Rebase / Merge / Reset to origin / Abort**.
3. Never silently fails with a raw Git refusal; every outcome is a clear result
   or a clear prompt.

## Behavior

On **Pull** of branch `X` (with upstream `origin/X`):

1. **Fetch** `git fetch origin X` to refresh the `origin/X` remote-tracking ref.
   (Fetch only — do *not* use `X:X`, which would try to move the local ref and
   re-trigger the checked-out refusal.)
2. **Classify** by comparing local `X` to the refreshed `origin/X`
   (recompute ahead/behind; the cached values may be stale):
   - **up-to-date** (ahead 0, behind 0) → toast, no-op.
   - **behind only** (ahead 0, behind > 0) → **fast-forward** (no prompt).
   - **ahead only** (ahead > 0, behind 0) → toast "local is ahead of origin;
     nothing to pull" (Pull is not Push).
   - **diverged** (ahead > 0 and behind > 0) → open the **reconciliation menu**.
3. **No upstream** (`origin/X` does not exist) → toast "no origin/X to pull from".

### Where the operation runs

Look up `X`'s checkout location via `git worktree list --porcelain`. Three cases:

| Branch state | Fast-forward / Reset | Rebase / Merge |
|---|---|---|
| Not checked out anywhere | `git update-ref refs/heads/X origin/X` (pure pointer move) | **Hidden** (no working tree to run in) |
| Current branch of the running worktree | `git merge --ff-only origin/X` | `git rebase origin/X` / `git merge origin/X` |
| Checked out in another worktree | `git -C <wt> merge --ff-only origin/X` | `git -C <wt> rebase origin/X` / `git -C <wt> merge origin/X` |

Rationale: fast-forward and reset are pointer moves that work on a bare ref via
`update-ref`; rebase and merge create/replay commits and require a working tree,
so they are only offered when `X` has one.

### Reconciliation menu (diverged only)

Reuse the existing `Overlay::Menu`. Items:

- **Rebase** — `rebase origin/X` (replay local commits on top of origin).
  *Hidden when `X` has no worktree.*
- **Merge** — `merge origin/X` (create a merge commit).
  *Hidden when `X` has no worktree.*
- **Reset to origin** — hard reset / `update-ref` to `origin/X`; **discards local
  commits**. Always available. Because it is destructive, selecting it opens a
  yes/no `Overlay::Confirm` before running.
- **Abort** — close the menu, do nothing.

Rebase/merge conflicts are not resolved in-app: the operation's failure (and the
"fix it in `<worktree>`" hint) surfaces through the existing `Overlay::Results`.
The tool is a pointer/sync tool, not a merge UI.

## Components & touch points

- **`git/operations.rs`**
  - `pull_status(repo_path, branch) -> PullState` — fetch + classify
    (`UpToDate | Behind | Ahead | Diverged | NoUpstream`), carrying ahead/behind
    and the resolved worktree path (if any).
  - `worktree_for_branch(repo_path, branch) -> Option<PathBuf>` — parse
    `git worktree list --porcelain`.
  - Worktree-aware executors returning `OperationResult`:
    `fast_forward_branch`, `reset_branch_to_origin`, `rebase_branch_onto_origin`,
    `merge_origin_into_branch`. Each picks `update-ref` vs `-C <wt>` vs in-place
    based on the resolved worktree. Replace the current `pull_branch`/
    `fast_forward` pair.
- **`types.rs`** — add `BranchAction::{PullRebase, PullMerge, PullReset}` with
  `label()`s; keep `Pull` as the entry action.
- **`app.rs`**
  - Pull handler: call `pull_status`, then either run the ff/up-to-date/ahead
    outcome directly or open the reconciliation `Overlay::Menu`.
  - Remove the hardcoded `is_current = false`.
  - Map `PullRebase`/`PullMerge`/`PullReset` in `execute_confirmed_action`;
    route `PullReset` through a confirm first.
- **`ui/menu.rs`, `ui/help.rs`, `ui/status_bar.rs`** — labels/shortcuts for the
  new menu items and help text.

## Testing

Integration tests in `tests/integration.rs` using `setup_test_repo()` /
`setup_remote_test_repo()`:

- Behind-only branch **not** checked out → Pull fast-forwards via `update-ref`;
  local ref now equals `origin/X`; working tree elsewhere untouched.
- Behind-only branch checked out in **another worktree** → Pull fast-forwards in
  that worktree (no "refusing to fetch" error). This is the regression test for
  the reported bug.
- Diverged branch → `pull_status` returns `Diverged`; `reset_branch_to_origin`
  makes local equal `origin/X` and drops the local commit.
- Diverged branch with no worktree → rebase/merge are not offered (menu
  construction omits them); reset still works.
- Up-to-date and ahead-only → correct no-op classifications.

## Open questions (resolve during planning/implementation)

1. **Upstream resolution.** Assume `origin/X` by name, or honor the branch's
   configured upstream (`git rev-parse --abbrev-ref X@{u}`)? Configured upstream
   is more correct for branches that track a differently-named or
   different-remote ref; `origin/X` is simpler. *Leaning: use `@{u}` when set,
   fall back to `origin/X`.*
2. **Dirty working tree on a checked-out branch.** `reset --hard` discards
   uncommitted changes, and `rebase`/`merge` will refuse. Do we pre-check
   `git -C <wt> status --porcelain` and warn/skip with a clear message, or just
   let the op run and surface Git's result? *Leaning: pre-check and warn,
   especially before a destructive reset.*
3. **Ahead-only outcome.** Just toast, or proactively offer Push (the action
   already exists elsewhere)? Pull arguably shouldn't push, but a one-key nudge
   may be nice.
4. **Fetch scope.** `git fetch origin X` (targeted, fast) vs relying on a recent
   `fetch --all`. Targeted keeps Pull snappy; confirm it updates the
   `refs/remotes/origin/X` tracking ref we compare against.
5. **Post-op refresh for other worktrees.** After updating a branch checked out
   in another worktree, should the Worktrees view's status for that worktree be
   refreshed (it now points at a new commit)?
6. **Reset wording/guardrails.** Exact confirm copy and whether to show the
   number of local commits that would be discarded.

## Out of scope / YAGNI

- No global "pull all" command.
- No temporary-worktree spin-up to rebase/merge a branch that isn't checked out
  (rebase/merge are simply hidden there).
- No in-app conflict resolution.

## Suggested PR split

1. **Fix + fast-forward correctness:** worktree-aware fast-forward,
   `worktree_for_branch`, remove the `is_current` hardcode, classification for
   up-to-date / behind / ahead / no-upstream. Ships a working, reviewable Pull.
2. **Divergence reconciliation:** `Diverged` handling, reconciliation menu,
   `PullRebase`/`PullMerge`/`PullReset` (+ reset confirm), help/status text.
