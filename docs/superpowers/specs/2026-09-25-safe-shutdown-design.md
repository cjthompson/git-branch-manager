# Safe Shutdown Design

## Purpose

Quitting git-branch-manager (GBM) should be prompt when ongoing work is harmless, while never silently interrupting a repository or filesystem mutation. A user must retain an explicit escape hatch for a stuck or unwanted operation, with a warning that force quitting can cause data loss or leave partial state.

## Entry points

- `q` requests an interactive quit. It immediately freezes the confirmed-action queue, preventing the next queued job from starting while the user decides.
- Ctrl+C, whether received as a terminal key or SIGINT, and SIGTERM request a noninteractive safe shutdown. They freeze and discard queued jobs, stop harmless work, restore the terminal, print a short shutdown message to stdout when stdout is usable (stderr otherwise), and wait for active mutations to settle. A second signal is not an implicit force quit. SIGKILL cannot be handled.
- Terminal restoration runs after normal, forced, and error exits. Signal handlers only set a flag; they do not render, lock, spawn, or perform cleanup.

## Work classes and ordinary `q`

| State | Behavior |
|---|---|
| No active work, no queued work, no SQLite operation | Exit immediately. |
| Only harmless read work | Request cancellation, stop new reads, reap any spawned children, and exit promptly. Do not wait for a long read to finish. |
| SQLite cache query or transaction active | Stop new cache work and let the current connection scope close. Let a write transaction commit or roll back before exiting; never terminate its Rust thread to simulate cancellation. |
| Confirmed jobs queued, none active | Show the queued section of the shutdown modal. Do not start a job until the user chooses. |
| Mutation or other unsafe operation active | Show the in-progress section. Continue the active operation to a result or its normal recovery boundary. |
| Active unsafe work and queued jobs | Show both sections, separated and scrollable. The queue remains frozen until the user chooses. |

Classify work by whether interruption can leave changed repository state, changed remote state, partially deleted files, or an incomplete cache write. A read-only Git/Graph/squash worker can be stopped; an active merge, rebase, cherry-pick, checkout with stash, push, worktree deletion, or similar mutation must settle. A fetch may change refs and therefore follows the mutation rule once started. The classification belongs to the operation, not to whether it uses `git2`, a child process, or Rust filesystem calls.

## Interactive shutdown modal

- Title: `Cleaning up...`.
- In-progress section: short operation names such as `Deleting worktree`; a settled/total progress bar; show each completion, failure, or safe cancellation briefly before removing it. Settled means finished, not necessarily successful.
- Queued section: short action names and a count of paused jobs. Do not mix paused jobs into the in-progress progress denominator.
- Stack sections with a horizontal separator on normal/narrow terminals. A wide layout may use columns with a vertical separator. The stacked body scrolls using the existing modal scroll behavior.
- Queued-only actions: `Run & quit` (unfreeze queue and exit when all jobs settle), `Discard & quit` (discard queued jobs and exit after any active work settles), and `Stay` (cancel quit and resume normal queue processing).
- Active-only actions: `Stay` and `Force quit...`. Waiting is the default; the app exits automatically once active work settles.
- Active plus queued actions: `Run & quit`, `Discard & quit`, `Stay`, and `Force quit...`. Completion of the active operation must not start the next queued job before `Run & quit` is chosen.
- `Force quit...` opens a second confirmation: `Force quit GBM? Active operations may stop partway through and leave repository files or worktrees in a partial state. Data loss is possible.` Choices are `Keep waiting` and `Force quit now`. Force quitting discards queued jobs, requests termination of tracked child process groups, gives owning workers a bounded opportunity to reap them, restores the terminal, and exits. In-process mutations can be cut off by process exit; the warning must not promise recovery or a clean repository.
- If an operation fails during shutdown, show its failure result in the modal long enough to inspect; do not silently exit and lose the error. The user can `Stay` to use the existing Results UI or confirm exit after reviewing the failure.

## Process and worker ownership

- A shared shutdown gate prevents new unrelated background work after quit begins. An already-running mutation retains a scoped permit to launch the follow-up commands needed to complete or recover (for example, `stash pop` or `rebase --abort`). `Run & quit` explicitly permits queued jobs to start; `Discard & quit` does not. Force quit revokes all permits. Spawning and registration are one atomic operation with respect to permit validation and gate changes.
- Each tracked child has an owning worker that drains its pipes and calls `wait`; the registry keeps cancellation/group signaling handles and retains entries until the owner has reaped the child. PID tracking alone does not reap children.
- Safe-read cancellation may signal its process group. Ordinary quit never signals an active mutation process group. Force quit may signal all tracked groups after the explicit confirmation.
- In-process workers check cooperative shutdown at safe boundaries. Cache connections are owned by their worker scopes; a cache writer reports completion after its transaction and connection are dropped.
- `gh pr view --web` must still reap its `gh` wrapper, without treating the user-launched browser as a GBM child that must be closed.

## Scope and dependencies

The safe-shutdown plan is the single implementation owner for three existing child-lifecycle bug sites: `run_git_cancellable` (undrained pipes and a killed child without `wait`), `gh pr view --web` (spawn-and-drop), and `compute_patch` (write-error return without `wait`). The three September 25 git2 conversion plans are authoritative for conversion scope. They can land before or after this work, so the remaining process inventory must be refreshed at implementation time.

The separate project task #111 covers a general in-app modal for viewing and cancelling individual queued jobs while GBM remains open. The shutdown modal may display queued jobs, but this plan does not implement task #111's general queue-management UI.

## Acceptance scenarios

1. Quit immediately after launch while Graph/squash workers are active: GBM exits promptly, no tracked `git`/`gh` child survives or remains a zombie, and no later worker starts another child.
2. Quit with only queued mutations: the queue stays paused until `Run & quit`, `Discard & quit`, or `Stay` is chosen.
3. Quit during merge/rebase/push/fetch or worktree deletion: active work continues to a reported outcome; no next queued job starts without an explicit choice.
4. Force quit during a stuck mutation: a second confirmation appears, tracked child groups are terminated and reaped as far as the bounded cleanup permits, and the terminal is restored.
5. Quit during a cache transaction: the connection scope closes after commit or rollback; a partial transaction is never presented as a completed cache write.
6. Ctrl+C and SIGTERM take the safe noninteractive path and print a shutdown line after terminal restoration when output is possible.
7. A subprocess with more output than a pipe buffer completes or cancels without deadlock; `gh pr view --web` and `compute_patch` error paths reap their direct children.
