# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Test Commands

Cargo requires the Rust toolchain on PATH. Either export it or use the full path:

```sh
export PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH"
```

- **Build:** `cargo build`
- **Run:** `cargo run` (must be inside a git repo)
- **Test all:** `cargo test`
- **Single test:** `cargo test test_name`
- **Lint:** `cargo clippy`

## Architecture

Three-layer architecture:

1. **CLI layer** (`cli.rs`) -- clap-derived argument parsing. Single `--base` flag.
2. **Domain/git layer** (`git/`) -- all git operations. `branch.rs` lists branches and detects base branch via git2. `merge_detection.rs` handles regular merge detection (git2 `graph_descendant_of`) and squash-merge detection (shells out to git CLI). `operations.rs` performs deletes (local via git2, remote via `git push --delete`).
3. **Presentation/UI layer** (`ui/`) -- ratatui rendering. `render.rs` dispatches to view-specific modules (`branch_list.rs`, `confirm.rs`, `results.rs`, `help.rs`). `theme.rs` holds style constants.

`types.rs` defines shared types used across all layers: `BranchInfo`, `MergeStatus`, `TrackingStatus`, `BranchAction`, `OperationResult`.

`app.rs` owns all application state in a single `App` struct and runs the event loop. It dispatches key events per current `View` variant.

`main.rs` orchestrates startup: parse CLI, open repo via `git2::Repository::discover`, detect base branch, gather all branch data, then hand off to the TUI.

### Why git2 + git CLI

git2 handles branch listing, commit traversal, merge-base checks, and local branch deletion. Squash-merge detection requires `git commit-tree` and `git cherry`, which have no git2 equivalent -- these shell out to the git CLI. Remote branch deletion also shells out (`git push --delete`).

## Key Design Decisions

- **No async.** Git operations are fast enough synchronously. The TUI loop uses `crossterm::event::poll` with a 250ms timeout.
- **Data gathered before TUI starts.** All branch info (including squash-merge detection) is collected in `main.rs` before `ratatui::init()`. The TUI never calls git for data.
- **Single `App` struct owns all state.** View enum, cursor, selection, results -- all fields on `App`. No interior mutability or shared state.
- **Operations produce `OperationResult`, not errors.** Each delete returns success/failure per branch. The app continues through all selected branches and shows results at the end.
- **Terminal safety.** `ratatui::restore()` is called after `app.run()` returns. For panics, a panic hook should call `ratatui::restore()` to prevent leaving the terminal in raw mode.
- **Base branch auto-detection.** Checks `refs/remotes/origin/HEAD` first, then falls back to main/master/develop. `--base` flag overrides.
- **Current branch and base branch are protected.** They cannot be selected for deletion.

## Squash-Merge Detection Algorithm

For each branch not already detected as regularly merged:

1. `git merge-base <base> <branch>` -- find the common ancestor.
2. `git commit-tree <branch>^{tree} -p <ancestor> -m _` -- create a temporary commit representing "all branch changes squashed onto the ancestor."
3. `git cherry <base> <temp_commit>` -- check if equivalent content already exists in base.
4. A `-` prefix in the cherry output means the content is already in base (squash-merged). A `+` prefix means it is not.

This lives in `git/merge_detection.rs::is_squash_merged`.

## Phase 2 Planned Features

- **Switch/checkout:** Operates on the cursor position (not checked selection). Single branch only.
- **Archive:** Tag a branch before deleting it (e.g., `archive/<branch-name>`). Configurable tag prefix, default `archive/`.
