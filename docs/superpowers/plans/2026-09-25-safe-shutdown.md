# Safe Shutdown Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task by task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let GBM quit promptly when background work is harmless, wait visibly for unsafe work, and offer an explicitly confirmed force quit without leaving untracked children.

**Architecture:** A shared shutdown runtime freezes new unrelated work, gives active mutations scoped permits for follow-up commands, tracks active work and children, and lets child-owning workers reap their own children. The action queue gains a pause state; the App renders a shutdown modal from live operation snapshots. Keyboard quit is interactive, while Ctrl+C and SIGTERM restore the terminal, print status, and wait safely without requiring input.

**Tech Stack:** Rust 2021, ratatui/crossterm, git2, rusqlite, `std::process`, `signal-hook`, direct `libc` dependency for process-group signals and test PTYs.

**Spec:** [2026-09-25-safe-shutdown-design.md](../specs/2026-09-25-safe-shutdown-design.md). The three git2 conversion plans listed below are authoritative for which shell-outs they convert; this plan owns shutdown and child-process lifecycle.

## Relationship to the git2 conversion plans

The three conversion plans are the source of truth for their scopes:

- [Straightforward git2 conversions](2026-09-25-git2-straightforward-conversions.md): original call points #1, #2, and #7.
- [Branch delete UX and git2 conversion](2026-09-25-git2-branch-delete-ux.md): original call point #4 and its confirmation behavior.
- [git2 parity and benchmark conversions](2026-09-25-git2-parity-and-benchmarks.md): candidate call points #3, #5, #6, and #8–#15, subject to parity/performance gates.

This plan is the **single owner** of child-process cleanup after those conversions. Its Task 3 fixes `run_git_cancellable` pipe drainage and reaping, `gh pr view --web` wrapper reaping, and `compute_patch`'s write-error path; it also inventories and manages shell-outs still present in live code. Tasks 2–3 own process registration and kill/reap behavior; Tasks 5–6 own `q`, SIGINT/SIGTERM, and terminal restoration. Do not treat a conversion candidate as removed until its own plan's tests and gates pass.

## Global Constraints

- Ordinary quit must never signal an active repository/filesystem mutation, including fetch, push, merge, rebase, stash, cherry-pick, checkout, and worktree deletion.
- Force quit requires a second confirmation and warns about data loss and partial repository/worktree state.
- `q` freezes the action queue before its next job starts. `Run & quit` is the only shutdown choice that resumes it. Signals discard the unstarted queue and wait for active unsafe work.
- Preserve unrelated dirty work. At plan-writing time `src/app.rs`, `src/view/graph.rs`, and `tests/integration.rs` are modified, and several other plan files are untracked. Recheck exact status/diffs at execution time. Do not restore or stage unrelated paths.
- The three git2 conversion plans may change the shell-out inventory. Enumerate live production `Command::new` sites before migrating them; do not assume an older inventory is exhaustive or that `operations.rs::git_cmd` covers other modules.
- Keep the local `Cargo.toml` `-devN` version out of commits. Follow repository version instructions during code iteration; run `cargo build` after each completed task and before claiming completion.
- The separate pending project task #111 owns the general in-app queue viewer/cancel modal. This plan shows queued jobs only inside the quit flow.
- Never run terminal rendering, locks, process control, or SQLite calls inside an async signal handler. SIGKILL is outside application control.

## File Map

- Create `src/shutdown.rs`: shutdown request state, worker/cache guards, spawn gate, process registry, and snapshots. Export from `src/lib.rs` for integration tests.
- Create `src/ui/shutdown.rs`: shutdown modal and force-quit confirmation rendering, using `ui/modal.rs` and `ModalScroll`.
- Modify `src/job_queue.rs`: pause/resume/discard semantics and read-only snapshots of current/draining/queued actions.
- Modify `src/app.rs` and `src/ui/render.rs`: `q`/Ctrl+C handling, modal state, progress updates, choices, and automatic safe exit.
- Modify `src/main.rs`: construct and pass the runtime, register SIGINT/SIGTERM flags, restore the terminal on every exit path, and print signal shutdown status after restoration.
- Modify `src/git/cache.rs` and its interactive worker callers (`main.rs`, `git/{branch,cherry_loader,diagnostics,graph,squash_loader}.rs`, `app.rs`): track SQLite connection scopes and prevent new cache work after shutdown begins. Keep noninteractive dump behavior intact.
- Modify live production shell-out call sites (`git/{operations,merge_detection,graph,github,...}.rs`, `app.rs`): use the managed process API. Include any remaining sites after the git2 conversion work lands.
- Test in `src/shutdown.rs`, `src/job_queue.rs`, `src/app.rs`, `src/ui/shutdown.rs`, and `tests/process_cleanup.rs`; use existing `tests/integration.rs` only for scenarios that need its Git fixtures.

## Review Focus

- Completion of an active job while a queue is paused must not launch the next job.
- A child spawned concurrently with quit must be registered under a valid permit or rejected; active mutation recovery commands must remain permitted until that mutation settles.
- A child that fills stdout/stderr pipes must still finish or cancel and be reaped by its owning worker.
- An SQLite writer caught mid-transaction must finish commit/rollback and drop its connection before safe exit; a read worker must not begin a later write after the gate closes.
- A narrow terminal and a long action list must keep both shutdown sections and every action reachable by scrolling.

---

### Task 1: Freeze the confirmed-action queue without losing active results

**Files:** Modify `src/job_queue.rs`; test in its existing test module.

**Interfaces:** Add `QueueExitMode { Normal, Paused, DrainThenExit }`, `pause_for_quit()`, `resume_normal()`, `drain_then_exit()`, `discard_queued()`, `has_active()`, and `queued_summary() -> Vec<QueuedJobSummary>`. `QueuedJobSummary` contains a short action label and target count, not a worktree path. Keep existing enqueue/execute results and `JobEvent` unchanged.

- [ ] Add `paused_queue_does_not_advance_after_active_completion`: inject a running job, queue two jobs, call `pause_for_quit()`, deliver the active result, and assert `poll()` emits the result while `queued_summary().len() == 2` and no next job starts. Add `resume_normal_starts_next_queued_job` and `discard_queued_preserves_active_job`.
- [ ] Run `cargo test paused_queue_does_not_advance_after_active_completion` and record the failing assertion.
- [ ] Add the queue mode and guard every `try_advance()` path, including the draining/panic result paths:

  ```rust
  fn try_advance(&mut self) {
      if self.is_busy() || self.exit_mode == QueueExitMode::Paused { return; }
      if let Some(job) = self.queued.pop_front() { self.start(job); }
  }
  ```

  `DrainThenExit` may advance until empty but must reject newly enqueued jobs. `Normal` retains today's behavior. Preserve each completed result for App refresh even during shutdown.
- [ ] Run the focused tests and `cargo build`. Inspect the diff and keep the version hunk unstaged.

### Task 2: Establish a shared shutdown gate and work ledger

**Files:** Create `src/shutdown.rs`; modify `src/lib.rs`; test in `src/shutdown.rs`.

**Interfaces:** `ShutdownRuntime::new() -> Arc<Self>`, `begin_background(WorkKind, &'static str) -> Option<WorkGuard>`, `begin_action(&'static str) -> Option<WorkGuard>`, `begin_cache_write(&WorkPermit, &'static str) -> Option<WorkGuard>`, `freeze_background()`, `permit_queued_actions()`, `resume_normal()`, `request_stop_reads()`, `request_force_stop()`, `snapshot() -> ShutdownSnapshot`, and `wait_for_cache_close()` are the public contract. `WorkKind` is `ReadOnly`, `CacheWrite`, or `Mutation`. `WorkGuard::permit() -> WorkPermit` returns a cloneable permit for nested command/cache calls; the guard deregisters on `Drop`. `ShutdownSnapshot` owns short labels and counts. `request_force_stop()` is called only after the second confirmation.

- [ ] Write concurrency tests: `freeze_background()` races with `begin_background()` but no new unrelated background guard may be created after freeze returns; an existing mutation permit still starts its recovery command; `permit_queued_actions()` admits only action-queue jobs; dropping a `CacheWrite` guard wakes a waiting shutdown.
- [ ] Run `cargo test shutdown::tests` and confirm the missing API fails compilation.
- [ ] Implement a mutex-protected gate and active-work map, with a condition variable for cache closure. Acquire the same gate lock for `begin_background()`, `begin_action()`, and gate transitions:

  ```rust
  pub fn begin_background(self: &Arc<Self>, kind: WorkKind, label: &'static str)
      -> Option<WorkGuard>;
  pub fn begin_action(self: &Arc<Self>, label: &'static str)
      -> Option<WorkGuard>;
  pub fn begin_cache_write(self: &Arc<Self>, permit: &WorkPermit,
      label: &'static str) -> Option<WorkGuard>;
  pub fn freeze_background(&self);
  pub fn permit_queued_actions(&self);
  pub fn resume_normal(&self);
  pub fn request_force_stop(&self);
  pub fn snapshot(&self) -> ShutdownSnapshot;
  ```

  Freezing rejects new unrelated background work, preserves active mutation permits, and does not dispose of existing guards. A read-only permit is revoked on quit and cannot begin later cache work. Starting a queued action requires both the queue's `DrainThenExit` mode and runtime permission. Avoid a process-global singleton; pass the runtime to interactive workers. The App action queue remains the authority for which confirmed mutation is active.
- [ ] Run focused tests, `cargo test shutdown::tests`, and `cargo build`.

### Task 3: Own and reap every remaining child process

**Files:** Extend `src/shutdown.rs`; modify live production shell-out call sites in `src/git/operations.rs`, `src/git/merge_detection.rs`, `src/git/graph.rs`, `src/git/github.rs`, `src/app.rs`, and any other files found by the inventory. Test in `src/shutdown.rs` and `tests/process_cleanup.rs`.

**Interfaces:** `run_output(&self, command: Command, permit: &WorkPermit, label: &'static str) -> io::Result<Output>` and `run_status(...) -> io::Result<ExitStatus>` are the only interactive-mode spawn paths. Preserve command arguments, working directory, environment, and output/error mapping. The registry records each direct child and process group while its owning worker retains the `Child` handle and performs `wait`.

- [ ] Inventory production shell-outs with `rg -n 'Command::new|\.spawn\(|\.output\(|\.status\(' src` and classify each by effects. Exclude test-only command sites and noninteractive dump paths only when they cannot run in the TUI process. Record the final list in a table in `src/shutdown.rs` module docs.
- [ ] Add a test using a child that writes more than one pipe buffer on both stdout and stderr, plus a cancellable long-lived child. Assert complete capture on success, prompt cancellation for a safe read, and `wait` completion for every direct child. Add a concurrent-spawn/freeze test that records child PIDs, then checks none escaped the registry.
- [ ] Run the focused tests under a short test timeout and verify the old `run_git_cancellable` path times out or fails its cancellation/reap assertion where reproducible.
- [ ] Implement permit validation, spawn, and registration under the gate lock. On Unix, use `CommandExt::process_group(0)` to place each managed child in its own group; add `libc` as a **direct** dependency for group signaling. Spawn stdout/stderr reader threads before polling `try_wait`, close stdin as appropriate, then `wait` and join readers on success, error, cancellation, and force-stop paths. `request_force_stop()` revokes all permits and signals registered process groups; `wait_for_children_bounded(Duration)` gives owners time to reap. Remove registry entries only after `wait`. Do not use `waitpid(-1)` or store PIDs as a substitute for child ownership.
- [ ] Route `run_git_cancellable` through the managed runner. Quit-driven cancellation signals only safe reads; preserve today's explicit per-job cancellation behavior, including its result reporting. Fix the existing `gh pr view --web` spawn-and-drop by waiting for its `gh` wrapper on its background thread. Fix `compute_patch` so a failed stdin write kills and waits for `patch-id` before returning.
- [ ] Migrate remaining interactive `git` and `gh` call sites, passing the active worker's `WorkPermit` through the call graph (including `job_queue::execute_action_with_remote`, `operations` functions, and Graph/squash worker entry points). Keep public no-runtime wrappers where noninteractive dump/tests require them. If a `git2` conversion removed a shell-out, do not recreate it. Preserve `gh pr view --web` browser detachment.
- [ ] Run focused process tests, affected Git integration tests, `cargo clippy -- -D warnings`, and `cargo build`. Re-run the inventory and confirm every TUI shell-out is managed or has a documented exception.

### Task 4: Track cache connection boundaries and stop late writes

**Files:** Modify `src/git/cache.rs`, interactive cache callers in `src/main.rs`, `src/app.rs`, `src/git/branch.rs`, `src/git/cherry_loader.rs`, `src/git/diagnostics.rs`, `src/git/graph.rs`, and `src/git/squash_loader.rs`; test in `src/git/cache.rs` and `src/shutdown.rs`.

**Interfaces:** Add `BranchCache::load_with_shutdown(repo_path, WorkPermit)` for the interactive path; existing `load` and `load_from_path` retain their noninteractive/test behavior. Wrap the actual SQLite connection scope, not merely a call site's `save()` invocation. A post-freeze reader may finish computation but must not begin a new cache transaction.

- [ ] Add a test that blocks a cache transaction between begin and commit, requests shutdown, and verifies the shutdown waiter remains blocked until commit/rollback and connection drop. Add a test that a worker finishing a read after freeze cannot start a new `save()` transaction. Confirm reopened cache state is either the old full value or the new full value.
- [ ] Run those tests and record the failing result before implementation.
- [ ] Pass a worker permit through all interactive cache owners and wrap `open_conn`/transaction work with `WorkKind::CacheWrite` or a read connection guard. The guard must outlive the connection and transaction:

  ```rust
  let _guard = shutdown.begin_cache_write(&worker_permit, "Updating cache")?;
  let mut conn = open_conn(&self.path)?;
  let tx = conn.transaction()?;
  // existing statements and commit; guard drops after tx and conn
  ```

  Adapt the current `save(&self)` error-return style without changing cache persistence semantics. Do not hold the shutdown mutex during SQLite calls or the 5-second busy timeout. Keep dump-mode cache operations independent of the interactive shutdown runtime.
- [ ] Run cache tests, the affected Graph/squash tests, and `cargo build`.

### Task 5: Render the interactive shutdown and force-quit modals

**Files:** Create `src/ui/shutdown.rs`; modify `src/ui/mod.rs`, `src/ui/render.rs`, `src/app.rs`; test in `src/ui/shutdown.rs` and `src/app.rs`.

**Interfaces:** Add `Overlay::Shutdown { scroll: ModalScroll, choice: ShutdownChoice }` and `Overlay::ConfirmForceQuit { choice: ForceQuitChoice }`. `ShutdownView` contains active summaries, settled/total progress, and paused queued summaries. Render through the existing `ModalSpec` shell.

- [ ] Add TestBackend rendering tests for active-only, queued-only, and combined states. Assert the title `Cleaning up...`, an in-progress progress bar, short action names, and a visible separator. Test a narrow terminal with more rows than fit: scrolling must reach the final row and choices.
- [ ] Run the focused tests and confirm the new overlay is absent.
- [ ] Implement the stacked layout first: `In progress`, a horizontal rule, then `Queued operations`. A wide two-column layout is optional; if added, use a vertical rule and keep the same scrolling semantics. Show recently settled rows briefly, marking failure distinctly from success. The labels are `Run & quit`, `Discard & quit`, `Stay`, and, when active unsafe work exists, `Force quit...`.
- [ ] Implement the second confirmation with `Keep waiting` and `Force quit now` and the spec's explicit data-loss copy. Escape returns to the cleaning-up modal without changing the quit state.
- [ ] Run TestBackend and App overlay tests, `cargo fmt --check`, and `cargo build`.

### Task 6: Drive safe quit from App and signals from main

**Files:** Modify `src/app.rs`, `src/main.rs`, `Cargo.toml`, and `Cargo.lock`; test in `src/app.rs`, `src/shutdown.rs`, and `tests/process_cleanup.rs`.

**Interfaces:** App receives `Arc<ShutdownRuntime>` and `Arc<AtomicBool>` for signals. `App::run` returns `io::Result<ExitOutcome>` where `ExitOutcome` is `Safe`, `SignalPending`, or `ForcePending`. A terminal guard in `main.rs` restores raw mode, alternate screen, mouse capture, and cursor on every return/error path. Keep CLI dump paths untouched.

- [ ] Add App tests for all five quit states in the spec. In particular, deliver an active-job completion while the queue is paused and assert no queued job starts; choose `Run & quit` and assert the queue drains; choose `Discard & quit` and assert it is emptied; choose `Stay` and assert normal dispatch resumes. Test failed active completion remains inspectable instead of auto-exiting.
- [ ] Run those focused tests and record their initial failure.
- [ ] Make `q` pause the action queue and freeze unrelated background starts *before* snapshotting work. Keep the active mutation's permit valid for follow-up steps. If only read-safe work exists, request cancellation and return once managed read children are reaped and active SQLite scopes are closed. If a queue or unsafe operation exists, open/update the shutdown modal each event-loop tick. `Run & quit` changes the queue to `DrainThenExit` and calls `permit_queued_actions()`; `Stay` calls `resume_normal()` and reopens normal dispatch. Do not suppress normal `JobEvent` refresh/reporting during shutdown.
- [ ] Register SIGINT/SIGTERM with `signal-hook` into an atomic flag; handle a crossterm Ctrl+C key in the same path because raw mode may deliver it as a key instead of a signal. Signal handling freezes/discards the queue and waits for unsafe work without user input. No handler calls `App`, `libc::killpg`, SQLite, or terminal APIs.
- [ ] For signals, have `App::run` return `SignalPending` after it freezes/discards the queue and requests safe-read cancellation. Install the terminal restoration guard immediately after entering raw mode, so later terminal setup errors also restore it. In `main`, restore the TTY, then print `GBM: shutting down; waiting for active operations` to stdout when usable (stderr on write failure). Add `App::wait_for_safe_shutdown()` to keep polling active results and cache/process completion without rendering; print completion/failure after it returns. For confirmed force quit, return `ForcePending`, restore the TTY, call `request_force_stop()` and `wait_for_children_bounded(Duration::from_secs(2))`, then exit the process even if an in-process mutation remains. Ordinary safe shutdown may wait for an unsafe mutation until it reports an outcome.
- [ ] Add a signal test against a subprocess on a PTY: send SIGTERM, verify queued work does not start, active mutation is allowed to finish, and the terminal is restored. Add a Ctrl+C key test against App state, and a direct process-registry signal test for child reaping. Use captured PIDs/PGIDs; never infer success from a system-wide `ps | grep git` count.
- [ ] Run focused tests, `cargo test`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo build`. Inspect `git diff --check` and the staged `Cargo.toml` version hunk before any commit.

### Task 7: Exercise the full shutdown matrix and audit coverage

**Files:** Add/modify `tests/process_cleanup.rs`; modify focused tests in `tests/integration.rs` only when necessary; update the plan's command inventory note if live code changed.

**Interfaces:** No production API changes. Tests launch a controlled GBM/child scenario, capture exact PIDs or process-group IDs, and assert each tracked child exits; they do not scan unrelated system processes.

- [ ] Test immediate `q` during startup Graph/squash work; queued-only choices; active merge/rebase/fetch; partial worktree deletion; a cache transaction; failed action completion; confirmed force quit; SIGINT/SIGTERM. Use small deterministic fake commands for lifecycle tests and temporary Git repositories for repository-state tests. Check the targeted repo's refs, sequencer state, and worktree metadata after each case.
- [ ] Run the targeted integration tests. A failure caused by an unrelated existing dirty checkout or environment restriction must be reported separately from the shutdown behavior under test.
- [ ] Manually run GBM in a real TTY, press `q` under each modal state, and send SIGTERM from another shell. Verify readable output after the terminal returns, modal scrolling at narrow width, and no targeted child PID remains alive or defunct.
- [ ] Run `cargo test` to completion and record its final exit code/summary, then `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --check`, `cargo build`, and `git diff --check`. Stop optional testing once these gates and the concrete lifecycle risks pass.

## Completion

The implementation is complete only when every ordinary quit path either exits promptly after safe read cleanup or visibly waits for unsafe work, the queue never advances after `q` without `Run & quit`, force quit requires its warning, SIGINT/SIGTERM take a noninteractive safe path, SQLite scopes close, and every managed direct child is reaped. Report any operation that cannot be made safely interruptible and any signal/terminal limits demonstrated by tests.
