# Plan: squash-merge markers and optional merge arrows in Graph view

## Context

In the Graph view, a squash-merge landing commit on the base branch gets a `≈` marker
(`src/ui/graph_render/mod.rs:206-217`), but the source branch tip shows nothing in the graph
column. The only source hint is a `≈ <branch>` label appended to the Refs column text
(`src/ui/graph_render/ref_pane.rs:162-190`), which truncates when remote ref names fill the column.
Example: `4359b0c` in `~/dev/personal/claude-code-config` squash-merges the branch at `219897b`.
Cherry-picked tips already get `★` in the graph column (`mod.rs:218-231`), so no change there.

Goal: make confirmed squash merges visible at both ends, and let the user opt in to merge-style
arrows between them.

## Slices (separate branches, no PRs for this repo)

Each branch is cut from `origin/main` after `git fetch`, in its own worktree under
`.claude/worktrees/ct/<name>`, per the PR Cadence rule. They are independent: neither needs the
other to build or pass tests. Both edit `src/ui/graph_render/mod.rs`, so a textual conflict is
possible if both are merged; that is not a dependency.

› `ct/graph-squash-tip-marker` ← `origin/main`
› `ct/graph-squash-arrows-option` ← `origin/main`

### Slice A: tip marker (always on)

- Draw `symbols.graph_squash_commit` in the graph column on a branch tip that is a confirmed
  squash source (exact match or fuzzy match, reusing `GraphCommit::squash_match_confidence()` /
  `fuzzy_squash_match()`).
- Style with `theme.squash_merged`, same as the destination marker. Reuse the marker-position
  lookup already in `mod.rs:206-217` (first of `* ● o ○`).
- Keep the Refs-column label as is.

### Slice B: "Show merge arrows for squash commits" option (default off)

- Add a third row to `draw_graph_options` (`src/ui/graph_render/mod.rs:304`), checkbox style like
  "Include remote refs".
- Persist like `include_remotes`: new `Option<bool>` field in `src/config.rs`, save/load beside
  `save_include_remotes` (`src/app/settings.rs:81`), read in `src/app/construct.rs:57`.
- Overlay state: extend `Overlay::GraphOptions` (`src/ui/render/context.rs:95`); update key
  handling in `src/app/overlay_keys.rs:741-775` (cursor bound is `min(1)` today; "Load older
  history" is cursor-indexed in `loading.rs:93`, so keep it addressable).
- Rendering: when on, draw connectors from the source tip lane to the `≈` destination row using
  the same connector glyphs regular merges use (`graph_symbol` / `is_merge_connector` in
  `src/ui/graph_render/lanes.rs`). Apply in the render layer only; do not alter the gleisbau
  layout or `GraphLine.graph`. Where a lane cell is occupied, skip the arrow and keep the marker.
- This is a render-only toggle: no graph reload needed.

## Dependencies on logged tasks

- Arrows for remote-only branches need `#001` (remote tips as squash sources). Without it the
  option works for local branches only.
- `#002` (force delete for known-merged branches) is unrelated.

## Verification

- `cargo build` and `cargo test` (the project requires a build after every task).
- Slice A: render test in `src/ui/graph_render/tests/squash_refs.rs` asserting the tip row
  contains the squash glyph for exact and fuzzy matches and not for unmatched branches.
- Slice B: render tests with the option on and off; overlay test in
  `src/app/tests/startup_settings.rs` for the third row; config round-trip test in `config.rs`.
- Manual: `cargo run` in `~/dev/personal/claude-code-config` with remote refs on, toggle the new
  option, check `4359b0c` / `219897b`.

## Not in scope

- Cherry-pick arrows.
- The `includes_remotes` fast-path bug in `src/git/graph/load.rs:59-70` (unlogged).
- Version bump (local `-devN` edits only, never committed on feature branches).

## Open items

- Arrow routing across occupied lanes is unverified; confirm against real gleisbau output while
  implementing slice B.
