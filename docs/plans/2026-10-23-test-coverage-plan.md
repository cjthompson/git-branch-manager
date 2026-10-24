# Test coverage plan

Baseline: `main` at `d2e1b9c`, measured 2026-10-23 with `cargo llvm-cov --all-targets --no-fail-fast -- --include-ignored`.

- **Overall line coverage: 78.98%** (4,254 of 20,234 lines missed); function 82.12%, region 78.98%.
- 892 tests: 889 pass, 3 fail. The failures are ignored diagnostics in `tests/integration/graph/patch_parity.rs` and are out of scope here.
- Doctests: none.

## Ordering rule

Files are worked in ascending line coverage. Ties go to more missed lines first. Files at 100% are excluded. The full ranked list is in the appendix.

Tiers used below:

| Tier | Line coverage | Files |
|---|---|---:|
| T0 | 0% | 4 |
| T1 | under 40% | 6 |
| T2 | 40% to under 60% | 7 |
| T3 | 60% to under 75% | 17 |
| T4 | 75% to under 90% | 36 |
| T5 | 90% to under 100% | 47 |

## Structural constraint: bin-only modules

`main.rs` declares `ahead_behind`, `app`, `dump`, `logging`, `phase1`, and `terminal` as binary-only modules. The `tests/integration/*` crate links only the library, so it cannot reach them. Tests for these files must be in-crate unit tests:

- `app/*` uses the existing `src/app/tests/` directory and its `render_app` helper.
- Other bin-only files use a `#[cfg(test)] mod tests;` declaration with a sibling tests file.
- `tempfile` is a dev-dependency and is available to both.

Most T0 and T1 gaps come from this, not from missing test effort.

## Tier 0: zero coverage

### 1. `ahead_behind.rs` (0 of 106 lines)

- **Only caller:** `phase1.rs:199`, the TUI startup path. Dump mode never reaches it.
- **Test location:** `src/ahead_behind/tests.rs`, wired from the bottom of `src/ahead_behind.rs`.
- **Fixture:** a `git2::Repository` in a `tempfile` directory, commits and refs created through git2 (no git CLI). Build `BranchInfo` values in a local helper; check for an existing constructor first.
- **Cases:**
  - tracked branch with upstream, empty cache: returns `(name, Some(ahead), Some(behind))` and writes the cache entry
  - same call again with the same cache: exercises the cache-hit path
  - `TrackingStatus::Tracked { gone: true }`: skipped, absent from output
  - `TrackingStatus::Local`: skipped, absent from output
  - tracked branch missing from the repo: find-branch error, absent from output
  - tracked branch with no upstream configured: upstream error, absent from output
  - upstream ref pointing at a blob: peel error, absent from output
  - local branch ref pointing at a blob: branch peel error, if libgit2 accepts the ref
  - graph error: needs an object-database fixture that libgit2 can't walk. Expect this branch to stay uncovered unless one is easy to build; say so in the PR.
- **Done when:** every case above passes, and any remaining uncovered lines are listed in the PR description with the reason.

### 2. `phase1.rs` (0 of 151 lines)

- **Entry:** `spawn_phase1_loader(repo_path, base_branch, cache_root)` returns a receiver and spawns two threads.
- **Message order:** `Phase1Msg::Fast`, then `AheadBehind`, `MergeBaseCommits`, and `MergeStatuses`, per the variant comments in `src/app/mod.rs`.
- **Test location:** `src/phase1/tests.rs`.
- **Cases:**
  - base branch, one tracked branch with upstream, one unmerged branch, one merged branch: assert message order and contents
  - base branch only: the merge-detection thread is not spawned (`phase1.rs:70` comment); assert no merge-status message
  - second run with the same cache root and an unchanged base tip: exercises the cache fast path (around `phase1.rs:99-102`)
- **Rules:**
  - Use `CacheRoot::at(tempdir)`. Never `CacheRoot::from_env()`, which writes to the real user cache.
  - Receive with `recv_timeout`, never a blocking `recv`, so a regression fails instead of hanging.
- **Done when:** the three cases pass and the remaining uncovered lines are error paths, listed in the PR description.

### 3. `logging.rs` (0 of 35 lines)

- **Blocker:** `init_debug_log()` installs a global `tracing` subscriber with `.init()`, which can run once per process. It also reads `GBM_DEBUG` from the process environment. Neither is safe in unit tests.
- **Refactor:** extract a pure function that maps the `GBM_DEBUG` value (`Option<&str>`) plus a default-path closure to `Option<PathBuf>`. Test every branch: unset, `0`, `false`, empty, `1`, `TRUE`, and an explicit path.
- **Test location:** `src/logging/tests.rs`, or a `#[cfg(test)]` module in `logging.rs`.
- **Left uncovered:** the subscriber-install lines (about 20 of 35). Covering them needs a process-level test, and dump mode skips logging by design (`main.rs:32`). This needs the decision in "Open decisions" below.

### 4. `terminal.rs` (0 of 14 lines)

- **Blocker:** `run_app` enables raw mode and the alternate screen on real stdout. It needs a TTY.
- **Options:**
  - A. Exclude `terminal.rs` from the report with an ignore regex, and say so in the report.
  - B. Drive it through a pseudo-terminal test, which adds a dev-dependency.
  - C. Leave it uncovered.
- **Recommendation:** A. See "Open decisions."

## Tier 1: under 40%

Work each file from its HTML uncovered-line view, group uncovered functions by behavior, and add tests in the layer the file lives in.

| File | Gap | Where tests go |
|---|---|---|
| `app/settings.rs` | 187 of 220 lines missed | `src/app/tests/settings.rs`, following the `startup_settings.rs` pattern |
| `app/mouse_search.rs` | 116 of 146 missed | `src/app/tests/mouse_search.rs` |
| `git/operations/remote.rs` | 161 of 217 missed | extend `tests/integration/remote_ops.rs`. Public surface: `checkout_remote_branch`, `delete_remotes_batch*`, `fetch_remote`, `pull_remote`, `merge_remote_into_current`, `cherry_pick_remote` |
| `app/keys_views.rs` | 85 of 131 missed | `src/app/tests/` |
| `job_queue/execute.rs` | 170 of 273 missed | extend `src/job_queue/tests/execute_local.rs` and `execute_remote.rs`. Cover each `BranchAction` mapping in `execute_action_with_remote` |
| `git/tags.rs` | 87 of 140 missed | extend `tests/integration/tags_direct.rs`. Public surface: `list_tags`, `delete_tag`, `delete_tags_batch`, `delete_remote_tags_batch`, `push_tag` |

## Tiers 2 to 5

Tiers 2 to 5 (T2 through T5, 107 files) are in the appendix. Each one is planned when its turn comes, using the same method as Tier 1. Planning them now would be guesswork.

## Per-file workflow

1. Run `cargo llvm-cov --all-targets --no-fail-fast -- --include-ignored`, then `cargo llvm-cov report --html`.
2. Open the file's uncovered lines and group them by function.
3. Write tests in the file's layer (see the structural constraint above).
4. Re-run coverage and confirm the file's line count rose.
5. Run `cargo test`, `cargo clippy`, and `cargo build`. The project `CLAUDE.md` requires `cargo build` after every task.

## Delivery

- Each file, or a cluster of closely related files, gets its own `ct/` branch cut from `origin/main`. Each is a separate PR. Test-only changes are independently shippable.
- Do not touch `Cargo.toml`'s version, per the project `CLAUDE.md`.
- Run `pre-push-check` before pushing and after the PR is created.

## Open decisions

1. **`terminal.rs` and the TUI entry path:** exclude from the report (option A), pseudo-terminal test (B), or leave uncovered (C)?
2. **`logging.rs` refactor:** approve extracting the pure `GBM_DEBUG` resolver?
3. **Target:** the Tier 0 and Tier 1 work, raising each file to 80% line coverage, adds roughly 800 covered lines. That moves overall coverage from 78.98% to about 83%. Confirm that target, or name another.
4. **Failing ignored parity tests:** confirm they stay out of scope for this plan.

## Appendix: ranked files below 100%

Source: the coverage run above. Line counts are from `cargo llvm-cov`'s lines column.

| # | File | Line cover | Missed / total lines | Tier |
|---|---|---:|---:|---|
| 1 | `phase1.rs` | 0.00% | 151 / 151 | T0 |
| 2 | `ahead_behind.rs` | 0.00% | 106 / 106 | T0 |
| 3 | `logging.rs` | 0.00% | 35 / 35 | T0 |
| 4 | `terminal.rs` | 0.00% | 14 / 14 | T0 |
| 5 | `app/settings.rs` | 15.00% | 187 / 220 | T1 |
| 6 | `app/mouse_search.rs` | 20.55% | 116 / 146 | T1 |
| 7 | `git/operations/remote.rs` | 25.81% | 161 / 217 | T1 |
| 8 | `app/keys_views.rs` | 35.11% | 85 / 131 | T1 |
| 9 | `job_queue/execute.rs` | 37.73% | 170 / 273 | T1 |
| 10 | `git/tags.rs` | 37.86% | 87 / 140 | T1 |
| 11 | `app/event_loop.rs` | 41.25% | 47 / 80 | T2 |
| 12 | `app/overlay_keys.rs` | 47.37% | 410 / 779 | T2 |
| 13 | `main.rs` | 50.63% | 39 / 79 | T2 |
| 14 | `git/operations/mod.rs` | 52.63% | 27 / 57 | T2 |
| 15 | `ui/toast.rs` | 53.33% | 21 / 45 | T2 |
| 16 | `git/diagnostics/workers.rs` | 54.46% | 51 / 112 | T2 |
| 17 | `app/keys.rs` | 55.56% | 68 / 153 | T2 |
| 18 | `app/confirm.rs` | 60.98% | 64 / 164 | T3 |
| 19 | `git/diagnostics/verify.rs` | 61.86% | 45 / 118 | T3 |
| 20 | `git/operations/sync.rs` | 63.18% | 109 / 296 | T3 |
| 21 | `job_queue/cascade.rs` | 63.28% | 94 / 256 | T3 |
| 22 | `app/mod.rs` | 63.64% | 16 / 44 | T3 |
| 23 | `app/keys_results_settings.rs` | 63.70% | 102 / 281 | T3 |
| 24 | `git/github.rs` | 66.89% | 50 / 151 | T3 |
| 25 | `git/operations/worktree_ops.rs` | 67.27% | 54 / 165 | T3 |
| 26 | `app/refresh.rs` | 68.14% | 65 / 204 | T3 |
| 27 | `types/diagnostics.rs` | 68.42% | 6 / 19 | T3 |
| 28 | `view/graph/mod.rs` | 68.46% | 88 / 279 | T3 |
| 29 | `types/action.rs` | 69.70% | 20 / 66 | T3 |
| 30 | `app/drain.rs` | 69.96% | 201 / 669 | T3 |
| 31 | `git/branch/mod.rs` | 70.39% | 45 / 152 | T3 |
| 32 | `git/operations/local.rs` | 70.51% | 64 / 217 | T3 |
| 33 | `ui/diagnostics.rs` | 71.19% | 34 / 118 | T3 |
| 34 | `git/squash_loader.rs` | 74.03% | 40 / 154 | T3 |
| 35 | `git/branch/metadata.rs` | 75.26% | 48 / 194 | T4 |
| 36 | `ui/info_modal/ref_fields.rs` | 75.48% | 51 / 208 | T4 |
| 37 | `ui/render/mod.rs` | 75.90% | 67 / 278 | T4 |
| 38 | `git/cherry_loader.rs` | 76.42% | 25 / 106 | T4 |
| 39 | `git/merge_detection/reachable.rs` | 76.47% | 40 / 170 | T4 |
| 40 | `app/helpers.rs` | 76.67% | 21 / 90 | T4 |
| 41 | `ui/info_modal/fields.rs` | 76.69% | 38 / 163 | T4 |
| 42 | `git/branch/merge_base.rs` | 76.71% | 34 / 146 | T4 |
| 43 | `ui/confirm/mod.rs` | 76.83% | 19 / 82 | T4 |
| 44 | `ui/status_bar.rs` | 76.85% | 25 / 108 | T4 |
| 45 | `git/diagnostics/mod.rs` | 77.22% | 36 / 158 | T4 |
| 46 | `config.rs` | 77.30% | 32 / 141 | T4 |
| 47 | `git/status.rs` | 77.92% | 17 / 77 | T4 |
| 48 | `app/menu_graph.rs` | 78.63% | 50 / 234 | T4 |
| 49 | `ui/info_modal/mod.rs` | 80.00% | 39 / 195 | T4 |
| 50 | `app/loading.rs` | 80.11% | 36 / 181 | T4 |
| 51 | `ui/results.rs` | 82.26% | 44 / 248 | T4 |
| 52 | `ui/dump_render/mod.rs` | 82.80% | 27 / 157 | T4 |
| 53 | `ui/commit_details.rs` | 83.09% | 23 / 136 | T4 |
| 54 | `view/graph/nav.rs` | 84.27% | 14 / 89 | T4 |
| 55 | `git/branch/remote.rs` | 85.25% | 27 / 183 | T4 |
| 56 | `git/graph/state.rs` | 85.56% | 13 / 90 | T4 |
| 57 | `git/merge_detection/mod.rs` | 85.71% | 18 / 126 | T4 |
| 58 | `view/tags.rs` | 85.71% | 7 / 49 | T4 |
| 59 | `ui/cells/mod.rs` | 85.90% | 33 / 234 | T4 |
| 60 | `git/cache/persist.rs` | 85.92% | 29 / 206 | T4 |
| 61 | `git/merge_detection/primary_code.rs` | 86.71% | 23 / 173 | T4 |
| 62 | `view/list_state/sort.rs` | 87.10% | 12 / 93 | T4 |
| 63 | `ui/graph_render/lanes.rs` | 87.14% | 9 / 70 | T4 |
| 64 | `git/worktree_delete/mod.rs` | 87.50% | 11 / 88 | T4 |
| 65 | `git/graph/model.rs` | 87.91% | 11 / 91 | T4 |
| 66 | `app/menu_worktree.rs` | 88.44% | 26 / 225 | T4 |
| 67 | `view/mod.rs` | 88.75% | 27 / 240 | T4 |
| 68 | `ui/modal.rs` | 89.19% | 20 / 185 | T4 |
| 69 | `ui/executing.rs` | 89.29% | 6 / 56 | T4 |
| 70 | `ui/confirm/lines.rs` | 89.70% | 24 / 233 | T4 |
| 71 | `app/primary_code.rs` | 90.00% | 8 / 80 | T5 |
| 72 | `git/commit_details.rs` | 90.05% | 20 / 201 | T5 |
| 73 | `git/cache/entries.rs` | 90.32% | 18 / 186 | T5 |
| 74 | `app/menu.rs` | 90.53% | 9 / 95 | T5 |
| 75 | `ui/help.rs` | 90.91% | 11 / 121 | T5 |
| 76 | `view/list_state/mod.rs` | 91.02% | 23 / 256 | T5 |
| 77 | `git/operations/squash_message.rs` | 91.92% | 8 / 99 | T5 |
| 78 | `git/cache/db.rs` | 92.03% | 11 / 138 | T5 |
| 79 | `types/format.rs` | 92.42% | 5 / 66 | T5 |
| 80 | `git/graph/patch.rs` | 92.93% | 13 / 184 | T5 |
| 81 | `ui/graph_render/mod.rs` | 92.96% | 19 / 270 | T5 |
| 82 | `git/graph/load.rs` | 93.05% | 21 / 302 | T5 |
| 83 | `git/worktree.rs` | 93.28% | 17 / 253 | T5 |
| 84 | `git/graph/squash.rs` | 93.30% | 15 / 224 | T5 |
| 85 | `ui/graph_render/ref_pane.rs` | 93.48% | 15 / 230 | T5 |
| 86 | `app/menu_remote_tag.rs` | 93.51% | 10 / 154 | T5 |
| 87 | `job_queue/queue.rs` | 93.97% | 14 / 232 | T5 |
| 88 | `git/graph/refs.rs` | 94.16% | 17 / 291 | T5 |
| 89 | `ui/render/status_text.rs` | 94.29% | 4 / 70 | T5 |
| 90 | `git/operations/commit.rs` | 94.47% | 11 / 199 | T5 |
| 91 | `git/merge_detection/squash.rs` | 94.67% | 8 / 150 | T5 |
| 92 | `dump.rs` | 94.79% | 10 / 192 | T5 |
| 93 | `ui/list_render/mod.rs` | 94.87% | 14 / 273 | T5 |
| 94 | `view/column/mod.rs` | 95.16% | 6 / 124 | T5 |
| 95 | `app/rows.rs` | 95.26% | 13 / 274 | T5 |
| 96 | `app/delete_preflight.rs` | 95.26% | 12 / 253 | T5 |
| 97 | `ui/job_status.rs` | 95.29% | 4 / 85 | T5 |
| 98 | `git/graph/cherry.rs` | 95.73% | 10 / 234 | T5 |
| 99 | `git/capability/mod.rs` | 95.91% | 7 / 171 | T5 |
| 100 | `ui/filter_ui.rs` | 96.15% | 5 / 130 | T5 |
| 101 | `git/cache/prune.rs` | 96.15% | 4 / 104 | T5 |
| 102 | `git/graph/backends.rs` | 96.64% | 8 / 238 | T5 |
| 103 | `git/fuzzy_match.rs` | 97.42% | 4 / 155 | T5 |
| 104 | `git/cache/mod.rs` | 97.44% | 1 / 39 | T5 |
| 105 | `git/graph/relationships.rs` | 97.70% | 4 / 174 | T5 |
| 106 | `view/list_state/nav.rs` | 97.89% | 2 / 95 | T5 |
| 107 | `view/worktrees.rs` | 97.96% | 1 / 49 | T5 |
| 108 | `app/construct.rs` | 98.05% | 3 / 154 | T5 |
| 109 | `ui/settings.rs` | 98.10% | 2 / 105 | T5 |
| 110 | `view/branches.rs` | 98.33% | 2 / 120 | T5 |
| 111 | `view/sort_keys.rs` | 98.44% | 3 / 192 | T5 |
| 112 | `theme/mod.rs` | 98.67% | 1 / 75 | T5 |
| 113 | `types/info.rs` | 98.68% | 1 / 76 | T5 |
| 114 | `app/menu_branch.rs` | 98.83% | 2 / 171 | T5 |
| 115 | `view/filter/mod.rs` | 98.92% | 2 / 185 | T5 |
| 116 | `ui/graph_render/row.rs` | 99.38% | 1 / 161 | T5 |
| 117 | `symbols.rs` | 99.52% | 1 / 208 | T5 |
