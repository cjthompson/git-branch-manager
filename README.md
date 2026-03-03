# git-branch-manager

Interactive TUI for managing local git branches with squash-merge detection.

## Features

- List local branches with merge status, tracking info, and age
- Detect squash-merged branches (GitHub "squash and merge" workflow)
- Detect regular merged branches
- Multi-select branches with quick-select shortcuts
- Batch delete local branches, or local + remote together
- Auto-detect base branch (from remote HEAD, or main/master/develop fallback)

## Installation

```sh
cargo install --path .
```

## Usage

Run inside any git repository:

```sh
git branch-manager
```

Override the base branch (default is auto-detected):

```sh
git branch-manager --base develop
```

## Keybindings

| Key | Action |
|-----|--------|
| `j` / `Down` | Move cursor down |
| `k` / `Up` | Move cursor up |
| `Space` | Toggle selection |
| `a` | Select all (except base/current) |
| `n` | Deselect all |
| `m` | Select merged + squash-merged |
| `i` | Invert selection |
| `d` | Delete local (selected branches) |
| `D` | Delete local + remote (selected branches) |
| `?` | Show help |
| `q` / `Esc` | Quit |

In the confirmation dialog, press `y` to confirm or `n` to cancel. In the results view, any key exits.

## Squash-Merge Detection

GitHub's "squash and merge" rewrites commits, so `git branch -d` thinks the branch is unmerged. This tool detects squash-merged branches by reconstructing what the squashed commit would look like and checking if equivalent content exists in the base branch. Branches detected as squash-merged are displayed distinctly from regular merges and unmerged branches.

## License

MIT
