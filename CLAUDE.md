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

If `cargo` is not on PATH, prefix with: `export PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH"`

## Architecture

```
main.rs          CLI parsing → repo open → phase 1 → spawn squash checker → TUI loop
├── cli.rs       clap derive struct (--base, --list flags)
├── app.rs       App struct (all state), event loop, key dispatch per View
├── types.rs     Shared types used across all layers (in lib.rs for test access)
├── git/
│   ├── branch.rs           Branch listing (git2), base branch detection
│   ├── merge_detection.rs  Regular merge (git2) + squash-merge (git CLI)
│   ├── squash_loader.rs    Background thread for progressive squash-merge detection
│   └── operations.rs       Delete local (git2), delete remote (git CLI)
└── ui/
    ├── render.rs       Top-level draw dispatcher (matches on View enum)
    ├── branch_list.rs  Main screen: scrollable multi-select list
    ├── confirm.rs      Centered overlay for confirming destructive actions
    ├── results.rs      Post-operation results display
    ├── help.rs         Keybinding reference overlay
    └── theme.rs        Style constants (colors, modifiers)
```

**lib.rs** re-exports `git` and `types` modules so integration tests can import them. Binary-only modules (`app`, `cli`, `ui`, `git::squash_loader`) stay private.

### Data Flow

1. `main.rs` runs phase 1 (branch listing + regular merge detection via git2) synchronously, then spawns a background thread for squash-merge detection
2. The TUI starts immediately after phase 1 — squash-merge results arrive progressively via `mpsc::Receiver` and are applied on each event loop tick
3. `App` owns all state in a flat struct; `squash_rx: Option<Receiver<SquashResult>>` drains the background channel
4. Key events mutate `App` directly; rendering reads `&App` immutably
5. After operations execute, `refresh_branches()` re-runs phase 1 and spawns a new squash checker
6. The app loops back to the branch list after showing results (does not exit)

### git2 vs git CLI

- **git2**: branch listing, commit traversal, `graph_descendant_of` for merge detection, local branch deletion
- **git CLI** (via `std::process::Command`): squash-merge detection (`commit-tree` + `cherry` have no git2 equivalent), remote branch deletion (`git push --delete`)

### Squash-Merge Detection

The core differentiating feature. Located in `git/merge_detection.rs::is_squash_merged`. For each unmerged branch:

```
ancestor     = git merge-base <base> <branch>
temp_commit  = git commit-tree <branch>^{tree} -p <ancestor> -m _
result       = git cherry <base> <temp_commit>
```

A `-` prefix from `cherry` means the squashed content already exists in base. A `+` means it does not. This shells out because git2 doesn't expose `commit-tree` or `cherry`.

### View System

`app::View` enum drives which screen is rendered and which keys are active:
- `BranchList` → main interaction screen
- `Confirm { action }` → overlay on top of branch list
- `Results` → full-screen operation results, any key → refresh + back to BranchList
- `Help` → overlay on top of branch list

Overlays (`Confirm`, `Help`) render the branch list underneath, then draw a centered rect on top using `ratatui::widgets::Clear` + `Block`.

## Adding New Features

### Adding a new branch operation (e.g., archive, checkout)

1. Add variant to `BranchAction` in `types.rs`, implement `label()`
2. Add the git operation function in `git/operations.rs`
3. Add keybinding in `app.rs::handle_branch_list_key` that transitions to `View::Confirm { action: BranchAction::NewAction }`
4. Handle the new action in `app.rs::execute_action`
5. Update `ui/help.rs` keybinding list and `ui/branch_list.rs` status bar text

### Adding a new view/screen

1. Add variant to `View` enum in `app.rs`
2. Create `ui/new_view.rs`, add `pub fn draw(frame: &mut Frame, app: &App)`
3. Register in `ui/mod.rs` and add match arm in `ui/render.rs::draw`
4. Add `handle_new_view_key` method in `app.rs` and wire into `handle_event`

### Adding new branch metadata columns

1. Add field to `BranchInfo` in `types.rs`
2. Populate it in `git/branch.rs::list_branches`
3. Render it in `ui/branch_list.rs` by adding a `Span` to the line

### Adding CLI flags

1. Add field to `Cli` struct in `cli.rs` (clap derive)
2. Thread it through `main.rs` to wherever it's needed

## Testing

Integration tests in `tests/integration.rs` create temporary git repos with known branch states. Tests can run in parallel — all git CLI commands use explicit `current_dir()` rather than process-global `set_current_dir`.

To test a new git feature: use the `setup_test_repo()` helper to get a `(TempDir, Repository)`, then create the branch scenario with `std::process::Command` git calls.

## Phase 2 Planned

- **Switch/checkout**: operates on cursor position (not selection), single branch only
- **Archive**: `git tag <prefix>/<branch> <branch>` then delete; prompt for prefix, default `archive/`
