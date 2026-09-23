# Why `42fe178` isn't detected as a squash of `ct/fix-tui-quit-hang`

## Context

In `~/dev/claude-monitor`, `42fe178` ("Squash merge of ct/fix-tui-quit-hang") landed on `main`, but
`gbm --branches` reports `ct/fix-tui-quit-hang` as `unmerged`.

Topology: branch forked at `aee23e3` (3 commits, tip `b4e9328`). `main` then gained 3 unrelated
commits (`7134888`, `c4469ed`, `1513eb6`) before the squash `42fe178` was made on top of `1513eb6`.
The squash was conflict-resolved, so its content is NOT byte-identical to the branch:

| file | branch (`aee23e3..b4e9328`) | squash (`42fe178^..42fe178`) |
|---|---|---|
| `claude_monitor/__init__.py`, `pyproject.toml` | `1.1.0 → 1.1.5` | `1.1.3 → 1.1.6` |
| `CHANGELOG.md` | adds `## 2026-09-22` header | header already present on main |
| `claude_monitor/app_base.py` | same lines, different context | (patch-id differs from context only) |

## Root cause, per detection tier (Branches view, `src/git/squash_loader.rs:158-200`)

1. **Exact (`is_squash_merged`, `src/git/merge_detection.rs:267`)** — `commit-tree` + `git cherry`
   compares whole-patch patch-ids. Version-bump lines + changelog header + shifted context make the
   patch-ids differ (`44e0639…` vs `6a846a2…`) → `+` → not merged. Expected; exact tier can't see this.
2. **Merge-tree (`merge_tree_confirms`, `:378`)** — `git merge-tree --write-tree main <branch>`
   conflicts on `CHANGELOG.md`, `__init__.py`, `pyproject.toml` → fails closed.
3. **Fuzzy (`likely_squash_merged`, `:469-472`)** — **this is the bug.** It scores
   `diff(merge_base, branch)` against `diff(merge_base, main)`, i.e. against the *cumulative* diff of
   all 4 main commits since the fork. The 3 unrelated commits dilute it:
   - branch tokens 355, base tokens 1854, intersection 353 → Jaccard **0.19** (< 0.75 threshold)
   - file overlap 12/33 = 0.36 → actually fails the 0.5 prefilter first, so `score()` returns `None`
   - Against the squash commit alone (`42fe178^..42fe178`) the same metric is **0.975** (350/359).

So the Branches-view fuzzy tier only works when the squash is the *only* thing on base since the
fork — any other activity on `main` pushes it below threshold.

## Root cause (Graph view — the view the question is about)

- Exact tier (`graph.rs:606-648`) is the only one that fills `possible_squash_merge_sources`, which is
  all `possible_squash_source_spans` (`graph_render.rs:622-643`) renders.
- Fuzzy tier (`graph.rs:650-672`) scores per-base-commit diffs — `42fe178` would score ~97% — but
  `FuzzySquashMatch` stores only `similarity_percent`, no branch name, and **no production code reads
  `fuzzy_squash_match`** (only test fixtures). Test
  `ref_pane_hides_possible_squash_sources_without_an_exact_match` pins that behavior.
- Patch cache keeps `diff_text` on hits (`graph.rs:956-960`), so reloads aren't the problem.
- Not verified live: that `ct/fix-tui-quit-hang` (checked out in a worktree) lands in
  `displayed_branch_names_by_tip` as `Diverged`; confirm in TUI during implementation.

Systemic note: this project's `-devN` version-bump workflow means a branch's version lines nearly
always conflict with `main` once `main` moves, so the exact tier will almost never fire for these
squashes — the fuzzy tier has to work.

## Proposed fix 1 (Graph view — primary)

- Unify the model: replace `is_possible_squash_merge` + `possible_squash_merge_sources` +
  `fuzzy_squash_match` on `GraphCommit` / `GraphEnrichmentUpdate` (`graph.rs:36-41,71-85`) with
  `squash_match_confidence: Option<SquashMatchConfidence { sources: Vec<String>, similarity_percent: u8 }>`.
- Producer (`compute_possible_squash_updates`): exact patch-id match → `similarity_percent: 100`;
  fuzzy → best-scoring tip's `source_names` (pair `branch_diffs` with names) + its percent,
  `fuzzy_match::classify` switches `.round()` → `.floor()`: since it already rejects `similarity >= 1.0`, fuzzy is always `<= 99`, so `100` means exact with no special case.
- Consumers read one field: node glyph (`graph_render.rs:193`), ref-pane spans
  (`possible_squash_source_spans`, one code path, percent suffix), info modal (`info_modal.rs:264`).
- Flip `ref_pane_hides_possible_squash_sources_without_an_exact_match`.
- Separate branch `ct/graph-fuzzy-squash-source` off `origin/main`.

## Proposed fix 2 (Branches view)

In `likely_squash_merged` (`src/git/merge_detection.rs:430`), after `merge_tree_confirms` fails,
score the branch diff against each first-parent commit on `base` in `merge_base..base`
(`git rev-list --first-parent --no-merges <ancestor>..<base>`), using each commit's own
`compute_diff(<c>^, <c>)`, and return the best `classify()` result. Keep the existing cumulative
comparison as the first attempt (covers the "squash is the only base commit" case cheaply).

- Bound the walk (e.g. cap at N=200 commits, and skip commits whose touched-file set fails the
  existing `passes_prefilter` cheaply — `score()` already does this).
- Reuse `compute_diff`, `fuzzy_match::score`, `fuzzy_match::classify`; no new scoring logic.
- `LikelySquashMerged` results are never cached (`cache.rs:467`), so no cache-format change.

- Separate branch `ct/fuzzy-squash-per-commit` off `origin/main` (disjoint from fix 1:
  `merge_detection.rs` + tests vs `graph.rs` + `graph_render.rs`).

## Tests (`tests/integration.rs`, alongside `test_squash_scenario_20*`)

Failing-first scenario: fork `feature` from base; add an unrelated large commit on base; then squash
`feature` onto base with a conflict-resolved version-bump line differing. Assert
`likely_squash_merged(...)` is `Some(FuzzyMatch{..})` (currently `None`). Plus a negative control:
unrelated base commits only, no squash → still `None`.

## Verification

- `cargo test` (new + existing scenario-20 tests), `cargo clippy`, `cargo build`.
- `cd ~/dev/claude-monitor && cargo run --manifest-path ~/dev/git-branch-manager/Cargo.toml -- --branches --color=never | grep quit-hang`
  → Merge column shows likely-squash (fuzzy ~97%) instead of `unmerged`.
- Graph view in TUI on claude-monitor: `42fe178` row shows `≈ ct/fix-tui-quit-hang ~97%`.
- Bump `Cargo.toml` `-devN` locally only; never stage or commit the version line (project CLAUDE.md).
