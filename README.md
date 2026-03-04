# git-branch-manager

Interactive TUI for managing local git branches with squash-merge detection.

Ever accumulate dozens of local branches that GitHub says were "squash and merged" but `git branch -d` refuses to delete because it thinks they're unmerged? This tool fixes that.

## Quick Start

```sh
cargo install --path .
cd your-repo
git branch-manager
```

## Features

- **Squash-merge detection** — identifies branches that were squash-merged into the base branch, even though git considers them unmerged
- **Regular merge detection** — also detects conventionally merged branches
- **Full-screen TUI** — scrollable branch list with merge status, remote tracking info, and branch age
- **Multi-select** — toggle individual branches or use quick-select shortcuts (all, none, merged-only, invert)
- **Batch operations** — delete local branches, or delete local + remote in one action
- **Auto-detect base branch** — reads `origin/HEAD`, falls back to main/master/develop
- **Non-destructive loop** — after an operation, results are shown and the branch list refreshes so you can keep working

## Usage

```sh
# Run in any git repo (auto-detects base branch)
git branch-manager

# Override the base branch
git branch-manager --base develop

# Non-interactive mode: print branch list to stdout
git branch-manager --list
```

## Keybindings

### Branch List

| Key | Action |
|-----|--------|
| `j` / `↓` | Move cursor down |
| `k` / `↑` | Move cursor up |
| `Space` | Toggle selection on current branch |
| `a` | Select all (except base and current branch) |
| `n` | Deselect all |
| `m` | Select merged + squash-merged branches |
| `i` | Invert selection |
| `d` | Delete selected branches (local only) |
| `D` | Delete selected branches (local + remote) |
| `?` | Show help overlay |
| `q` / `Esc` | Quit |

### Confirmation

| Key | Action |
|-----|--------|
| `y` | Confirm and execute |
| `n` / `Esc` | Cancel, return to branch list |

### Results

| Key | Action |
|-----|--------|
| Any key | Return to branch list (refreshed) |

## How Squash-Merge Detection Works

When a branch is squash-merged via GitHub (or similar), git creates a new single commit on the base branch. The original branch commits are not ancestors of this new commit, so `git branch --merged` reports the branch as unmerged.

This tool detects squash merges by:

1. Finding the common ancestor between the base branch and the feature branch
2. Creating a temporary commit that squashes all feature branch changes onto that ancestor
3. Using `git cherry` to check if equivalent content already exists in the base branch

If the content matches, the branch is marked as **squash-merged**.

## Building

Requires Rust (stable). Install via [rustup](https://rustup.rs/).

```sh
# Build
cargo build

# Build release
cargo build --release

# Run tests
cargo test -- --test-threads=1

# Lint
cargo clippy
```

Tests require `--test-threads=1` because the squash-merge detection tests use `set_current_dir` which is process-global.

## License

MIT
