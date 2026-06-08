# View Dump Flags — Design

**Date:** 2026-06-07
**Status:** implemented on branch `view-dump-flags`
**Branch / worktree:** `view-dump-flags` (`.claude/worktrees/view-dump-flags`, based on `071378e`)

## Goal

Add non-interactive CLI flags that print the full, fully-enriched contents of
each TUI view to stdout — `--branches`, `--remotes`, `--tags`, `--worktrees` —
rendering the same columns and colors the corresponding tab would show, but only
after every background loader has completed.

This serves two purposes:

1. **Performance harness.** Each flag runs the view's whole synchronous load path
   and emits the existing `GBM_TIMING_LOG` spans, so a run is a deterministic,
   scriptable performance capture (no TUI, no pane, no screen-scraping).
2. **Correctness oracle.** The rendered rows are stable across runs, so a change
   can be proven behavior-preserving by diffing `--branches` output before and
   after — while timing is read separately from the timing log.

This is **piece 1** of the larger automation goal. **Piece 2** — the
make-change → test → document → loop harness that consumes these flags — is out
of scope for this spec (it lives in `~/.claude/plans/gbm-performance-workflow.md`).

## Non-goals

- No interactive-TUI automation (osascript / iTerm Python API). The dump flags
  replace the need to drive a pane for measurement.
- No new metrics or telemetry beyond the existing tracing spans.
- JSON output is **not implemented in v1**, but the architecture must make adding
  it a minimal, additive change — a new renderer over the same structured rows
  plus a `--format` flag, with **no change to the enrichment path or the data
  model**. See "Output model & renderer seam" and "JSON extensibility (v2)".
  TSV and other formats remain out of scope.

## CLI surface (`cli.rs`)

Add to the `Cli` struct (clap derive):

- `--branches`, `--remotes`, `--tags`, `--worktrees` — four `bool` flags. **At
  most one** may be set per invocation; setting two or more is a usage error
  (keeps per-view timing attribution unambiguous). Enforce via clap
  `conflicts_with_all` or a manual post-parse check.
- `--color <when>` — `auto | always | never`, default `auto`. Backed by a
  `ColorChoice` enum.
  - `auto` → emit ANSI color iff stdout is a TTY (`std::io::IsTerminal`).
  - `always` → always color (for a human piping into `less -R`).
  - `never` → never color (the loop uses this for stable, ANSI-free diffs).
- `--format` is intentionally **not** added in v1; it arrives with JSON in v2 and,
  when set to `json`, overrides `--color`.

**`--list` reconciliation.** `--list` becomes a deprecated alias of `--branches`
(its current ad-hoc 4-column output in `main.rs:43` is a strict subset of the new
faithful dump). Update `CLAUDE.md` and any integration test that asserts the old
`--list` output. A short deprecation note is printed to stderr when `--list` is
used.

## Architecture

### Dispatch

`main.rs` gains a single pre-TUI branch: if any dump flag is set, resolve the
repo + base branch (already done), call a new `dump` orchestrator for the chosen
view, print to stdout, and return — before any TUI/terminal setup. The existing
`if cli.list` block is replaced by this dispatch.

### Synchronous enrichment (`src/dump.rs`, new)

The TUI streams loader results through `mpsc` channels drained over many event
loop ticks. The dump path instead runs the **same loader functions to
completion, inline**, then merges their results into the same row types the TUI
holds. One function per view:

- **`--branches`** — the heavy path, and the primary perf target:
  1. `branch::list_branches_phase1` (branch list + tracking + ahead/behind +
     regular-merge detection).
  2. Squash-merge detection (`squash_loader` / `merge_detection::is_squash_merged`),
     run to completion rather than streamed.
  3. `github::fetch_open_prs`, merged into each `BranchInfo` by `headRefName`.
  This reproduces the post-drain state of `app.rs` (squash status + PR column
  folded into `BranchInfo`) without the channels.
- **`--worktrees`** — `worktree::list_worktrees` + `status_and_age` enrichment to
  completion.
- **`--remotes`** / **`--tags`** — their respective loaders to completion.

All of this runs under the same tracing spans, so `GBM_TIMING_LOG` captures are
identical in vocabulary to a real startup load.

### Output model & renderer seam

Enrichment does not render. Each per-view orchestrator returns a structured
`ViewDump` value:

```
struct ViewDump<'a, T> {
    base: Option<String>,          // Some(branch) for --branches, None otherwise
    rows: Vec<T>,                  // enriched, default-sorted row structs
    columns: &'a [ColumnDef<T>],   // the view's existing column defs
}
```

Rendering is a separate consumer of `ViewDump`, selected by an internal
`OutputFormat` enum (v1: only `Table`). v1 ships one renderer (the table writer
below); v2 adds a JSON renderer over the **same** `ViewDump`. This seam is what
keeps JSON additive — the enrichment path and `ViewDump` never change when a new
format is added.

> **As-built note:** the `ViewDump` struct and `OutputFormat` enum were not
> materialized as named types in the implementation. Instead, the seam was
> realized via `render_table`'s parameters (`base`, `rows`, `columns`,
> `render_row`) in `src/ui/dump_render.rs`, and each per-view arm in `src/dump.rs`
> passes those values directly. The data/presentation separation holds and JSON
> remains an additive renderer — a v2 `render_json` function would accept the same
> parameters without touching the enrichment path.

### Table renderer (v1) (`src/ui/dump_render.rs`, new)

The dump must not re-implement column layout, or it will drift from the TUI.
Reuse each view's existing `ColumnDef<T>` definitions and `RowRenderer`
(`view/*.rs`) — the same inputs `ui/list_render.rs` feeds to ratatui. The table
writer:

1. Sorts rows by the view's **default sort** (the `ColumnDef` compare fn the TUI
   defaults to) for deterministic, diffable output.
2. For each row, obtains the renderer's styled cells (the same `Span`/`Cell`
   values the TUI draws), and emits a fixed-width column line.
3. Translates each cell's ratatui `Style`/`Color` to ANSI SGR codes via the
   active `theme`, gated by `ColorChoice`. `Never` emits plain text; `Always` /
   `Auto`-on-TTY emit SGR.

A column-header row precedes the data rows. For `--branches`, a `base: <branch>`
line precedes the table (preserving the current `--list` header).

> **Open implementation detail (resolve in the plan, not here):** whether
> `RowRenderer` currently returns reusable styled cells or whether a small
> refactor is needed to expose them to both the ratatui path and the table
> writer. The principle is fixed: one source of truth for columns and colors.

### JSON extensibility (v2)

Adding JSON is intended to be a small, additive change with no impact on
enrichment:

1. Derive `serde::Serialize` on the row structs (`BranchInfo`, the worktree /
   remote / tag row types) — or a thin per-view DTO if a row struct carries
   fields that shouldn't serialize.
2. Add a `JsonRenderer` that serializes `ViewDump.rows` via `serde_json`.
3. Add `--format <table|json>` to the CLI (default `table`). `--format json`
   implies no color and ignores `--color`.

`dump.rs`, the `ViewDump` model, and the loaders are untouched. This is the
entire v2 change surface.

## Output format (example, `--color=never`)

```
base: main

  NAME          UPSTREAM            AB        MERGE      PR    AGE
* ct/alert-13   origin/ct/alert-13  +2101/-1  unmerged   #482  2d
  ct/zp-cli     (local)             -         unmerged   -     5d
  ct/old-feat   origin/ct/old-feat  -         squash✓    #310  3w
```

Columns, widths, and the leading `*` current-branch marker come from the existing
branches view definition. The other three views follow their own column defs.

## Error handling

- Multiple dump flags → clap usage error, non-zero exit.
- Empty result set → a short message to **stderr** (matching today's
  `eprintln!("No branches found.")`), exit 0.
- Loader failure (e.g. `gh` not installed for the PR column) → degrade
  gracefully: render the rows that did load, note the missing column's failure on
  stderr, exit 0. The dump should never panic the way a missing optional loader
  might block the TUI.

## Testing

Integration tests in `tests/integration.rs` using `setup_test_repo()`:

- Build known branch / worktree / tag states, run each dump flag with
  `--color=never`, and assert the exact stdout (stable because color is off and
  ordering is the default sort).
- A `--color=always` test asserting that SGR codes appear (a coarse check that
  styling is wired, not a full color assertion).
- A test that two dump flags together is a usage error.
- Update / replace the existing `--list` test to cover the alias behavior.

## Files touched

- `src/cli.rs` — four flags, `--color`, `ColorChoice`.
- `src/main.rs` — replace the `--list` block with dump dispatch.
- `src/dump.rs` *(new)* — per-view synchronous orchestration + the `ViewDump`
  model and `OutputFormat` seam.
- `src/ui/dump_render.rs` *(new)* — table renderer (v1); JSON renderer added in v2.
- `src/ui/mod.rs` — declare `dump_render`.
- possibly `src/view/*.rs` / `src/ui/list_render.rs` — minimal refactor to share
  styled-cell production (TBD in the plan).
- `tests/integration.rs` — dump tests.
- `CLAUDE.md` — document the new flags; note `--list` deprecation.

## Out of scope

Piece 2: the make-change → test → document → loop automation that drives these
flags, parses `GBM_TIMING_LOG`, updates the slow-function tracker, and iterates.
That builds on this feature once it ships.
