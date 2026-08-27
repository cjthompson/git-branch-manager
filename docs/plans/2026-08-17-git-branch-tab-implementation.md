# Git Branch Tab Implementation

## Status

The initial Graph tab was committed as `827fd5e` (`feat: add graph tab`). This checkout also contains subsequent uncommitted Graph refinements; check the working tree before relying on a commit description as the complete implementation.

## Goal and user-visible behavior

The Graph tab is the first tab and the default view when the TUI opens. It shows the repository's commit DAG, commit summaries, abbreviated object IDs, and inline ref decorations in a stable right-hand column. The existing Branches, Remote, Tags, and Worktrees tabs remain in the cycle after Graph.

The Graph tab is deliberately a separate view model rather than another sortable `ListState`: graph rows contain non-commit connector lines, and each graph/ref row shares one cursor and scroll offset.

## Architecture

### Data flow

1. `main.rs` creates `App`, applies any CLI symbol override, and starts the initial graph worker immediately so the TUI can launch without waiting for git traversal.
2. `App` owns a `GraphState` and an `mpsc::Receiver<Result<GraphSnapshot, GraphLoadError>>`.
3. `git::graph::spawn_graph_loader` runs the loader on a worker thread and sends only owned application types across the channel. Repository handles and Gleisbau values never cross the thread boundary.
4. `App::drain_channels` applies the result, clears the `Loading graph...` toast, and schedules a redraw.
5. `ui::render` dispatches `ViewId::Graph` to `ui::graph_render::render_graph_view`.

### Loader strategy

`src/git/graph.rs` uses Gleisbau as the primary graph engine:

- Gleisbau is opened against the repository with a 500-commit default limit.
- `Characters::thin()` is used for ASCII/Unicode; `Characters::round()` is used for Powerline.
- Graph lines are rendered with `print_graph_terminal`, then mapped back to commit indices so each rendered line can be associated with its commit when needed.
- Refs are collected separately with git2 and attached to commits by object ID.

If Gleisbau returns an error or panics, the loader falls back to `git log --graph --topo-order --decorate --oneline --no-color` with machine-readable record separators. The fallback is represented as `GraphSource::GitCliFallback` and rendered with a visible banner. When remote refs are enabled, the fallback uses `--all`; otherwise it uses `--branches`.

The loader collects local branches and tags by default. Remote branches are collected only when the Graph options overlay enables them; the remote `HEAD` symbolic refs are omitted. Tag-only history is retained when remotes are included.

### Application state

`src/view/graph.rs` contains:

- `GraphState`: snapshot, loading/error state, history limit, remote-ref option, and one shared cursor/offset.
- `GRAPH_PAGE_SIZE = 500`: initial history size and increment for loading older history.

`GraphState::selected_commit_line` maps the commit cursor through graph connector lines. This is important: the cursor counts commits, while the renderer scrolls through all graph lines.

### Rendering

`src/ui/graph_render.rs` renders a bordered Graph view as one synchronized row stream:

- `Commits`: graph topology, short OID, and summary.
- `Refs`: inline local/remote/tag markers, lane numbers, status badges, and branch names on the matching commit row.

The ref column has a stable width and a visual separator, so commit summaries cannot displace refs. Loading, error, and Gleisbau-fallback states are rendered inside the Graph block. The fallback state includes a retry hint.

Graph lanes use the active `Theme` palette (`title`, `ahead`, `behind`, and `remote_title`, cycling by lane). Merge connector spans use the origin lane's color, so a merge's incoming line remains visually tied to the source commit rather than changing color at the merge point.

## Symbols and line styles

Graph symbols are part of `SymbolSet`; do not hard-code them in the renderer:

| Symbol set | Regular commit | Merge commit | Left connector | Right connector | Line style |
| --- | --- | --- | --- | --- | --- |
| ASCII | `o` | `+` | `<` | `>` | thin ASCII conversion |
| Unicode | `●` | `○` | `◀` | `▶` | thin Unicode |
| Powerline | `●` | `` | `◀` | `▶` | rounded Unicode |

The Powerline merge marker is intentionally one cell wide, and the renderer preserves each source graph column so merge connectors remain aligned with following rows.

Changing the global symbol set with `Y` reloads the graph. The selected symbol set also determines whether the loader asks Gleisbau for rounded or thin line characters.

## Input and overlays

Global controls continue to work as before:

- `Tab` / `Shift-Tab`: cycle views; Graph is first in the cycle.
- `j/k`, arrows, `PageUp/PageDown`, `g/G`, `Home/End`: navigate commit rows and their inline refs.
- `r`: reload the graph.
- `L`: load 500 older commits.
- `o`: open Graph options.

The Graph options overlay provides:

- Include remote refs (toggle, then Enter to reload).
- Load older history (+500 commits).

Graph does not use the generic list selection, sorting, filtering, or operations menu. Inline refs are display-only; the cursor selects their commit row and does not execute a branch operation.

## File map

| File | Responsibility |
| --- | --- |
| `src/git/graph.rs` | Graph snapshot types, compact ref counts, Gleisbau loader, git CLI fallback, ref collection, line-style selection, loader tests |
| `src/view/graph.rs` | Graph state, shared cursor/scroll offset, history/remote options |
| `src/ui/graph_render.rs` | Synchronized graph/ref rows, line rendering, lane colors, symbol translation, options overlay, rendering tests |
| `src/view/mod.rs` | `ViewId::Graph`, tab order, labels, cycle tests |
| `src/ui/render.rs` | Graph dispatch, Graph overlay dispatch, Graph status-bar text |
| `src/app.rs` | Graph state/channel ownership, startup load, channel draining, key handling, reload/options actions |
| `src/main.rs` | Initial worker startup and symbol-aware loader options |
| `src/symbols.rs` | Graph commit/merge/connector glyphs for each symbol set |
| `src/ui/help.rs` | Graph-specific help entries |
| `tests/integration.rs` | Real temporary-repository graph loader coverage and fallback cases |
| `Cargo.toml` / `Cargo.lock` | `git2 = 0.21` and pinned Gleisbau dependency |

## Verification completed

The implementation was verified on the committed tree with:

```text
cargo test                 266 library + 11 binary + 80 integration tests passed
cargo build                passed
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
git diff --check
```

The graph-specific coverage includes:

- Gleisbau graph loading with merge lanes, inline refs, and compact ref counts.
- CLI fallback for shallow repositories.
- Remote-ref inclusion and tag-only history in fallback mode.
- History pagination capped at the requested 500-commit page size.
- Theme lane colors and origin-lane merge connector colors.
- Unicode/Powerline commit markers and directional connector symbols.
- Powerline marker width preservation.
- Loading-toast clearing after a graph result arrives.
- Default Graph tab, Graph options, navigation, and older-history reload behavior.

## Follow-up seams

Future work should preserve these boundaries unless the product requirements change:

- Keep `GraphSnapshot` owned and channel-safe; do not send a `git2::Repository` or Gleisbau graph object through `mpsc`.
- Keep graph loading bounded by `GraphLoadOptions::max_count`; increasing the default for all repositories would hurt startup responsiveness.
- If ref actions are added, implement them as explicit Graph-pane actions rather than routing the Graph view through generic list operations.
- If graph filtering/search is added, define whether it filters commits, graph connector lines, or both; the current cursor/offset mapping assumes connector lines remain in the rendered snapshot.
- If a new symbol set is added, define its graph commit/merge/arrow glyphs and line style in `SymbolSet` and add renderer width tests before enabling it.
- Persisting the remote-ref toggle or history size is not implemented; both currently live for the app session only.
