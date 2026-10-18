# Plan: split every Rust file over 400 lines

## Context

27 `.rs` files under `src/` and `tests/` are over 400 lines, about 54k lines in total. The largest are `src/app.rs` (11234), `tests/integration.rs` (10011) and `src/git/graph.rs` (3597). These files are hard to navigate and review, and parallel branches keep conflicting in them. The goal is that **every** `.rs` file ends at ≤400 lines, test code included. That covers both the inline `#[cfg(test)]` modules and the `tests/*.rs` integration files.

User decisions:
- **Scope:** everything ≤400, test code included.
- **Delivery:** one refactor branch, with **one commit per source file split**. `cargo test` must pass after each commit.

Non-Rust files are out of scope. That covers `Cargo.lock`, `docs/**/*.md` and images.

This is a **pure move refactor**. Behavior, public paths and test names stay the same. The only signature changes allowed are visibility (`pub(super)` / `pub(crate)`) and extracting a helper method from an over-long function body.

## Ground rules (apply to every file)

1. **Module layout:** the repo uses only the `foo/mod.rs` style (`src/git/mod.rs`, `src/ui/mod.rs`, `src/view/mod.rs`), so convert each `foo.rs` to `foo/mod.rs` with `git mv` to keep the history. `mod.rs` holds every type with private fields that tests or children touch, plus the `mod` declarations and the `pub use` re-exports. That way every existing path (`crate::git::graph::X`, `git_branch_manager::...`) still resolves. Sibling modules cannot see each other's private items, but children can see their parent's.
2. **Privacy:** child modules can see their ancestors' private items. So `impl App` blocks in `src/app/*.rs` can use App's private fields with no change. A private method that moves into a child file and is called from a sibling or from tests becomes `pub(super)`. Existing `pub` items stay `pub`.
3. **Inline tests:** move them to `foo/tests.rs` if the result is ≤400 lines. Otherwise use `foo/tests/mod.rs` (shared helpers, `use super::*;`) plus `foo/tests/<group>.rs` children. Glob imports chain, so private helpers in `tests/mod.rs` are visible to the child files without `pub`. The parent declares the module with `#[cfg(test)] mod tests;`.
4. **Over-long single functions** (for example `App::drain_channels` at 810 lines and `App::handle_overlay_key` at 832 lines) get broken into `&mut self` helper methods, one per channel or overlay arm. The dispatcher keeps the original order and the `dirty` flag / `self.overlay.take()` semantics.
5. **Per-commit gate:** run `cargo build && cargo test && cargo clippy` and fix every new warning, unused imports especially. Then run the size check from the Verification section on the files touched.
6. **Test parity:** record the test count before and after each split. `cargo test 2>&1 | grep 'test result'` gives the counts per binary, and the total must never drop.
7. **Version:** follow CLAUDE.md. Use a local `-devN` bump only and never stage it.
8. **Imports:** after a move, trim each file's `use` list to what it actually uses, and let clippy/rustc confirm it.

## Setup

- Create a new worktree off `main`:
  `git worktree add ../.worktrees/split-large-files -b refactor/split-large-files main`.
  Do not reuse the current `graph-row-backgrounds` worktree, because it has uncommitted changes.
- Do the work while no feature branch touching `app.rs` or `git/graph.rs` is open, or rebase those branches immediately afterwards. A move refactor of this size causes conflicts with them.
- Capture the baseline: the total test count and the list of files over 400 lines.

## Order of work

Go from the smallest blast radius to the largest. That way the mechanics get proven on easy files before `app.rs`. Each file below is one task and one commit. The phase lead-ins are plain text, not step headings, so they do not create tasks.

**Phase A: files where moving the inline tests is enough.** Convert `foo.rs` to `foo/mod.rs`, and move the test module to `foo/tests.rs`. None of these test modules touch private items.

#### `src/ui/shared.rs`

190 prod / 300 test lines. Convert to `ui/shared/mod.rs` and move the test module to `ui/shared/tests.rs`.

#### `src/view/column.rs`

198 prod / 299 test lines. Convert to `view/column/mod.rs` and move the test module to `view/column/tests.rs`.

#### `src/ui/dump_render.rs`

227 prod / 243 test lines. Convert to `ui/dump_render/mod.rs` and move the test module to `ui/dump_render/tests.rs`.

#### `src/git/worktree_delete.rs`

212 prod / 243 test lines. Convert to `git/worktree_delete/mod.rs` and move the test module to `git/worktree_delete/tests.rs`.

#### `src/view/filter.rs`

235 prod / 290 test lines. Convert to `view/filter/mod.rs` and move the test module to `view/filter/tests.rs`.

#### `src/ui/cells.rs`

312 prod / 352 test lines. Convert to `ui/cells/mod.rs` and move the test module to `ui/cells/tests.rs`.

**Phase B: small and medium `ui/`, `view/` and `theme` splits.**

#### `src/theme.rs` (424 prod)

`theme/mod.rs`: struct, `dim_fg`/`accent_fg`/`from_name`/`next`, and the inline tests (about 175 lines). `theme/palettes.rs`: `impl Theme { dark, light, solarized, dracula }` (lines 55–395, about 345).

#### `src/ui/confirm.rs` (413, no tests)

`mod.rs`: types 14–79, `draw_confirm`, `confirm_footer`, `draw_confirm_cancel` (about 185). `lines.rs`: `initial_lines`, `push_risk_lines`, `final_lines` and the style helpers, 160–391.

#### `src/view/graph.rs` (446 + 240 tests)

`mod.rs`: 1–349. `nav.rs`: a second `impl GraphState` (`load_older_history` through `ensure_visible`, 350–427) plus `cursor_for_target` (`pub(super)`). `tests.rs`.

#### `src/ui/render.rs` (581 + 258 tests)

`context.rs`: `Overlay` and its impl, plus `RenderContext` (46–182), re-exported. `mod.rs`: `draw` (183–504). Extract the overlay dispatch (327–496) into `overlay.rs::draw_overlay` so `mod.rs` sits well under 400. `status_text.rs`: `default_status_text`, plus `branch_like_summary` and `format_branch_like` as `pub(super)` because the tests use them. `tests.rs`.

#### `src/ui/list_render.rs` (507 + 315 tests)

`params.rs`: `CellContext`, `RowRenderer`, `ListRenderParams`, re-exported. `ladder.rs`: `LadderColumn`, `FULLY_COMPACT_LEVEL`, `demoted_at_level`, `resolve_ladder_level`, `header_alignment`, `visible_column_indices` (71–162), all `pub(super)`. `mod.rs`: `render_list_view`, a 345-line function; extract the header-building stage (281–306) as a helper so it doesn't sit at about 370. `tests.rs`.

#### `src/view/list_state.rs` (588 + 619 tests)

The struct stays in `mod.rs` (`impl` 32–339), because its private fields (`selected`, `sort_column`, `table_state`) are used by the free fns and the tests. `nav.rs`: `nav_*`, `select_*`, `invert_selection` (340–465). `sort.rs`: `sort_items` through `collect_targets` (466–588). `mod.rs` does `pub use nav::*; pub use sort::*;`, because `app.rs` calls `list_state::nav_down` and friends. Tests go to `tests/{mod (helpers sample_branches, name_column), state, nav_select, sort}.rs`.

#### `src/ui/info_modal.rs` (771 + 551 tests)

`mod.rs`: types, keeping the private `InfoField` and `FieldSpan` because tests read `.label` and `.value`, plus `draw_info_modal`, `get_title` and the line layout 659–771 (about 310). `fields.rs`: `build_fields`, `short_oid`, `graph_commit_fields` and the `*_str` helpers. `ref_fields.rs`: `branch_fields`, `remote_fields`, `tag_fields`, `worktree_fields` (355–621). Tests: `tests/mod.rs` (helpers plus 814–1012) and `tests/fields.rs` (1013–1323).

Every public path that `app.rs` or the tests use must still resolve via `pub use`:
- `ui::render::{draw, Overlay, RenderContext}`
- `ui::list_render::{render_list_view, CellContext, RowRenderer, ListRenderParams}`
- `ui::info_modal::{draw_info_modal, InfoHitRegion, InfoModalFocus, InfoModalRow}`
- `ui::confirm::{ConfirmChoice, ConfirmStage, ConfirmTarget, DeletePreflight, DeleteRisk, draw_confirm, draw_confirm_cancel}`
- the `view::list_state` free fns (`nav_*`, `select_*`, `apply_sort`, `collect_targets`, …)
- `view::graph::{GraphState, GRAPH_PAGE_SIZE}`

**Phase C: `git/` files with no inline tests, plus `types.rs`.** The shared helpers stay in `mod.rs`, so the children can reach them without visibility changes.

#### `src/git/diagnostics.rs` (587)

`mod.rs`: consts, `AuditCtx`, `audit_cache`, `remote_branch_names`, `apply_fix`, `spawn_cache_verifier` (about 270). `workers.rs`: the `Squash/CherryCandidate` structs and `run_*_candidates`, all `pub(super)` including the struct fields. `verify.rs`: `local_branches`, `record_merge_status`, `verify_*`, `status_label`, `short_oid` (`pub(super)` where `mod.rs` calls them).

#### `src/git/merge_detection.rs` (796)

`mod.rs`: `ResolvedPrimaryRef` (its fields stay `pub`; `tests/primary_code_content.rs:153` constructs it), `check_cancel`, `lookup_primary_candidate`, `resolve_primary_ref_for_code_check`. `reachable.rs`: `BaseReachable`, the `build_reachable_*` fns, `detect_merged_branches`, `apply_merge_statuses`. `primary_code.rs`: `check_ref_code_in_primary`, `configure_scratch_merge_attributes`. `squash.rs`: `is_squash_merged`, `is_cherry_picked`, `merge_tree_confirms`, `compute_diff`, `likely_squash_merged`.

#### `src/git/branch.rs` (998)

`mod.rs`: `GitError`, `Result`, `detect_base_branch`, `list_branches*`, `get_commit_hash` (about 240). `remote.rs`: `list_remote_branches_phase1`, `spawn_remote_enricher` and their helpers. `merge_base.rs`: `MERGE_BASE_WALK_LIMIT`, `fill_merge_base_commits(_cached)`, `compute_merge_base_short`. `metadata.rs`: `collect_branch_metadata` (`pub(super)`).

#### `src/git/operations.rs` (1061)

`mod.rs`: `git_cmd`, `cancelled`, `run_with_progress`, `run_git_cancellable` and the error classifiers (1–160). `local.rs`: local delete, checkout, fetch, `delete_locals_with_progress`. `sync.rs`: fast-forward, pull, push, force-push, merge, rebase. `remote.rs`: remote checkout, delete, fetch, pull, merge, cherry-pick. `worktree_ops.rs`: create/remove worktree. Name it this because `worktree.rs` would clash with `use crate::git::worktree`.

#### `src/types.rs` (668 + 260 tests)

`mod.rs`: `MergeStatus`, `SquashConfidence`, `TrackingStatus`, `PrStatus`, `PrInfo`, `PrMap`, plus `pub use child::*` for every child, because code uses `use crate::types::*`. `action.rs`: `BranchAction`, `OperationResult`, `FailureCause`, `ProgressUpdate` and the `*Result` structs. `info.rs`: `BranchInfo`, `RemoteBranchInfo`, `WorktreeInfo`, `TagInfo`, `ChangedFile*`, `WorkingTreeStatus`, `PrimaryBranchCodeMatch`. `diagnostics.rs`: the `Diag*`/`CacheAudit` types. `format.rs`: the `format_age*` helpers. `tests.rs`.

**Phase D: large library files with tests.**

#### `src/git/capability.rs` (246 + 492 tests)

Production: no split. Tests: `tests/mod.rs` (`test_branch`, `linked_worktree`) plus `checkout_delete.rs`, `remote_sync.rs`, `integrate.rs`.

#### `src/git/cache.rs` (950 + 711 tests)

Production: `mod.rs`: `CacheRoot`, `CacheEntry`, `MergeBaseData`, `GraphPatchEntry`, `BranchCache`. `prune.rs`: retention and `prune_stale_caches*`. `persist.rs`: an `impl BranchCache` with `load*`, patch-id lookup/store, `save`. `entries.rs`: an `impl BranchCache` with the lookup/insert/hit/miss/clear methods. `db.rs`: SQLite helpers (`open_conn`, `ensure_schema`, `read_*`, `sidecar_path`, …) as `pub(super)`. Tests: `tests/mod.rs` (`DAY`, `temp_cache`, `git`, `set_mtime`) plus `paths_prune.rs`, `entries.rs`, `graph_patch.rs`.

#### `src/job_queue.rs` (1092 + 730 tests)

Production: `mod.rs`: the job types and the `ActionJobQueue` struct (1–164). `queue.rs`: the main `impl ActionJobQueue`. `test_hooks.rs`: the `*_for_test` impl, which is not `cfg(test)` because app tests use it. `cascade.rs`: `execute_branch_name_cascade` plus the extracted worktree-arm helpers. `execute.rs`: `execute_action_with_remote`. That function is **417 lines on its own**, so lift its three worktree match arms (935–1063) into helpers in `cascade.rs`. Tests: `tests/mod.rs` (`queue`, `job`, `inject`, `wait_until`, `run_git`) plus `queue.rs`, `execute_local.rs`, `execute_remote.rs`.

#### `src/git/graph.rs` (2366 + 1230 tests)

Production: `mod.rs`: `GraphSnapshot`, the enrichment message/cancel/observer types, `spawn_graph_updater` (about 125). `model.rs`: `GraphCommit`, `GraphRef*`, the relationship types. `state.rs`: `GraphRepositoryState/Delta`, `GraphLoadOptions`, `GraphLineStyle`, `GraphLoadError`. `load.rs`: `load_graph`, `update_graph_incrementally`, the spawn fns. `backends.rs`: gleisbau and git-CLI loaders. `relationships.rs`: `compute_relationships*`, `RelationshipAccumulator`, `apply_squash_enrichment`. `squash.rs`, `cherry.rs`, `patch.rs`, `refs.rs`. Roughly 15 items become `pub(super)`; the Explore agent's list covers them (`collect_ref_data`, `merge_relationships`, the `PatchJob` family with its fields, …). Tests: `tests/mod.rs` plus `fixtures.rs` (the shared native/merge pair fixtures), `confidence.rs`, `enrichment_cancel.rs`, `incremental.rs`, `loaders.rs`, `relationships.rs`, `cherry_scan.rs`, `merge_scan.rs`.

#### `src/ui/graph_render.rs` (899 + 1675 tests)

Production: `mod.rs`: `render_graph_view`, header, `render_graph_rows`, `draw_graph_options` (about 310). `row.rs`: compose/truncate/skip spans, widths, ref-name joining. `ref_pane.rs`: ref pane spans/state/style, including the `#[cfg(test)]` `ref_pane_spans`. `lanes.rs`: `graph_spans` through `short_oid`. The cross-cluster helpers become `pub(super)`. Tests: `tests/mod.rs` (imports, `fallback_state`, `render_commit_lines`, `fuzzy_commit`) plus `ref_state.rs`, `fuzzy.rs`, `ref_scroll.rs`, `squash_refs.rs`, `lanes.rs`, `current_branch.rs`, `head_state.rs`. Note that tests 903–1017 currently sit *above* the `use super::*` line.

The integration tests import `git::graph::{GraphCommit, GraphRef, GraphRefKind, load_graph_with_squash_annotations}` and `job_queue::{ActionJob, ActionJobQueue, JobStatusView}` by full path. Re-export every currently-`pub` item from its `mod.rs`.

**Phase E: `src/main.rs`.**

#### `src/main.rs` (587)

Declare all new modules in `main.rs`, not in `lib.rs`, because they are binary-only.
- `src/phase1.rs`: the phase-1 loader thread body (`main.rs` 87–281) as `spawn_phase1_loader(...) -> Receiver<app::Phase1Msg>`.
- `src/ahead_behind.rs`: `compute_ahead_behind` (403–587, about 185 lines), keeping `#[instrument]`.
- `src/logging.rs`: `init_debug_log` and `default_debug_log_path` (359–401).
- `src/terminal.rs`: terminal setup, run and restore around `app.run` (330–349).
- `main.rs` keeps CLI parsing, cache prune, repo/base discovery, dump dispatch and App wiring, about 130 lines.

**Phase F: `src/app.rs`.**

#### `src/app.rs` (11234: 5836 prod, 5398 tests)

Run `git mv src/app.rs src/app/mod.rs`, so that `mod app;` in `main.rs` still resolves. The hub `mod.rs` holds the imports, `Phase1Msg`, `RemoteFetchCompletion`, the `App` struct, the private `PrimaryCodeCheck*` structs and the private free fns (`branch_input_fingerprint`, `now_ms`, `copy_to_clipboard`, `drain_channel`). That is about 290 lines plus the `mod` lines. Re-export `pub(crate) use rows::{render_branch_row, render_remote_row, render_tag_row, render_worktree_row};`, because `src/dump.rs:26-29` imports them from `crate::app`. Also re-export the free helpers the tests use by name (`graph_affected_views`, `first_enabled_index`).

Do this in **one commit**. Port the clusters below in order inside the working tree, running the gate after each one. Only the final state is committed:

| Child file | Current lines | Content |
|---|---|---|
| `app/construct.rs` | 292–451 | `new` (`#[cfg(test)]`), `with_cache_root` |
| `app/event_loop.rs` | 452–550 | `run`, `build_render_context` |
| `app/drain.rs` | 571–1380 dispatcher | `drain_channels` calls the `drain_*` helpers below in the same order |
| `app/drain_graph.rs` | 574–790 | graph result/update, enrich, commit details, file diff |
| `app/drain_enrich.rs` | 791–1056 | phase-1, squash, cherry, remote-squash, remote/worktree enrich |
| `app/drain_loads.rs` | 1057–1380 | PR/tag/remote/worktree loads, op results, job queue, cache audit, progress, fetch, toast |
| `app/keys.rs` | 1381–1636 | `handle_event`, `handle_key`, `handle_key_inner`, `handle_common_list_key` |
| `app/keys_views.rs` | 1637–1783 | per-view key handlers, `jump_to_graph*` |
| `app/overlay_keys.rs` | 1786–2023 | `handle_overlay_key` dispatcher plus Help, Confirm, ConfirmCancelJob and Menu arms |
| `app/overlay_commit.rs` | 2024–2299 | CommitDetails and CommitDiff arms |
| `app/overlay_misc.rs` | 2300–2617 | InfoModal, Results/Executing/Settings/Filter/GraphOptions and Diagnostics arms |
| `app/keys_results_settings.rs` | 2618–2940 | results/recovery/settings/filter key handlers |
| `app/mouse_search.rs` | 2941–3200 | search and mouse |
| `app/menu.rs` | 3201–3310 | context menu core, info modal row |
| `app/menu_graph.rs` | 3311–3516 | graph menu, `refresh_open_graph_menu` |
| `app/menu_branch.rs` | 3517–3714 | branch menu |
| `app/menu_remote_tag.rs` | 3715–3901 | remote and tag menus |
| `app/menu_worktree.rs` | 3902–4114 | worktree menu, `execute_menu_action` |
| `app/primary_code.rs` | 553–570 and the `start_primary_code_check` part of 3902–4114 | primary-code check and cancel flag |
| `app/delete_preflight.rs` | 4115–4393 | delete selection, preflight, choices |
| `app/confirm.rs` | 4394–4587 | push/delete selected, `open_confirm*`, sorting |
| `app/loading.rs` | 4588–4802 | `ensure_view_loaded`, graph and other loader spawns |
| `app/refresh.rs` | 4803–5054 | refresh/fetch/clear-cache |
| `app/settings.rs` | 5055–5329 | diagnostics, config saves, sort/filter helpers, toast |
| `app/helpers.rs` | 5331–5488 | enabled-index helpers, confirmation-command helpers, `graph_affected_views` |
| `app/rows.rs` | 5490–5835 | `branch_prefix_style`, column helpers, `render_*_row` |

Overlay arm methods take the destructured overlay fields by value and return/re-set `self.overlay` exactly as the inline arms do today. Port each arm verbatim first, then lift it into a method. Diff-review the commit carefully.

**App tests (5837–11234):** move them to `app/tests/mod.rs`, which holds the shared helpers (about 345 lines: 5839–5911, 6057–6250, 8698–8731 and 9067–9109) with `use super::*;`. The test functions move into about 20 child files. Each one is ≤366 lines and starts with `use super::*;`:
`enrichment_sort`, `render_rows`, `startup_settings`, `merge_confidence`, `navigation`, `jump_to_graph`, `primary_code`, `commit_details`, `commit_details_nav`, `graph_action_refresh`, `graph_updates`, `delete_preflight`, `operation_results`, `modal_keys`, `recovery`, `force_confirm`, `info_modal_worktree_menu`, `graph_enrichment`, `graph_enrichment_publish`, `cancel_job`.
Some groups are non-contiguous today, and their ranges are combined. The test function names stay the same. Only their module path changes, from `app::tests::x` to `app::tests::<group>::x`, which matters for `cargo test <name>` filters.

**Phase G: integration test crates.** Two files, one task each.

**Layout (both files):** turn each file into a single-binary directory crate. `tests/integration.rs` becomes `tests/integration/main.rs` and `tests/modal_shell.rs` becomes `tests/modal_shell/main.rs`, each with `mod` submodules.
- The binary count and link time stay the same.
- The target names don't change, so `cargo test --test integration <name>` (cited in the doc comment at `integration.rs:32`) keeps working.
- Files under `tests/<name>/` aren't auto-discovered as extra binaries.
- `Cargo.toml` has no `[[test]]` entries, so nothing to update there.
- Delete the old `.rs` file in the same commit, because having both is a duplicate-target error.
- Shared helpers become `pub(crate)`, and each submodule does `use crate::common::*;`.
- `tests/primary_code_content.rs` (204 lines) and `tests/primary_ref.rs` (201 lines) stay as they are.

#### `tests/integration.rs` (10011 lines, 197 tests)

- `common.rs` (about 340 lines): the imports, `TestDir` with its impl and `Drop`, `manager_command`, `test_branches`, `setup_test_repo`, `run_git`, `git_output`, `snapshot_files`, `check_primary_code`, `run_git_with_env`, `git_blob_oid`, `setup_remote_test_repo` (2802–2827) and `paired_squash_fixture` (1130–1165).
- `test_dir.rs`: the `TestDir` self-tests (116–159).
- `graph/`: `lanes`, `labels` (together with `setup_graph_label_fixture`, which only these tests use), `refs`, `squash`, `squash_bounds`, `patch_cache`, `incremental`, `patch_parity`.
- `squash/`: `s01_04`, `s05_07`, `s08`, `likely`, `s09_11`, `s12_15`, `s16_19`, `s20`, `s21_enrichment`. Use the scenario-number grouping from the file's existing `S01`–`S21` sections.
- `capabilities/`: `mod.rs` holds the module-only helpers (2039–2120) and the tests from 2121–2367; `more.rs` takes 2368–2647.
- `cherry/`: `mod.rs` holds `rev` and `advance_main` plus 8875–9092; `sources.rs` takes 9819–10011.
- **Top-level files:** `branches`, `local_ops`, `remotes`, `remote_status`, `wt_status`, `merge_rebase`, `remote_ops`, `worktree_ops`, `worktree_progress`, `worktree_list`, `tags_direct`, `primary_code`, `primary_code_state`, `dump`, `cache_audit`, `worktree_merge_status`, `commit_details`.
- The Explore output has the line ranges for each group. Several groups are non-contiguous, so re-derive the ranges right before moving, with `grep -n '^fn \|^#\[test\]'`.

#### `tests/modal_shell.rs` (1865 lines, 27 tests)

- `support.rs`: imports, the `render_overlay*` helpers, `render_modal_semantic_sample`, and the cell/contrast assertions.
- `fixtures.rs`: `BaseOverlayFixture`, `base_overlay_fixtures`, `noop_*_row`.
- **Test files:** `overlays`, `confirm`, `details`, `action_rows`, `themes`, `extremes`.

## Critical files

- The hubs: `src/app/mod.rs` (moved from `app.rs`), `src/main.rs`, `src/git/mod.rs`, `src/ui/mod.rs`, `src/view/mod.rs`, `src/lib.rs`. `lib.rs` should not need changes, since its module list stays the same.
- `src/dump.rs` imports `crate::app::render_*_row`, so those re-exports must keep it compiling.
- `scripts/gen-arch-diagram.sh` and `docs/architecture/architecture.svg`: regenerate them once at the end so the diagram reflects the new modules.
- `CLAUDE.md`: update the Architecture section's file references (e.g. `git/merge_detection.rs::is_squash_merged`, `app.rs::execute_confirmed_action`, the `tests/integration.rs` mention under Testing).

## Verification

After each commit:
- Run `cargo build`, `cargo test` and `cargo clippy`. They must pass, with no new warnings.
- Compare test counts with the baseline (the sum of `test result: ok. N passed` lines, plus ignored). The total must stay the same. Test names gain module prefixes, which is expected, but the count must not change.
- Run the size check on the files that commit touched.

At the end:
1. Run `find src tests -name '*.rs' -exec wc -l {} + | awk '$1>400 && $2!="total"'`. It must print nothing.
2. Run `cargo test` in full, and compare the total with the baseline.
3. Run a smoke check of the binary with `cargo run -- --branches --color=never > /tmp/after.txt` (and likewise `--graph` if it exists, `--remotes`, `--tags`, `--worktrees`). Diff the output against the same commands run on `main` before the refactor. They must be identical.
4. Launch `cargo run` interactively. Cycle the tabs, open the menu, help, confirm, commit details and info modal overlays, and quit. This exercises the `handle_overlay_key` and `drain_channels` extraction.
5. Run `git diff main --stat`. The insertions and deletions should roughly balance, apart from the `mod` lines, re-exports and `pub(super)` markers.
6. Run `./scripts/gen-arch-diagram.sh`, and commit the regenerated diagram and the `CLAUDE.md` path updates as a final docs commit.
7. Follow CLAUDE.md for accepting the work: squash-merge to `main`, then make a separate patch version bump commit.
