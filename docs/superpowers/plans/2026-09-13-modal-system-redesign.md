# Modal System Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the independently rendered overlays with one reusable modal shell and two consistent content layouts—Confirmation and Results—so every modal has predictable chrome, keyboard hints, scrolling, and theme semantics.

**Architecture:** `ui::modal` will own the shared frame: title, fixed footer, scrollable body viewport, standard key-hint rendering, and semantic modal styles. Existing overlays will become consumers of that shell. `Overlay::Confirm` will become a structured decision flow with safe and destructive paths; `Overlay::Results` will become a per-result accordion with one expanded result and per-branch recovery actions.

**Tech Stack:** Rust, Ratatui, Crossterm, existing `Theme`, `Overlay`, `OperationResult`, `FailureCause`, and Ratatui `TestBackend` rendering tests.

**Spec:** User-approved modal redesign, 2026-09-13. The review mockup is `work/tui-modal-options.html`; this plan carries the durable behavioral and visual requirements so implementation does not depend on that untracked artifact.

## Global Constraints

- Keep exactly one modal grammar: shared shell, fixed title/footer, and a body viewport that alone scrolls when the terminal is short.
- The footer is the only place for generic navigation and dismissal instructions. Body shortcut keys are allowed only in reusable, selectable action rows; remove prose such as `(press r to remove worktree + delete)`.
- Use `Theme` semantic styles only in renderers. Add semantic theme fields where the current palette cannot distinguish a branch, worktree path, commit hash, command, result state, or destructive warning; do not introduce renderer-local `Color` values.
- Preserve keyboard accelerators while making the selected action and `Enter` behavior explicit. `Esc` always means cancel or close and is advertised only in the footer.
- Confirmation and Results state must be per branch/result. Never batch-recover every matching failed row from one `!` or `r` keystroke.
- A destructive recovery path must show its own final confirmation before starting a force delete or forced worktree removal. The primary worktree remains non-removable.
- For a dirty linked worktree, show a red warning surface and at most five modified or untracked paths, each with its status; show a count for remaining matching files. Do not list staged-only files in that warning.
- At constrained heights, retain the title and footer, scroll the body to keep the focused row visible, then compact whitespace and secondary copy before hiding essential target, warning, or action information. Show page-navigation hints only when the body overflows.
- Preserve the current compact terminal presentation where it is needed for narrow dimensions; this is terminal geometry handling, not a separate web-responsive visual system.
- Retain all eleven current `Overlay` variants: Help, Menu, InfoModal, Confirm, Executing, Results, Settings, Filter, GraphOptions, Diagnostics, and DiagnosticsReport.
- Each tier is independently testable and committed only after its focused tests, `cargo fmt --all -- --check`, `cargo clippy`, `cargo test`, `cargo build`, and `git diff --check` pass for its completed scope.

---

## File and Layout Map

| File | Responsibility after this plan |
| --- | --- |
| `src/ui/modal.rs` | New reusable shell, footer/key-hint model, body viewport, scroll state, standard action row renderer, and modal rendering tests. |
| `src/ui/mod.rs` | Exposes the new modal module. |
| `src/theme.rs` | Defines modal-only semantic styles, including contrasting destructive warning surface styles for all built-in themes. |
| `src/ui/shared.rs` | Retains generic geometry/text helpers; moves only shared modal helpers to `ui::modal` when that avoids parallel modal APIs. |
| `src/ui/{help,menu,info_modal,executing,settings,filter_ui,diagnostics}.rs`, `src/ui/graph_render.rs` | Use the base shell and standard footer/action-row grammar. |
| `src/ui/confirm.rs` | Implements the Confirmation layout and structured preflight/risk rendering. |
| `src/ui/results.rs` | Implements the Results accordion and typed, expandable result details. |
| `src/ui/render.rs` | Holds the additional overlay state needed by confirmation and results layouts and dispatches their renderers. |
| `src/app.rs` | Owns transitions, selection, scrolling, focus, final destructive confirmation, and per-result recovery dispatch. |
| `src/types.rs` | Continues to provide `ChangedFile`, `ChangedFileKind`, `FailureCause`, and `OperationResult`; only change it if a reusable non-UI typed result field proves necessary. |
| `tests/modal_shell.rs` | New integration-style `TestBackend` coverage for shell invariants and all overlay mappings. |

## Shared Interaction Contract

The shell exposes three body forms without creating another family of modal widgets:

| Consumer | Body form | Footer contract |
| --- | --- | --- |
| Help, InfoModal, Executing, DiagnosticsReport | Base/read-only body; optional body scroll | `[j/k] Scroll` and `[PgUp/PgDn] Page` only on overflow; `[Esc] Close` or `[Esc] Cancel` for executing work. |
| Menu, Settings, Filter, GraphOptions, Diagnostics | Base/selectable action list | `[j/k] Select [Enter] Choose [Esc] Cancel/Close`; direct row accelerators remain valid. |
| Confirm | Confirmation choice list, then final destructive confirmation when required | `[j/k] Select [Enter] Choose [Esc] Cancel`; a row may show its accelerator, command, and description. |
| Results | Result list with a single expanded accordion and optional nested action focus | Results focus: `[j/k] Select [Enter] Expand [Tab] Actions [Esc] Close`; action focus: `[j/k] Select [Enter] Choose [Tab] Results [Esc] Close`. |

The reusable action row has this visual structure, with styles selected from `Theme`:

```text
▸ [r] Review forced removal
      Opens the final data-loss confirmation.
```

The shell footer—not the row—owns generic `j/k`, `Enter`, `Tab`, paging, and `Esc` language.

### Task 1: Tier 1 — Create reusable modal foundations

**Files:**
- Create: `src/ui/modal.rs`
- Modify: `src/ui/mod.rs`
- Modify: `src/theme.rs`
- Modify: `src/ui/shared.rs`
- Test: `tests/modal_shell.rs`

**Interfaces:**
- Consumes: `Theme`, Ratatui `Frame`, `Rect`, `Line`, `Span`, `Block`, and the existing `centered_rect` geometry helper.
- Produces: `ModalSpec`, `ModalAreas`, `ModalScroll`, `ModalFooter`, `ModalActionRow`, and `draw_modal_shell`; later renderers use these instead of allocating a `Block`, border, footer, and body rectangle themselves.

- [ ] **Step 1: Write failing shell tests before creating consumers.**

  Create `tests/modal_shell.rs` with a `TestBackend` render of a deliberately short terminal. Import the planned public API and assert that the title renders at the top border, the footer occupies the final inner row, and body content cannot overwrite it:

  ```rust
  use git_branch_manager::ui::modal::{draw_modal_shell, ModalFooter, ModalSpec};
  use ratatui::{backend::TestBackend, widgets::Paragraph, Terminal};

  #[test]
  fn shell_pins_footer_when_body_is_taller_than_viewport() {
      let mut terminal = Terminal::new(TestBackend::new(40, 8)).unwrap();
      terminal.draw(|frame| {
          let areas = draw_modal_shell(
              frame,
              &ModalSpec::new("Example", ModalFooter::hints(&[("Esc", "Close")]), 30, 8),
              &Theme::dark(),
          );
          frame.render_widget(Paragraph::new("one\ntwo\nthree\nfour\nfive\nsix"), areas.body);
      }).unwrap();
      let buffer = terminal.backend().buffer();
      assert!(buffer.content.iter().any(|cell| cell.symbol() == "E"));
      assert!(buffer.content.iter().any(|cell| cell.symbol() == "C"));
  }
  ```

- [ ] **Step 2: Run the new test and confirm it fails for the missing shell API.**

  Run: `cargo test --test modal_shell shell_pins_footer_when_body_is_taller_than_viewport`

  Expected: compile failure naming `git_branch_manager::ui::modal` or its missing public items.

- [ ] **Step 3: Implement the minimal, reusable shell.**

  Create `src/ui/modal.rs` with these stable boundaries:

  ```rust
  pub struct ModalSpec<'a> {
      pub title: Line<'a>,
      pub footer: ModalFooter<'a>,
      pub preferred_width: u16,
      pub max_height: u16,
  }

  pub struct ModalAreas {
      pub outer: Rect,
      pub body: Rect,
      pub footer: Rect,
  }

  #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
  pub struct ModalScroll { pub offset: u16 }

  pub fn draw_modal_shell(
      frame: &mut Frame,
      spec: &ModalSpec<'_>,
      theme: &Theme,
  ) -> ModalAreas;
  ```

  `draw_modal_shell` must clear the centered outer rectangle, render the themed panel/title, reserve and render its fixed footer before returning the body viewport, and never let `body.height` include the footer row. Add `ModalScroll::clamp` and `ModalScroll::ensure_visible` so each consumer can keep its selected row in the returned viewport without duplicating offset arithmetic. Define a reusable action-row renderer that composes selected state, optional accelerator, command, and secondary description.

  Extend `Theme` with named modal semantics, including a readable destructive warning foreground/background pair for dark, light, solarized, and dracula. Move or wrap the current `key_hint` behavior so all modal key/hint rendering has one implementation.

- [ ] **Step 4: Add focused shell tests.**

  Add `scroll_clamps_and_keeps_selected_row_visible` with a ten-row body, a three-row viewport, and selection index nine; assert that the resulting offset exposes row nine and does not exceed seven. Add `action_row_composes_key_command_and_description`; render the `r` row and assert its cells contain `[r]`, `Review forced removal`, and `Opens the final data-loss confirmation.` in their corresponding key, command, and secondary styles. Add `built_in_themes_have_a_readable_modal_warning_surface`; for each `Theme::dark`, `Theme::light`, `Theme::solarized`, and `Theme::dracula`, assert the warning style has both foreground and background colors. Assert rendered cell contents/styles rather than internal line counts alone.

- [ ] **Step 5: Verify and commit the foundation only.**

  Run: `cargo fmt --all -- --check && cargo test --test modal_shell && cargo clippy && cargo test && cargo build && git diff --check`

  Commit only the five Tier 1 files with: `feat(ui): add reusable modal shell`.

### Task 2: Tier 2 — Migrate every base modal to the shared shell

**Files:**
- Modify: `src/ui/help.rs`
- Modify: `src/ui/menu.rs`
- Modify: `src/ui/info_modal.rs`
- Modify: `src/ui/executing.rs`
- Modify: `src/ui/settings.rs`
- Modify: `src/ui/filter_ui.rs`
- Modify: `src/ui/diagnostics.rs`
- Modify: `src/ui/graph_render.rs`
- Modify: `src/ui/render.rs`
- Modify: `src/app.rs`
- Test: `tests/modal_shell.rs`

**Interfaces:**
- Consumes: Tier 1 `ModalSpec`, `ModalAreas`, `ModalScroll`, and action-row/footer helpers.
- Produces: all non-confirmation/non-results overlays rendered through the base shell; the existing overlay-specific state remains authoritative for settings, filters, diagnostics, and info details.

- [ ] **Step 1: Add failing mapping and footer tests.**

  Add one table-driven `TestBackend` test per base overlay. Its expected footer must use the shared grammar and its output must contain no parenthetical inline instruction such as `press r`:

  ```rust
  #[test]
  fn every_base_overlay_uses_the_shared_shell_and_footer() {
      for overlay in base_overlay_fixtures() {
          let rendered = render_overlay(overlay, 80, 20);
          assert!(rendered.contains("Esc"));
          assert!(!rendered.contains("(press "));
      }
  }
  ```

  `base_overlay_fixtures()` must return exactly Help, Menu, InfoModal, Executing, Settings, Filter, GraphOptions, Diagnostics, and DiagnosticsReport; `render_overlay` must build a `RenderContext` with the fixture and return the `TestBackend` buffer as text.

- [ ] **Step 2: Run the base-overlay tests and record current failures.**

  Run: `cargo test --test modal_shell every_base_overlay_uses_the_shared_shell_and_footer`

  Expected: failure until each legacy renderer delegates its panel/body/footer geometry to `ui::modal`.

- [ ] **Step 3: Convert read-only base modals.**

  Convert Help, InfoModal, Executing, and DiagnosticsReport to render body content only in the shell viewport. Keep existing information/action focus semantics in InfoModal, keep `Esc` cancellation for Executing, and use `ModalScroll` for overflow. DiagnosticsReport must retain its audit header content in the body scroll stream so the shell has the sole fixed header/footer contract.

- [ ] **Step 4: Convert selectable base modals.**

  Convert Menu, Settings, Filter, GraphOptions, and Diagnostics to reusable action rows and the standard selection footer. Preserve their existing direct key behavior, but render row keys through the action-row helper; convert bare prose instructions such as `Space toggle Enter apply Esc cancel` into the standardized footer hints.

- [ ] **Step 5: Update input routing and overflow behavior.**

  Update `App::handle_overlay_key` only where required to support consistent `j/k`, arrow, page, and `Esc` behavior. Keep active selection visible after keyboard navigation. Do not change the operation performed by a Menu, Filter, Settings, GraphOptions, or Diagnostics selection.

- [ ] **Step 6: Verify and commit the base migration.**

  Run: `cargo fmt --all -- --check && cargo test --test modal_shell && cargo test ui:: && cargo clippy && cargo test && cargo build && git diff --check`

  Commit with: `refactor(ui): render base overlays through modal shell`.

### Task 3: Tier 3 — Build the Confirmation decision layout

**Files:**
- Modify: `src/ui/confirm.rs`
- Modify: `src/ui/render.rs`
- Modify: `src/app.rs`
- Modify: `src/theme.rs`
- Test: `tests/modal_shell.rs`
- Test: `src/app.rs`

**Interfaces:**
- Consumes: Tier 1 shell/action rows and existing `BranchAction`, `WorktreeInfo`, `ChangedFile`, and `ChangedFileKind` data.
- Produces: `DeletePreflight`, `DeleteRisk`, confirmation choice cursor/scroll state, and a two-stage destructive recovery flow.

- [ ] **Step 1: Add failing confirmation behavior tests.**

  Add four focused app tests. `safe_delete_choice_starts_only_the_selected_branch_action` opens a preflight for `feature/safe`, selects its `y` action, presses `Enter`, and asserts that the queue action is `DeleteLocal` with exactly `feature/safe`. `force_choice_opens_final_confirmation_before_enqueuing` opens `feature/unmerged`, selects its `r` review action, presses `Enter`, and asserts that the overlay is the final confirmation while the queue is empty. `dirty_worktree_warning_lists_at_most_five_modified_or_untracked_files` builds one staged-only entry, four modified entries, and three untracked entries; it asserts that six non-staged file names are visible neither all at once nor staged-only, that five listed names appear, and that the remainder count is two. `confirmation_body_scroll_keeps_selected_choice_visible` uses enough risks to exceed the body viewport, moves to the final action row, and asserts the body offset makes that row visible.

- [ ] **Step 2: Run the confirmation tests and confirm the legacy string/extra-key model cannot satisfy them.**

  Run: `cargo test confirmation_ && cargo test dirty_worktree_warning_`

  Expected: failures because `Overlay::Confirm` currently carries a formatted `reason: Option<String>` and immediate `extra_keys` action swaps.

- [ ] **Step 3: Replace formatted preflight prose with structured confirmation data.**

  Define UI-facing data in `src/ui/confirm.rs` and store it in `Overlay::Confirm` instead of a prejoined reason string:

  ```rust
  pub struct DeletePreflight { pub risks: Vec<DeleteRisk> }

  pub enum DeleteRisk {
      UniqueCommits { branch: String, base: String },
      CheckedOut { branch: String, worktree: PathBuf, is_main: bool },
      DirtyWorktree { worktree: PathBuf, files: Vec<ChangedFile>, omitted: usize },
  }
  ```

  Build these values in `App::build_delete_preflight`. Filter `WorkingTreeStatus::changed_files` to `Modified` and `Untracked`, retain the first five in their current stable status order, and set `omitted` to the remainder. Continue to use the already typed `FailureCause::CheckedOutInWorktree` to protect primary worktrees.

- [ ] **Step 4: Render the two confirmation stages through the shell.**

  The initial layout is a choice list. A safe operation is the selected default and can start when chosen; a recovery row has an accelerator and descriptive command text, but choosing it opens a final destructive confirmation rather than enqueuing immediately. Render target branch names, worktree paths, commands, hashes when available, and statuses with their named `Theme` styles. Render `DirtyWorktree` as a high-contrast red warning surface containing the bounded file list and `+N more` count.

  The final destructive stage repeats the exact target/action, makes the data-loss consequence explicit, and accepts only its confirm/cancel keys. It must not offer a batch target list or silently change targets.

- [ ] **Step 5: Implement confirmation navigation and final-action dispatch.**

  Add selected choice and body-scroll state to the `Overlay::Confirm` variant. Make `j/k` and arrows move choices, direct row keys select the matching choice, and `Enter` execute the selected safe action or advance to the destructive confirmation. Only the final confirmation may enqueue force-delete or force-remove worktree jobs.

- [ ] **Step 6: Verify and commit the confirmation layout.**

  Run: `cargo fmt --all -- --check && cargo test confirmation_ && cargo test dirty_worktree_warning_ && cargo test && cargo clippy && cargo build && git diff --check`

  Commit with: `feat(ui): add structured confirmation modal`.

### Task 4: Tier 4 — Build the per-result accordion and recovery flow

**Files:**
- Modify: `src/ui/results.rs`
- Modify: `src/ui/render.rs`
- Modify: `src/app.rs`
- Test: `tests/modal_shell.rs`
- Test: `src/app.rs`

**Interfaces:**
- Consumes: Tier 1 shell/action rows, Tier 3 final-confirmation entry point, and typed `OperationResult`/`FailureCause`.
- Produces: results selection state, one-open accordion state, nested action focus, and selected-result recovery routing.

- [ ] **Step 1: Add failing Results tests.**

  Add four tests using one success, one `NotMerged` failure, one non-primary `CheckedOutInWorktree` failure, and one `Other` failure. `results_expands_only_one_selected_branch_at_a_time` expands the `NotMerged` row, then the worktree row, and asserts `expanded_index` changes from the first row to the second. `results_recovery_targets_only_the_expanded_result` enters the `NotMerged` action row and asserts the final confirmation names only that branch. `worktree_recovery_opens_final_confirmation_before_job_start` activates the worktree action and asserts a final confirmation exists while the job queue remains empty. `accordion_details_show_typed_summary_before_raw_git_message` asserts the rendered typed summary precedes the raw Git error text in the expanded body.

- [ ] **Step 2: Run those tests and confirm the current global `!`/`r` behavior fails.**

  Run: `cargo test results_ && cargo test worktree_recovery_`

  Expected: failures because the current Results handler gathers every matching failure and immediately starts a batch job.

- [ ] **Step 3: Add Results state and render the accordion.**

  Replace `Overlay::Results { results }` with fields for `selected_index`, `expanded_index: Option<usize>`, `focus`, and body scroll. Use separate selection and expansion state; opening one row closes the previously expanded row. Collapsed rows show status, branch, and a concise typed failure summary. Expanded rows show the recoverable cause, the relevant worktree path/action, raw Git details behind a `View raw Git details` action, and any recovery action rows.

- [ ] **Step 4: Route recovery through the selected row only.**

  In Results focus, `Enter` toggles the selected row and `Tab` enters its expanded action list. In Action focus, `Enter` activates the selected row. For `NotMerged`, `Review force deletion` opens Tier 3's final destructive confirmation for only that branch. For a non-primary `CheckedOutInWorktree`, `Review worktree removal` opens that same final confirmation for only that branch. For a primary worktree or unclassified failure, omit recovery but retain raw details. Remove the global Results `!` and `r` batch paths.

- [ ] **Step 5: Verify render, keyboard, and recovery tests.**

  Run: `cargo fmt --all -- --check && cargo test results_ && cargo test worktree_recovery_ && cargo test --test modal_shell && cargo test && cargo clippy && cargo build && git diff --check`

  Commit with: `feat(ui): add per-result recovery accordion`.

### Task 5: Tier 5 — Complete semantic polish and full-overlay audit

**Files:**
- Modify: `src/theme.rs`
- Modify: `src/ui/modal.rs`
- Modify: every changed overlay renderer from Tiers 2–4 as required by the audit
- Test: `tests/modal_shell.rs`

**Interfaces:**
- Consumes: all completed layouts and `Theme` modal semantics.
- Produces: verified visual consistency across dark, light, solarized, and dracula without renderer-local palette choices.

- [ ] **Step 1: Add a theme matrix test for all semantic modal tokens.**

  Render a representative base action row, confirmation warning/file list, and results status/detail row in all four built-in themes. Assert each semantic style has the foreground/background fields required by its use and that a selected row preserves its background when key spans are patched onto it.

- [ ] **Step 2: Audit all eleven overlay render paths.**

  Search for `Block::default`, `centered_rect`, `key_hint`, `Press `, and `press ` under `src/ui`. For each remaining overlay, either route it through `draw_modal_shell` or document why it is a non-modal surface. No `Overlay` variant may keep an independently composed modal footer.

- [ ] **Step 3: Fix only audit findings.**

  Replace remaining raw color/style construction in modal renderers with the named theme semantics. Keep text roles distinct: branch, worktree, commit hash, command, success, failure, warning, and secondary details. Do not alter table/list rendering outside overlays.

- [ ] **Step 4: Perform terminal-size regression renders.**

  Use `TestBackend` at narrow/short, normal, and wide/tall sizes. Assert title/footer persistence, focused-row visibility, no footer/body overlap, no line containing a legacy inline key instruction, and no panic for Unicode target names.

- [ ] **Step 5: Verify and commit the audit fixes.**

  Run: `cargo fmt --all -- --check && cargo test --test modal_shell && cargo test && cargo clippy && cargo build && git diff --check`

  Commit with: `refactor(ui): unify modal visual language`.

### Task 6: Tier 6 — End-to-end interaction regression and handoff

**Files:**
- Modify: `src/app.rs` only if end-to-end tests expose an interaction defect
- Modify: `tests/modal_shell.rs`
- Modify: `tests/integration.rs` only for a real Git/worktree scenario not representable with `OperationResult` fixtures
- Modify: `docs/superpowers/plans/2026-09-13-modal-system-redesign.md` to record any approved scope correction before completion

**Interfaces:**
- Consumes: all previous tiers.
- Produces: evidence that the modal system is usable from actual overlay state transitions and does not weaken destructive-operation safeguards.

- [ ] **Step 1: Add app-level keyboard transition tests.**

  Cover the full sequence: open delete preflight → choose safe path or recovery review → final destructive confirmation → enqueue exactly one target → mixed results → expand one result → recover only that selected branch. Cover `Esc`, `Tab`, arrows, `j/k`, page keys on overflow, and a terminal resize/re-render of the same state.

- [ ] **Step 2: Add an integration scenario for a dirty linked worktree.**

  Use the existing temporary Git repository test helpers to create a linked worktree containing more than five modified/untracked files. Verify the presented preflight data, bounded file count, omitted count, non-primary recoverability, and primary-worktree rejection.

- [ ] **Step 3: Run the full verification matrix.**

  Run: `cargo fmt --all -- --check && cargo test --test modal_shell && cargo test --test integration && cargo test -- --test-threads=1 && cargo clippy && cargo build && git diff --check`

  Expected: every command succeeds with no new warnings and no unrelated working-tree changes staged.

- [ ] **Step 4: Manually verify the agreed visual flows.**

  In each built-in theme, inspect: a normal confirmation; a unique-commit force review; a dirty-worktree warning with five files plus remainder; mixed Results with a single expanded accordion; a base selector; and a read-only scrolling modal. Confirm the footer grammar, semantic colors, compact short-terminal behavior, and absence of inline `press key` prose.

- [ ] **Step 5: Commit and prepare the plan closeout.**

  Commit any Tier 6 test-only or defect-fix files with: `test(ui): cover modal system interactions`. Update the linked project-plan task notes with the exact verification evidence; do not mark the plan complete until all child tiers are terminal.

## Coverage Review

| Approved requirement | Planned tier |
| --- | --- |
| Reusable modal base is the first independently deliverable slice | Tier 1 |
| Every current overlay maps to the base shell, Confirmation, or Results | Tiers 2–5 |
| Standard footer and no inline key prose | Tiers 1–2, audit in Tier 5 |
| Choice-list confirmation with safe/recovery paths | Tier 3 |
| Forced path gets final data-loss confirmation | Tiers 3–4 |
| Dirty-worktree red warning and maximum five modified/untracked paths | Tier 3, integration proof in Tier 6 |
| Per-branch Results accordion, one open at a time | Tier 4 |
| Per-result, never batch, recovery | Tier 4 |
| Fixed header/footer and scrollable/compact short body | Tier 1, migrated in Tiers 2–4, regression in Tier 5 |
| Theme-only semantic coloring across dark and light themes | Tier 1, audit in Tier 5 |
