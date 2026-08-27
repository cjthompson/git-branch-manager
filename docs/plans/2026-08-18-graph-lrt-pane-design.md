# Graph LRT Pseudo-pane Design

## Status

Approved design. This document is the visual and behavioral contract for the next Graph renderer change; it does not implement the deferred squash-indicator or horizontal-scrolling work.

## Goal

Replace the current mixed ref decorations in the Graph view with a commit-aligned right pseudo-pane. It must remain part of the same vertical row stream as the DAG: scrolling a commit row always scrolls its Graph metadata with it.

At normal width the pane is:

```text
LRT │ State │ Refs
```

The left side remains the graph topology, abbreviated OID, and commit summary. The visual divider between the graph and pseudo-pane remains fixed.

## LRT cells

`LRT` is three adjacent one-cell positions. They describe only live refs attached directly to that commit, not dim branch-track context shown on ancestor rows.

- `L`: existing `SymbolSet::current_branch` glyph when a local branch points at the commit.
- `R`: `@` in ASCII, `☁` in Unicode, and `\uf0c2` in Powerline when a remote ref points at the commit.
- `T`: `#` in ASCII, `⌑` in Unicode, and `\uf02b` in Powerline when a tag points at the commit.

Local refs use `theme.primary_text`, remote refs use `theme.remote_title`, and tags use `theme.squash_merged`, matching the abbreviated SHA color. Every glyph must occupy one terminal cell.

Matching local and remote refs are both represented. If their tips coincide, both `L` and `R` are populated but only the local name appears under `Refs`; this avoids duplicate `origin/<name>` text. If their tips differ, each tip retains its own live marker and name.

## State cell

`State` is exactly five cells at normal width. It summarizes the alphabetically first tracked local ref on the commit; all local names remain visible in `Refs` if multiple refs share a tip. A worktree marker is appended last.

| Condition | State text | Style |
| --- | --- | --- |
| ahead 1 through 9, behind 0 | active up-arrow plus count, e.g. `↑2` | `theme.ahead` |
| ahead 10 or more, behind 0 | active up-arrow only | `theme.ahead` |
| behind 1 through 9, ahead 0 | active down-arrow plus count, e.g. `↓2` | `theme.behind` |
| behind 10 or more, ahead 0 | active down-arrow only | `theme.behind` |
| ahead and behind are both nonzero | `RB` | `theme.unmerged` |
| ahead and behind are both zero | active in-sync glyph, e.g. `≡` | `theme.in_sync` |
| no upstream tracking | blank | default |

If any local ref at the commit is checked out in an additional linked worktree, append `WT` after the tracking text: `↑2 WT`, `RB WT`, or `≡ WT`. The primary repository checkout is intentionally not marked.

Graph no longer shows merged, unmerged, squash-merged, pending, or base-branch merge status badges. Those statuses remain part of the other views and their existing data flows.

## Refs and branch-track context

The `Refs` cell lists visible local names, then remote names, then bare tag names, with two spaces between names. It uses the ref-type styles above. The tag glyph appears only in the T position; it is not repeated before tag names.

When a commit has no live ref, but `GraphCommit::branch` supplies visual-track context, render that label dimly in `Refs` with blank LRT and State cells. Do not prefix it with the old `L -` decoration.

## Responsive behavior

The pseudo-pane uses one-third of the Graph content width and has no maximum. When the terminal is too narrow, it degrades without moving below the DAG:

1. Width below 9 cells: show only the three LRT positions.
2. Width 9 through 14 cells: show compact `LRT│State` fields without padding.
3. Width 15 cells or wider: show padded `LRT │ State │ Refs` fields.

The renderer clamps the one-third calculation to the space physically available after the graph/pane divider. Connector lines still produce blank pseudo-pane rows.

## Non-goals and deferred work

- Graph refs remain display-only; this change adds no branch actions, filtering, or persistence.
- Focus-aware horizontal scrolling is separate future work. It will pin the DAG and LRT/State cells while horizontally scrolling the focused commit-summary or Refs text region.
- A possible squash-merged-commit indicator is separate future work. It must use an exact patch match and never a commit-message heuristic.

