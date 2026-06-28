# DRYness Review

This review covers the current working tree with a focus on duplicated behavior and places where the view framework still leaks per-tab plumbing. The code already has a useful generic `ListState` and `render_list_view`; the issues below are the remaining duplication hotspots that make new views or column changes more expensive than they need to be.

## Repeated Semantic Column Definitions

### TLDR

Branches, remotes, worktrees, and tags each declare their columns manually, even when the columns represent the same concept: age, merge status, ahead/behind, and PR. Examples include duplicate ahead/behind and PR comparators in `src/view/branches.rs:38` and `src/view/remotes.rs:24`, duplicate age column layouts in `src/view/branches.rs:65`, `src/view/remotes.rs:51`, `src/view/worktrees.rs:39`, and `src/view/tags.rs:24`, and duplicate merge-status ranking in `src/view/branches.rs:72`, `src/view/remotes.rs:58`, and `src/view/worktrees.rs:46`.

### Impact

Changing one shared behavior requires remembering every tab that copied it. For example, changing the merge status sort order, PR sort ordering, age width, or responsive hide threshold can silently diverge across tabs. This also raises the cost of adding another view because the developer has to rediscover the same column conventions instead of composing existing column builders.

### Possible Resolution(s)

- Add column builder helpers in `src/view/column.rs` or a new `src/view/columns.rs`, such as `age_column`, `merge_status_column`, `ahead_behind_column`, and `pr_column`.
- Extend `ViewItem` where the data already exists, so shared columns can be expressed against trait methods instead of concrete `BranchInfo` and `RemoteBranchInfo` fields.
- Keep view-specific columns local, but compose common columns from shared helpers. For example, branch and remote definitions can share the A/B, PR, Age, and Status columns while retaining their own name/local/remote columns.
- Extract semantic comparators first if full column builders are too large a step: `compare_ahead_behind`, `compare_pr_number`, and `merge_status_rank`.

### Test Cases

- Unit-test each shared comparator once with branch and remote sample data, including `None` ahead/behind values and PR-present versus PR-absent ordering.
- Assert that branch, remote, tag, and worktree age columns have the same width and hide-threshold policy after the helper is introduced.
- Assert that branch, remote, and worktree merge status sorting produce the same order: merged, squash-merged, unmerged, pending.
- Add a regression test that changing a shared column helper affects every view that uses it.

## Repeated Filter Token Lists

### TLDR

Filter token definitions are copied between views. Status tokens are repeated in `src/view/branches.rs:90`, `src/view/remotes.rs:76`, and `src/view/worktrees.rs:64`. Age tokens are repeated in all four views, including `src/view/tags.rs:45`. PR and sync tokens are repeated in both branch-like views at `src/view/branches.rs:107` and `src/view/remotes.rs:93`.

### Impact

The filter parser is centralized, but the advertised token sets are not. A label, key binding, or token spelling can drift by view even though the behavior underneath is shared. This is especially risky for discoverability: the filter overlay may show different shortcuts for equivalent filters, or a new token may be supported by parsing but omitted from one tab's menu.

### Possible Resolution(s)

- Add token groups in `src/view/filter.rs`, such as `status_tokens()`, `age_tokens()`, `pr_tokens()`, and `sync_tokens()`.
- Build each view's token list by concatenating the groups it supports.
- Consider a small capability enum, for example `FilterCapability::{Status, Age, Pr, Sync}`, and a helper that expands capabilities into ordered tokens.
- Keep view-specific filtering disabled by omission, but make the omission explicit through capabilities rather than repeated literal token lists.

### Test Cases

- Unit-test the token group helpers for exact key, label, and token values.
- Assert that branches and remotes expose identical PR and sync token groups.
- Assert that branches, remotes, and worktrees expose identical status token groups.
- Assert that all four views expose identical age token groups.
- Add a filter overlay test or snapshot that confirms token order remains stable after composition.

## Active View Dispatch Is Repeated Throughout App and Rendering

### TLDR

The app repeatedly matches `active_view` to recover the active state, columns, tokens, renderer, header metadata, search state, and sort behavior. Examples include state fields in `src/app.rs:55`, filter-token selection in `src/app.rs:279` and `src/app.rs:1017`, common key handling in `src/app.rs:649`, search handling in `src/app.rs:1051`, mouse handling in `src/app.rs:1110`, header sorting in `src/app.rs:1147`, sorting commands in `src/app.rs:1856`, filter helpers in `src/app.rs:2125`, and render dispatch in `src/ui/render.rs:96`.

### Impact

Every cross-view feature is implemented four times in slightly different shape. That makes behavior changes noisy and easy to miss. It also prevents the generic `ListState` and `render_list_view` abstractions from carrying their full weight because callers still need to know every concrete view at each interaction point.

### Possible Resolution(s)

- Introduce an `ActiveView` access layer on `App` with methods such as `active_filter_tokens`, `active_filter_query`, `set_active_filter`, `active_status_bar_items`, and `active_header_columns`.
- Add focused helpers for common mutations: `with_active_state`, `with_active_state_mut`, and `sort_active_by_column`.
- For typed operations that cannot return a single generic type, keep a localized `match`, but move repeated mechanics into small helpers. For example, header-click sorting can call the same helper in each branch instead of repeating the set/toggle/apply sequence.
- Consider a `ViewRuntime<T>` wrapper only if the lifetime and generic complexity stays lower than the current four parallel field sets.

### Test Cases

- Unit-test active-view helpers by switching `active_view` across all four views and verifying they read/write the expected `ListState`.
- Regression-test common keys (`j`, `k`, space, `a`, `n`, `i`, `s`, `S`) on each active view.
- Regression-test mouse header sorting on all four views: first click sets ascending sort, second click toggles direction.
- Regression-test search and filter query updates on each active view to ensure helper methods do not write to the wrong tab.

## Row Rendering Reimplements Shared Cells

### TLDR

The table renderer is generic, but row renderers still duplicate cell-level logic. Branch and remote rows duplicate ahead/behind, PR, age, and merge-status cells in `src/app.rs:2438` and `src/app.rs:2592`. Worktrees duplicate age and merge-status cells in `src/app.rs:2793`. Tags duplicate age rendering in `src/app.rs:2728`.

### Impact

Visual and responsive behavior can drift across tabs. A change to PR coloring, merge-status text, symbol-only status at narrow widths, or age display must be made in several places. Since these are user-facing cells, drift is visible and hard to explain: the same concept can sort or render differently depending on the tab.

### Possible Resolution(s)

- Move reusable cell builders into `src/ui/shared.rs` or a new `src/ui/cells.rs`: `age_cell`, `merge_status_cell`, `ahead_behind_cell`, and `pr_cell`.
- Reuse the existing `ViewItem` methods for generic cells where possible: `last_commit_date`, `ahead`, `behind`, `pr_info`, and `merge_status`.
- Keep name/path/message cells local to each row renderer, since those are genuinely view-specific.
- Add one wrapper for the base-branch blank status case instead of baking that exception into the generic merge-status cell.

### Test Cases

- Unit-test shared cell builders with compact and non-compact `CellContext` inputs.
- Assert merge status cells render symbol-only below width 70 and full text at width 70 or above.
- Assert ahead/behind cells omit zero and `None` counts, show both non-zero counts, and include the expected symbols.
- Assert PR cells use the expected style for draft, open, merged, and closed PR states.
- Add rendering snapshots for branch, remote, tag, and worktree rows at narrow and wide widths.

## Status Bar Text Duplicates Summary Logic

### TLDR

`default_status_text` builds separate strings for each view and duplicates branch-like summary counting for branches and remotes in `src/ui/render.rs:260`. Both branch-like status bars count total, selected, merged, and squashed items before rendering nearly identical shortcut text.

### Impact

Status bar behavior can drift when adding or changing common commands. The branch and remote bars already differ mainly by nouns and available shortcuts, but their counting logic is copied. If selection, merge-count semantics, or shortcut labels change, the update needs to happen in multiple branches.

### Possible Resolution(s)

- Extract a `branch_like_summary` helper that computes total, selected, merged, and squashed for any item exposing `merge_status`.
- Represent shortcut text as per-view data, for example `StatusBarSpec { noun, show_selected, shortcuts }`.
- Keep the final rendered text in one formatter that accepts summary counts and a shortcut list.
- Consider moving view-specific status-bar specs next to view definitions so columns, filters, and status bar metadata live together.

### Test Cases

- Unit-test branch-like summary counts for branches and remotes, including selected items and squash-merged items.
- Unit-test final status text for each view to preserve current shortcut labels.
- Add a regression test that branch and remote summaries count merge statuses the same way.
- Verify clickable status-bar regions are still generated correctly for all shortcuts after the formatter changes.

## View-Specific Selection-to-Action Helpers Repeat Target Collection

### TLDR

Action helpers repeat the same pattern: collect selected indices, filter pinned/protected items when needed, map items into target names, then open a confirm overlay. Examples include branch deletion in `src/app.rs:1757`, remote deletion in `src/app.rs:1781`, tag deletion and push in `src/app.rs:1801`, and worktree removal in `src/app.rs:1827`.

### Impact

The target-selection rules are easy to copy incorrectly. The branch and worktree helpers filter pinned/main items, tags do not, and remote branches filter pinned items. Because each helper builds the confirm overlay itself, changes to confirmation behavior or empty-selection handling must be repeated.

### Possible Resolution(s)

- Add a generic helper like `confirm_selected(action, return_view, state, target_mapper)` where `target_mapper` returns `Option<String>`.
- Keep action selection local (`DeleteTag` versus `DeleteTagAndRemote`, `WorktreeRemove` versus `WorktreeForceRemove`), but share the selected-index and confirm-overlay mechanics.
- Extract target mappers for branch name, remote short name, tag name, and worktree path.
- Use the same helper for `get_cursor_targets` where practical, or at least centralize target-name extraction by view.

### Test Cases

- Unit-test target mappers for all four item types, including pinned/base/current/main exclusions.
- Unit-test that empty target lists do not open a confirm overlay.
- Unit-test that each action sets the correct `return_view` and `BranchAction`.
- Regression-test mixed selections where protected and unprotected items are selected together.

## Additional Repo-Wide Findings

The following findings come from a broader pass over the rest of the codebase. They intentionally skip the view/tab rendering issues already documented above.

## Git Command Execution Is Reimplemented Across Git Modules

### TLDR

`src/git/operations.rs` has a local `git_cmd` helper and cancellable runner at `src/git/operations.rs:10` and `src/git/operations.rs:27`, but other git modules build commands independently. Tags shell out directly in `src/git/tags.rs:103`, `src/git/tags.rs:126`, and `src/git/tags.rs:154`. Worktrees use a separate `git_out` helper in `src/git/worktree.rs:8`. GitHub integration has another command setup for `gh` in `src/git/github.rs`. These wrappers duplicate current directory setup, stdin behavior, environment configuration, output decoding, and failure handling.

### Impact

Command behavior can drift by module. For example, `operations.rs` sets `GIT_TERMINAL_PROMPT=0`, while `worktree.rs` sets `GIT_OPTIONAL_LOCKS=0`, and tag commands have their own non-cancellable path. If the project needs a shared timeout, tracing, cancellation, environment policy, stderr decoding, or test hook, every module must be updated separately.

### Possible Resolution(s)

- Add a shared command runner module, for example `src/git/command.rs`, with helpers for `git`, `gh`, stdout capture, stderr capture, and cancellable execution.
- Move `git_cmd` and `run_git_cancellable` out of `operations.rs` so `tags.rs`, `worktree.rs`, `github.rs`, and `merge_detection.rs` can reuse them.
- Encode command intent in small result types: `CommandOk { stdout, stderr }`, `CommandFailed { stderr }`, and `CommandCancelled`.
- Keep per-command environment additions possible, but make the baseline environment consistent.

### Test Cases

- Unit-test the shared runner using a harmless command to verify current directory, stdin nulling, environment injection, stdout trimming, and stderr capture.
- Unit-test cancellable execution with a long-running command and assert the child is killed and reported as cancelled.
- Regression-test tag push/delete and branch push/delete still produce the same `OperationResult` messages after moving to the shared runner.
- Verify worktree listing still disables optional locks if that remains required.

## OperationResult Construction Is Copied Throughout Operations

### TLDR

Most operation functions manually construct the same `OperationResult` shape for success, command failure, and spawn error. Examples include checkout in `src/git/operations.rs:97`, fetch in `src/git/operations.rs:146`, fast-forward in `src/git/operations.rs:172`, push in `src/git/operations.rs:232`, force-push in `src/git/operations.rs:260`, remote checkout in `src/git/operations.rs:428`, remote delete in `src/git/operations.rs:493`, remote fetch/pull in `src/git/operations.rs:520`, worktree creation/removal in `src/git/operations.rs:632` and `src/git/operations.rs:664`, and tag operations in `src/git/tags.rs:66` and `src/git/tags.rs:153`.

### Impact

The code repeats noisy boilerplate, and failure semantics are inconsistent. Some failures preserve stderr, some replace all failures with a generic conflict message, and some tag failures drop stderr entirely. This makes behavior harder to audit and makes new operations more likely to return subtly different messages or actions.

### Possible Resolution(s)

- Add constructors on `OperationResult`, such as `success(action, name, message)`, `failure(action, name, message)`, `from_output(action, name, output, success_message)`, and `from_io_error(action, name, error)`.
- Add a helper that maps the shared command-runner result into `OperationResult`, including the cancellation case.
- Use specialized wrappers only where an operation has domain-specific recovery, such as merge abort or rebase abort.
- Consider moving operation messages near `BranchAction` if labels and success messages should stay aligned.

### Test Cases

- Unit-test `OperationResult` constructors for branch name, action, success flag, and message.
- Regression-test representative operations: one success path, one command stderr failure path, one IO error path, and one cancellation path.
- Verify conflict operations still run their abort command and return the existing conflict message.
- Verify tag remote-delete failures preserve useful stderr once migrated.

## Stash Around Checkout/Merge/Rebase Is Duplicated

### TLDR

Auto-stash behavior appears as inline `stash push` and `stash pop` pairs in checkout, merge, and rebase flows: `src/git/operations.rs:85`, `src/git/operations.rs:306`, and `src/git/operations.rs:372`. Each operation is responsible for remembering to pop the stash on early failure and after the main command.

### Impact

This is a correctness-sensitive pattern hidden as repeated boilerplate. Any future operation that needs temporary stashing can forget cleanup, and the current code does not centralize what happens when `stash push` or `stash pop` fails. A failure in the middle of an operation can leave user state in a harder-to-reason-about condition.

### Possible Resolution(s)

- Add a `with_auto_stash(repo_path, enabled, operation)` helper that pushes before running a closure and pops in one cleanup path.
- Have the helper return enough detail to report stash failures if desired.
- Use the helper for checkout, merge, and rebase first before adding it to more operations.
- Consider a small guard type whose `Drop` attempts to pop only when a stash was successfully created.

### Test Cases

- Regression-test checkout with dirty worktree still stashes and pops around the checkout.
- Unit-test the helper with a failing operation and assert cleanup still runs.
- Test behavior when stash push fails and when stash pop fails, even if the first implementation only records/logs those failures.
- Verify merge and rebase conflict paths still attempt cleanup.

## Branch Loading and Enrichment Logic Is Split Between Main and Branch Module

### TLDR

Branch metadata is collected in several phases, but the phase orchestration and graph computations are split across `main.rs` and `src/git/branch.rs`. `main.rs` owns the startup background loader and sends `Phase1Msg` values at `src/main.rs:92`, while `src/git/branch.rs` owns `list_branches_phase1`, `list_branches_fast`, `list_branches`, and `collect_branch_metadata` at `src/git/branch.rs:75`, `src/git/branch.rs:100`, `src/git/branch.rs:264`, and `src/git/branch.rs:298`. Ahead/behind and merge-base computation exist inside `collect_branch_metadata` at `src/git/branch.rs:348` and `src/git/branch.rs:368`, then are reimplemented for deferred startup enrichment in `src/main.rs:195` and `src/main.rs:227`.

### Impact

The fast path and full path can drift. If branch metadata changes, the synchronous list mode, startup TUI path, and refresh path may not all get the same data or error handling. The split also puts domain logic in `main.rs`, which makes it harder to test without running the application entrypoint.

### Possible Resolution(s)

- Move `compute_merge_bases`, `compute_ahead_behind`, and the startup phase-1 orchestration into `src/git/branch.rs` or a new `src/git/branch_loader.rs`.
- Represent the progressive branch load as a single API that returns a receiver of domain messages instead of making `main.rs` build the thread by hand.
- Reuse the same graph helper functions from both full metadata collection and deferred enrichment.
- Keep `main.rs` limited to CLI parsing, app construction, terminal setup, and wiring receivers into `App`.

### Test Cases

- Unit-test ahead/behind helper once and use it from both fast enrichment and full branch listing.
- Unit-test merge-base helper once and verify hash truncation remains consistent.
- Add an integration test that `list_branches_phase1` and the progressive loader converge to equivalent branch metadata after all messages are applied.
- Verify `--list` mode and TUI startup report the same tracking and age data for the same repository.

## Background Loader Patterns Are Repeated By Feature

### TLDR

The app repeatedly creates channels, sets loading/toast state, spawns a thread, does work, and sends one result. Examples include auto-fetch and worktree preload in `src/main.rs:150` and `src/main.rs:161`, tag loading in `src/app.rs:1906`, remote loading in `src/app.rs:1919`, worktree loading in `src/app.rs:1960`, remote fetch in `src/app.rs:1971`, `spawn_pr_loader` in `src/git/pr_loader.rs:8`, `spawn_squash_checker` in `src/git/squash_loader.rs:13`, and `enrich_worktrees` in `src/git/worktree.rs:73`.

### Impact

Each loader handles errors, dropped receivers, loading flags, and user feedback differently. Some loaders clear toasts in the drain path, some do not surface errors at all, and the app entrypoint duplicates loader setup that already exists later in `App`. This makes it hard to add cancellation, progress, logging, or consistent failure UI for background work.

### Possible Resolution(s)

- Add small spawn helpers for one-shot loaders and streaming enrichment loaders.
- Move startup auto-fetch and preload worktree setup into `App` methods so launch-time and in-app behavior use the same path.
- Standardize loader result messages to include success/error state instead of silently returning on failure.
- Keep specialized workers, but share the channel/thread/toast/loading-state mechanics.

### Test Cases

- Unit-test loader methods set the appropriate `loading` flag and receiver field.
- Regression-test startup worktree preload and in-app worktree loading go through the same method.
- Test dropped receiver behavior for streaming loaders to ensure threads exit cleanly.
- Test failure paths produce a consistent toast or result message once errors are surfaced.

## Working Tree Status Detection Has Two Implementations

### TLDR

The main repository status uses git2 in `src/git/status.rs:5`, while worktree enrichment parses `git status --porcelain` in `src/git/worktree.rs:115`. Both produce the same `WorkingTreeStatus` with `has_staged`, `has_unstaged`, and `has_untracked`.

### Impact

The same domain concept can disagree depending on which path produced it. The git2 path includes specific status bits, while the porcelain parser has its own interpretation of index/worktree columns. If ignored files, renamed files, type changes, or nested untracked directories need new behavior, two implementations must be updated and tested.

### Possible Resolution(s)

- Prefer one implementation for both main repo and worktrees. Since `status.rs` already uses git2, expose a path-based helper that opens a repository at a worktree path and calls the same status logic.
- If shelling out remains necessary for worktrees, move porcelain parsing into a shared parser function and test it independently.
- Add conversion helpers from git2 status and porcelain status into `WorkingTreeStatus` so the policy is explicit.

### Test Cases

- Unit-test staged, unstaged, untracked, renamed, deleted, and type-change cases against the shared policy.
- Regression-test main repo status and a secondary worktree status for equivalent file changes.
- Unit-test porcelain parser lines if the CLI-based implementation remains.
- Verify clean repositories still return `WorkingTreeStatus::clean()`.

## Domain Formatting and Conversion Rules Are Scattered

### TLDR

Several domain conversions live inline near their callers rather than in one place. Merge status is converted to list-mode text in `src/main.rs:60`, cache strings in `src/git/cache.rs:24` and `src/git/cache.rs:48`, row/status UI text elsewhere, and PR state is converted in `src/git/github.rs`. Commit hash truncation appears in branch merge-base handling at `src/main.rs:218` and `src/git/branch.rs:375`, tag hashes at `src/git/tags.rs:52`, and worktree HEAD hashes at `src/git/worktree.rs:52`.

### Impact

String forms become accidental APIs. A cache value, CLI label, UI label, or short-hash length can drift without type-level pressure. The cache already uses `"squash_merged"` while user-facing text uses `"squash-merged"`, which is fine if intentional, but there is no centralized conversion making the distinction explicit.

### Possible Resolution(s)

- Add explicit conversion methods for `MergeStatus`: cache key, CLI label, and UI label if they intentionally differ.
- Add a shared `short_hash(hash, len)` helper or typed `ShortHash` constructor.
- Add a PR status conversion helper for GitHub API state and draft flag.
- Keep user-facing formatting separate from persisted/cache formatting, but define both centrally.

### Test Cases

- Unit-test every `MergeStatus` cache key and label mapping.
- Unit-test `short_hash` with empty, shorter-than-limit, exact-limit, and longer-than-limit inputs.
- Unit-test GitHub PR state conversion, especially draft taking precedence over state.
- Regression-test cache roundtrip for every cacheable merge status.
