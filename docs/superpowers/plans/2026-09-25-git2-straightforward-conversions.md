# Straightforward git2 Conversions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task by task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove three small Git subprocess calls without changing their results or repository state.

**Architecture:** Replace object lookup, worktree HEAD date lookup, and synthetic commit creation at their existing call sites. Preserve existing public functions and fail-closed behavior. This plan can ship independently of the other two conversion plans.

**Tech Stack:** Rust, existing `git2 = "0.21"`, Git CLI as a test oracle, `cargo test`.

**Spec:** This plan's Goal, Global Constraints, tasks, and completion criteria are the source of truth for this conversion slice. The older `/Users/chris/.claude/plans/is-it-possible-that-spicy-thompson.md` is historical source inventory only.

## Global Constraints

- Scope is original call points **#1, #2, and #7 only**. Do not change branch deletion, remote operations, worktree creation/listing, status, or merge-tree behavior.
- Preserve the currently dirty `src/app.rs`, `src/view/graph.rs`, and `tests/integration.rs` unless the live status differs at execution time; inspect the diff before editing a dirty file.
- Keep the local `Cargo.toml` `-devN` iteration version out of commits. Increment it on each code change as instructed by `AGENTS.md`; run `cargo build` after every completed task.
- No new dependencies. Do not claim the orphan-process problem is solved: other load-time Git subprocesses remain.

## Review Focus

- A missing or invalid ref must retain today's false/now fallback rather than panic.
- Worktree HEAD age must come from that worktree, not the caller's HEAD.
- Synthetic commit creation must not move HEAD, any branch, or the index.
- The synthetic tree must come from `branchish`, including when `commit_hash` overrides the branch name.
- Squash detection must still fail closed when identity, tree, or parent lookup fails.

---

### Task 1: Replace the base-tree `rev-parse` in merge-tree confirmation (#1)

**Files:** Modify `src/git/merge_detection.rs:378`; test `tests/integration.rs` alongside the existing merge-tree confidence tests.

**Interface:** Keep `merge_tree_confirms(repo_path: &Path, base_branch: &str, branchish: &str) -> bool`. The only intended change is how `base_tree` is obtained.

- [ ] Add a test using `setup_test_repo()` that compares a valid base tree OID obtained by `Repository::revparse_single(base)?.peel(ObjectType::Tree)?.id()` with `git rev-parse <base>^{tree}`; include an invalid base that returns `false` from the public confidence path.
- [ ] Run that focused test and record its completion and exit code.
- [ ] Replace only the `rev-parse` subprocess with `Repository::open(repo_path)`, `revparse_single(base_branch)`, and `peel(ObjectType::Tree)`. Compare OIDs, rather than formatted output strings, after parsing the existing `merge-tree` output. Return `false` on every lookup or parse error.
- [ ] Run the focused test, `cargo test`, and `cargo build`; inspect `git diff --check` and the staged Cargo version hunk before any commit.

### Task 2: Read each worktree's HEAD date with git2 (#2)

**Files:** Modify `src/git/worktree.rs:291`; test `tests/integration.rs` near the worktree tests.

**Interface:** Keep `head_commit_date(dir: &Path) -> DateTime<Utc>` and `status_and_age` unchanged externally.

- [ ] Add a test with a linked worktree whose HEAD commit timestamp differs from the primary worktree; assert the linked row receives its own timestamp. Add an invalid/unborn HEAD case and assert the existing `Utc::now()` fallback within a bounded time interval.
- [ ] Run the focused test and record the expected failure against the old implementation only where the new behavior is actually distinguishable.
- [ ] Replace `git log -1 --format=%ct HEAD` with `Repository::open(dir)`, `head()?.peel_to_commit()?.time().seconds()`, and `Utc.timestamp_opt(seconds, 0).single()`. Retain the present-time fallback on failure.
- [ ] Run the focused test, `cargo test`, and `cargo build`; inspect `git diff --check` and the staged Cargo version hunk before any commit.

### Task 3: Create the synthetic squash-check commit without a subprocess (#7)

**Files:** Modify `src/git/merge_detection.rs:267`; test `tests/integration.rs` near `test_squash_merged_branch_detection`.

**Interface:** Keep `is_squash_merged(repo_path, base_branch, branch_name, commit_hash, merge_base) -> bool`. `git cherry` remains a subprocess.

- [ ] Add a test that records HEAD, candidate branch OID, index tree, and all local refs before `is_squash_merged`; call it for both a squash-positive and a squash-negative candidate, then assert those values are unchanged. Include a `commit_hash` pointing to a different candidate tip to prove the selected tree is used.
- [ ] Run the focused test. Record the result; the state-invariance assertion may already pass because the current `git commit-tree` also creates an unreachable commit.
- [ ] Resolve `branchish` and `ancestor` to commits using `git2`. Read the tree from the resolved `branchish` commit, obtain author/committer signatures without changing refs, then call `repo.commit(None, &author, &committer, "_", &tree, &[&ancestor_commit])`. Pass the returned OID to the existing `git cherry` call. **Never** call `commit(Some("HEAD"), ...)` or use the current HEAD's tree.
- [ ] Confirm `git cherry` receives the OID and produces the same boolean as the old path for the existing squash scenario suite. Run `cargo test`, `cargo clippy -- -D warnings`, and `cargo build`; inspect `git diff --check` and the staged Cargo version hunk before any commit.

## Completion

All three conversions preserve public results and repository refs in focused tests, the full suite completes successfully, and `cargo build` completes successfully. Report the remaining Git subprocesses separately; do not present this plan as shutdown cleanup.
