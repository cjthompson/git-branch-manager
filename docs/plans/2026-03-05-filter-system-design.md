# Filter System Design

## Summary

Add a filter menu (F key) that populates the search bar with structured filter tokens. The search bar is the single source of truth for all filtering — text and structured. Users discover filters via the menu, power users type tokens directly.

## Filter Tokens

| Token | Meaning |
|-------|---------|
| `status:merged` | Merged branches |
| `status:squash` | Squash-merged branches |
| `status:unmerged` | Unmerged branches |
| `pr:yes` | Has a PR |
| `pr:no` | No PR (negative — always excludes) |
| `sync:ahead` | Ahead of remote (can push) |
| `sync:behind` | Behind remote (can pull) |
| `age:<7d` | Newer than 7 days |
| `age:<30d` | Newer than 30 days |
| `age:>30d` | Older than 30 days |
| `age:>90d` | Older than 90 days |
| `age:<Nd` / `age:<Nm` / `age:<Ny` | Custom: days/months/years |
| (plain text) | Branch name substring |

## Filter Logic

- **Same-type positive filters OR together**: `status:merged status:squash` shows merged OR squash-merged
- **Negative filters always AND-exclude**: `pr:no` excludes branches with PRs regardless of other matches
- **Different filter types AND together**: `status:merged sync:ahead` must be merged AND ahead
- **Text search ANDs with everything**: `feature status:merged` name contains "feature" AND is merged
- **Pinned branches** (base, current) always shown regardless of filters

## Filter Menu

Overlay opened with `F` key. Same visual pattern as Help/Confirm overlays.

```
+-- Filters ---------------------------+
|                                      |
|  Status                              |
|  [m] Merged          [s] Squashed    |
|  [u] Unmerged                        |
|                                      |
|  Pull Requests                       |
|  [p] Has PR          [P] No PR      |
|                                      |
|  Sync                                |
|  [a] Ahead (push)    [b] Behind     |
|                                      |
|  Age                                 |
|  [1] < 7 days   [2] < 30 days       |
|  [3] > 30 days  [4] > 90 days       |
|  [n] Newer than (custom)             |
|  [o] Older than (custom)             |
|                                      |
|  [c] Clear all filters               |
|                                      |
+--------------------------------------+
```

- Toggle items (m/s/u/p/P/a/b/1-4): add or remove token from search query, close menu
- Custom age (n/o): close menu, activate search bar with `age:<` or `age:>` prefilled
- Active filters show highlighted/checked state in the menu
- `Esc` closes without changes

## Status Bar

When filters active:
```
 filter: "status:merged age:<30d" (5/25 shown) -- [F]ilter [/]edit [Esc]clear
```

## Implementation

### New files
- `src/ui/filter.rs` — overlay renderer

### Modified files
- `src/app.rs` — `View::Filter { cursor: usize }`, `handle_filter_key()`, `FilterSet` struct + parsing, updated `matches_search()`
- `src/ui/mod.rs` — register filter module
- `src/ui/render.rs` — match arm for Filter view
- `src/ui/branch_list.rs` — status bar shows filter info
- `src/ui/help.rs` — add F keybinding

### FilterSet struct

```rust
struct FilterSet {
    statuses: Vec<MergeStatus>,    // OR together
    pr: Option<bool>,              // Some(true)=has PR, Some(false)=no PR
    sync_ahead: bool,
    sync_behind: bool,
    age_newer: Option<i64>,        // seconds threshold
    age_older: Option<i64>,        // seconds threshold
    text: String,                  // remaining text for name match
}
```

Parsed from search_query on each call to matches_search (or cached when query changes).
