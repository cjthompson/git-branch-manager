# git-branch-manager: Git Branch Manager TUI — Implementation Plan

## Context

Managing local git branches is tedious, especially when using GitHub's "squash and merge" which makes branches appear unmerged to git. This tool provides an interactive TUI to view, select, and batch-operate on local branches — with squash-merge detection as its core differentiator.

## Design Decisions

- **Binary**: `git-branch-manager` (invocable as `git branch-manager` as a subcommand)
- **Language**: Rust, latest stable edition 2024
- **TUI**: ratatui + crossterm (full alternate-screen rendering)
- **Git interface**: `git2` (libgit2 bindings) for branch listing, age, tracking status, merge detection. Shell out to `git` CLI for squash-merge detection (commit-tree + cherry have no git2 equivalent)
- **Repo scope**: Current directory only (must be inside a git repo)
- **Base branch**: Auto-detect from `refs/remotes/origin/HEAD`, fallback chain main→master→develop, `--base` flag override
- **No async runtime** — git commands are fast, `crossterm::event::poll` with timeout for TUI loop

## Squash-Merge Detection Algorithm

The critical feature. For each branch not already detected as merged via git2:

```sh
ancestor=$(git merge-base <base> <branch>)
temp=$(git commit-tree $(git rev-parse <branch>^{tree}) -p $ancestor -m _)
result=$(git cherry <base> $temp)
# "-" prefix = squash-merged, "+" prefix = unmerged
```

This works by creating a temporary commit that represents "what if all branch changes were squashed" and checking if equivalent content exists in the base branch. These specific commands (commit-tree, cherry) require shelling out since git2 doesn't expose them directly.

## File Structure

```
Cargo.toml
src/
  main.rs              Entry point: parse CLI, validate repo, gather data, run TUI
  cli.rs               clap derive struct (--base flag)
  app.rs               App state, event loop, key dispatch per view
  types.rs             Shared types: BranchInfo, MergeStatus, TrackingStatus, BranchAction, OperationResult
  git/
    mod.rs             Re-exports
    repo.rs            git2 Repository wrapper, open/validate repo
    branch.rs          List branches via git2, detect base branch, tracking status, dates
    merge_detection.rs Regular merge detection (git2) + squash-merge detection (shell out)
    operations.rs      Delete local (git2), delete remote (shell out to git push --delete)
  ui/
    mod.rs             Re-exports
    render.rs          Top-level draw fn dispatching to current view
    branch_list.rs     Main screen: scrollable multi-select list with status columns
    confirm.rs         Centered overlay: "Delete N branches? [y/n]"
    results.rs         Post-operation results table
    help.rs            Keybinding reference overlay
    theme.rs           Style constants (colors for merged/squash-merged/unmerged/error)
tests/
  integration.rs       Creates temp git repos, tests squash-merge detection end-to-end
```

## Key Data Types

```rust
enum MergeStatus { Merged, SquashMerged, Unmerged }

enum TrackingStatus {
    Tracked { remote_ref: String, gone: bool },
    Local,
}

struct BranchInfo {
    name: String,
    is_current: bool,
    is_base: bool,
    tracking: TrackingStatus,
    last_commit_date: DateTime<Utc>,
    merge_status: MergeStatus,
}

enum View { BranchList, Confirm { action: BranchAction }, Results, Help }

enum BranchAction { DeleteLocal, DeleteLocalAndRemote }

struct App {
    base_branch: String,
    branches: Vec<BranchInfo>,
    view: View,
    cursor: usize,
    selected: Vec<bool>,
    list_scroll_offset: usize,
    results: Vec<OperationResult>,
    should_exit: bool,
}
```

## Application Flow

1. **Startup**: Parse CLI → validate git repo (git2) → detect base branch → gather branch data (all queries happen before TUI starts)
2. **Branch List**: User sees all local branches with name, tracking, age, merge status. Keyboard-driven selection.
3. **Select**: SPACE=toggle, a=all (except base/current), n=none, m=merged+squash-merged, i=invert
4. **Action**: d=delete local, D=delete local+remote
5. **Confirm**: Overlay shows action + affected branches, y/n
6. **Execute**: Run git operations, collect results per branch
7. **Results**: Show OK/FAIL per operation, any key to exit

## Keybindings (Branch List View)

| Key | Action |
|-----|--------|
| j/↓ | Move cursor down |
| k/↑ | Move cursor up |
| SPACE | Toggle selection |
| a | Select all (except base/current) |
| n | Deselect all |
| m | Select merged + squash-merged |
| i | Invert selection |
| d | Delete local (selected) |
| D | Delete local + remote (selected) |
| ? | Show help |
| q/Esc | Quit |

## Dependencies

```toml
[dependencies]
ratatui = "0.30"
crossterm = "0.29"
git2 = "0.20"
clap = { version = "4", features = ["derive"] }
anyhow = "1"
thiserror = "2"
chrono = { version = "0.4", features = ["clock"] }

[dev-dependencies]
tempfile = "3"
```

## Implementation Order

### Step 1: Project scaffold + types
- `cargo init --name git-branch-manager`
- Cargo.toml with all dependencies
- `src/types.rs` — all shared data types
- `src/cli.rs` — clap argument struct
- `src/main.rs` — skeleton that parses args and prints "hello"
- Verify: `cargo build` succeeds

### Step 2: Git layer (git2 + shell)
- `src/git/repo.rs` — open repo via git2, validate
- `src/git/branch.rs` — list branches via git2 API (names, tracking, dates), detect base branch
- `src/git/merge_detection.rs` — regular merge via git2 merge_base + graph_descendant_of, squash-merge via shelling out (commit-tree + cherry)
- `src/git/mod.rs` — re-exports
- Unit tests for parsing and detection logic
- Verify: `cargo test` passes, can run binary in a git repo and print branch data to stdout

### Step 3: Basic TUI with branch list
- `src/ui/theme.rs` — style constants
- `src/ui/branch_list.rs` — render branch list with columns
- `src/ui/render.rs` — dispatch to branch_list
- `src/ui/mod.rs` — re-exports
- `src/app.rs` — App struct, event loop, cursor movement, scrolling
- Wire into main.rs: gather data → run TUI
- Verify: TUI launches, shows branches, scrolls, q quits

### Step 4: Selection + quick-select
- SPACE toggle, a/n/m/i quick-selects in app.rs event handler
- Update branch_list.rs to render [x]/[ ] checkboxes
- Status bar showing selection count
- Verify: can select/deselect branches interactively

### Step 5: Operations + confirmation + results
- `src/ui/confirm.rs` — confirmation overlay
- `src/ui/results.rs` — results view
- `src/git/operations.rs` — delete_local (git2 branch.delete()), delete_remote (git push --delete)
- Wire action keys (d/D) → confirm → execute → results
- Verify: can select branches, confirm delete, see results

### Step 6: Help overlay + polish
- `src/ui/help.rs` — keybinding reference
- Edge cases: no branches, empty selections, current branch protection
- Terminal safety: panic hook for restore
- Verify: full workflow end-to-end

### Step 7: Integration tests
- `tests/integration.rs` — temp repo with known branch states
- Test: regular merge detection, squash-merge detection, delete operations
- Verify: `cargo test` passes all tests

### Step 8: Documentation
- CLAUDE.md for Claude Code context
- README.md with usage and installation

## Phase 2 (Future)

Deferred to a separate plan after Phase 1 ships:
- **Switch/checkout**: Operates on the cursor position (not the checked selection). Single branch only.
- **Archive**: Tag branch before deleting (e.g., `archive/<branch-name>`). Prompt for tag prefix with default `archive/`.

## Error Handling

- **Pre-TUI errors** (not a repo, can't detect base): print to stderr, exit 1
- **Git module**: `thiserror` typed errors (GitError enum)
- **App/main**: `anyhow::Result` wrapping
- **Operation failures**: captured as `OperationResult { success: false }`, displayed in results view — app continues for remaining branches
- **Terminal safety**: `ratatui::restore()` in panic hook guarantees terminal cleanup

## Verification

After each step, run:
- `cargo build` — compiles without errors
- `cargo test` — all tests pass
- `cargo clippy` — no warnings
- Manual test in a real git repo with branches in various states (merged, squash-merged, unmerged, gone remote)

For the full end-to-end test:
1. Create a test repo with branches in known states
2. Run `git-branch-manager` → verify branch list shows correct merge statuses
3. Select merged branches → delete → verify they're gone
4. Verify the tool exits cleanly and terminal is restored
