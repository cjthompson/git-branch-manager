# Tier 6 report — modal interaction regression and handoff

## Scope completed

- Added PageUp/PageDown dispatch for overflow in initial Confirm and Results
  overlays. The shared renderers already honor `ModalScroll`; this is the
  narrow input-routing fix exposed by the regression.
- Added one App transition regression covering delete preflight, cancel with
  Esc, arrows and `j/k` navigation, final single-target confirmation and
  enqueue, mixed Results expansion, compact/rerendered Results state, Tab
  action focus, and recovery for only the expanded result.
- Added a real temporary-Git linked-worktree scenario: one modified tracked
  file plus six untracked files, all preserved by enrichment; non-primary
  delete rejection remains recoverable while primary deletion remains
  rejected. The App-private dirty-worktree presentation/boundary remains
  covered by `dirty_worktree_warning_lists_at_most_five_modified_or_untracked_files`.

## TDD evidence

`page_keys_scroll_overflowing_confirm_and_results_bodies` was added before the
input change. Its first executable red run failed at the expected Confirm
PageDown assertion: the overlay was retained but its `body_scroll.offset` did
not advance. (An initial assertion about offscreen text was corrected first;
the actual rendered overflow is the selected choice's non-zero scroll offset.)

After the minimal dispatch addition, that focused test passes. The complete
App target then passes 63/63, including
`modal_keyboard_transition_preserves_single_target_recovery_across_resize`.

The real Git integration scenario initially proved an environmental path
representation difference only: Git reported the macOS physical
`/private/var/...` path while `tempfile` constructed `/var/...`. The final
assertion uses the repository's established linked-worktree suffix contract;
it still verifies the typed non-primary recovery and is independent of that
platform alias.

## Deterministic visual evidence and manual limitation

No literal human/manual TTY inspection was performed or claimed. The available
automated visual evidence is TestBackend rendering:

- `cargo test --test modal_shell` passes 18/18.
- `confirmation_dirty_warning_renders_five_files_and_exact_omitted_count`
  proves five displayed files and the exact remainder.
- `final_confirmation_repeats_exact_target_action_and_confirm_cancel_footer`
  proves final-review text and footer grammar.
- `accordion_details_show_typed_summary_before_raw_git_message` proves the
  single expanded Results detail ordering.
- `semantic_modal_styles_render_in_every_builtin_theme`,
  `all_overlays_keep_title_and_footer_chrome_at_terminal_extremes`, and
  `terminal_extremes_keep_focused_menu_confirm_and_results_rows_visible_without_footer_overlap`
  prove four built-in themes, semantic styles, footer/no-inline-prose rules,
  and 48x8/80x24/160x50 geometry from deterministic buffers.

The plan's literal “base selector” has no corresponding current overlay. The
closest executable fixture is `GraphOptions`; it is included in the all-overlay
TestBackend fixture rather than being misrepresented as a base-selector manual
flow.

## Verification

- `cargo test --bin git-branch-manager`: pass, 63 passed / 0 failed.
- `cargo test --test modal_shell`: pass, 18 passed / 0 failed.
- `cargo test --test integration`: the Tier 6 scenario passes; suite result is
  152 passed / 5 failed / 1 ignored. The five failures are the established
  cache/squash baseline:
  - `test_graph_patch_cache_branch_change_invalidates`
  - `test_graph_patch_cache_hit_avoids_recomputation`
  - `test_graph_patch_cache_populated_after_load`
  - `test_spawn_cache_verifier_applies_fix_and_persists`
  - `test_squash_scenario_18_branch_advances_after_cached_as_squash_merged`
- `cargo test -- --test-threads=1`: library 354/354 and App 63/63 pass;
  integration ends with that same 152 passed / 5 failed / 1 ignored baseline.
- `cargo clippy`: pass.
- `cargo build`: pass.
- `git diff --check`: pass.
- `cargo fmt --all -- --check`: exit 1 due inherited broad formatting drift in
  `src/git/cache.rs`, `cherry_loader.rs`, `fuzzy_match.rs`, `graph.rs`,
  `operations.rs`, `tags.rs`, `job_queue.rs`, `main.rs`, `symbols.rs`,
  `types.rs`, `ui/shared.rs`, `view/list_state.rs`, and pre-existing parts of
  `tests/integration.rs`. Neither `src/app.rs` nor the new integration hunk is
  reported. The formatting command was not allowed to leave unrelated changes.

No plan or task ledger was modified.

## Fix round 1

### Review repairs

- The initial Confirm and Results renderers used `ensure_visible` on every
  draw, which immediately undid a manual PageUp/PageDown. `ModalScroll` now
  keeps its last focus row privately: page movement changes only the offset;
  a changed choice/result/action focus re-establishes visibility. This is a
  narrow internal state change, with no public application API added.
- Final destructive Confirm no longer resets its scroll offset during render.
  It accepts PageUp/PageDown and renders paging in its footer. At widths below
  64 columns the compact grammar is `[y/Enter] Yes  [n/Esc] No  [PgUp/Dn]
  Scroll`; wider terminals retain the descriptive Confirm/Cancel labels and
  add `[PgUp/Dn] Page`.
- The full keyboard transition now drives recovery through the final Enter.
  A temporary real Git `feature/first` force-delete remains the queue's
  current job while `feature/unmerged` is one pending job. After the first
  completion, the test polls the real queue and asserts that the new current
  target is exactly `feature/unmerged`; no queue-introspection API was added.
- Added the missing real-Git-to-presentation chain in the private App test:
  a linked worktree has a modified `README.md` and six untracked files,
  `list_worktrees` plus `enrich_worktrees` provides the actual facts, and
  `delete_selected_branches` produces the rendered preflight. It asserts the
  five displayed paths and omitted count of two. The existing integration
  test continues to prove actual enrichment and non-primary recovery plus
  primary rejection.

### TDD and deterministic visual evidence

`page_keys_scroll_overflowing_confirm_and_results_bodies` first failed after
a Confirm redraw because `feature/choice-0` was still visible after PageDown.
The focused test passed after the focused-row preservation change.

`final_destructive_confirmation_pages_on_a_compact_terminal` first failed
because PageDown re-rendered the initial final-confirmation headline and the
old footer. After the narrow input/render repair it passes, as does the
modal-shell TestBackend regression
`compact_final_confirmation_keeps_paged_body_and_scroll_hint`: its 48x7
buffer omits the initial headline after paging, includes the data-loss line,
and includes the compact confirm/cancel/scroll footer.

The transition and real-Git presentation additions close coverage gaps rather
than requiring further production changes. The real-Git test's first fixture
lookup exposed macOS `/var` versus `/private/var` path spelling, so it
compares canonical worktree paths before feeding the unchanged facts into the
App.

No literal human/manual TTY inspection was performed or claimed. TestBackend
is deterministic visual evidence only. It cannot verify a human's perception
of semantic colors or a literal manual flow. The plan's literal base-selector
wording remains unsupported by a current overlay; GraphOptions is still the
closest automated fixture.

### Focused repair verification

- `cargo test --bin git-branch-manager`: pass, 65 passed / 0 failed.
- `cargo test --test modal_shell`: pass, 19 passed / 0 failed.
- `cargo test --test integration test_dirty_linked_worktree_reports_all_changes_and_remains_recoverable`:
  pass, 1 passed / 0 failed / 157 filtered; this remains the durable actual
  Git dirty-linked-worktree and recovery-policy test.
- `rustfmt --edition 2021 --check src/app.rs src/ui/confirm.rs src/ui/diagnostics.rs src/ui/help.rs src/ui/info_modal.rs src/ui/modal.rs src/ui/results.rs tests/modal_shell.rs`:
  pass.
- `git diff --check`: pass.

The repair did not rerun the broad inherited matrix because no non-test
behavior outside the scoped paging change was touched. The earlier full-matrix
record remains authoritative: workspace-wide `cargo fmt --all -- --check`
fails solely on listed inherited formatting drift, and the full integration
suite has the five cache/squash baseline failures. Neither baseline is hidden
or treated as a repair regression.

## Fix round 2

`ModalScroll` was introduced by unreleased P008 rather than inherited from the
main-base commit `8bbed7e`. It is therefore safe to finalize its first public
contract now: the type is `#[non_exhaustive]`, retains public `offset`,
`Default`, and the methods used by the binary crate, and documents that
external callers must construct it with `Default` plus those methods instead
of a struct literal. This preserves room for the private focused-row state
added in Fix round 1 without adding any queue or UI accessor.

`rg "ModalScroll\\s*\\{" src tests` found only the type definition; all
current construction sites use `ModalScroll::default()`. Validation: `cargo
test --test modal_shell` passed 19/19 (including its downstream-style
construction sites), and `git diff --check` passed. No broad matrix was
rerun for this documentation/representation-contract-only repair.

## Whole-branch review repair

### Scope and behavior

- Confirm and Results now construct their bodies before their footer, so the
  reusable width-aware footer contract can add Page only when the actual body
  overflows. At 48 columns, the compact grammar retains every control instead
  of truncating a trailing hint. Final destructive confirmation retains its
  concise `[y/Enter] Yes  [n/Esc] No  [PgUp/Dn] Scroll` grammar when paging is
  real, while a non-overflowing final review omits Page entirely.
- `ModalScroll` now records the prior body row count and viewport privately.
  A same-overlay resize or body-row-count change re-establishes focus, while
  PageUp/PageDown remains persistent when focus and geometry are unchanged.
  The existing P008 `#[non_exhaustive]` public contract is unchanged.
- Settings rows now use `ModalActionRow` styling and the compact
  `[j/k] Select  [Enter] Choose  [Esc] Close` footer. Enter has the same
  choosing effect as the existing forward cycle keys; arrows and direct cycle
  keys remain supported.
- The public unit `Overlay::Filter` remains as a compatibility display path.
  Normal App invocation now opens the new stateful `FilterSelection { cursor
  }` representation. It renders selectable action rows, accepts j/k and
  arrows plus Enter, retains Esc and direct filter-token/clear keys, and keeps
  a selected action in the compact viewport.

### TDD and deterministic visual evidence

- Slice A first failed at 48x8 because overflowing Confirm did not display
  `[PgUp/Dn]`; the focused TestBackend regression passes after the measured
  footer implementation. It also asserts Results retains j/k, Enter, Tab,
  Esc, and Page without clipping and that Page is absent when either body does
  not overflow.
- Slice B first failed after rendering the exact same Confirm state tall then
  compact: its unchanged focused row was offscreen. The focused TestBackend
  test now passes for both Confirm and Results.
- Slice C first compiled red because the required active `FilterSelection`
  state did not exist. After the narrow compatibility-preserving state and
  renderer addition, the TestBackend test proves Settings and Filter selected
  action-row backgrounds and compact standard footers. The App keyboard test
  drives j/k, arrows, and Enter for both overlays, asserts Filter selection
  toggles `merge:squash`, preserves the direct `m` toggle, and closes with
  Esc.

No literal human/manual TTY inspection was performed or claimed. The 48x8
TestBackend buffers are deterministic rendering evidence only; they do not
establish human perception or a literal manual/base-selector flow. No plan or
ledger was modified.

### Focused verification

- `cargo test --bin git-branch-manager`: pass, 66 passed / 0 failed.
- `cargo test --test modal_shell`: pass, 22 passed / 0 failed.
- `cargo test --lib filter_ui::tests`: pass, 4 passed / 0 failed / 350
  filtered.
- `rustfmt --check src/app.rs src/ui/modal.rs src/ui/confirm.rs
  src/ui/results.rs src/ui/settings.rs src/ui/filter_ui.rs src/ui/render.rs
  tests/modal_shell.rs`: pass.
- `git diff --check`: pass.

No broad baseline matrix was rerun for this scoped repair. The earlier report
records its inherited cache/squash integration and broad-formatting baselines;
they were not hidden or reclassified as this repair's results.
