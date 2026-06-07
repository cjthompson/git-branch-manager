# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```sh
cargo build                          # build
cargo run                            # run TUI (must be inside a git repo)
cargo run -- --list                  # non-interactive branch list to stdout
cargo run -- --base develop          # override base branch
cargo test                           # run all tests
cargo test test_squash               # run a single test by name
cargo clippy                         # lint
```

**After completing any task, ALWAYS run `cargo build` to verify the code compiles.**

If `cargo` is not on PATH, prefix with: `export PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH"`

## Architecture

A rendered, auto-generated layered dependency diagram lives at
[`docs/architecture/architecture.svg`](docs/architecture/architecture.svg)
(regenerate with `./scripts/gen-arch-diagram.sh`). The layers, top → bottom,
with arrows meaning "depends on":

- **Entrypoint** (`main.rs`) — CLI parse, terminal setup, spawns the background
  loaders, runs the event loop.
- **App state machine** (`app.rs`) — the `App` struct holds all state; drives the
  event loop, per-view key dispatch, and draining of the background channels.
- **UI widgets** (`ui/`) — ratatui rendering: `render.rs` (frame dispatcher +
  `Overlay` enum), `list_render.rs` (generic table for every view), overlays
  (`confirm`, `executing`, `results`, `menu`, `help`, `settings`, `filter_ui`),
  bars (`status_bar`, `tab_bar`, `toast`), and `shared` helpers.
- **View models** (`view/`) — per-view column/sort/filter definitions: `ViewId`
  (Branches/Remotes/Tags/Worktrees), generic `ListState<T>`, `ColumnDef<T>`,
  `FilterSet`, and one def module per view.
- **Git backend** (`git/`) — `branch`, `merge_detection`, `squash_loader`,
  `operations`, `cache`, `pr_loader`, `github`, `worktree`, `tags`, `status`.
- **Config & presentation** — `config.rs` (TOML load/save), `theme.rs`,
  `symbols.rs`, `cli.rs` (clap derive: `--base`, `--list`, `--symbols`).
- **Domain types** (`types.rs`) — shared models (`BranchInfo`, `MergeStatus`,
  `BranchAction`, `OperationResult`, …); the dependency sink.

**lib.rs** re-exports `cli, config, git, symbols, theme, types, ui, view` so
integration tests can import them. `app` is the only binary-only module
(declared `mod app;` in `main.rs`).

### Data Flow

The TUI launches immediately; all heavy git work is backgrounded and streamed
into `App` via `mpsc` channels drained each event-loop tick.

1. `main.rs` spawns a phase-1 thread that sends staged `app::Phase1Msg`s in
   order: `Fast` (branch list + tracking, no merge detection) → `AheadBehind` →
   `MergeBaseCommits` → `MergeStatuses` (regular-merge detection). Squash-merge
   detection then streams in separately via `git::squash_loader` (`SquashResult`s).
2. Separate threads load remote branches and (optionally, per config) worktrees.
3. `App` owns all state in a flat struct, holding an `Option<Receiver<_>>` per
   stream; `git::cache` persists merge-base/squash results to speed reloads.
4. Key/mouse events mutate `App` directly; rendering reads it immutably each frame.
5. A confirmed action runs on a background thread via `execute_confirmed_action`,
   surfaced through the `Overlay::Executing` progress overlay, then
   `Overlay::Results`; the relevant view's data is refreshed afterward.
6. The app loops (does not exit after an operation).

### git2 vs git CLI

- **git2**: branch listing, commit traversal, `graph_descendant_of` for merge detection, local branch deletion
- **git CLI** (via `std::process::Command`): squash-merge detection (`commit-tree` + `cherry` have no git2 equivalent), remote operations (`push --delete`, fetch/pull), merge/rebase, worktree management
- **`gh` CLI**: PR status (`git::github` / `git::pr_loader`)

### Squash-Merge Detection

The core differentiating feature. Located in `git/merge_detection.rs::is_squash_merged`. For each unmerged branch:

```
ancestor     = git merge-base <base> <branch>
temp_commit  = git commit-tree <branch>^{tree} -p <ancestor> -m _
result       = git cherry <base> <temp_commit>
```

A `-` prefix from `cherry` means the squashed content already exists in base. A `+` means it does not. This shells out because git2 doesn't expose `commit-tree` or `cherry`.

### View / Overlay System

Two enums drive the screen:

- **`view::ViewId`** — the active primary view: `Branches`, `Remotes`, `Tags`,
  `Worktrees`, cycled with `next()`/`prev()` (the tab bar). `App.active_view`
  selects which `ListState<T>` and per-view key handler are live.
- **`ui::render::Overlay`** — an optional overlay drawn on top: `Help`, `Menu`,
  `Confirm`, `Executing`, `Results`, `Settings`, `Filter`.

When `App.overlay` is `Some`, `handle_overlay_key` consumes keys; otherwise the
per-view handler (`handle_branches_key`, `handle_remotes_key`, …) runs after
`handle_common_list_key`. Overlays render the active list underneath, then draw
a centered rect on top using `ratatui::widgets::Clear` + `Block`.

## Adding New Features

### Adding a new branch operation (e.g., archive)

1. Add a variant to `BranchAction` in `types.rs`, implement `label()`
2. Add the git operation function in `git/operations.rs` (return `OperationResult`)
3. In the relevant per-view key handler in `app.rs` (e.g. `handle_branches_key`),
   open `Overlay::Confirm { action: BranchAction::NewAction, targets }`
4. Map the new `BranchAction` to your operation in `app.rs::execute_confirmed_action`
5. Update `ui/help.rs` and the status-bar text in `ui/status_bar.rs`

### Adding a new view/tab

1. Add a variant to `ViewId` in `view/mod.rs` and wire it into `next()`/`prev()`
2. Add a `view/<name>.rs` def (columns via `ColumnDef`, a `RowRenderer`) and a
   `ListState<T>` field on `App`
3. Render it through `ui/list_render.rs`; add the tab label in `ui/tab_bar.rs`
4. Add a `handle_<name>_key` method in `app.rs` and dispatch on `active_view`

### Adding a new metadata column

1. Add the field to the row type in `types.rs` (e.g. `BranchInfo`) and populate
   it in the corresponding `git/` loader
2. Add a `ColumnDef` (name, widths, compare fn) in that view's `view/*.rs`
3. Emit its cell in the view's `RowRenderer` (rendered by `ui/list_render.rs`)

### Adding CLI flags

1. Add field to `Cli` struct in `cli.rs` (clap derive)
2. Thread it through `main.rs` to wherever it's needed

## Testing

Integration tests in `tests/integration.rs` create temporary git repos with known branch states. Tests can run in parallel — all git CLI commands use explicit `current_dir()` rather than process-global `set_current_dir`.

To test a new git feature: use the `setup_test_repo()` helper to get a `(TempDir, Repository)`, then create the branch scenario with `std::process::Command` git calls.

## Not Yet Implemented

Checkout, worktree management, and remote/tag operations have all landed. Still
outstanding:

- **Archive**: `git tag <prefix>/<branch> <branch>` then delete; prompt for
  prefix, default `archive/`. (Tag listing/deletion already exists in
  `git/tags.rs` + the Tags view; this is the branch→archive-tag operation.)
