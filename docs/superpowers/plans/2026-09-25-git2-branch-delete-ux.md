# Branch Delete UX and git2 Conversion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task by task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Use the app's merge classifications to delete local branches with `git2`, with an opt-in setting that skips confirmation for confidently integrated alternative merges.

**Architecture:** Keep classification and user choice separate from the final ref mutation. The UI decides whether to show confirmation; the job carries an exact classification snapshot, and the worker revalidates the relevant refs and worktree ownership immediately before `Branch::delete`. This plan is independent of the other two Git conversion plans.

**Tech Stack:** Rust, `git2 = "0.21"`, existing `Config`, `App` confirmation flow, `ActionJobQueue`, integration tests with temporary Git repositories.

**Spec:** This plan's Goal, Global Constraints, tasks, and completion criteria are the source of truth for this conversion slice. The older `/Users/chris/.claude/plans/is-it-possible-that-spicy-thompson.md` is historical source inventory only.

## Global Constraints

- Scope is original call point **#4 only**, plus the setting and confirmation behavior needed to make its conversion safe. Do not migrate other Git subprocesses in this plan.
- The default setting is confirmation **on**, preserving today's visible delete flow until a user opts out.
- The opt-out applies only to a **single local-only branch delete** classified as `SquashMerged`, `LocalSquashMerged`, `RemoteSquashMerged`, `CherryPicked`, `LocalCherryPicked`, or `RemoteCherryPicked`. It does not affect `LikelySquashMerged`, `Unmerged`, `Pending`, regular merge statuses, bulk deletion, remote deletion, or worktree removal.
- Never delete a branch checked out in any worktree. A stale, missing, or changed classification fails closed and returns an actionable result; it never becomes an implicit force delete.
- Preserve the current already-gone success result, `NotMerged` recovery, linked-worktree recovery, and final confirmation for explicit force deletion.
- Preserve unrelated dirty work. Follow `AGENTS.md`: local `-devN` iteration version changes are not committed; run `cargo build` after every completed task.

## Review Focus

- A branch that advances after its status was displayed must remain present.
- A base ref that is rewound after classification must not authorize deletion from stale evidence.
- A branch checked out in a linked worktree after preflight must remain present.
- `LikelySquashMerged` must never be treated as confirmed integration.
- Selecting multiple branches, including a mix of confirmed and unmerged statuses, must retain confirmation for the entire selection.

---

### Task 1: Define exact deletion evidence and a worker-side validation result

**Files:** Modify `src/types.rs`, `src/git/merge_detection.rs` or the relevant enrichment result types, and `src/job_queue.rs`; test the focused unit modules and `tests/integration.rs`.

**Interfaces:** Add a per-target `DeleteEvidence` carrying the full candidate tip OID, the exact local/remote base OIDs used for classification, and the confirmed `MergeStatus`. Carry it with the queued delete job; do not infer identity from `BranchInfo.merge_base_commit`, which is only an abbreviated merge-base hash. Add a typed stale-evidence failure so the UI can refresh and offer confirmation.

- [ ] Write tests that distinguish matching evidence from changed branch tip, changed local base, changed remote base, and missing ref; each changed/missing case must refuse the deletion.
- [ ] Add the evidence to the classification result and queue target representation, keeping the existing display status intact. Capture full OIDs at the point the status is computed or loaded from its cache key.
- [ ] At worker execution, resolve the current ref OIDs and compare them to the evidence before any delete. Re-run worktree ownership lookup using `try_list_worktrees`; propagate lookup errors rather than treating them as no owner.
- [ ] Run the focused tests and `cargo build`.

### Task 2: Replace local branch `-d/-D` dispatch with explicit git2 rules

**Files:** Modify `src/git/operations.rs:162` and `src/job_queue.rs:653`; test `tests/integration.rs:1513` and neighboring delete tests.

**Interfaces:** Keep `delete_local` and `delete_local_force` result shapes. Safe deletion checks whether the branch tip is reachable from the same configured upstream that Git `branch -d` uses, or from HEAD when no upstream exists. An alternative-merge delete requires verified evidence from Task 1. Force deletion skips the merge-status check only after the explicit force path, while still rejecting checked-out worktrees.

- [ ] Extend tests for an unmerged branch, ordinary merged branch, squash-merged branch, cherry-picked branch, branch whose upstream differs from the displayed base, missing branch, and branch checked out in a linked worktree.
- [ ] Run the focused tests before implementation and record the failures that represent newly specified behavior.
- [ ] Implement the safe ancestry check using fresh `git2` refs and `graph_descendant_of`, with an explicit equality case because that method excludes equality. Use `Branch::delete()` only after the ancestry or verified alternative-merge rule passes. Keep explicit force deletion and typed error mapping.
- [ ] Run focused tests, `cargo test`, and `cargo build`; inspect `git diff --check`.

### Task 3: Add a persisted confirmation setting and route the single-branch action

**Files:** Modify `src/config.rs`, `src/ui/settings.rs`, `src/app.rs` settings key handling and delete dispatch, and `src/job_queue.rs`; test config tests and app confirmation tests.

**Interfaces:** Add `confirm_alternative_merge_deletes: Option<bool>`; `None` and `Some(true)` mean confirm. Add one Settings row named `Confirm alternative-merge deletes` with `on/off` values. Persist only this field through `Config::update`.

- [ ] Add config round-trip/default tests and app tests for each of the six eligible statuses; assert the default opens confirmation and `Some(false)` queues one local delete directly. Assert that `LikelySquashMerged`, `Unmerged`, `Pending`, regular merged statuses, bulk selection, remote deletion, and worktree deletion still open their existing confirmation flow.
- [ ] Run the focused tests and record the expected failures.
- [ ] Add the Settings row, cursor handling, and field-specific persistence. Route eligible single-branch deletes to the existing queue with `DeleteEvidence`; keep all other actions on the existing confirmation path. The setting changes confirmation only; worker validation from Tasks 1–2 remains mandatory.
- [ ] Run focused tests, `cargo test`, and `cargo build`. Verify the setting persists across a config reload.

### Task 4: Preserve recoverable results when evidence changes

**Files:** Modify `src/app.rs` result/recovery handling and `src/ui/results.rs` only if its existing generic result rendering cannot show the typed cause; test app and integration tests.

**Interface:** A stale-evidence result leaves the branch intact, refreshes branch/worktree data, and leads back to the ordinary delete confirmation path. It must never silently retry as `DeleteLocalForce`.

- [ ] Add a deterministic test that enqueues an eligible delete, changes the candidate tip or base ref before the worker executes it, and asserts the branch still exists and the result names the changed evidence.
- [ ] Add a test that makes worktree lookup fail at execution time and asserts no deletion occurs.
- [ ] Implement result-to-UI routing without adding a new destructive shortcut. Keep the existing `NotMerged` and linked-worktree recovery actions working.
- [ ] Run focused tests, the full `cargo test` suite, `cargo clippy -- -D warnings`, and `cargo build`. Manually exercise default/on/off Settings behavior and single versus bulk deletion in a disposable repository.

## Completion

The `-d/-D` subprocesses are gone from the local-delete path, alternative merge opt-out behaves exactly as specified, stale evidence and worktree ownership fail closed, and the existing recovery tests remain green. This plan does not promise elimination of all orphan Git processes.
