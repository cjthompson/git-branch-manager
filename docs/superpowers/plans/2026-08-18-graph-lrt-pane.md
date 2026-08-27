# Graph LRT Pseudo-pane Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render Graph ref metadata as the responsive, commit-aligned `LRT │ State │ Refs` pseudo-pane defined in the approved design.

**Architecture:** Keep `GraphSnapshot` as an owned worker payload. Simplify `GraphRef` to only the data rendered by Graph: type, name, tracking counts, and linked-worktree presence. The renderer aggregates direct refs per commit into fixed-width LRT and State cells plus a styled name list; dim branch-track labels remain context, not live refs.

**Tech Stack:** Rust, git2, Git CLI worktree discovery, Gleisbau, Ratatui, Crossterm, cargo test.

**Spec:** `docs/plans/2026-08-18-graph-lrt-pane-design.md`

## Global Constraints

- Preserve the existing dirty worktree. Do not stash, reset, discard, commit, push, or merge changes.
- Retain a bounded Graph load and the Gleisbau-first / Git-CLI-fallback strategy.
- Keep only owned application types in `GraphSnapshot`; do not send a `git2::Repository`, Gleisbau type, or worktree handle over `mpsc`.
- Remote refs stay opt-in, and `origin/HEAD` remains omitted.
- Preserve the shared vertical graph/ref cursor and scroll offset. No ref actions, filters, persistence, or horizontal scrolling are part of this plan.
- Use the active `SymbolSet`, and assert that every new Powerline marker is one terminal cell wide.

---

## File structure

- `src/git/graph.rs` owns the reduced Graph ref DTO and enriches local refs with linked-worktree presence while collecting the worker-safe snapshot.
- `src/symbols.rs` owns the one-cell remote and tag glyphs for each symbol set.
- `src/ui/graph_render.rs` owns right-pane sizing, fixed-cell composition, styles, and renderer tests.
- `tests/integration.rs` owns real-repository coverage for linked worktrees, matching remote refs, and tracking counts.
- `docs/plans/2026-08-17-git-branch-tab-implementation.md` remains the durable Graph handoff and must describe the shipped LRT renderer.

### Task 1: Reduce Graph ref metadata and add linked-worktree data

**Files:**

- Modify: `src/git/graph.rs:1-8`, `src/git/graph.rs:53-75`, `src/git/graph.rs:402-605`
- Modify: `tests/integration.rs:475-515`

**Interfaces:**

- Produces `GraphRef { name, kind, tracking, has_linked_worktree }`.
- Produces `GraphRefTracking { ahead, behind }`.
- Removes `GraphRefStatus`, `GraphRef::status`, and `GraphRefTracking::remote_name` from Graph-only code.

- [ ] **Step 1: Write failing loader tests**

  In `test_graph_refs_include_remote_tracking_state`, remove the assertion for `tracking.remote_name` but retain `(ahead, behind) == (1, 0)`. Add `test_graph_refs_mark_only_linked_worktrees` that creates `feature/linked`, runs `git worktree add .worktrees/linked feature/linked`, loads the Graph, and asserts that its local `GraphRef` has `has_linked_worktree == true` while `main` has `false`.

  Add a remote-enabled assertion that a local `ahead` ref and `origin/ahead` are both retained at their distinct commit tips:

  ```rust
  assert!(with_remotes.commits.iter().any(|commit| {
      commit.refs.iter().any(|reference| reference.name == "ahead")
  }));
  assert!(with_remotes.commits.iter().any(|commit| {
      commit.refs.iter().any(|reference| reference.name == "origin/ahead")
  }));
  ```

- [ ] **Step 2: Run the focused integration tests and confirm they fail**

  Run:

  ```text
  cargo test test_graph_refs_mark_only_linked_worktrees --test integration
  cargo test test_graph_refs_include_remote_tracking_state --test integration
  ```

  Expected: the new worktree assertion cannot compile until `GraphRef` carries the linked-worktree field.

- [ ] **Step 3: Simplify the DTO and loader**

  In `src/git/graph.rs`, remove the `MergeStatus` import, `GraphRefStatus`, `GraphRef::status`, `graph_local_status`, and `collect_remote_statuses`. Replace `GraphRefTracking` with:

  ```rust
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct GraphRefTracking {
      pub ahead: u32,
      pub behind: u32,
  }
  ```

  Add the owned boolean to `GraphRef`:

  ```rust
  pub has_linked_worktree: bool,
  ```

  At the beginning of `collect_ref_data`, derive one set from the existing worktree parser:

  ```rust
  let linked_worktree_branches = crate::git::worktree::list_worktrees(repo_path)
      .into_iter()
      .filter(|worktree| !worktree.is_main)
      .filter_map(|worktree| worktree.branch)
      .collect::<HashSet<_>>();
  ```

  Set the new field only on local refs with `linked_worktree_branches.contains(name)`; set it to `false` on remote and tag refs. Continue collecting remote refs even when the display option is off so local tracking counts work. When remotes are enabled, insert every non-HEAD remote ref at its own target OID. Do not invoke branch merge-status or remote-enrichment loaders from Graph.

- [ ] **Step 4: Update every Graph fixture**

  Add `has_linked_worktree: false` to all `GraphRef` literals in `src`, `tests`, and the Graph handoff examples. Remove now-invalid `status` fields and `remote_name` fixture fields.

- [ ] **Step 5: Re-run focused loader tests**

  Run:

  ```text
  cargo test test_graph_refs_mark_only_linked_worktrees --test integration
  cargo test test_graph_refs_include_remote_tracking_state --test integration
  cargo test test_load_graph_includes_remote_refs_only_when_requested --test integration
  ```

  Expected: all pass; local-only snapshots contain no remote `GraphRef`, and remote-enabled snapshots retain direct remote refs.

### Task 2: Add one-cell right-pane glyphs

**Files:**

- Modify: `src/symbols.rs:1-139`
- Test: `src/symbols.rs:141-222`

**Interfaces:**

- Produces `SymbolSet::graph_remote_ref` and `SymbolSet::graph_tag_ref`, each `&'static str`.
- Local LRT uses the pre-existing `SymbolSet::current_branch` field.

- [ ] **Step 1: Write failing symbol tests**

  Add a test that asserts these exact values and one-cell widths:

  ```rust
  let ascii = SymbolSet::ascii();
  let unicode = SymbolSet::unicode();
  let powerline = SymbolSet::powerline();
  assert_eq!((ascii.graph_remote_ref, ascii.graph_tag_ref), ("@", "#"));
  assert_eq!((unicode.graph_remote_ref, unicode.graph_tag_ref), ("☁", "⌑"));
  assert_eq!((powerline.graph_remote_ref, powerline.graph_tag_ref), ("\u{f0c2}", "\u{f02b}"));
  for marker in [powerline.graph_remote_ref, powerline.graph_tag_ref] {
      assert_eq!(ratatui::text::Span::raw(marker).width(), 1);
  }
  ```

- [ ] **Step 2: Run the symbol test and confirm it fails**

  Run:

  ```text
  cargo test graph_ref_markers_are_width_safe
  ```

  Expected: compilation fails because the two `SymbolSet` fields do not exist.

- [ ] **Step 3: Add the fields and values**

  Add `graph_remote_ref` and `graph_tag_ref` beside the other Graph-specific fields in `SymbolSet`, then populate all three constructors with the exact values asserted above. Do not change existing graph topology glyphs or `tracking_link`.

- [ ] **Step 4: Re-run the symbol test**

  Run:

  ```text
  cargo test graph_ref_markers_are_width_safe
  ```

  Expected: pass with the Powerline markers each measured as one cell.

### Task 3: Render the responsive LRT pseudo-pane

**Files:**

- Modify: `src/ui/graph_render.rs:92-420`
- Test: `src/ui/graph_render.rs:675-857`

**Interfaces:**

- Consumes `GraphRef { name, kind, tracking, has_linked_worktree }` from Task 1 and the glyph fields from Task 2.
- Produces private `ref_pane_width`, `ref_pane_spans`, and `ref_pane_state_spans` helpers returning `Vec<Span<'static>>`.

- [ ] **Step 1: Replace obsolete renderer tests with failing LRT tests**

  Remove `graph_ref_lane_comes_from_its_commit` and `graph_refs_are_ordered_and_show_tracking_and_merge_status`. Add tests using ASCII symbols for:

  ```rust
  assert!(rendered.contains("*@#"));
  assert!(rendered.contains("+2 WT"));
  assert!(!rendered.contains("m +"));
  assert!(!rendered.contains("u -"));
  ```

  Add one test for each state: `+9`, `+` for 10 ahead, `-9`, `-` for 10 behind, `RB`, `=`, and `WT` suffix. Add a matching local/remote test that checks both markers are set while `origin/main` is omitted from the name list only when `main` shares that same commit. Add a branch-track test that checks its `Refs` text is dim and its LRT/State prefix is blank.

  Add a `TestBackend` test at width 120 that finds `LRT │ State │ Refs`, proves the pane begins at one-third width, and proves a long summary cannot move the graph/pane divider. Add narrow-width tests for widths 8, 9, and 15, asserting markers-only, compact-state, and full layouts respectively.

- [ ] **Step 2: Run the focused renderer tests and confirm they fail**

  Run:

  ```text
  cargo test graph_ref_ --lib
  cargo test graph_places_refs_in_the_same_scrolling_row_as_the_commit --lib
  ```

  Expected: the old lane/status assertions fail or the new LRT helper is missing.

- [ ] **Step 3: Implement pane sizing and field composition**

  Replace the fixed `ref_column_width` body with the one-third calculation:

  ```rust
  fn ref_pane_width(content_width: u16) -> u16 {
      let available = content_width.saturating_sub(1);
      available.min((content_width / 3).max(3))
  }
  ```

  Use its result for the current `graph_width = content_width - ref_width - 1` calculation. Compose right-pane fields in these exact modes:

  ```text
  width 0..=8   LRT
  width 9..=14  LRT│State
  width >=15    LRT │ State │ Refs
  ```

  Keep `compose_row` responsible for the outer graph/pane divider and truncation. Use the existing active separator (`|` in ASCII, `│` otherwise) inside the pane; only the full-width layout surrounds internal separators with spaces.

- [ ] **Step 4: Implement ref aggregation and State**

  Replace `graph_ref_spans`, `ref_tracking_spans`, and `ref_status_spans` with a single `ref_pane_spans` helper. It must:

  1. Sort direct refs by `LocalBranch`, `RemoteBranch`, `Tag`, then name.
  2. Populate L, R, and T independently from all direct refs.
  3. Choose the alphabetically first tracked local ref for the upstream portion of State.
  4. Append `WT` if any direct local ref has `has_linked_worktree`.
  5. Render no merge-status text.
  6. Suppress a remote name only when a same-commit local ref has the same post-remote short name; retain it on all other remote rows.
  7. When there are no direct refs, render `GraphCommit::branch` in the Refs field without the old `L -` prefix and leave LRT/State blank.

  Style tracking values individually with `theme.ahead`, `theme.behind`, `theme.unmerged` for `RB`, and `theme.in_sync`; style `WT` with `theme.primary_text`; pass every span through `selected_style` when its row is selected. Keep names styled by `ref_style` so tag color remains `theme.squash_merged`.

- [ ] **Step 5: Re-run the focused renderer tests**

  Run:

  ```text
  cargo test graph_ref_ --lib
  cargo test graph_places_refs_in_the_same_scrolling_row_as_the_commit --lib
  cargo test powerline_ --lib
  ```

  Expected: all LRT, State, responsive-width, existing row-sync, and Powerline geometry tests pass.

### Task 4: Update the handoff and verify the integrated behavior

**Files:**

- Modify: `docs/plans/2026-08-17-git-branch-tab-implementation.md`
- Verify: `src/git/graph.rs`, `src/symbols.rs`, `src/ui/graph_render.rs`, `tests/integration.rs`

**Interfaces:**

- Consumes the completed owned snapshot and renderer behavior from Tasks 1 through 3.
- Produces an accurate future-agent handoff without changing user controls.

- [ ] **Step 1: Update the Graph handoff**

  Replace references to lane-number and merge-status ref decorations with the final `LRT │ State │ Refs` behavior, including the linked-worktree `WT` suffix, one-third responsive pane width, matching-remote treatment, and no horizontal scrolling. Keep its worker-boundary, bounded-history, remote-option, fallback, and symbol-width guidance.

- [ ] **Step 2: Run the full verification suite**

  Run:

  ```text
  cargo fmt --all -- --check
  cargo test
  cargo build
  cargo clippy --all-targets -- -D warnings
  git diff --check
  ```

  Expected: every command succeeds with no formatting, test, build, lint, or whitespace failures.

- [ ] **Step 3: Review the final diff without mutating Git state**

  Run:

  ```text
  git status --short
  git diff -- src/git/graph.rs src/symbols.rs src/ui/graph_render.rs tests/integration.rs docs/plans/2026-08-17-git-branch-tab-implementation.md
  ```

  Confirm that unrelated pre-existing changes are untouched, no commit was created, and both plan documents remain available for the next agent.

## Execution handoff

Execute Tasks 1 through 4 in order. Task 1 provides the DTO required by the renderer; Task 2 provides the glyphs; Task 3 consumes both; Task 4 is the documentation and full-suite gate. Do not begin the deferred squash-indicator or horizontal-scrolling work described in the design document.

