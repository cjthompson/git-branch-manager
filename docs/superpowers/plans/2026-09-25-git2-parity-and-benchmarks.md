# git2 Parity and Benchmark Conversions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task by task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Evaluate and convert the remaining behavior-preserving Git subprocess calls only where `git2` matches Git CLI semantics and acceptable runtime performance.

**Architecture:** Establish a repeatable CLI baseline, make one conversion family at a time, and compare results against the same repository fixtures. Network operations share one transport/callback layer. A failed parity, authentication, cancellation, or performance gate leaves that call on Git CLI and records the reason rather than weakening existing behavior.

**Tech Stack:** Rust, `git2 = "0.21"`, Git CLI as oracle, local bare Git remotes, optional real SSH/HTTPS smoke repositories, `cargo test`.

**Spec:** This plan's Goal, Global Constraints, tasks, and completion criteria are the source of truth for this conversion slice. The older `/Users/chris/.claude/plans/is-it-possible-that-spicy-thompson.md` is historical source inventory only.

## Global Constraints

- Scope is original call points **#3, #5, #6, #8, #9, #10, #11, #12, #13, #14, and #15**. Branch deletion (#4) belongs exclusively to `2026-09-25-git2-branch-delete-ux.md`; #1, #2, and #7 belong to `2026-09-25-git2-straightforward-conversions.md`.
- No intentional UX changes: branch/worktree status, ref effects, errors, cancellation, and per-target results must remain equivalent. If parity cannot be established, keep the CLI path.
- Network conversions must use configured remotes and refspecs, not hard-coded `refs/heads/*` or `origin` except where the existing action explicitly chooses `origin`.
- Do not replace the measured `git status` path based on flag mapping alone. Existing `src/git/status.rs` records a 90+ second `git2` scan in a large worktree.
- Preserve unrelated dirty work. Follow `AGENTS.md`: increment the local `-devN` version while iterating without committing it, and run `cargo build` after every completed task.
- The plan does not implement process shutdown/reaping. Remaining shell calls must be inventoried and reported after each conversion family.

## Review Focus

- Linked-worktree enumeration omits the primary worktree unless it is added explicitly.
- A branch-specific fetch can update a local branch, unlike a normal remote-tracking fetch.
- A successful network transport call can still have a rejected push ref; per-ref callback status is required.
- Current-branch pull must update the checked-out files as well as the branch ref.
- The Git CLI may use credential helpers, configured refspecs, fsmonitor, and untracked cache unavailable to a naive `git2` replacement.

---

### Task 1: Build a parity and timing baseline for the candidate sites

**Files:** Test `tests/integration.rs`; create `docs/plans/2026-09-25-git2-conversion-baseline.md` containing measured commands, repository sizes, durations, and outcomes.

**Interfaces:** Tests use `setup_test_repo()` for local behavior and `setup_remote_test_repo()` for bare-remote operations. Record CLI and candidate outputs using the same fixture state, cloning or recreating the fixture between mutating calls.

- [ ] Record the exact present call sites and expected side effects for #3, #5, #6, #8–#15; include current cancellation and error paths from `src/git/operations.rs`, `src/git/worktree.rs`, `src/git/merge_detection.rs`, `src/git/status.rs`, and `src/git/tags.rs`.
- [ ] Add or identify fixtures for detached/stale worktrees, renamed and conflicting merges, configured nonstandard fetch refspecs, remote rejection, current/non-current pull, staged/unstaged/untracked/conflicted files, and disjoint histories.
- [ ] Time the existing CLI implementation at least five times each on a small disposable repo and a representative large repo, recording median and slowest result. For status, include the large monorepo that triggered the documented 90+ second `git2` result if available; otherwise explicitly record that the status performance gate remains unevaluated.
- [ ] Run `cargo test` and `cargo build`; save baseline output without altering product behavior.

### Task 2: Convert remote ahead/behind only if startup latency stays acceptable (#3)

**Files:** Modify `src/git/branch.rs:230`; test `tests/integration.rs` remote-enrichment tests.

**Interface:** Keep `spawn_remote_enricher(...) -> Receiver<RemoteEnrichResult>`. Use `repo.graph_ahead_behind(remote_tip, base_tip)` for each remote ref and a single `Revwalk` count of commits reachable from base for the disjoint-history comparison. Include equality handling and `u32` conversion checks.

- [ ] Add differential tests comparing each remote ref's ahead, behind, merged, and disjoint result with the existing `for-each-ref %(ahead-behind:...)` plus `rev-list --count` values. Include an in-sync ref, diverged ref, disjoint ref, and absent base.
- [ ] Run the focused tests; capture the CLI baseline.
- [ ] Replace both subprocesses as one conversion, preserving `Option` results on lookup or graph errors and preserving the background channel's per-branch output order.
- [ ] Repeat the Task 1 timing procedure with many remote refs. Retain the conversion only if median and slowest startup enrichment do not regress by more than 10% on the representative large repo. Run `cargo test` and `cargo build`.

### Task 3: Compare linked-worktree listing and creation against Git CLI (#5, #6)

**Files:** Modify `src/git/worktree.rs:89` and `src/git/operations.rs:861` only if parity passes; test `tests/integration.rs` worktree cases.

**Interfaces:** `try_list_worktrees` must still return the primary worktree first with its branch and HEAD, then linked worktrees with accurate path, branch/detached state, and seven-character hash. `create_worktree` must attach the requested existing branch at `.worktrees/<sanitized-name>` and leave the caller's HEAD, index, and files unchanged.

- [ ] Extend the existing worktree tests with primary-only, linked branch, detached linked worktree, stale/prunable entry, locked entry, and caller-inside-linked-worktree fixtures. Compare each parsed row to `git worktree list --porcelain`; retain an observable `Err` for invalid repositories.
- [ ] Add creation tests that record the caller's HEAD/index/worktree state before and after, and assert that the new worktree is attached to the exact branch with no extra branch created. Include branch names with `/` and an existing destination path.
- [ ] Use `Repository::worktrees()` plus an explicitly constructed primary entry for listing. For creation, pass the requested existing branch through `WorktreeAddOptions::reference` to `Repository::worktree(name, path, Some(&options))`; verify how `checkout_existing` interacts with a sanitized worktree name before enabling it. Never call `checkout_head` on the source repository.
- [ ] Run the focused tests and compare error results with CLI. If stale/locked worktree behavior or creation cleanup differs, leave that specific call on CLI and document it in the baseline report. Run `cargo test` and `cargo build` for any retained conversion.

### Task 4: Gate merge-tree conversion on exact result parity (#8)

**Files:** Modify `src/git/merge_detection.rs:378` only if parity passes; test the existing merge-tree confidence scenarios in `tests/integration.rs`.

**Interface:** `merge_tree_confirms` stays `bool` and fails closed. Call `repo.merge_trees(&ancestor_tree, &base_tree, &branch_tree, opts)`, check `index.has_conflicts()` before `write_tree_to`, and compare the written tree OID to the base tree OID. `MergeOptions::find_renames(true)` is a method call, not a writable field.

- [ ] Add differential fixtures for clean replay, content conflict, rename, rename conflict, file mode change, binary change, and multiple merge bases; record CLI `git merge-tree --write-tree` status and tree OID.
- [ ] Run focused tests and compare the candidate index/tree to CLI. For cases where `git2`'s merge algorithm yields a different tree or conflict decision, retain CLI rather than changing the confidence classification.
- [ ] If every fixture agrees, replace the subprocess and run the full squash scenario suite, `cargo test`, and `cargo build`. Record both accepted and rejected cases in the baseline report.

### Task 5: Introduce one network transport boundary and convert fetch (#9, #10)

**Files:** Modify `src/git/operations.rs:283` and the `fetch_remote`/`fast_forward`/`pull_remote` paths; create a focused `src/git/remote_transport.rs` only if shared authentication, progress, cancellation, and error mapping otherwise duplicate across call sites; test `tests/integration.rs` remote fixtures.

**Interfaces:** A transport helper accepts an existing repository, the selected configured remote, the action's refspecs, and its cancellation flag. It returns structured success/error and per-ref updates. `fetch --all` iterates configured remotes and respects each remote's fetch refspecs and prune setting. Branch-specific fetch preserves its destination-ref behavior and checked-out-branch safeguards.

- [ ] Add tests for two remotes, a custom fetch refspec, prune on/off, missing remote, non-current branch fast-forward, and attempts to update a checked-out branch. Record actual ref OIDs before and after each CLI action.
- [ ] Implement Git-compatible noninteractive credentials for the repository's supported SSH/HTTPS configurations, progress callbacks, cancellation callbacks, and meaningful error mapping. Do not move a remote action to `git2` if its current credential-helper behavior cannot be matched.
- [ ] Convert `fetch_sync`, `fetch`, `fetch_prune`, `fetch_remote`, `fast_forward`, and `pull_remote` one at a time using the helper and configured refspecs. Check both local branch and remote-tracking refs after each action; do not assume `Remote::fetch` reproduces `git fetch origin b:b` without testing.
- [ ] Run local bare-remote tests, then an authorized real SSH and HTTPS smoke test in disposable repositories. If those repositories are unavailable, leave SSH/HTTPS-dependent paths on CLI and record the incomplete gate. Run `cargo test` and `cargo build` for retained conversions.

### Task 6: Convert current-branch pull only after fast-forward parity (#11)

**Files:** Modify `src/git/operations.rs:365`; test `tests/integration.rs:3377` and related pull tests.

**Interface:** Resolve the current branch's configured upstream, fetch it, verify the fetched tip is a descendant of or equal to current HEAD, then update the checked-out tree/index and branch ref with failure handling. `Repository::merge_commits` alone is not a pull and must not be used as the operation's completion condition.

- [ ] Add tests for current branch behind, in sync, diverged, dirty worktree, missing upstream, cancelled fetch, and non-current branch unaffected. Check both ref OIDs and working-tree file contents.
- [ ] Run focused tests against the current CLI path as the baseline.
- [ ] Implement the fast-forward sequence behind the Task 5 transport boundary. Do not move the ref if checkout preflight fails; preserve `--ff-only` refusal on divergence and the existing `OperationResult` action/message shape.
- [ ] Run focused tests and manual disposable-repo pull smoke. If dirty-tree or failure ordering differs, keep `git pull --ff-only` and document the gap. Run `cargo test` and `cargo build` for a retained conversion.

### Task 7: Convert push, remote-branch deletion, and tags as one result model (#12, #13, #14)

**Files:** Modify `src/git/operations.rs:399`, `src/git/operations.rs:623`, and `src/git/tags.rs:89`; reuse the Task 5 transport boundary; test `tests/integration.rs` push/delete/tag cases.

**Interface:** Use explicit branch/tag refspecs. For a branch push, set upstream only after the remote accepted that ref. Batch deletion returns per-target results and retains the current individual fallback. Tag push uses `refs/tags/<name>:refs/tags/<name>`; tag deletion uses `:refs/tags/<name>`.

- [ ] Add local bare-remote tests for successful push with upstream, rejected push, successful multi-delete, one failed deletion with individual fallback, annotated and lightweight tag push, and tag deletion. Compare remote and local-tracking ref OIDs with the current CLI paths.
- [ ] Use `RemoteCallbacks::push_update_reference` to record a server rejection even when the transport method returns `Ok(())`. Carry per-ref outcomes into `OperationResult` and do not report a rejected ref as success.
- [ ] Convert the three families incrementally, retaining cancellation and retry behavior. Keep `--force-with-lease` as CLI; its lease semantics are outside this plan.
- [ ] Run local bare-remote tests and real SSH/HTTPS smoke under the same availability gate as Task 5. Keep a family on CLI if credentials or per-ref errors differ. Run `cargo test`, `cargo clippy -- -D warnings`, and `cargo build` for retained conversions.

### Task 8: Re-evaluate status only with measured performance and flag parity (#15)

**Files:** Modify `src/git/status.rs:15` only if both gates pass; test existing working-tree status cases in `tests/integration.rs`.

**Interface:** Preserve `WorkingTreeStatus` booleans and `changed_files` entries, including staged plus modified on the same path, untracked paths, rename paths, and conflicts. Continue to return `WorkingTreeStatus::clean()` on scan failure unless separately approved as a UX change.

- [ ] Extend tests for rename/copy, conflict, ignored file, and the full staged/unstaged/untracked matrix. Compare `Repository::statuses()` mapping against `git status --porcelain=v2 --untracked-files=normal` in the same fixture.
- [ ] Benchmark CLI and `git2` scans at least five times on the representative large worktree, recording median and slowest elapsed time and whether fsmonitor/untracked cache is active. Do not change the production path while measuring.
- [ ] Convert only if all mapped results agree and both median and slowest `git2` times are no more than 10% above CLI. Otherwise retain the current CLI implementation and document the measured reason.
- [ ] Run status-focused tests, `cargo test`, and `cargo build` if converted; preserve the benchmark record either way.

## Completion

Each accepted conversion has a completed parity/performance record, relevant tests, and a successful `cargo build`. Every rejected or untested conversion remains on CLI with its specific blocker recorded. The final report lists remaining shell calls, including `for-each-ref`, `merge-base`, `diff`, `cherry`, `patch-id`, graph fallback, rebase, cherry-pick, stash, merge, and `gh`; it makes no zero-orphan claim without a separate shutdown/reaping design.
