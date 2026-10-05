# Graph Relationship Lineage Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Persist complete squash-merge and cherry-pick relationship records (lineage) for each repository, hydrate them into the Graph before the structural snapshot is published, and keep them through ref deletion, object pruning, cache expiry, and the R-key cache clear.

**Architecture:** Six independently reviewable tickets.
- **X** adds a cancellation token so superseded enrichment workers stop.
- **A** replaces the Graph's squash flags with paired `GraphRelationship` records and shows fuzzy squash sources.
- **B** computes squash patches in-process with git2 when text, rename and mode patch IDs match the CLI and it is at least as fast; binary parity remains a low-priority diagnostic.
- **P** replaces `git cherry` with a bounded git2 patch-ID scan that pairs each picked commit with its destination, and makes applying enrichment a merge that never erases anything.
- **L** is the lineage ticket (supersedes cancelled #053). It adds a durable SQLite store keyed by the Git common directory, kept under the application *data* directory and separate from the disposable `BranchCache`. The enrichment worker writes accepted relationships to it; loader and existing updater workers hydrate from it read-only before publishing, including newly displayed endpoints.
- **D** caches fuzzy-pair scores in the transient cache.

**Tech Stack:** Rust 2021, `git2` 0.21 (`Diff::patchid`, `Revwalk`, `Oid::hash_object`), `rusqlite` 0.32 (bundled), `dirs` 6, `chrono`, ratatui, `tempfile` for tests.

**Spec:** `docs/superpowers/specs/2026-10-02-graph-refresh-and-lineage-design.md`, especially the sections "Task 053 requirements and judgment" and "Important implementation implications for 053". Also read `docs/reviews/2026-10-02-ui-stall-regression-review.md`. Executors read both. Ticket A folds in "Proposed fix 1" from `docs/plans/2026-09-22-squash-match-confidence.md`.

---

## Tickets

Each ticket's anchor is its slice heading below. The requirement lists are the ones to record in the task tracker.

### X — Cancel superseded Graph enrichment workers
- **Type / priority / tags:** fix / medium / `graph`, `enrichment`, `concurrency`
- **Anchor:** `## Slice X — ct/graph-enrichment-cancel`
- **Depends on:** none
- **Requirements:**
  1. Add an `EnrichmentCancel` token backed by an App-owned `Arc<AtomicU64>`. Store the signal at every `graph_generation` increment, through a single `App::bump_graph_generation()` helper that `request_graph_update` also uses.
  2. `spawn_possible_squash_enrichment` takes the signal. A superseded worker returns without sending.
  3. Check cancellation before each squash patch job is dispatched, before each cherry tip is processed, and while processing cherry output commits. P adds checks on every git2 revwalk yield, including merges.
  4. Test both a pre-cancelled worker and a worker superseded after it starts; each drops its sender without publishing. Use deterministic synchronization rather than sleeps.
  5. Test that enrichment spawned after an enrichment-invalidating incremental update still publishes.

### A — Pair squash sources in Graph relationships and show fuzzy sources
- **Type / priority / tags:** task / high / `graph`, `squash`, `fuzzy`, `ui`
- **Anchor:** `## Slice A — ct/graph-squash-relationships`
- **Depends on:** none
- **Requirements:**
  1. Replace `GraphCommit`'s `is_possible_squash_merge`, `possible_squash_merge_sources` and `fuzzy_squash_match` fields with `relationships: Vec<GraphRelationship>`. Keep the old values available as accessors.
  2. Exact squash detection records each destination together with its sorted, deduplicated destination ref snapshots, the source tip OID, and all of the tip's branch names.
  3. Fuzzy detection keeps every accepted (destination, source tip) pair with its percent, including other fuzzy sources for a destination that also has an exact source. Skip only a pair already recorded as exact; accessors select the strongest confidence for presentation.
  4. `fuzzy_match::classify` floors the percent, so fuzzy is always ≤ 99. The existing Branches-view likely-squash assertions still pass.
  5. The ref pane shows the best fuzzy source with a `~N%` suffix, and the info modal shows `Possible Squash Merge From`. Exact matches keep the glyph and show no percent.
  6. Both carry-forward paths, in `app.rs` and in `update_graph_incrementally`, copy `relationships`. The existing exact, fuzzy and graph-patch cache tests keep their meaning.

### B — Compute Graph squash patches in-process with git2 when at parity
- **Type / priority / tags:** task / low / `graph`, `git2`, `perf`
- **Anchor:** `## Slice B — ct/graph-git2-squash-patches`
- **Depends on:** none
- **Requirements:**
  1. Add `git2_diff_patch`, which computes the patch ID and diff text in-process.
  2. Check patch-ID equality against `git diff --binary --full-index | git patch-id --stable` for rename, binary, mode and text cases; require fuzzy similarity ≥ 0.99 only for textual hunks and expect `None` for binary-only, mode-only and rename-only diffs. Gate the switch on text, rename and mode parity. Binary mismatches are low-priority, nonblocking diagnostics; prefer the generic `show_binary(true)` option without a bespoke special-case path.
  3. Switch `compute_patch` and bump `GRAPH_DIFF_VERSION` to 2 only if a cold-cache run is at least as fast. Record the timings using a span this ticket adds.

### P — Pair cherry-picks with destinations via a bounded git2 patch-ID scan
- **Type / priority / tags:** task / high / `graph`, `cherry-pick`, `git2`, `cache`
- **Anchor:** `## Slice P — ct/graph-cherry-pick-pairing`
- **Depends on:** A, X
- **Requirements:**
  1. Replace the `git cherry` subprocess with git2 revwalks and patch IDs. Pair each cherry-picked source commit with every matching base destination within the scan, exact matches only; retain all source patch buckets and scan to history end or the bound, even after every source has one match. Capture sorted, deduplicated destination ref snapshots.
  2. Bound the scans at 500 source commits per tip and 2,000 destination commits per tip, counting all yielded commits including merges, and check cancellation on every yield before skipping merges. Exceeding a bound logs `scan_incomplete` on the default log target, and unmatched sources produce nothing.
  3. Cache commit patch IDs, including `None`, in an on-demand `commit_patch_id` table. A cache file from before the table existed is treated as all misses.
  4. The enrichment message carries `Vec<GraphRelationship>`. Applying it merges each relationship into every displayed endpoint, adding or strengthening and never removing.
  5. A destination commit shows `Cherry-picked From <short oid> (<refs>)`, and the source commit stays marked as cherry-picked.
  6. On text, rename and mode-change fixtures, the set of commits marked cherry-picked equals `git cherry`'s set. Each fixture adds an unrelated base commit before picking. Stop on those parity failures; binary equality and unrelated-binary checks remain low-priority, nonblocking diagnostics, using generic diff options rather than a bespoke fallback. Test pick/revert/repick retains both destinations.

### L — Persist squash-merge and cherry-pick lineage (supersedes cancelled #053)
- **Type / priority / tags:** task / high / `graph`, `lineage`, `sqlite`, `persistence`
- **Anchor:** `## Slice L — ct/graph-relationship-lineage`
- **Depends on:** P (and A and X through P)
- **Requirements** (R1–R2 are the original #053 text; R3–R7 are reworded; R8 is new):
  1. Document the existing problem: current graph loading persists diff-derived patch data, but it recomputes cherry-pick detection and does not persist final squash/cherry relationships. Relationship annotations arrive only after background enrichment completes, leaving the graph without markers for roughly 30 seconds on this repository.
  2. Add durable repository-scoped lineage records with fields: type (sm for squash merge or c-p for cherry-pick), destination_hash (full destination commit OID), destination_ref (destination ref name captured at detection), source_hash (full original source commit OID), and source_ref (original branch/ref name captured at detection). Store OIDs as text so records remain useful after source refs are deleted and Git prunes source objects.
  3. Use a stable uniqueness key of type plus full destination and source OIDs, index destination_hash and source_hash, retain the five scalar columns as primary captured snapshots, and transactionally union all source and destination ref names in separate child tables. Reconstruct both unions without a join cross-product. Store lineage in a separate per-repository database under the application data directory, keyed by the Git common directory, that neither 60-day cache pruning nor the R-key cache clear can match or delete.
  4. When enrichment completes, persist accepted exact and fuzzy squash relationships and exact cherry-pick relationships from the worker, even when the result is no longer published; preserve the fuzzy similarity score and never write a partial or invented tuple.
  5. In Graph loader and existing updater workers, hydrate stored lineage for the resolved current base before publishing the snapshot, including newly displayed or refilled endpoints. Resolve an absent base option with `branch::detect_base_branch`, never as all bases. Fresh detection and App metadata carry-forward merge relationships and both ref unions without erasing hydrated data; no lineage SQLite access runs on the UI thread. The updater hydration adapter is removable with the updater and creates no dependency on P003.
  6. Use cached lineage to annotate the destination commit and, when it is present in the loaded graph, the source commit, including source-end "Squash-merged Into" / "Possibly Squash-merged Into" details; a missing source ref or pruned source object must not erase provenance.
  7. Add tests covering reloads under both `main` and `develop` after the same pair is observed under both bases, default-base filtering with mixed histories, stale/concurrent upserts preserving both ref unions, deleted source refs and pruned source objects, worker-side hydration on ref removal/window refill, per-repository isolation (linked worktrees share, clones isolated), retention through transient-cache pruning and the R-key clear, old transient cache files with no migration, warm hydration while fresh enrichment is cancelled, and a locked or unwritable lineage root.
  8. Keep exact and fuzzy presentation distinct and never let lineage change MergeStatus, Branches merge columns, or delete eligibility; record cold and warm marker timings with isolated explicit data/cache roots without deleting real durable history, and state that only stored relationships appear immediately.

### D — Cache fuzzy squash pair scores in the transient cache
- **Type / priority / tags:** task / medium / `graph`, `fuzzy`, `cache`, `perf`
- **Anchor:** `## Slice D — ct/graph-fuzzy-pair-cache`
- **Depends on:** A
- **Requirements:**
  1. Add `FUZZY_SCORING_VERSION`, and key each pair as `{destination}:{merge_base}:{tip}:v{version}`.
  2. Store scored pairs in an on-demand `fuzzy_pair` table, including pairs that did not match. Do one lookup and one store per run.
  3. A cache hit skips `score`, including cached `None`; a local test scoring seam observes zero calls on the second load while it yields the same fuzzy match and sources.
  4. A cache file without the table is treated as all misses. Define this fixture independently of P so the slice depends only on A.

---

## Problem statement (L requirement 1)

Graph loading already caches patch-derived data. The `graph_patch` table in `BranchCache` stores `(old, new) → (patch_id, diff_text)`. It does not persist the *final* relationships:
- `compute_cherry_pick_updates` (`src/git/graph.rs:1047`) re-runs `git cherry` once per diverged tip on every load.
- `compute_possible_squash_updates` (`src/git/graph.rs:846`) rebuilds squash annotations only inside background enrichment.
- Every relationship marker arrives in one batched message after both detectors finish. As a result, the Graph shows no markers for roughly 30 seconds on this repository. That figure is a recorded task observation, not a new benchmark; L5 measures it.

Three identity gaps currently prevent persisting complete tuples:
1. **Exact squash:** it keeps the source *names* (`PatchTarget::BranchTip { source_names }`) but drops the source tip OID.
2. **Fuzzy squash:** it reduces to the maximum percent (`branch_diffs: Vec<Vec<u8>>`), losing which source matched.
3. **Cherry-pick:** `git cherry` reports the source commit (`-` lines) but never the destination commit.

## Performance boundary (L requirement 8)

Cached *positive* relationships give immediate markers on warm loads. A missing record does **not** prove that no relationship exists. Cold loads, newly created relationships, and new commits still need background detection, and this plan promises no particular speedup for them.

Lineage does not change Graph topology refresh. It supplies annotations only; task #047's automatic refresh still discovers changed refs. This plan also does not fix the synchronous Branches refresh, the UI-thread `GraphRepositoryState::capture`, or worktree concurrency. Those belong to `docs/superpowers/plans/2026-10-02-graph-refresh-rollback.md` and the spec's other delivery units.

## Global Constraints

- **Integration surface.** Integrate only through:
  - `GraphSnapshot`
  - `graph::spawn_graph_loader(PathBuf, GraphLoadOptions)`
  - `graph::spawn_possible_squash_enrichment`
  - `GraphEnrichmentMsg`
  - `graph::apply_squash_enrichment` / `GraphState::apply_squash_enrichment`

  The rollback plan proposes removing `update_graph_incrementally`, `spawn_graph_updater`, `GraphUpdateMsg`, `GraphRepositoryState`, and `pending_graph_deltas`. Until then, keep them compiling and carrying `relationships` forward (A1), and add only a small optional hydration adapter at the updater's worker publication boundary (L3). Pass `lineage_root: Some(self.lineage_root.clone())` in updater options (`app.rs:4983`); hydrate the worker result before sending, and merge latest App metadata into that result without overwriting hydrated data. Keep the store and detectors independent of updater topology internals; removing the updater later removes only the adapter. No dependency on P003. Keep the names `GraphEnrichmentMsg.updates` and `GraphState::apply_squash_enrichment`, so the enrichment drain call site at `src/app.rs:703` stays textually unchanged.
- **Workers only.** Hydration, every SQLite access (lineage store and transient cache), detection, and every Git operation run in worker threads. No new Git, disk, or SQLite work runs in `App::drain_channels`, `App::with_cache_root`, or the R-key handler (`App::clear_cache_and_refresh`). In-memory annotation merges in channel draining are allowed. The App stores only a `LineageRoot` (a path) and an `Arc<AtomicU64>`.
- **Read-only hydration.** Loader and updater workers open lineage with `LineageStore::open_read_only`:
  - It runs no DDL and no meta write.
  - It sets `busy_timeout` to 200 ms.
  - Any error yields "no relationships", not a failed load.

  Only the enrichment worker writes.
- **No migration.** The lineage store is a new database file. The transient cache keeps its existing filename and schema. New transient tables use `CREATE TABLE IF NOT EXISTS` and are queried by key, never loaded by `read_all`. This replaces the "migration" wording of original #053 requirement 7, per the user's instruction: "we don't need to do a migration because it's a temporary cache file. just create a new database with the right schema."
- **Record shape.** Scalar fields: `type` (`sm` | `c-p`), `destination_hash`, `destination_ref`, `source_hash`, `source_ref`, and a nullable `similarity_percent` (`NULL` = exact). The scalar refs retain primary captured snapshots; `lineage_source_ref` and `lineage_destination_ref` transactionally retain every observed name. The in-memory model uses sorted, deduplicated `source_refs` and `destination_refs`. OIDs are full 40-hex `TEXT`. The primary key is `(type, destination_hash, source_hash)`, with indexes on `destination_hash` and on `source_hash`; ref observations never change identity or overwrite prior base membership.
- **Never invent or persist partial tuples.** A record is written only when both OIDs came from the same detection pairing. A source with no discovered destination is not written.
- **Durable storage location.**
  - `LineageRoot::from_env()` resolves to `$GBM_DATA_DIR`, else `dirs::data_dir()/git-branch-manager`, else `std::env::temp_dir()/git-branch-manager`.
  - The filename is `git-bm-lineage-<oid>.sqlite3`, where `<oid>` = `git2::Oid::hash_object(ObjectType::Blob, canonical_common_dir_bytes)`. Use this rather than `DefaultHasher`, which is not stable across Rust releases.
  - `prune_stale_caches` and `BranchCache::clear` must never match or delete this file.
- **Tests use explicit roots.** `GraphLoadOptions.lineage_root` is `Option<LineageRoot>` and defaults to `None`. Lineage is opened only when a caller passes a root. `App::new` (the test constructor), `TestDir`, and `manager_command` (via `GBM_DATA_DIR`) use temporary directories.
- **Presentation.** Exact and fuzzy stay distinct:
  - Exact means `similarity_percent == 100` and shows the squash glyph.
  - Fuzzy means `<= 99`. It shows the `[fuzzy squash N%]` suffix and the ref-pane `~N%` suffix, never the exact glyph.
  - Graph lineage never feeds `MergeStatus`, Branches merge columns, or delete eligibility.
- **Out of scope.** Paired source/destination colors (#054, depends on #065) and connectors (#055, depends on #065 and #054; dependency already fixed). These visual follow-ups need no requirement changes here. This plan only puts the data on both endpoints and shows text in the details panes. Binary insertion special cases are low priority and outside the critical path; B and P record diagnostics without adding a new binary task or bespoke dispatch/fallback.
- **Logging.** Spans and logs use the default module target (`git_branch_manager::git::…`). The subscriber's default filter `git_branch_manager=debug` (`src/main.rs:381-382`) captures them. Do not set a custom `target:`.
- **Test commands.** Multiple filters go after `--`: `cargo test -- name_a name_b`.
- **Project rules.**
  - Run `cargo build` after every task.
  - Per `CLAUDE.md`, the main agent alone sets `Cargo.toml` to `X.Y.Z-devN` locally and increments it on each code change. **Never stage or commit the version line.** Check `git diff --cached Cargo.toml` before every commit.
  - Subagents never touch the version.
- **Scan bounds.** `MAX_CHERRY_SOURCE_COMMITS = 500` branch-side yields per tip, and `MAX_CHERRY_DESTINATION_SCAN = 2_000` base-side yields per tip. Count merges toward each bound, check cancellation before skipping them, and scan destinations until exhaustion or the bound even after all sources have a first match. Exceeding a bound records nothing for unmatched sources and logs `scan_incomplete`.

## Branching and slices

The user's PR cadence rule, quoted verbatim from `~/.claude/CLAUDE.md`:

> **Default: every PR is cut from the repo's default branch and targets it.** A PR
> is "independently shippable" only if it can be reviewed and merged on its own,
> in any order relative to my other open PRs, and contains working reviewable
> code (not a bare migration or empty stub). Never slice a single function across
> PRs.
>
> **Two checkpoints. This is where the rule gets violated — do not skip them.**
>
> 1. *Before writing code*, list the slices and give each its own branch off the
>    default branch (see **Branching**). Never build one linear branch and carve
>    PR branches out of the commit chain: once slice B sits on top of slice A,
>    B's diff contains A's and the chain is forced no matter what base you pass.
> 2. *Before running `gh pr create`*, write one line per PR — `<branch> ← <base>`
>    — and confirm every base is the default branch. Any other base needs the
>    proof below, in writing, on that line.
>
> **Proof required for a non-default base.** Run `git diff --name-only` on both
> branches; name the shared file and the specific breakage — fails to build,
> fails tests, or duplicates the other PR's work — that targeting the default
> branch would cause. Disjoint paths mean there is no dependency. Not
> dependencies: the order the commits happened in, avoiding conflicts, logical or
> narrative sequence, reviewer convenience, or the slices coming out of one work
> session.

This is a `~/dev` repo with no CI, so no PRs are opened.
- **Branch base:** each slice is a `ct/` branch cut from the freshly fetched default branch (`git fetch && git symbolic-ref --short refs/remotes/origin/HEAD`).
- **Acceptance:** squash-merge into `main`, then a separate version-bump commit on `main`, per `CLAUDE.md`.
- **Dependent slices:** a slice that needs another is cut from `main` **after** that slice merges, never from another `ct/` branch.

| Slice | Branch | Base | Contents |
| --- | --- | --- | --- |
| X | `ct/graph-enrichment-cancel` | `origin/main` (now) | X1 |
| A | `ct/graph-squash-relationships` | `origin/main` (now) | A1, A2 |
| B | `ct/graph-git2-squash-patches` | `origin/main` (now) | B1 |
| P | `ct/graph-cherry-pick-pairing` | `origin/main` **after A and X merge** | P1, P2, P3 |
| L | `ct/graph-relationship-lineage` | `origin/main` **after P merges** | L1–L5 |
| D | `ct/graph-fuzzy-pair-cache` | `origin/main` **after A merges** | D1 |

**Dependency proofs.** Each is a build or duplication breakage on today's `main`. Re-verify each one with `git diff --name-only` before cutting the branch.
- **P needs A:** P constructs `GraphRelationship`, `RelationshipKind` and `RelationshipMatch`, which A adds in `src/git/graph.rs`. P does not compile without them.
- **P needs X:** `compute_cherry_pick_relationships` takes `&EnrichmentCancel`, and P's `spawn_possible_squash_enrichment` signature includes X's `latest_generation` parameter. Neither exists on `main`.
- **L needs P:** `hydrate_lineage` calls `apply_squash_enrichment(snapshot, &[GraphRelationship])`, and `persist_relationships` consumes `GraphEnrichmentMsg.updates: Vec<GraphRelationship>`. Both signatures come from P, so L fails to build without P. Persisting exact cherry-picks would also duplicate P's pairing.
- **D needs A:** D's pair key reads `SourceTip { tip_oid, merge_base }`, which A introduces in the fuzzy loop. D does not compile without it.
- **B:** no dependency. It edits only `compute_patch`, `GRAPH_DIFF_VERSION` and its own test and span.
- **Conflicts only, not dependencies:**
  - P and D both edit `ensure_schema` in `cache.rs`.
  - X, A, P and D all touch `compute_possible_squash_updates` or the enrichment spawn, but each change is complete on its own.
  - B and P both touch `graph.rs` imports.

> **Base-branch caveat:** local `main` was 4 commits ahead of `origin/main` (`4bc8f7a`), and the spec, review, and rollback plan were untracked. Resolve this before cutting any slice: push `main`, or let the user decide otherwise.

## Review Focus

These are the input classes and failure modes most likely to bite a user that the spec implies but no requirement names. Each has a test in its owning task.

1. **The same commit is both a squash destination and a cherry-pick destination.** A single-commit branch squashed onto base has the same patch ID as its only commit. Both records must coexist, with different `type` values, without double-rendering or dropping either. Test: `same_commit_records_squash_and_cherry_pick` (P2).
2. **The same pair detected under more than one base.** Hydration filters membership in `destination_refs`, resolving an absent `--base` through `branch::detect_base_branch`; it never loads all bases. Tests: `hydration_skips_relationships_for_other_base`, `same_pair_hydrates_under_both_bases`, `default_base_filters_mixed_lineage` (L3).
3. **Two branches at the same tip.** There is one record per `(type, destination, source)`, but both names are kept in `lineage_source_ref` and both are shown. Test: `shared_tip_keeps_all_source_names` (L1).
4. **A long-lived branch whose base side has thousands of commits.** The destination scan stops at the bound and writes no partial tuple. Test: `cherry_scan_bound_writes_no_partial_record` (P1).
5. **The lineage database is locked by another process, or its root is unwritable.** The loader still publishes the structural snapshot promptly (hydration gives up after 200 ms), and enrichment still publishes in-session markers. Tests: `locked_lineage_does_not_delay_graph`, `unwritable_lineage_root_does_not_block_graph` (L3).
6. **Ref removal refills the displayed window with historical destinations.** Hydrate those endpoints in the updater worker before sending, even after the original source ref and object are gone; App carry-forward merges rather than overwriting that worker result. Test: `updater_refill_hydrates_before_publication` (L3).
7. **A source is picked, reverted, and picked again.** Retain both destination pairs; finding the first match must not consume a source patch bucket. Test: `cherry_pick_repicks_keep_all_destinations` (P2).
8. **An exact squash source shares a destination with a distinct fuzzy source.** Preserve both facts while the confidence accessor selects exact. Test: `exact_destination_keeps_distinct_fuzzy_source` (A1).

---

## File structure

| File | Slices | Responsibility |
| --- | --- | --- |
| `src/git/graph.rs` | X, A, B, P, L, D | cancel token, `GraphRelationship` model and accessors, detectors, `apply_relationships`, hydration call, git2 patch helpers |
| `src/git/lineage.rs` (new) | L | `LineageRoot`, `LineageStore`: durable schema, upsert, read-only lookup |
| `src/git/mod.rs` | L | `pub mod lineage;` |
| `src/git/cache.rs` | P, L, D | on-demand transient tables `commit_patch_id` (P) and `fuzzy_pair` (D); `prune_stale_caches_at` made `pub(crate)` (L) |
| `src/git/fuzzy_match.rs` | A, D | `classify` uses `.floor()` (A); `FUZZY_SCORING_VERSION` (D) |
| `src/view/graph.rs` | A, P | `GraphState::apply_squash_enrichment` signature follows the update type |
| `src/ui/graph_render.rs`, `src/ui/info_modal.rs`, `src/ui/commit_details.rs`, `src/ui/render.rs` | A, P, L | accessors; fuzzy sources (A); cherry provenance (P3); squash source-end text (L4) |
| `src/app.rs` | X, A, P, L | `bump_graph_generation` and signal (X); carry-forward (A); `lineage_root` field and pass-through (L) |
| `src/main.rs` | L | pass `LineageRoot::from_env()` |
| `tests/integration.rs` | A, B, P, L, D | fixtures, `TestDir::lineage_root`, `GBM_DATA_DIR`, scenario tests |
| `CLAUDE.md` | L | Data Flow and Architecture note |

Shared test helpers for `tests/integration.rs`. Add each one in the first task that uses it:

```rust
fn rev(dir: &std::path::Path, spec: &str) -> String {
    git_output(dir, &["rev-parse", spec]).trim().to_string()
}

/// An unrelated base commit before every cherry-pick, so a pick never
/// recreates the source's exact tree+parent+metadata (identical OID).
fn advance_main(dir: &std::path::Path, label: &str) {
    std::fs::write(dir.join(format!("{label}.base")), label).unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-q", "-m", &format!("base {label}")]);
}
```

Also add this to `impl TestDir`:

```rust
fn graph_options_for(&self, base: &str) -> graph::GraphLoadOptions {
    let mut options = self.graph_options();
    options.base_branch = Some(base.to_string());
    options.max_count = 50;
    options
}
```

---

## Slice X — ct/graph-enrichment-cancel

### Task X1: Cancel superseded Graph enrichment workers

**Files:**
- Modify: `src/git/graph.rs:557-588` (`spawn_possible_squash_enrichment`), `:846-1019` (cancel checks before dispatch), `:1047` (cancel check per tip), and `:1258-1330` (`load_patch_ids`: skip dispatching jobs once cancelled)
- Modify: `src/app.rs` (new field; `bump_graph_generation`; `spawn_graph_load:4636`; `request_graph_update:4980`; both spawn call sites `:599` and `:667`)
- Test: `src/git/graph.rs` tests, `src/app.rs` tests

**Interfaces:**
- Produces (used by P and L):

```rust
#[derive(Clone, Debug)]
pub struct EnrichmentCancel { latest: Arc<AtomicU64>, generation: u64 }
impl EnrichmentCancel {
    pub fn new(latest: Arc<AtomicU64>, generation: u64) -> Self;
    pub fn never() -> Self;
    pub fn is_cancelled(&self) -> bool;   // latest.load(Acquire) != generation
}
pub fn spawn_possible_squash_enrichment(
    snapshot: GraphSnapshot, repo_path: PathBuf, requested_base: Option<String>, generation: u64,
    cache_root: CacheRoot, latest_generation: Arc<AtomicU64>,
) -> Receiver<GraphEnrichmentMsg>;
// App: graph_generation_signal: Arc<AtomicU64>; fn bump_graph_generation(&mut self)
```

- [ ] **Step 1: Write the failing tests.** In `src/git/graph.rs` tests:

```rust
#[test]
fn enrichment_cancel_tracks_latest_generation() {
    let latest = Arc::new(AtomicU64::new(3));
    let cancel = EnrichmentCancel::new(Arc::clone(&latest), 3);
    assert!(!cancel.is_cancelled());
    latest.store(4, Ordering::Release);
    assert!(cancel.is_cancelled());
    assert!(!EnrichmentCancel::never().is_cancelled());
}
```

In `tests/integration.rs`:

```rust
#[test]
fn cancelled_enrichment_publishes_nothing() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();
    let options = tmpdir.graph_options_for("main");
    let snapshot = graph::load_graph(dir, options.clone()).unwrap();
    let latest = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(9));
    let rx = graph::spawn_possible_squash_enrichment(
        snapshot, dir.to_path_buf(), Some("main".into()), 1, options.cache_root, latest,
    );
    assert!(rx.recv().is_err(), "a cancelled worker drops its sender without publishing");
}
```

Also add `started_enrichment_superseded_publishes_nothing` in the `graph.rs` test module. Factor a private worker runner used by the spawn function and a test-only/local callback at the first dispatch or processed cherry output commit. Start generation 1, block the callback on channels, wait for its `started` notification, advance the signal to generation 2, then release the worker. Assert no further jobs are dispatched and `recv_timeout` returns `Disconnected` rather than a stale message. Exercise supersession during squash dispatch and while processing cherry output; P extends the test to git2 traversal including a merge-only stretch. Use channels/barriers and bounded receive deadlines, never sleeps or a process-global mutable hook. This checks the real runner after work has started, beyond the pre-cancelled test above.

In `src/app.rs` tests, next to the generation tests near `:10463`:

```rust
#[test]
fn bump_graph_generation_keeps_signal_in_step() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = App::new(dir.path().to_path_buf(), "main".into(), Config::default());
    let before = app.graph_generation;
    app.bump_graph_generation();
    assert_eq!(app.graph_generation, before + 1);
    assert_eq!(app.graph_generation_signal.load(Ordering::Acquire), app.graph_generation);
}

#[test]
fn enrichment_after_invalidating_update_still_publishes() {
    // Reuse the fixture of the existing test that drives an enrichment-invalidating
    // `request_graph_update` (find it with
    // `rg -n "graph_update_invalidates_enrichment" src/app.rs`), then:
    //   1. drive the update until `graph_enrich_rx` is respawned (the :667 path);
    //   2. wait_until(|| { app.drain_channels(); <snapshot has its expected squash marker> });
    //   3. assert the marker is present.
    // Before this fix the respawned worker saw signal != generation and never sent.
}
```

When writing `enrichment_after_invalidating_update_still_publishes`, copy the setup of the existing test found by that `rg` command; it already builds a repo whose update sets `invalidates_enrichment`. Replace the comment block with that real setup plus steps 1–3. Use the module's `wait_until(done)` (`src/app.rs:6162`, fixed 60-second deadline).

- [ ] **Step 2: Run the tests to confirm they fail.**

Run: `cargo test -- enrichment_cancel cancelled_enrichment started_enrichment_superseded bump_graph_generation enrichment_after_invalidating`
Expected: compile errors (`EnrichmentCancel`, `bump_graph_generation` missing).

- [ ] **Step 3: Implement.**

```rust
impl EnrichmentCancel {
    pub fn new(latest: Arc<AtomicU64>, generation: u64) -> Self { Self { latest, generation } }
    pub fn never() -> Self { Self { latest: Arc::new(AtomicU64::new(0)), generation: 0 } }
    pub fn is_cancelled(&self) -> bool { self.latest.load(Ordering::Acquire) != self.generation }
}
```

1. **Worker.** `spawn_possible_squash_enrichment` builds `let cancel = EnrichmentCancel::new(latest_generation, generation);` and passes `&cancel` to both detectors. It then does `if cancel.is_cancelled() { return; }` before `tx.send`.
2. **Squash detector.** `compute_possible_squash_updates` gains a `cancel: &EnrichmentCancel` parameter and returns early (`Vec::new()`) when cancelled before `load_patch_ids`. `load_patch_ids` gains `cancel` and has `next_patch_job` callers stop when `cancel.is_cancelled()`.
3. **Cherry detector.** `compute_cherry_pick_updates` gains `cancel`, returns completed results when cancelled before a tip, and checks cancellation while processing each commit in `git cherry` output before doing more work. P replaces that subprocess and adds per-yield checks to its source and destination git2 walks, even merges before skipping them. X alone does not introduce revwalks.
4. **Synchronous helper.** `load_graph_with_squash_annotations` passes `&EnrichmentCancel::never()`.
5. **App.** Add `graph_generation_signal: Arc<AtomicU64>` (initialized to `Arc::new(AtomicU64::new(0))` in `with_cache_root`), plus:

```rust
fn bump_graph_generation(&mut self) {
    self.graph_generation = self.graph_generation.saturating_add(1);
    self.graph_generation_signal.store(self.graph_generation, Ordering::Release);
}
```

   - Replace the increment in `spawn_graph_load` (`:4636`) and the one in `request_graph_update` (`:4980`) with `self.bump_graph_generation();`. Check with `rg -n "graph_generation = " src/app.rs` that no other writer remains.
   - At both spawn call sites, append `Arc::clone(&self.graph_generation_signal)`.

- [ ] **Step 4: Run the tests to confirm they pass.** Run `cargo test`, then `cargo clippy --all-targets`, then `cargo build`. Expected: PASS.

- [ ] **Step 5: Commit.**

```bash
git add src tests
git diff --cached Cargo.toml   # must show no version change
git commit -m "fix: cancel superseded Graph enrichment workers"
```

---

## Slice A — ct/graph-squash-relationships

### Task A1: Paired squash relationship model

**Files:**
- Modify: `src/git/graph.rs`
  - `:33-41` (`GraphEnrichmentUpdate`)
  - `:83-127` (`GraphCommit`, `FuzzySquashMatch`)
  - **`:425-447` (`update_graph_incrementally`: the `GraphCommit` literal at `:429-431` and the carry-forward at `:442-445`)**
  - `:846-1036` (`compute_possible_squash_updates`, `apply_squash_enrichment`)
  - `:1235-1250` (`PatchTarget`)
- Modify: `src/git/fuzzy_match.rs:187-198` (`classify`)
- Modify: `src/app.rs:636-648` (carry-forward)
- Modify: every `GraphCommit { … }` fixture in `src/app.rs`, `src/view/graph.rs`, `src/ui/*.rs`, and `tests/integration.rs`
- Test: `src/git/graph.rs` tests, `src/git/fuzzy_match.rs` tests, `tests/integration.rs`

**Interfaces:**
- Produces (used by A2, P, L, D):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RelationshipKind { SquashMerge, CherryPick }
impl RelationshipKind {
    pub fn code(self) -> &'static str;                 // "sm" | "c-p"
    pub fn from_code(code: &str) -> Option<Self>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationshipMatch { Exact, Fuzzy { similarity_percent: u8 } }
impl RelationshipMatch {
    pub fn similarity_percent(self) -> u8;              // Exact => 100
    pub fn stronger(self, other: Self) -> Self;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphRelationship {
    pub kind: RelationshipKind,
    pub matching: RelationshipMatch,
    pub destination_oid: String,
    pub destination_refs: Vec<String>,   // sorted, deduplicated captured names
    pub source_oid: String,
    pub source_refs: Vec<String>,   // sorted, deduplicated
}
impl GraphRelationship { pub fn key(&self) -> (RelationshipKind, &str, &str); }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SquashMatchConfidence { pub sources: Vec<String>, pub similarity_percent: u8 }

impl GraphCommit {
    pub fn squash_match_confidence(&self) -> Option<SquashMatchConfidence>;
    pub fn is_possible_squash_merge(&self) -> bool;
    pub fn possible_squash_merge_sources(&self) -> Vec<String>;
    pub fn fuzzy_squash_match(&self) -> Option<FuzzySquashMatch>;
}
```

- `GraphCommit` loses `is_possible_squash_merge`, `possible_squash_merge_sources` and `fuzzy_squash_match`, and gains `pub relationships: Vec<GraphRelationship>`. `is_cherry_picked_commit` stays a field until P.
- `GraphEnrichmentUpdate` becomes `{ oid: String, relationships: Vec<GraphRelationship>, is_cherry_picked_commit: bool }`.

- [ ] **Step 1: Write the failing unit tests** in `src/git/graph.rs`'s test module:

```rust
fn squash(dest: &str, source: &str, names: &[&str], matching: RelationshipMatch) -> GraphRelationship {
    GraphRelationship {
        kind: RelationshipKind::SquashMerge,
        matching,
        destination_oid: dest.into(),
        destination_refs: vec!["main".into()],
        source_oid: source.into(),
        source_refs: names.iter().map(|n| n.to_string()).collect(),
    }
}

#[test]
fn squash_confidence_prefers_exact_and_unions_its_sources() {
    let commit = GraphCommit {
        oid: "d".into(),
        relationships: vec![
            squash("d", "t1", &["feature/b"], RelationshipMatch::Exact),
            squash("d", "t2", &["feature/a"], RelationshipMatch::Exact),
            squash("d", "t3", &["feature/c"], RelationshipMatch::Fuzzy { similarity_percent: 97 }),
        ],
        ..GraphCommit::default()
    };
    let confidence = commit.squash_match_confidence().unwrap();
    assert_eq!(confidence.similarity_percent, 100);
    assert_eq!(confidence.sources, ["feature/a", "feature/b"]);
    assert!(commit.is_possible_squash_merge());
    assert_eq!(commit.fuzzy_squash_match(), None);
}

#[test]
fn squash_confidence_reports_best_fuzzy_source() {
    let commit = GraphCommit {
        oid: "d".into(),
        relationships: vec![
            squash("d", "t1", &["feature/low"], RelationshipMatch::Fuzzy { similarity_percent: 80 }),
            squash("d", "t2", &["feature/high"], RelationshipMatch::Fuzzy { similarity_percent: 97 }),
        ],
        ..GraphCommit::default()
    };
    assert_eq!(
        commit.squash_match_confidence(),
        Some(SquashMatchConfidence { sources: vec!["feature/high".into()], similarity_percent: 97 })
    );
    assert!(!commit.is_possible_squash_merge());
    assert_eq!(commit.fuzzy_squash_match(), Some(FuzzySquashMatch { similarity_percent: 97 }));
}

#[test]
fn squash_confidence_ignores_relationships_where_commit_is_the_source() {
    let commit = GraphCommit {
        oid: "t1".into(),
        relationships: vec![squash("d", "t1", &["feature/x"], RelationshipMatch::Exact)],
        ..GraphCommit::default()
    };
    assert_eq!(commit.squash_match_confidence(), None);
}

#[test]
fn stronger_match_prefers_exact_then_higher_percent() {
    use RelationshipMatch::*;
    assert_eq!(Exact.stronger(Fuzzy { similarity_percent: 99 }), Exact);
    assert_eq!(Fuzzy { similarity_percent: 80 }.stronger(Fuzzy { similarity_percent: 90 }), Fuzzy { similarity_percent: 90 });
}
```

In `src/git/fuzzy_match.rs` tests:

```rust
#[test]
fn classify_floors_so_fuzzy_never_reports_100() {
    let score = FuzzyScore { similarity: 0.996, file_overlap_ratio: 1.0, union_size: 1_000 };
    assert_eq!(classify(&score), Some(99));
}
```

`MIN_UNION_SIZE_FOR_FUZZY` must be ≤ 1,000 for this fixture. Check it and raise `union_size` if needed.

- [ ] **Step 2: Run the tests to confirm they fail.**

Run: `cargo test --lib -- squash_confidence classify_floors stronger_match`
Expected: compile errors.

- [ ] **Step 3: Add the model and accessors**, after `GraphCommit`:

```rust
impl RelationshipKind {
    pub fn code(self) -> &'static str {
        match self { Self::SquashMerge => "sm", Self::CherryPick => "c-p" }
    }
    pub fn from_code(code: &str) -> Option<Self> {
        match code { "sm" => Some(Self::SquashMerge), "c-p" => Some(Self::CherryPick), _ => None }
    }
}

impl RelationshipMatch {
    pub fn similarity_percent(self) -> u8 {
        match self { Self::Exact => 100, Self::Fuzzy { similarity_percent } => similarity_percent }
    }
    // Exact is 100 and fuzzy is floored to <= 99, so the larger percent is the stronger match.
    pub fn stronger(self, other: Self) -> Self {
        if self.similarity_percent() >= other.similarity_percent() { self } else { other }
    }
}

impl GraphRelationship {
    pub fn key(&self) -> (RelationshipKind, &str, &str) {
        (self.kind, self.destination_oid.as_str(), self.source_oid.as_str())
    }
}

impl GraphCommit {
    pub fn squash_match_confidence(&self) -> Option<SquashMatchConfidence> {
        let incoming = || {
            self.relationships.iter().filter(|r| {
                r.kind == RelationshipKind::SquashMerge && r.destination_oid == self.oid
            })
        };
        let best = incoming().map(|r| r.matching.similarity_percent()).max()?;
        let mut sources = incoming()
            .filter(|r| r.matching.similarity_percent() == best)
            .flat_map(|r| r.source_refs.iter().cloned())
            .collect::<Vec<_>>();
        sources.sort();
        sources.dedup();
        Some(SquashMatchConfidence { sources, similarity_percent: best })
    }

    pub fn is_possible_squash_merge(&self) -> bool {
        self.squash_match_confidence().is_some_and(|c| c.similarity_percent == 100)
    }

    pub fn possible_squash_merge_sources(&self) -> Vec<String> {
        self.squash_match_confidence()
            .filter(|c| c.similarity_percent == 100)
            .map(|c| c.sources)
            .unwrap_or_default()
    }

    pub fn fuzzy_squash_match(&self) -> Option<FuzzySquashMatch> {
        self.squash_match_confidence()
            .filter(|c| c.similarity_percent < 100)
            .map(|c| FuzzySquashMatch { similarity_percent: c.similarity_percent })
    }
}
```

- [ ] **Step 4: Rewrite the squash producer to keep paired identities.** In `compute_possible_squash_updates`:
  1. Change `PatchTarget::BranchTip { source_names }` to `PatchTarget::BranchTip(SourceTip)`.
  2. Push `SourceTip { tip_oid: tip.clone(), merge_base: merge_base.clone(), source_names }`.
  3. Replace `source_names_by_patch` with `sources_by_patch: HashMap<String, Vec<SourceTip>>`.
  4. Replace `branch_diffs` with `Vec<(SourceTip, Vec<u8>)>`.
  5. Build the relationships:

```rust
#[derive(Clone, Debug)]
struct SourceTip {
    tip_oid: String,
    merge_base: String,
    source_names: Vec<String>,
}

let mut relationships = Vec::<GraphRelationship>::new();
let mut exact_pairs = HashSet::<(String, String)>::new();
for (patch_id, sources) in &sources_by_patch {
    let Some(base_oids) = base_oids_by_patch.get(patch_id) else { continue };
    for destination in base_oids {
        for source in sources {
            exact_pairs.insert((destination.clone(), source.tip_oid.clone()));
            relationships.push(GraphRelationship {
                kind: RelationshipKind::SquashMerge,
                matching: RelationshipMatch::Exact,
                destination_oid: destination.clone(),
                destination_refs: vec![base_branch.clone()],
                source_oid: source.tip_oid.clone(),
                source_refs: source.source_names.clone(),
            });
        }
    }
}
for (destination, base_diff) in &base_diffs {
    for (source, branch_diff) in &branch_diffs {
        if exact_pairs.contains(&(destination.clone(), source.tip_oid.clone())) {
            continue; // Only this pair is already exact; other fuzzy sources remain facts.
        }
        let Some(percent) = crate::git::fuzzy_match::score(branch_diff, base_diff)
            .and_then(|score| crate::git::fuzzy_match::classify(&score))
        else {
            continue;
        };
        relationships.push(GraphRelationship {
            kind: RelationshipKind::SquashMerge,
            matching: RelationshipMatch::Fuzzy { similarity_percent: percent },
            destination_oid: destination.clone(),
            destination_refs: vec![base_branch.clone()],
            source_oid: source.tip_oid.clone(),
            source_refs: source.source_names.clone(),
        });
    }
}
```

   Then emit one `GraphEnrichmentUpdate` per base-lane commit, as today:
   - `relationships`: the entries whose `destination_oid == commit.oid`.
   - `is_cherry_picked_commit`: `false`.

   In slice A, `apply_squash_enrichment` assigns `commit.relationships = update.relationships.clone()` and `commit.is_cherry_picked_commit = update.is_cherry_picked_commit`. That is acceptable for now because nothing persists yet; P2 changes it to a merge. `merge_enrichment_updates` keeps its shape. Cherry updates carry `relationships: Vec::new()`, and `and_modify` only sets the cherry flag.

   Add `exact_destination_keeps_distinct_fuzzy_source` using one destination, one exact source patch, and a distinct diverged tip with a classified fuzzy diff. Assert both `(destination, source)` records remain in detector output, the exact pair is emitted only once, and the destination's confidence accessor reports 100 with only the exact source names. The retained fuzzy record must keep its own tip OID, refs and percent. This tests detector output rather than only constructing accessor fixtures.

- [ ] **Step 5: Floor the fuzzy percent.** In `src/git/fuzzy_match.rs::classify`, change `.round()` to `.floor()`. Update the `FuzzySquashMatch` doc comment to "floored from the raw f32 score; fuzzy is always <= 99".
  - **This also affects the Branches view:** `merge_detection::likely_squash_merged` (`src/git/merge_detection.rs:794`) calls `classify`.
  - Run `cargo test -- test_squash_scenario` and check the assertion `similarity_percent: 75` at `tests/integration.rs:5825`. If flooring changes it (the raw score was ≥ 75.5), update the expected value to the floored number and state it in the commit message.

- [ ] **Step 6: Migrate fixtures, readers, and both carry-forward paths.**
  - **Fixtures:** in every `GraphCommit { … }` literal, delete the three removed field lines and add `relationships: Vec::new(),` once. Fixtures that set `is_possible_squash_merge: true` with sources, or `fuzzy_squash_match: Some(..)`, build equivalent `relationships` with the `squash(...)` pattern instead, using the fixture's own OID as the destination. Find them with `rg -n "is_possible_squash_merge: true|fuzzy_squash_match: Some" src tests`.
  - **Readers:** `.is_possible_squash_merge` → `.is_possible_squash_merge()`, `.possible_squash_merge_sources` → `.possible_squash_merge_sources()`, `.fuzzy_squash_match` → `.fuzzy_squash_match()`. Fix the borrow sites, e.g. `commit.fuzzy_squash_match.as_ref()` → `commit.fuzzy_squash_match()`.
  - **Writers:** `src/app.rs:10583-10591` become `commit.relationships = vec![...]` or `.clear()`.
  - **`src/app.rs:640-646`:** replace the three squash lines with `commit.relationships = old.relationships.clone();`.
  - **`src/git/graph.rs:429-431`:** replace the three squash fields in the `GraphCommit` literal with `relationships: Vec::new(),`.
  - **`src/git/graph.rs:442-445`:** replace the three squash lines with `record.relationships = old.relationships.clone();` and keep the cherry line.

- [ ] **Step 7: Run the whole suite.** Run `cargo test`, then `cargo clippy --all-targets`, then `cargo build`. Expected: all pass, and the existing exact-squash, fuzzy and graph-patch cache integration tests keep their meaning.

- [ ] **Step 8: Commit.**

```bash
git add src tests
git diff --cached Cargo.toml
git commit -m "refactor: keep paired squash identities in Graph relationships"
```

### Task A2: Show fuzzy squash sources

**Files:**
- Modify: `src/ui/graph_render.rs:193-230`, `:630-650` (`possible_squash_source_spans`), `:1466` (test)
- Modify: `src/ui/info_modal.rs:315-332`, `src/ui/commit_details.rs:53-63`
- Test: `src/ui/graph_render.rs`, `src/ui/info_modal.rs` tests

**Interfaces:**
- Consumes: `GraphCommit::squash_match_confidence()` from A1.

- [ ] **Step 1: Write the failing tests.**
  - Rename `ref_pane_hides_possible_squash_sources_without_an_exact_match` to `ref_pane_shows_fuzzy_squash_source_with_percent`. Flip its assertion: a commit with one fuzzy 97% relationship from `feature/login` renders `format!("{} feature/login ~97%", symbols.status_squash_merged)`.
  - Add an `info_modal` test asserting `Possible Squash Merge (fuzzy)` = `97% similarity` and `Possible Squash Merge From` = `feature/login`.
  - Keep the exact tests (`"{sym} feature/auth +1"`, no percent).

- [ ] **Step 2: Run the tests to confirm they fail.** Run `cargo test --lib -- ref_pane_shows_fuzzy_squash_source_with_percent info_modal`. Expected: FAIL.

- [ ] **Step 3: Implement** `possible_squash_source_spans`:

```rust
let Some(confidence) = commit.squash_match_confidence() else { return Vec::new() };
let Some((first, rest)) = confidence.sources.split_first() else { return Vec::new() };
let more = if rest.is_empty() { String::new() } else { format!(" +{}", rest.len()) };
let percent = if confidence.similarity_percent == 100 {
    String::new()
} else {
    format!(" ~{}%", confidence.similarity_percent)
};
vec![Span::styled(
    format!("{} {first}{more}{percent}", symbols.status_squash_merged),
    selected_style(theme.squash_merged, selected, theme),
)]
```

   The glyph rule is unchanged: the exact glyph shows only when `is_possible_squash_merge()`. In `info_modal.rs`, add `Possible Squash Merge From` under the fuzzy branch, using `confidence.sources.join(", ")`.

- [ ] **Step 4: Run the tests to confirm they pass.** Run `cargo test`, then `cargo build`.

- [ ] **Step 5: Commit.** `git commit -m "feat: show fuzzy squash sources and confidence in Graph"` (check `git diff --cached Cargo.toml` first).

**Slice A acceptance:** in the TUI on `~/dev/claude-monitor`, the `42fe178` row's ref pane shows `≈ ct/fix-tui-quit-hang ~97%`.

---

## Slice B — ct/graph-git2-squash-patches

### Task B1: In-process squash patch computation, gated on parity

**Files:**
- Modify: `src/git/graph.rs` (`compute_patch`, `GRAPH_DIFF_VERSION` 1 → 2, a `tracing::info_span!("graph_squash_patches", jobs, misses)` around the miss-dispatch in `load_patch_ids`)
- Test: `tests/integration.rs`

**Interfaces:**
- Produces: `pub fn git2_diff_patch(repo: &git2::Repository, old: git2::Oid, new: git2::Oid) -> (Option<String>, Option<Vec<u8>>)`. It is `pub` so the integration test can call it; mark it `#[doc(hidden)]`.

- [ ] **Step 1: Write the parity test** `graph_git2_patch_matches_git_cli_patch_id`.
  - Build commits that rename, change mode, and edit text in the gating test; build binary insertions in a separate nonblocking diagnostic test (`#[ignore = "nonblocking binary diagnostic"]`), run explicitly and record its result.
  - For every `(parent, commit)` including the binary diagnostic, check that `graph::git2_diff_patch(...).0` equals the first field of `git diff --binary --full-index <parent> <commit> | git patch-id --stable`. Run that pipeline as two `Command`s, writing the diff stdout into `patch-id`'s stdin. Binary equality assertions remain in the diagnostic and cannot stop the gating suite.
  - For a case with textual hunks, assert `fuzzy_match::score(git2_text, cli_text)` is `Some` with `similarity >= 0.99`. For binary-only, mode-only and rename-only cases, assert `None`: the scorer intentionally has no textual tokens for them.
  - **Stop condition:** if text, rename or mode patch-ID equality fails, do not switch; record the mismatch in task notes. Keep binary equality checks in a separate diagnostic case that reports positive and unrelated binary insertions without blocking this slice. A binary collision or mismatch is low priority per the user's guidance; use the existing generic `show_binary(true)` diff option and do not implement a binary-specific fallback or dispatch path. If the switch ships with an unresolved binary difference, state that limit in task notes and the commit body.

- [ ] **Step 2: Run the test to confirm it fails.** Run `cargo test -- graph_git2_patch_matches`. Expected: compile error.

- [ ] **Step 3: Implement.**

```rust
#[doc(hidden)]
pub fn git2_diff_patch(repo: &git2::Repository, old: git2::Oid, new: git2::Oid) -> (Option<String>, Option<Vec<u8>>) {
    let (Ok(old_tree), Ok(new_tree)) = (
        repo.find_commit(old).and_then(|c| c.tree()),
        repo.find_commit(new).and_then(|c| c.tree()),
    ) else {
        return (None, None);
    };
    let mut options = git2::DiffOptions::new();
    options.show_binary(true);
    let Ok(diff) = repo.diff_tree_to_tree(Some(&old_tree), Some(&new_tree), Some(&mut options)) else {
        return (None, None);
    };
    if diff.deltas().len() == 0 {
        return (None, None);
    }
    let mut text = Vec::new();
    let printed = diff.print(git2::DiffFormat::Patch, |_, _, line| {
        if matches!(line.origin(), '+' | '-' | ' ') {
            text.push(line.origin() as u8);
        }
        text.extend_from_slice(line.content());
        true
    });
    if printed.is_err() {
        return (None, None);
    }
    (diff.patchid(None).ok().map(|id| id.to_string()), Some(text))
}
```

   `compute_patch` keeps its `(repo_path, old, new)` signature:
   - It opens `git2::Repository::open(repo_path)` (one per worker call is acceptable).
   - It parses the OIDs and calls `git2_diff_patch`.
   - Bump `GRAPH_DIFF_VERSION` to `2`.

- [ ] **Step 4: Benchmark.**
  1. On this repo, press R to clear the transient cache, then reopen Graph.
  2. Compare the `graph_squash_patches` span's `time.busy` before the switch (on `main`) and after it.
  3. Keep the switch only if it is at least as fast, per the user's rule: "prefer git2 operations over spawning git processes, if the git2 operation is at least the same performance."
  4. Otherwise revert the `compute_patch` call-site change, keep the test, and record the numbers.

- [ ] **Step 5: Run the suite and commit.** Run `cargo test` and `cargo build`, then `git commit -m "perf: compute Graph squash patches in-process with git2"`.

---

## Slice P — ct/graph-cherry-pick-pairing

Cut this from `main` after slices A and X merge.

### Task P1: git2 cherry-pick pairing with bounded destination discovery

**Behavior changes, stated explicitly:**
- **No displayed merge base needed.** Today `compute_cherry_pick_updates` only considers tips whose merge base is in the displayed window (`displayed_branch_relation`). That rule existed to avoid unbounded repository walks during Graph startup (`graph.rs:1148`). P1 drops it: destinations below the window are exactly what lineage needs, `git cherry` itself scanned them, and the walks are now bounded and cancellable. As a result, more tips become eligible than before.
- **Over-bound tips are skipped.** A tip with more than `MAX_CHERRY_SOURCE_COMMITS` (500) branch-only commits is skipped entirely, with a `scan_incomplete` log line. `git cherry` would have processed it.

**Files:**
- Modify: `src/git/graph.rs:1038-1126` (replace `compute_cherry_pick_updates` with `compute_cherry_pick_relationships`)
- Modify: `src/git/cache.rs` (on-demand `commit_patch_id` table)
- Test: `src/git/graph.rs` tests, `src/git/cache.rs` tests, `tests/integration.rs`

**Interfaces:**
- Consumes: A1 model; X1 `EnrichmentCancel`; `BranchCache`.
- Produces:

```rust
pub const MAX_CHERRY_SOURCE_COMMITS: usize = 500;
pub const MAX_CHERRY_DESTINATION_SCAN: usize = 2_000;
const CHERRY_PATCH_ID_VERSION: u32 = 1;
pub(crate) fn git2_patch_id(repo: &git2::Repository, commit: git2::Oid) -> Option<String>;
fn compute_cherry_pick_relationships(
    repo_path: &Path, snapshot: &GraphSnapshot, base_branch: &str, base_tip: &str,
    cache: &BranchCache, cancel: &EnrichmentCancel,
) -> Vec<GraphRelationship>;
pub(crate) fn compute_cherry_pick_relationships_with_bounds(
    repo_path: &Path, snapshot: &GraphSnapshot, base_branch: &str, base_tip: &str,
    cache: &BranchCache, cancel: &EnrichmentCancel, source_limit: usize, destination_limit: usize,
) -> Vec<GraphRelationship>;
// cache.rs, on-demand (never in read_all):
impl BranchCache {
    pub fn lookup_commit_patch_ids(&self, keys: &[String]) -> HashMap<String, Option<String>>;
    pub fn store_commit_patch_ids(&self, entries: &[(String, Option<String>)]);
}
```

In P1, the relationships are converted back into the existing per-OID update. The source commit's `GraphEnrichmentUpdate.is_cherry_picked_commit = true`, so P1 changes detection only, and its tests check the source side. P2 moves the message to relationships and attaches both endpoints.

- [ ] **Step 1: Write the failing tests.** In `tests/integration.rs`, using `rev` and `advance_main` from "File structure":

```rust
#[test]
fn cherry_pick_sources_match_git_cherry() {
    // Text, rename, and mode-change commits: the git2 pairing must mark the same
    // source set as `git cherry`, which it replaces.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();
    std::fs::write(dir.join("rename-me.txt"), "contents\n".repeat(20)).unwrap();
    std::fs::write(dir.join("script.sh"), "echo hi\n").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-q", "-m", "seed"]);
    run_git(dir, &["checkout", "-q", "-b", "feature/mixed"]);
    run_git(dir, &["mv", "rename-me.txt", "renamed.txt"]);
    run_git(dir, &["commit", "-q", "-m", "rename"]);
    std::fs::write(dir.join("text.txt"), "picked text\n").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-q", "-m", "text"]);
    run_git(dir, &["update-index", "--chmod=+x", "script.sh"]);
    run_git(dir, &["commit", "-q", "-m", "mode"]);
    let picks = git_output(dir, &["rev-list", "--reverse", "main..feature/mixed"]);
    run_git(dir, &["checkout", "-q", "main"]);
    advance_main(dir, "mixed");
    for oid in picks.lines() {
        run_git(dir, &["cherry-pick", oid]);
    }
    let expected = git_output(dir, &["cherry", "main", "feature/mixed"])
        .lines()
        .filter_map(|line| line.strip_prefix("- "))
        .map(str::to_string)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(expected.len(), 3, "fixture: git cherry sees all three picks");
    let snapshot = graph::load_graph_with_squash_annotations(dir, tmpdir.graph_options_for("main")).unwrap();
    let actual = snapshot.commits.iter()
        .filter(|c| c.is_cherry_picked_commit)
        .map(|c| c.oid.clone())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(actual, expected);
}
```

   **Parity stop condition:** if `cherry_pick_sources_match_git_cherry` cannot pass for text, rename or mode changes because patch IDs differ, do not ship P; record the failing case in task notes. Add a separate nonblocking binary diagnostic (`#[ignore = "nonblocking binary diagnostic"]`, run explicitly): pick one binary insertion, add an unrelated binary base insertion, and compare source sets with `git cherry` plus the direct binary patch-ID equality check. Record collisions/mismatches and any resulting binary limitation without making a special-case implementation or blocking P. The user has explicitly made binary insertions low priority.

   In `src/git/cache.rs` tests:

```rust
#[test]
fn commit_patch_ids_round_trip_including_none() {
    let (_dir, cache) = temp_cache();
    cache.store_commit_patch_ids(&[("a:v1".into(), Some("p".into())), ("b:v1".into(), None)]);
    let found = cache.lookup_commit_patch_ids(&["a:v1".into(), "b:v1".into(), "c:v1".into()]);
    assert_eq!(found.get("a:v1"), Some(&Some("p".to_string())));
    assert_eq!(found.get("b:v1"), Some(&None));
    assert!(!found.contains_key("c:v1"));
}

#[test]
fn commit_patch_ids_tolerate_cache_written_before_table() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.sqlite3");
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch("CREATE TABLE branch_cache (base_branch TEXT, branch_name TEXT, merge_status TEXT, commit_hash TEXT);").unwrap();
    drop(conn);
    let cache = BranchCache::load_from_path(path);
    assert!(cache.lookup_commit_patch_ids(&["a:v1".into()]).is_empty());
}
```

   In `src/git/graph.rs` tests, the bound and cancel test. It builds a repo with `std::process::Command` git calls in a `tempfile::tempdir()`:

```rust
#[test]
fn cherry_scan_bound_writes_no_partial_record() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    let git = |args: &[&str]| {
        assert!(std::process::Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args).current_dir(path).status().unwrap().success());
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["commit", "-q", "--allow-empty", "-m", "init"]);
    git(&["checkout", "-q", "-b", "feature/x"]);
    std::fs::write(path.join("x.txt"), "x").unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "x"]);
    let source = String::from_utf8(std::process::Command::new("git").args(["rev-parse", "HEAD"]).current_dir(path).output().unwrap().stdout).unwrap().trim().to_string();
    git(&["checkout", "-q", "main"]);
    std::fs::write(path.join("base.txt"), "b").unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "base"]);
    git(&["cherry-pick", &source]);
    for i in 0..5 {
        git(&["commit", "-q", "--allow-empty", "-m", &format!("filler {i}")]);
    }
    let cache_dir = tempfile::tempdir().unwrap();
    let options = GraphLoadOptions {
        base_branch: Some("main".into()),
        cache_root: CacheRoot::at(cache_dir.path()),
        ..GraphLoadOptions::default()
    };
    let snapshot = load_graph(path, options).unwrap();
    let base_tip = snapshot.commits.iter()
        .find(|c| c.refs.iter().any(|r| r.kind == GraphRefKind::LocalBranch && r.name == "main"))
        .unwrap().oid.clone();
    let cache = BranchCache::load_for_base(path, "main", &CacheRoot::at(cache_dir.path()));

    let bounded = compute_cherry_pick_relationships_with_bounds(
        path, &snapshot, "main", &base_tip, &cache, &EnrichmentCancel::never(), 500, 3);
    assert!(bounded.is_empty(), "destination is 6 commits down; bound 3 must record nothing");

    let unbounded = compute_cherry_pick_relationships_with_bounds(
        path, &snapshot, "main", &base_tip, &cache, &EnrichmentCancel::never(), 500, 2_000);
    assert_eq!(unbounded.len(), 1);
    assert_eq!(unbounded[0].source_oid, source);

    let latest = Arc::new(AtomicU64::new(2));
    let cancelled = EnrichmentCancel::new(latest, 1);
    assert!(compute_cherry_pick_relationships_with_bounds(
        path, &snapshot, "main", &base_tip, &cache, &cancelled, 500, 2_000).is_empty());
}
```

   Keep `test_graph_cherry_pick_enrichment_marks_branch_commits`. Insert `advance_main(dir, "graph-cherry");` after its `checkout main` and before its picks; it passes `--keep-redundant-commits`, so its picks can collide with the original commits too.

   Extend bound coverage with merge-heavy source and destination histories: every yielded merge consumes one budget slot, neither side patches merges, and source overflow skips the whole tip. Extend X's deterministic worker synchronization test to cancellation during both revwalks, including a merge-only stretch, and assert sender disconnection without stale publication.

- [ ] **Step 2: Run the tests to confirm they fail.** Run `cargo test -- cherry_pick_sources_match commit_patch_ids cherry_scan_bound`. Expected: compile errors.

- [ ] **Step 3: Implement the git2 helper.**

```rust
/// Stable patch ID of a single-parent commit's diff, computed in-process.
pub(crate) fn git2_patch_id(repo: &git2::Repository, oid: git2::Oid) -> Option<String> {
    let commit = repo.find_commit(oid).ok()?;
    if commit.parent_count() != 1 {
        return None;
    }
    let parent_tree = commit.parent(0).ok()?.tree().ok()?;
    let tree = commit.tree().ok()?;
    let mut options = git2::DiffOptions::new();
    options.show_binary(true); // Generic option, no binary-specific fallback or dispatch.
    let diff = repo.diff_tree_to_tree(Some(&parent_tree), Some(&tree), Some(&mut options)).ok()?;
    if diff.deltas().len() == 0 {
        return None;
    }
    diff.patchid(None).ok().map(|id| id.to_string())
}
```

- [ ] **Step 4: Add the on-demand transient table** to `ensure_schema`:

```sql
CREATE TABLE IF NOT EXISTS commit_patch_id (
    key      TEXT PRIMARY KEY,
    patch_id TEXT
);
```

```rust
pub fn lookup_commit_patch_ids(&self, keys: &[String]) -> HashMap<String, Option<String>> {
    let mut found = HashMap::new();
    let Ok(conn) = open_conn(&self.path) else { return found };
    for chunk in keys.chunks(400) {
        let sql = format!(
            "SELECT key, patch_id FROM commit_patch_id WHERE key IN ({})",
            vec!["?"; chunk.len()].join(",")
        );
        // A cache file written before this table existed has no table: all misses.
        let Ok(mut statement) = conn.prepare(&sql) else { return found };
        let Ok(rows) = statement.query_map(rusqlite::params_from_iter(chunk.iter()), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        }) else { return found };
        found.extend(rows.flatten());
    }
    found
}

pub fn store_commit_patch_ids(&self, entries: &[(String, Option<String>)]) {
    if entries.is_empty() {
        return;
    }
    let Ok(mut conn) = open_conn(&self.path) else { return };
    if ensure_schema(&conn).is_err() {
        return;
    }
    let Ok(tx) = conn.transaction() else { return };
    for (key, patch_id) in entries {
        let _ = tx.execute(
            "INSERT INTO commit_patch_id (key, patch_id) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET patch_id = excluded.patch_id",
            params![key, patch_id],
        );
    }
    let _ = tx.commit();
}
```

- [ ] **Step 5: Implement `compute_cherry_pick_relationships_with_bounds`.** `compute_cherry_pick_relationships` calls it with the two constants.
  1. **Setup.** Open `git2::Repository` in the worker. Candidate tips are displayed commits carrying local non-base refs, in snapshot order, deduplicated by OID. Keep an in-memory `HashMap<Oid, Option<String>>` patch-ID memo for the run, so base ranges that overlap across tips are computed once.
  2. **Sources.** For each tip, run a `Revwalk` with `push(tip)` and `hide(base_tip)`: these are the branch commits not in base. Check cancellation and increment the yield count before skipping merges (which consume the budget but have no patch ID). If the walk yields more than `source_limit` commits, `tracing::debug!(tip = %tip, "scan_incomplete: source bound")` and skip the tip entirely. A tip already contained in base yields nothing. Map each single-parent `source_oid` to its tip names and retain all entries in each patch-ID bucket; equal patches from multiple sources must not collapse to one source.
  3. **Patch IDs.** Look up keys `format!("{oid}:v{CHERRY_PATCH_ID_VERSION}")` in the memo, then `cache.lookup_commit_patch_ids`, and compute misses with `git2_patch_id`. Call `store_commit_patch_ids` with the misses once per tip. Check `cancel.is_cancelled()` before each computation; on cancel, return the relationships completed so far (each one is complete).
  4. **Destinations.** For each tip, run a `Revwalk` with `push(base_tip)` and `hide(tip)`: these are the base commits not on the branch. For each yield, check `cancel` and count it before skipping merges; get single-parent patch IDs and match against every entry in this tip's retained source patch-ID buckets. Never remove a bucket after a match or stop when each source has one destination. Scan until history exhaustion or `destination_limit` yields. If a further yield exceeds the bound, log `scan_incomplete: destination bound`; no patch computation runs outside the budget. Complete matches inside the bound remain valid.
  5. **Emit.** For every (source, destination) patch-ID match, emit `GraphRelationship { kind: CherryPick, matching: Exact, destination_oid, destination_refs: vec![base_branch.to_string()], source_oid, source_refs: sorted dedup tip names }`. Deduplicate by relationship key and union both ref sets. Sources with no destination emit nothing.
  6. **Callers.** The enrichment worker and `load_graph_with_squash_annotations` load a `BranchCache` once and pass it in. Convert the result to per-OID updates: for each relationship's `source_oid`, `is_cherry_picked_commit = true`. Remove the `git cherry` `Command`, and remove the `Command`/`Stdio` imports if nothing else uses them.

- [ ] **Step 6: Run the tests to confirm they pass.** Run `cargo test`, then `cargo build`. Expected: the new tests pass, and `test_graph_cherry_pick_enrichment_marks_branch_commits` still passes.

- [ ] **Step 7: Commit.** `git commit -m "feat: pair cherry-picked commits with their destinations via git2 patch IDs"` (check `git diff --cached Cargo.toml` first).

### Task P2: Relationship enrichment message and keep-everything endpoint merge

**Files:**
- Modify: `src/git/graph.rs`
  - `GraphEnrichmentMsg`, `GraphEnrichmentUpdate` (delete), `merge_enrichment_updates` (delete), `apply_squash_enrichment`
  - `compute_possible_squash_updates` → `compute_squash_relationships`, plus a new `compute_relationships`
  - Remove the `is_cherry_picked_commit` field and add accessors
  - `:442-445` carry-forward
- Modify: `src/view/graph.rs:202-209`, the UI readers of `is_cherry_picked_commit` (`graph_render.rs:205,224`, `info_modal.rs:326`, `commit_details.rs:53`), fixtures, and `src/app.rs:645`
- Test: `src/git/graph.rs` tests, `tests/integration.rs:7229-7380`

**Interfaces:**
- Produces:

```rust
pub struct GraphEnrichmentMsg { pub generation: u64, pub updates: Vec<GraphRelationship> }
pub fn compute_relationships(repo_path: &Path, snapshot: &GraphSnapshot, requested_base: Option<&str>,
                             cache_root: &CacheRoot, cancel: &EnrichmentCancel) -> Vec<GraphRelationship>;
pub fn apply_squash_enrichment(snapshot: &mut GraphSnapshot, updates: &[GraphRelationship]);
impl GraphCommit {
    pub fn is_cherry_picked_commit(&self) -> bool;
    pub fn cherry_pick_destinations(&self) -> Vec<&GraphRelationship>;  // this commit is the source
    pub fn cherry_pick_sources(&self) -> Vec<&GraphRelationship>;       // this commit is the destination
}
// GraphState::apply_squash_enrichment(&mut self, updates: &[GraphRelationship]) -> bool
```

- [ ] **Step 1: Write the failing tests.** In `src/git/graph.rs` tests:

```rust
fn snapshot(commits: Vec<GraphCommit>) -> GraphSnapshot {
    GraphSnapshot {
        source: GraphSource::Gleisbau,
        commits,
        lines: Vec::new(),
        ref_counts: GraphRefCounts::default(),
        max_count: 50,
        includes_remotes: false,
        generation: None,
    }
}

#[test]
fn enrichment_never_erases_existing_relationships() {
    let existing = squash("d", "gone", &["deleted/branch"], RelationshipMatch::Exact);
    let mut snap = snapshot(vec![
        GraphCommit { oid: "d".into(), relationships: vec![existing.clone()], ..GraphCommit::default() },
    ]);
    apply_squash_enrichment(&mut snap, &[]);
    assert_eq!(snap.commits[0].relationships, [existing.clone()]);

    let fresh = squash("d", "t2", &["feature/new"], RelationshipMatch::Fuzzy { similarity_percent: 88 });
    apply_squash_enrichment(&mut snap, &[fresh]);
    assert_eq!(snap.commits[0].relationships.len(), 2);

    let renamed = GraphRelationship {
        destination_refs: vec!["develop".into()],
        source_refs: vec!["feature/renamed".into()],
        ..existing
    };
    apply_squash_enrichment(&mut snap, &[renamed]);
    let merged = snap.commits[0].relationships.iter().find(|r| r.source_oid == "gone").unwrap();
    assert_eq!(merged.source_refs, ["deleted/branch", "feature/renamed"]);
    assert_eq!(merged.destination_refs, ["develop", "main"]);
}

#[test]
fn apply_attaches_relationship_to_both_displayed_endpoints() {
    let relationship = squash("d", "s", &["feature/x"], RelationshipMatch::Exact);
    let mut snap = snapshot(vec![
        GraphCommit { oid: "d".into(), ..GraphCommit::default() },
        GraphCommit { oid: "s".into(), ..GraphCommit::default() },
    ]);
    apply_squash_enrichment(&mut snap, &[relationship.clone()]);
    assert_eq!(snap.commits[0].relationships, [relationship.clone()]);
    assert_eq!(snap.commits[1].relationships, [relationship]);
}
```

   If `GraphRefCounts` has no `Default`, construct it with its fields.

   In `tests/integration.rs`, these tests need both endpoints, so they live here rather than in P1:

```rust
#[test]
fn cherry_pick_relationships_pair_source_with_destination() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();
    run_git(dir, &["checkout", "-q", "-b", "feature/pick"]);
    std::fs::write(dir.join("p.txt"), "p").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-q", "-m", "p"]);
    let source = rev(dir, "HEAD");
    run_git(dir, &["checkout", "-q", "main"]);
    advance_main(dir, "pick");
    run_git(dir, &["cherry-pick", &source]);
    let destination = rev(dir, "HEAD");

    let snapshot = graph::load_graph_with_squash_annotations(dir, tmpdir.graph_options_for("main")).unwrap();
    let source_commit = snapshot.commits.iter().find(|c| c.oid == source).unwrap();
    let destination_commit = snapshot.commits.iter().find(|c| c.oid == destination).unwrap();
    assert!(source_commit.is_cherry_picked_commit());
    let pair = source_commit.cherry_pick_destinations()[0];
    assert_eq!(pair.destination_oid, destination);
    assert_eq!(pair.destination_refs, ["main"]);
    assert_eq!(pair.source_refs, ["feature/pick"]);
    assert_eq!(destination_commit.cherry_pick_sources()[0].source_oid, source);
}

#[test]
fn same_commit_records_squash_and_cherry_pick() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();
    run_git(dir, &["checkout", "-q", "-b", "feature/single"]);
    std::fs::write(dir.join("s.txt"), "s").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-q", "-m", "s"]);
    run_git(dir, &["checkout", "-q", "main"]);
    advance_main(dir, "single");
    run_git(dir, &["merge", "-q", "--squash", "feature/single"]);
    run_git(dir, &["commit", "-q", "-m", "squash single"]);
    let destination = rev(dir, "HEAD");
    let snapshot = graph::load_graph_with_squash_annotations(dir, tmpdir.graph_options_for("main")).unwrap();
    let commit = snapshot.commits.iter().find(|c| c.oid == destination).unwrap();
    assert!(commit.is_possible_squash_merge());
    assert_eq!(commit.cherry_pick_sources().len(), 1);
}
```

   Add `cherry_pick_repicks_keep_all_destinations`: create a textual source on `feature/pick`, advance main independently, pick it and save destination 1, revert that destination, then pick the original source again and save destination 2. Run detection with a bound large enough for both, filter the `CherryPick` records for the original source, and assert both destination OIDs appear exactly once with `destination_refs == ["main"]`. The source endpoint exposes both destinations; each destination exposes the original source. Also test two source commits with the same patch ID so retaining a bucket preserves all source/destination pairs.

- [ ] **Step 2: Run the tests to confirm they fail.** Run `cargo test -- enrichment_never_erases apply_attaches cherry_pick_relationships_pair cherry_pick_repicks same_commit_records`. Expected: compile errors.

- [ ] **Step 3: Implement the merge and the message.**

```rust
/// Merge relationships into every displayed endpoint. Relationships are
/// immutable facts keyed by OIDs, so this only ever adds or strengthens.
pub fn apply_squash_enrichment(snapshot: &mut GraphSnapshot, updates: &[GraphRelationship]) {
    let index = snapshot.commits.iter().enumerate()
        .map(|(position, commit)| (commit.oid.clone(), position))
        .collect::<HashMap<_, _>>();
    for relationship in updates {
        let mut endpoints = vec![relationship.destination_oid.as_str()];
        if relationship.source_oid != relationship.destination_oid {
            endpoints.push(relationship.source_oid.as_str());
        }
        for oid in endpoints {
            if let Some(&position) = index.get(oid) {
                merge_relationship(&mut snapshot.commits[position].relationships, relationship);
            }
        }
    }
}

fn merge_relationship(existing: &mut Vec<GraphRelationship>, incoming: &GraphRelationship) {
    if let Some(current) = existing.iter_mut().find(|r| r.key() == incoming.key()) {
        current.destination_refs.extend(incoming.destination_refs.iter().cloned());
        current.destination_refs.sort();
        current.destination_refs.dedup();
        current.matching = current.matching.stronger(incoming.matching);
        current.source_refs.extend(incoming.source_refs.iter().cloned());
        current.source_refs.sort();
        current.source_refs.dedup();
    } else {
        let mut relationship = incoming.clone();
        relationship.destination_refs.sort();
        relationship.destination_refs.dedup();
        relationship.source_refs.sort();
        relationship.source_refs.dedup();
        existing.push(relationship);
    }
}

pub fn compute_relationships(
    repo_path: &Path, snapshot: &GraphSnapshot, requested_base: Option<&str>,
    cache_root: &CacheRoot, cancel: &EnrichmentCancel,
) -> Vec<GraphRelationship> {
    let Some((base_branch, base_tip)) = resolve_base(repo_path, snapshot, requested_base) else {
        return Vec::new();
    };
    let mut cache = BranchCache::load_for_base(repo_path, &base_branch, cache_root);
    let mut relationships = compute_squash_relationships(repo_path, snapshot, &base_branch, &base_tip, &mut cache, cancel);
    cache.save();
    if !cancel.is_cancelled() {
        relationships.extend(compute_cherry_pick_relationships(repo_path, snapshot, &base_branch, &base_tip, &cache, cancel));
    }
    relationships
}

fn resolve_base_name(repo_path: &Path, requested_base: Option<&str>) -> Option<String> {
    if let Some(base) = requested_base {
        return Some(base.to_string());
    }
    let repo = git2::Repository::open(repo_path).ok()?;
    crate::git::branch::detect_base_branch(&repo, None).ok()
}
```

   - Factor `resolve_base` out of the base/base-tip resolution that both detectors duplicate. Share a `resolve_base_name(repo_path, requested_base) -> Option<String>` helper that returns the explicit option or opens the repository and calls `branch::detect_base_branch(&repo, None)`; detection then separately finds the displayed local-branch tip. L hydration uses the same name helper without requiring a displayed tip. A missing resolved name yields no results, never all bases.
   - `compute_squash_relationships` returns the A1 relationships directly.
   - Delete `GraphEnrichmentUpdate` and `merge_enrichment_updates`. Set `GraphEnrichmentMsg.updates: Vec<GraphRelationship>`.
   - `spawn_possible_squash_enrichment` sends `compute_relationships(...)`.
   - `load_graph_with_squash_annotations` is `load_graph`, then `compute_relationships` with `EnrichmentCancel::never()`, then `apply_squash_enrichment`.
   - `GraphState::apply_squash_enrichment` takes `&[GraphRelationship]`.
   - The drain line at `app.rs:703` is unchanged.
   - Remove the `is_cherry_picked_commit` field and add the accessors:

```rust
pub fn is_cherry_picked_commit(&self) -> bool {
    !self.cherry_pick_destinations().is_empty()
}
pub fn cherry_pick_destinations(&self) -> Vec<&GraphRelationship> {
    self.relationships.iter()
        .filter(|r| r.kind == RelationshipKind::CherryPick && r.source_oid == self.oid)
        .collect()
}
pub fn cherry_pick_sources(&self) -> Vec<&GraphRelationship> {
    self.relationships.iter()
        .filter(|r| r.kind == RelationshipKind::CherryPick && r.destination_oid == self.oid)
        .collect()
}
```

   - Readers change `.is_cherry_picked_commit` to `.is_cherry_picked_commit()`.
   - Fixtures delete `is_cherry_picked_commit: …`. A fixture that needs `true` uses a cherry relationship whose `source_oid` is the fixture's own OID.
   - Delete the cherry carry-forward lines at `src/app.rs:645` and `src/git/graph.rs:445`; `relationships` already carries the data.

- [ ] **Step 4: Update the existing apply/stale tests** at `tests/integration.rs:7229-7380`:
  - Convert their `GraphEnrichmentUpdate`s to `GraphRelationship` lists.
  - Where a test asserted that an empty update **clears** markers, assert that it **retains** them instead, with the comment `// enrichment never erases relationships`.
  - Switch the `graph::compute_possible_squash_updates(...)` call at `:7231` to `graph::compute_relationships(..., &graph::EnrichmentCancel::never())`.

- [ ] **Step 5: Run the tests to confirm they pass.** Run `cargo test`, then `cargo clippy --all-targets`, then `cargo build`.

- [ ] **Step 6: Commit.** `git commit -m "feat: carry Graph relationships end to end and merge enrichment without erasing"` (check `git diff --cached Cargo.toml` first).

### Task P3: Cherry-pick provenance on both endpoints

**Files:**
- Modify: `src/ui/info_modal.rs` (commit fields), `src/ui/commit_details.rs`
- Test: `src/ui/info_modal.rs` tests

- [ ] **Step 1: Write the failing test.** A destination commit with one `CherryPick` relationship, from a source OID not present in the snapshot with `source_refs = ["feature/gone"]`, renders the field `Cherry-picked From` = `<short source> (feature/gone)`. A source commit keeps its existing cherry glyph (`is_cherry_picked_commit()`).

- [ ] **Step 2: Run the test to confirm it fails.** Run `cargo test --lib -- info_modal`.

- [ ] **Step 3: Implement.** Find the module's short-OID helper with `rg -n "fn short_oid" src/ui`.

```rust
for relationship in commit.cherry_pick_sources() {
    fields.push(InfoField {
        label: "Cherry-picked From",
        value: format!("{} ({})", short_oid(&relationship.source_oid), relationship.source_refs.join(", ")),
    });
}
```

   Add the same line to `commit_details.rs`, styled like its fuzzy line (`theme.modal_secondary` / `theme.modal_commit`).

- [ ] **Step 4: Run the tests to confirm they pass.** Run `cargo test`, then `cargo build`.

- [ ] **Step 5: Commit.** `git commit -m "feat: show cherry-pick provenance on Graph destination commits"`.

---

## Slice L — ct/graph-relationship-lineage

This is the lineage ticket (supersedes cancelled #053). Cut it from `main` after slice P merges. Its tasks are not merged individually: storage, write-back, and hydration ship together.

### Task L1: Durable lineage store, read-only open for hydration

**Files:**
- Create: `src/git/lineage.rs`
- Modify: `src/git/mod.rs` (`pub mod lineage;`), `src/git/cache.rs` (make `prune_stale_caches_at` `pub(crate)`)
- Test: `src/git/lineage.rs` tests

**Interfaces:**
- Produces:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineageRoot { path: PathBuf }
impl LineageRoot {
    pub fn from_env() -> Self;
    pub fn at(path: impl Into<PathBuf>) -> Self;
    pub fn path(&self) -> &Path;
}
pub struct LineageStore { path: PathBuf }
impl LineageStore {
    /// Writer: creates the root, schema, and meta row. Enrichment worker only.
    pub fn open(repo_path: &Path, root: &LineageRoot) -> Option<Self>;
    /// Reader: resolves the path only. No directory creation, DDL, or writes.
    pub fn open_read_only(repo_path: &Path, root: &LineageRoot) -> Option<Self>;
    pub fn path(&self) -> &Path;
    pub fn upsert(&self, relationships: &[GraphRelationship]) -> rusqlite::Result<()>;
    /// Read-only, 200 ms busy timeout; missing file or table => Ok(empty).
    pub fn lookup(&self, oids: &HashSet<String>) -> rusqlite::Result<Vec<GraphRelationship>>;
}
pub const LINEAGE_FILE_PREFIX: &str = "git-bm-lineage-";
```

- [ ] **Step 1: Write the failing tests** in `src/git/lineage.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::graph::{GraphRelationship, RelationshipKind, RelationshipMatch};
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git").args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args).current_dir(dir).status().unwrap();
        assert!(status.success(), "git {args:?}");
    }

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q", "-b", "main"]);
        git(dir.path(), &["commit", "-q", "--allow-empty", "-m", "init"]);
        dir
    }

    fn rel(kind: RelationshipKind, dest: char, source: char, refs: &[&str], matching: RelationshipMatch) -> GraphRelationship {
        GraphRelationship {
            kind,
            matching,
            destination_oid: dest.to_string().repeat(40),
            destination_refs: vec!["main".into()],
            source_oid: source.to_string().repeat(40),
            source_refs: refs.iter().map(|r| r.to_string()).collect(),
        }
    }

    fn oids(values: &[char]) -> HashSet<String> {
        values.iter().map(|c| c.to_string().repeat(40)).collect()
    }

    #[test]
    fn upsert_then_reopen_round_trips_records() {
        let repo = repo();
        let root_dir = tempfile::tempdir().unwrap();
        let root = LineageRoot::at(root_dir.path());
        let store = LineageStore::open(repo.path(), &root).unwrap();
        store.upsert(&[
            rel(RelationshipKind::SquashMerge, 'a', 'b', &["feature/x"], RelationshipMatch::Fuzzy { similarity_percent: 91 }),
            rel(RelationshipKind::CherryPick, 'c', 'd', &["feature/y"], RelationshipMatch::Exact),
        ]).unwrap();
        drop(store);
        let reader = LineageStore::open_read_only(repo.path(), &root).unwrap();
        let mut found = reader.lookup(&oids(&['a', 'd'])).unwrap();
        found.sort_by(|l, r| l.kind.cmp(&r.kind));
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].matching, RelationshipMatch::Fuzzy { similarity_percent: 91 });
        assert_eq!(found[1].kind, RelationshipKind::CherryPick);
        assert_eq!(found[1].matching, RelationshipMatch::Exact);
    }

    #[test]
    fn repeated_upsert_does_not_duplicate_and_keeps_strongest_match() {
        let repo = repo();
        let root = tempfile::tempdir().unwrap();
        let store = LineageStore::open(repo.path(), &LineageRoot::at(root.path())).unwrap();
        let fuzzy = rel(RelationshipKind::SquashMerge, 'a', 'b', &["feature/x"], RelationshipMatch::Fuzzy { similarity_percent: 80 });
        store.upsert(&[fuzzy.clone()]).unwrap();
        store.upsert(&[fuzzy.clone()]).unwrap();
        store.upsert(&[GraphRelationship { matching: RelationshipMatch::Fuzzy { similarity_percent: 90 }, ..fuzzy.clone() }]).unwrap();
        store.upsert(&[GraphRelationship { matching: RelationshipMatch::Fuzzy { similarity_percent: 70 }, ..fuzzy }]).unwrap();
        let found = store.lookup(&oids(&['a'])).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].matching, RelationshipMatch::Fuzzy { similarity_percent: 90 });
    }

    #[test]
    fn shared_tip_keeps_all_source_names() {
        let repo = repo();
        let root = tempfile::tempdir().unwrap();
        let store = LineageStore::open(repo.path(), &LineageRoot::at(root.path())).unwrap();
        store.upsert(&[rel(RelationshipKind::SquashMerge, 'a', 'b', &["feature/one"], RelationshipMatch::Exact)]).unwrap();
        store.upsert(&[rel(RelationshipKind::SquashMerge, 'a', 'b', &["feature/two"], RelationshipMatch::Exact)]).unwrap();
        let found = store.lookup(&oids(&['b'])).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].source_refs, ["feature/one", "feature/two"]);
    }

    #[test]
    fn stale_and_concurrent_upserts_union_both_ref_sets() {
        let repo = repo();
        let root = tempfile::tempdir().unwrap();
        let store = LineageStore::open(repo.path(), &LineageRoot::at(root.path())).unwrap();
        let original = rel(RelationshipKind::SquashMerge, 'a', 'b', &["feature/one"], RelationshipMatch::Exact);
        store.upsert(&[original.clone()]).unwrap();
        let newer = GraphRelationship {
            destination_refs: vec!["develop".into()],
            source_refs: vec!["feature/two".into()],
            ..original.clone()
        };
        std::thread::scope(|scope| {
            let store = &store;
            let original = &original;
            let newer = &newer;
            scope.spawn(move || store.upsert(&[newer.clone()]).unwrap());
            scope.spawn(move || store.upsert(&[original.clone()]).unwrap());
        });
        store.upsert(&[original]).unwrap(); // late stale writer must not erase newer refs
        let found = store.lookup(&oids(&['a'])).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].destination_refs, ["develop", "main"]);
        assert_eq!(found[0].source_refs, ["feature/one", "feature/two"]);
        assert_eq!(found[0].matching, RelationshipMatch::Exact);
    }

    #[test]
    fn linked_worktrees_share_and_clones_are_isolated() {
        let repo = repo();
        let root_dir = tempfile::tempdir().unwrap();
        let root = LineageRoot::at(root_dir.path());
        let worktree = repo.path().join("wt");
        git(repo.path(), &["worktree", "add", "-q", "-b", "wt", worktree.to_str().unwrap()]);
        let clone_parent = tempfile::tempdir().unwrap();
        let clone = clone_parent.path().join("clone");
        git(clone_parent.path(), &["clone", "-q", repo.path().to_str().unwrap(), clone.to_str().unwrap()]);

        let main_store = LineageStore::open(repo.path(), &root).unwrap();
        let wt_store = LineageStore::open(&worktree, &root).unwrap();
        let clone_store = LineageStore::open(&clone, &root).unwrap();
        assert_eq!(main_store.path(), wt_store.path());
        assert_ne!(main_store.path(), clone_store.path());

        main_store.upsert(&[rel(RelationshipKind::CherryPick, 'a', 'b', &["x"], RelationshipMatch::Exact)]).unwrap();
        assert_eq!(wt_store.lookup(&oids(&['a'])).unwrap().len(), 1);
        assert!(clone_store.lookup(&oids(&['a'])).unwrap().is_empty());
    }

    #[test]
    fn transient_cache_prune_and_clear_leave_lineage() {
        let repo = repo();
        let shared = tempfile::tempdir().unwrap();
        // Same directory for both roots: the pruner must not match lineage files.
        let store = LineageStore::open(repo.path(), &LineageRoot::at(shared.path())).unwrap();
        store.upsert(&[rel(RelationshipKind::SquashMerge, 'a', 'b', &["x"], RelationshipMatch::Exact)]).unwrap();
        let cache_root = crate::git::cache::CacheRoot::at(shared.path());
        let mut cache = crate::git::cache::BranchCache::load_for_base(repo.path(), "main", &cache_root);
        cache.save();
        cache.clear();
        let far_future = std::time::SystemTime::now() + std::time::Duration::from_secs(400 * 24 * 60 * 60);
        crate::git::cache::prune_stale_caches_at(&cache_root, far_future);
        assert!(store.path().exists());
        assert_eq!(store.lookup(&oids(&['a'])).unwrap().len(), 1);
    }

    #[test]
    fn read_only_open_creates_nothing() {
        let repo = repo();
        let parent = tempfile::tempdir().unwrap();
        let root = LineageRoot::at(parent.path().join("never-created"));
        let reader = LineageStore::open_read_only(repo.path(), &root).unwrap();
        assert!(reader.lookup(&oids(&['a'])).unwrap().is_empty());
        assert!(!root.path().exists(), "read-only open must not create the root");
    }

    #[test]
    fn non_repository_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        assert!(LineageStore::open(dir.path(), &LineageRoot::at(root.path())).is_none());
        assert!(LineageStore::open_read_only(dir.path(), &LineageRoot::at(root.path())).is_none());
    }
}
```

- [ ] **Step 2: Run the tests to confirm they fail.** Run `cargo test --lib -- lineage::`. Expected: compile errors.

- [ ] **Step 3: Implement `src/git/lineage.rs`.**

```rust
//! Durable, repository-scoped squash/cherry-pick lineage. Unlike `BranchCache`,
//! this database is never pruned or cleared: it holds history that cannot be
//! recomputed once source refs are deleted and Git prunes their objects.

use crate::git::graph::{GraphRelationship, RelationshipKind, RelationshipMatch};
use rusqlite::{params, Connection, OpenFlags};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const LINEAGE_FILE_PREFIX: &str = "git-bm-lineage-";
const LOOKUP_CHUNK: usize = 400;
const READ_BUSY_TIMEOUT: Duration = Duration::from_millis(200);
const WRITE_BUSY_TIMEOUT: Duration = Duration::from_secs(5);

const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS lineage (
        type               TEXT NOT NULL CHECK (type IN ('sm', 'c-p')),
        destination_hash   TEXT NOT NULL,
        destination_ref    TEXT NOT NULL,
        source_hash        TEXT NOT NULL,
        source_ref         TEXT NOT NULL,
        similarity_percent INTEGER,
        detected_at        TEXT NOT NULL,
        updated_at         TEXT NOT NULL,
        PRIMARY KEY (type, destination_hash, source_hash)
    );
    CREATE INDEX IF NOT EXISTS lineage_destination ON lineage(destination_hash);
    CREATE INDEX IF NOT EXISTS lineage_source ON lineage(source_hash);
    CREATE TABLE IF NOT EXISTS lineage_source_ref (
        type             TEXT NOT NULL,
        destination_hash TEXT NOT NULL,
        source_hash      TEXT NOT NULL,
        ref_name         TEXT NOT NULL,
        PRIMARY KEY (type, destination_hash, source_hash, ref_name)
    );
    CREATE TABLE IF NOT EXISTS lineage_destination_ref (
        type             TEXT NOT NULL,
        destination_hash TEXT NOT NULL,
        source_hash      TEXT NOT NULL,
        ref_name         TEXT NOT NULL,
        PRIMARY KEY (type, destination_hash, source_hash, ref_name)
    );
    CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineageRoot {
    path: PathBuf,
}

impl LineageRoot {
    pub fn from_env() -> Self {
        let path = std::env::var_os("GBM_DATA_DIR").map(PathBuf::from).unwrap_or_else(|| {
            dirs::data_dir().unwrap_or_else(std::env::temp_dir).join("git-branch-manager")
        });
        Self { path }
    }
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
}

pub struct LineageStore {
    path: PathBuf,
}

fn common_dir_identity(repo_path: &Path) -> Option<(String, String)> {
    let repo = git2::Repository::open(repo_path).ok()?;
    let common_dir = fs::canonicalize(repo.commondir()).unwrap_or_else(|_| repo.commondir().to_path_buf());
    let text = common_dir.to_string_lossy().into_owned();
    // A content hash is stable across Rust releases; DefaultHasher is not.
    let identity = git2::Oid::hash_object(git2::ObjectType::Blob, text.as_bytes()).ok()?;
    Some((text, identity.to_string()))
}

impl LineageStore {
    pub fn open(repo_path: &Path, root: &LineageRoot) -> Option<Self> {
        let (common_dir, identity) = common_dir_identity(repo_path)?;
        fs::create_dir_all(root.path()).ok()?;
        let store = Self { path: root.path().join(format!("{LINEAGE_FILE_PREFIX}{identity}.sqlite3")) };
        let conn = store.connect_writer().ok()?;
        conn.execute("INSERT OR IGNORE INTO meta(key, value) VALUES ('common_dir', ?1)", params![common_dir]).ok()?;
        Some(store)
    }

    pub fn open_read_only(repo_path: &Path, root: &LineageRoot) -> Option<Self> {
        let (_, identity) = common_dir_identity(repo_path)?;
        Some(Self { path: root.path().join(format!("{LINEAGE_FILE_PREFIX}{identity}.sqlite3")) })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn connect_writer(&self) -> rusqlite::Result<Connection> {
        let conn = Connection::open(&self.path)?;
        conn.busy_timeout(WRITE_BUSY_TIMEOUT)?;
        // Mirrors cache.rs::open_conn: a failed WAL switch must not block the store.
        let _ = conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;");
        conn.execute_batch(SCHEMA)?;
        Ok(conn)
    }

    pub fn upsert(&self, relationships: &[GraphRelationship]) -> rusqlite::Result<()> {
        if relationships.is_empty() {
            return Ok(());
        }
        let mut conn = self.connect_writer()?;
        let tx = conn.transaction()?;
        let now = chrono::Utc::now().to_rfc3339();
        for relationship in relationships {
            let similarity = match relationship.matching {
                RelationshipMatch::Exact => None,
                RelationshipMatch::Fuzzy { similarity_percent } => Some(similarity_percent),
            };
            let primary_ref = relationship.source_refs.iter().min().cloned().unwrap_or_default();
            let primary_destination = relationship.destination_refs.iter().min().cloned().unwrap_or_default();
            tx.execute(
                "INSERT INTO lineage (type, destination_hash, destination_ref, source_hash, source_ref,
                                      similarity_percent, detected_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
                 ON CONFLICT (type, destination_hash, source_hash) DO UPDATE SET
                     destination_ref = CASE WHEN lineage.destination_ref = '' THEN excluded.destination_ref
                                            ELSE lineage.destination_ref END,
                     source_ref = CASE WHEN lineage.source_ref = '' THEN excluded.source_ref
                                       ELSE lineage.source_ref END,
                     similarity_percent = CASE
                         WHEN lineage.similarity_percent IS NULL OR excluded.similarity_percent IS NULL THEN NULL
                         ELSE MAX(lineage.similarity_percent, excluded.similarity_percent) END,
                     updated_at = excluded.updated_at",
                params![
                    relationship.kind.code(),
                    relationship.destination_oid,
                    primary_destination,
                    relationship.source_oid,
                    primary_ref,
                    similarity,
                    now,
                ],
            )?;
            for name in &relationship.source_refs {
                tx.execute(
                    "INSERT OR IGNORE INTO lineage_source_ref (type, destination_hash, source_hash, ref_name)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![relationship.kind.code(), relationship.destination_oid, relationship.source_oid, name],
                )?;
            }
            for name in &relationship.destination_refs {
                tx.execute(
                    "INSERT OR IGNORE INTO lineage_destination_ref (type, destination_hash, source_hash, ref_name)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![relationship.kind.code(), relationship.destination_oid, relationship.source_oid, name],
                )?;
            }
        }
        tx.commit()
    }

    pub fn lookup(&self, oids: &HashSet<String>) -> rusqlite::Result<Vec<GraphRelationship>> {
        if oids.is_empty() || !self.path.exists() {
            return Ok(Vec::new());
        }
        let conn = Connection::open_with_flags(&self.path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        conn.busy_timeout(READ_BUSY_TIMEOUT)?;
        let oids = oids.iter().collect::<Vec<_>>();
        let mut found = std::collections::HashMap::<(String, String, String), GraphRelationship>::new();
        for chunk in oids.chunks(LOOKUP_CHUNK) {
            let placeholders = vec!["?"; chunk.len()].join(",");
            let sql = format!(
                "SELECT l.type, l.destination_hash, l.destination_ref, l.source_hash, l.source_ref,
                        l.similarity_percent,
                        (SELECT group_concat(s.ref_name, char(10)) FROM lineage_source_ref s
                         WHERE s.type = l.type AND s.destination_hash = l.destination_hash AND s.source_hash = l.source_hash),
                        (SELECT group_concat(d.ref_name, char(10)) FROM lineage_destination_ref d
                         WHERE d.type = l.type AND d.destination_hash = l.destination_hash AND d.source_hash = l.source_hash)
                 FROM lineage l
                 WHERE l.destination_hash IN ({placeholders}) OR l.source_hash IN ({placeholders})"
            );
            // A database without the table yet (another writer mid-create) is "no relationships".
            let Ok(mut statement) = conn.prepare(&sql) else { return Ok(Vec::new()) };
            let bound = chunk.iter().chain(chunk.iter()).map(|oid| oid.as_str());
            let rows = statement.query_map(rusqlite::params_from_iter(bound), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<u8>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                ))
            })?;
            for row in rows {
                let (code, destination, destination_ref, source, source_ref, similarity, source_names, destination_names) = row?;
                let Some(kind) = RelationshipKind::from_code(&code) else { continue };
                let mut source_refs = source_names
                    .map(|joined| joined.split('\n').map(str::to_string).collect::<Vec<_>>())
                    .unwrap_or_default();
                if !source_ref.is_empty() {
                    source_refs.push(source_ref);
                }
                source_refs.sort();
                source_refs.dedup();
                let mut destination_refs = destination_names
                    .map(|joined| joined.split('\n').map(str::to_string).collect::<Vec<_>>())
                    .unwrap_or_default();
                if !destination_ref.is_empty() {
                    destination_refs.push(destination_ref);
                }
                destination_refs.sort();
                destination_refs.dedup();
                let matching = similarity
                    .map(|similarity_percent| RelationshipMatch::Fuzzy { similarity_percent })
                    .unwrap_or(RelationshipMatch::Exact);
                found.insert(
                    (code, destination.clone(), source.clone()),
                    GraphRelationship { kind, matching, destination_oid: destination, destination_refs, source_oid: source, source_refs },
                );
            }
        }
        Ok(found.into_values().collect())
    }
}
```

   Register `pub mod lineage;` in `src/git/mod.rs`. Make `prune_stale_caches_at` in `cache.rs` `pub(crate)`. `cache_database_name` (`cache.rs:142`) accepts only the `git-bm-repo-cache-` and `git-bm-cache-` prefixes, and the shared-directory test pins that.

   The two correlated aggregate subqueries deliberately avoid joining both ref child tables, which would multiply names. The main row keeps its primary captured ref snapshots; stale or concurrent writers can only add observations to child tables, never replace base membership. The upsert transaction covers the row and both unions. Keep the existing deleted/pruned-source and transient-clear tests, and extend them to relationships with both `main` and `develop` destination snapshots.

- [ ] **Step 4: Run the tests to confirm they pass.** Run `cargo test --lib -- lineage::`, then `cargo build`. Expected: all lineage tests pass.

- [ ] **Step 5: Commit** on the slice branch: `git commit -m "feat: add durable repository-scoped relationship lineage store"`. Check `git diff --cached Cargo.toml` first.

### Task L2: Write detected relationships to the lineage store

**Files:**
- Modify: `src/git/graph.rs`: `GraphLoadOptions` and its `Default`; `spawn_possible_squash_enrichment`; a new `pub(crate) fn finish_enrichment`; `load_graph_with_squash_annotations`
- Modify: `src/app.rs`: `lineage_root` field; the `with_cache_root` signature; `spawn_graph_load`; both spawn call sites; `:4983` updater options (`lineage_root: Some(self.lineage_root.clone())`)
- Modify: `src/main.rs` (pass `LineageRoot::from_env()`)
- Modify: `tests/integration.rs` (`TestDir` lineage root, `preserve_tempdirs` returning a 3-tuple, `GBM_DATA_DIR`, and the `lineage` import)
- Test: `src/git/graph.rs` tests, `tests/integration.rs`

**Interfaces:**
- Produces:

```rust
pub struct GraphLoadOptions { /* existing */ pub lineage_root: Option<LineageRoot> }   // Default: None
pub fn spawn_possible_squash_enrichment(
    snapshot: GraphSnapshot, repo_path: PathBuf, requested_base: Option<String>, generation: u64,
    cache_root: CacheRoot, lineage_root: Option<LineageRoot>, latest_generation: Arc<AtomicU64>,
) -> Receiver<GraphEnrichmentMsg>;
/// Persist, then publish only if still current. Split out so the order is testable.
pub(crate) fn finish_enrichment(
    repo_path: &Path, lineage_root: Option<&LineageRoot>, generation: u64,
    updates: Vec<GraphRelationship>, cancel: &EnrichmentCancel, tx: &mpsc::Sender<GraphEnrichmentMsg>,
);
```

- [ ] **Step 1: Write the failing tests.**

   **Deterministic staleness test** in `src/git/graph.rs`. The generation moves after detection and before publication:

```rust
#[test]
fn stale_enrichment_persists_but_does_not_publish() {
    let dir = tempfile::tempdir().unwrap();
    assert!(std::process::Command::new("git").args(["init", "-q", "-b", "main"]).current_dir(dir.path()).status().unwrap().success());
    let root_dir = tempfile::tempdir().unwrap();
    let root = LineageRoot::at(root_dir.path());
    let relationship = GraphRelationship {
        kind: RelationshipKind::SquashMerge,
        matching: RelationshipMatch::Exact,
        destination_oid: "d".repeat(40),
        destination_refs: vec!["main".into()],
        source_oid: "a".repeat(40),
        source_refs: vec!["feature/x".into()],
    };
    let latest = Arc::new(AtomicU64::new(1));
    let cancel = EnrichmentCancel::new(Arc::clone(&latest), 1);
    latest.store(2, Ordering::Release);     // a newer load started after detection finished
    let (tx, rx) = mpsc::channel();
    finish_enrichment(dir.path(), Some(&root), 1, vec![relationship.clone()], &cancel, &tx);
    drop(tx);
    assert!(rx.recv().is_err(), "stale result is not published");
    let store = LineageStore::open_read_only(dir.path(), &root).unwrap();
    assert_eq!(store.lookup(&[relationship.destination_oid.clone()].into_iter().collect()).unwrap(), [relationship]);
}
```

   **Integration test.** Real detection persists. This test also hosts the out-of-window cherry test, which needs the store:

```rust
#[test]
fn enrichment_persists_detected_relationships() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();
    run_git(dir, &["checkout", "-q", "-b", "feature/sq"]);
    std::fs::write(dir.join("a.txt"), "a").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-q", "-m", "a"]);
    run_git(dir, &["checkout", "-q", "main"]);
    advance_main(dir, "sq");
    run_git(dir, &["merge", "-q", "--squash", "feature/sq"]);
    run_git(dir, &["commit", "-q", "-m", "squash sq"]);
    let destination = rev(dir, "HEAD");

    graph::load_graph_with_squash_annotations(dir, tmpdir.graph_options_for("main")).unwrap();
    let store = lineage::LineageStore::open_read_only(dir, &tmpdir.lineage_root()).unwrap();
    let stored = store.lookup(&[destination.clone()].into_iter().collect()).unwrap();
    assert!(stored.iter().any(|r| r.kind == graph::RelationshipKind::SquashMerge && r.destination_oid == destination));
}

#[test]
fn cherry_pick_destination_outside_displayed_window_is_found() {
    // Source, destination, and merge base are all older than the 10-commit window;
    // only the branch tip (a later commit) is displayed.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();
    run_git(dir, &["checkout", "-q", "-b", "feature/early"]);
    std::fs::write(dir.join("e.txt"), "e").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-q", "-m", "e"]);
    let source = rev(dir, "HEAD");
    run_git(dir, &["checkout", "-q", "main"]);
    advance_main(dir, "early");
    run_git(dir, &["cherry-pick", &source]);
    let destination = rev(dir, "HEAD");
    for i in 0..30 {
        run_git(dir, &["commit", "-q", "--allow-empty", "-m", &format!("filler {i}")]);
    }
    run_git(dir, &["checkout", "-q", "feature/early"]);
    std::fs::write(dir.join("late.txt"), "late").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-q", "-m", "late tip"]);
    run_git(dir, &["checkout", "-q", "main"]);

    let mut options = tmpdir.graph_options_for("main");
    options.max_count = 10;
    let snapshot = graph::load_graph_with_squash_annotations(dir, options).unwrap();
    assert!(snapshot.commits.iter().all(|c| c.oid != destination), "fixture: destination outside window");
    let store = lineage::LineageStore::open_read_only(dir, &tmpdir.lineage_root()).unwrap();
    let stored = store.lookup(&[source.clone()].into_iter().collect()).unwrap();
    assert!(stored.iter().any(|r| r.kind == graph::RelationshipKind::CherryPick
        && r.source_oid == source && r.destination_oid == destination));
}
```

   **Test scaffolding in `tests/integration.rs`:**
   - Add `lineage` to the `use git_branch_manager::git::{…}` import list.
   - Add a `lineage_root: Option<tempfile::TempDir>` field to `TestDir`, created in `new`, with an accessor `fn lineage_root(&self) -> lineage::LineageRoot`.
   - `graph_options()` sets `options.lineage_root = Some(self.lineage_root())`.
   - `manager_command` adds `command.env("GBM_DATA_DIR", test_dir.lineage_root().path())`.
   - `preserve_tempdirs` now returns `(Option<PathBuf>, Option<PathBuf>, Option<PathBuf>)` (repo, cache, lineage). Update `Drop` to print the kept lineage path.
   - Update `testdir_preserves_repo_and_cache_roots_when_requested` (`tests/integration.rs:129`) to destructure three values, assert that the lineage path is kept, and `remove_dir_all` it.

- [ ] **Step 2: Run the tests to confirm they fail.** Run `cargo test -- stale_enrichment_persists enrichment_persists_detected cherry_pick_destination_outside testdir_preserves`. Expected: compile errors.

- [ ] **Step 3: Implement.**

```rust
pub(crate) fn finish_enrichment(
    repo_path: &Path, lineage_root: Option<&LineageRoot>, generation: u64,
    updates: Vec<GraphRelationship>, cancel: &EnrichmentCancel, tx: &mpsc::Sender<GraphEnrichmentMsg>,
) {
    // Persist before the staleness check: accepted findings are complete tuples
    // that remain true even when the UI no longer wants this generation.
    if let Some(root) = lineage_root {
        persist_relationships(repo_path, root, &updates);
    }
    if cancel.is_cancelled() {
        return;
    }
    let _ = tx.send(GraphEnrichmentMsg { generation, updates });
}

fn persist_relationships(repo_path: &Path, root: &LineageRoot, relationships: &[GraphRelationship]) {
    let _span = tracing::info_span!("lineage_persist", count = relationships.len()).entered();
    let Some(store) = LineageStore::open(repo_path, root) else { return };
    if let Err(error) = store.upsert(relationships) {
        tracing::warn!(%error, "lineage upsert failed");
    }
}
```

   1. **`spawn_possible_squash_enrichment`:** gains `lineage_root: Option<LineageRoot>` before `latest_generation`. Its thread body is `let updates = compute_relationships(...); finish_enrichment(&repo_path, lineage_root.as_ref(), generation, updates, &cancel, &tx);`.
   2. **`load_graph_with_squash_annotations`:** after `compute_relationships`, call `persist_relationships` when `options.lineage_root` is `Some`, then apply.
   3. **App, field:** add `lineage_root: lineage::LineageRoot`.
   4. **App, constructor:** `with_cache_root` gains a trailing `lineage_root: lineage::LineageRoot` parameter. `main.rs` passes `lineage::LineageRoot::from_env()`, and `App::new` (test) passes `LineageRoot::at(test_cache_root.path().join("lineage"))`.
   5. **App, call sites:** both spawn call sites pass `Some(self.lineage_root.clone())` before the signal. `spawn_graph_load` and the updater options at `:4983` both set `lineage_root: Some(self.lineage_root.clone())`. L3 hydrates the updater's result before its worker sends it.
   6. **Remaining `GraphLoadOptions { … }` literals:** count them with `rg -n "GraphLoadOptions \{" src tests | rg -v "\.\.(graph::)?GraphLoadOptions::default\(\)"`. Then either add `lineage_root: None,` to each, or convert a literal to `..graph::GraphLoadOptions::default()` when it already sets `cache_root` explicitly. Choose per site to minimize edits, but **never** let a literal that relies on `Default` pick up a real lineage root: `Default` is `None`, so this is safe by construction.

- [ ] **Step 4: Run the tests to confirm they pass.** Run `cargo test`, then `cargo clippy --all-targets`, then `cargo build`.

- [ ] **Step 5: Commit.** `git commit -m "feat: persist detected Graph relationships to the lineage store"`.

### Task L3: Hydrate before publishing the structural snapshot

**Files:**
- Modify: `src/git/graph.rs` (`spawn_graph_loader`, `spawn_graph_updater:61`, `load_graph_with_squash_annotations`, new `hydrate_lineage`)
- Modify: `src/app.rs:636-648` (merge latest in-memory metadata into worker-hydrated relationships)
- Test: `tests/integration.rs`, `src/app.rs` tests

**Interfaces:**
- Produces: `pub fn hydrate_lineage(repo_path: &Path, snapshot: &mut GraphSnapshot, options: &GraphLoadOptions);`

- [ ] **Step 1: Write the failing tests** in `tests/integration.rs`:

```rust
fn squash_fixture(tmpdir: &TestDir) -> (String, String) {
    let dir = tmpdir.path();
    run_git(dir, &["checkout", "-q", "-b", "feature/hydrate"]);
    std::fs::write(dir.join("h.txt"), "h").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-q", "-m", "h"]);
    let source = rev(dir, "HEAD");
    run_git(dir, &["checkout", "-q", "main"]);
    advance_main(dir, "hydrate");
    run_git(dir, &["merge", "-q", "--squash", "feature/hydrate"]);
    run_git(dir, &["commit", "-q", "-m", "squash hydrate"]);
    (source, rev(dir, "HEAD"))
}

#[test]
fn warm_load_hydrates_without_running_detection() {
    let (tmpdir, _repo) = setup_test_repo();
    let (source, destination) = squash_fixture(&tmpdir);
    graph::load_graph_with_squash_annotations(tmpdir.path(), tmpdir.graph_options_for("main")).unwrap();
    // A structural load alone (no enrichment at all) now carries the markers.
    let snapshot = graph::spawn_graph_loader(tmpdir.path().to_path_buf(), tmpdir.graph_options_for("main"))
        .recv().unwrap().unwrap();
    let commit = snapshot.commits.iter().find(|c| c.oid == destination).unwrap();
    assert!(commit.is_possible_squash_merge());
    assert_eq!(commit.possible_squash_merge_sources(), ["feature/hydrate"]);
    let source_commit = snapshot.commits.iter().find(|c| c.oid == source).unwrap();
    assert!(source_commit.relationships.iter().any(|r| r.destination_oid == destination));
}

#[test]
fn hydrated_lineage_survives_deleted_ref_and_pruned_object() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();
    let (source, destination) = squash_fixture(&tmpdir);
    graph::load_graph_with_squash_annotations(dir, tmpdir.graph_options_for("main")).unwrap();
    run_git(dir, &["branch", "-D", "feature/hydrate"]);
    run_git(dir, &["reflog", "expire", "--expire=now", "--all"]);
    run_git(dir, &["gc", "-q", "--prune=now"]);
    let repo = git2::Repository::open(dir).unwrap();
    assert!(repo.find_commit(git2::Oid::from_str(&source).unwrap()).is_err(), "source object pruned");

    let snapshot = graph::load_graph_with_squash_annotations(dir, tmpdir.graph_options_for("main")).unwrap();
    let commit = snapshot.commits.iter().find(|c| c.oid == destination).unwrap();
    assert!(commit.is_possible_squash_merge());
    assert_eq!(commit.possible_squash_merge_sources(), ["feature/hydrate"]);
    assert!(commit.relationships.iter().any(|r| r.source_oid == source));
}

#[test]
fn hydration_skips_relationships_for_other_base() {
    let (tmpdir, _repo) = setup_test_repo();
    let (_, destination) = squash_fixture(&tmpdir);
    let store = lineage::LineageStore::open(tmpdir.path(), &tmpdir.lineage_root()).unwrap();
    store.upsert(&[graph::GraphRelationship {
        kind: graph::RelationshipKind::SquashMerge,
        matching: graph::RelationshipMatch::Exact,
        destination_oid: destination.clone(),
        destination_refs: vec!["develop".into()],
        source_oid: "1".repeat(40),
        source_refs: vec!["feature/elsewhere".into()],
    }]).unwrap();
    let snapshot = graph::spawn_graph_loader(tmpdir.path().to_path_buf(), tmpdir.graph_options_for("main"))
        .recv().unwrap().unwrap();
    let commit = snapshot.commits.iter().find(|c| c.oid == destination).unwrap();
    assert!(commit.relationships.iter().all(|r| r.destination_refs.iter().any(|name| name == "main")));
}

#[test]
fn unwritable_lineage_root_does_not_block_graph() {
    let (tmpdir, _repo) = setup_test_repo();
    let blocker = tmpdir.path().join("not-a-dir");
    std::fs::write(&blocker, "file").unwrap();     // create_dir_all fails beneath a file
    let mut options = tmpdir.graph_options_for("main");
    options.lineage_root = Some(lineage::LineageRoot::at(blocker.join("lineage")));
    assert!(graph::spawn_graph_loader(tmpdir.path().to_path_buf(), options.clone()).recv().unwrap().is_ok());
    assert!(graph::load_graph_with_squash_annotations(tmpdir.path(), options).is_ok());
}

#[test]
fn locked_lineage_does_not_delay_graph() {
    let (tmpdir, _repo) = setup_test_repo();
    let (_, destination) = squash_fixture(&tmpdir);
    graph::load_graph_with_squash_annotations(tmpdir.path(), tmpdir.graph_options_for("main")).unwrap();
    let store = lineage::LineageStore::open(tmpdir.path(), &tmpdir.lineage_root()).unwrap();
    let holder = rusqlite::Connection::open(store.path()).unwrap();
    holder.execute_batch("PRAGMA journal_mode=DELETE; BEGIN EXCLUSIVE;").unwrap();
    let started = std::time::Instant::now();
    let snapshot = graph::spawn_graph_loader(tmpdir.path().to_path_buf(), tmpdir.graph_options_for("main"))
        .recv().unwrap().unwrap();
    assert!(started.elapsed() < std::time::Duration::from_secs(3), "hydration gave up within its short timeout");
    assert!(snapshot.commits.iter().any(|c| c.oid == destination));
    holder.execute_batch("ROLLBACK;").unwrap();
}
```

   `locked_lineage_does_not_delay_graph` uses `BEGIN EXCLUSIVE` in rollback-journal mode, which blocks readers. In WAL mode, readers never block on an `IMMEDIATE` writer, so the journal-mode switch is what makes the lock bite. The 3-second ceiling allows for the structural load itself; the assertion's point is that the loader's lineage wait is far below `WRITE_BUSY_TIMEOUT`. If `PRAGMA journal_mode=DELETE` cannot switch because the writer left the database in WAL mode, first run `PRAGMA wal_checkpoint(TRUNCATE)` on `holder`, then switch.

   **Additional base-provenance tests:**
   - `same_pair_hydrates_under_both_bases`: create the squash fixture, point `develop` at the destination, and run real enrichment first with base `main`, then `develop`. Confirm one stored pair has `destination_refs == ["develop", "main"]`; structural warm loads with either explicit base must hydrate it. Delete the source ref, expire reflogs and prune its object, then repeat both warm loads and assert the OIDs and both ref snapshots remain.
   - `default_base_filters_mixed_lineage`: leave `GraphLoadOptions.base_branch = None`, derive the expected name through `branch::detect_base_branch(&repo, None)`, and store one relationship for that name and another relationship for a different base, both with displayed endpoints. Assert only the detected-base pair is hydrated. This must fail if `None` means all bases; do not rely on a fixed implicit base name.

   **Updater regression: `updater_refill_hydrates_before_publication`.** Use the existing incremental updater fixture and a small graph window. Persist a destination/source pair, remove the original source ref and prune its object, and keep a newer unrelated displayed ref so the historical destination is initially outside the window. Capture the initial state/snapshot, then remove that newer ref and create its `GraphRepositoryDelta`; the refill must bring the historical destination into the new window. Call `spawn_graph_updater` with the explicit temporary `lineage_root`, receive `GraphUpdateMsg` directly before any App drain or fresh detector, and assert the new destination already carries the historical OIDs/refs. Test App metadata carry-forward separately: a worker result contains hydrated relationship A while the current App snapshot contains relationship B for the same displayed OID; draining retains both. Repeat with the same relationship key but different source/destination names and assert both unions remain. All Git/SQLite setup stays in test fixtures or workers; the App merge only handles owned records.

   **App-level test in `src/app.rs`: warm hydration while fresh detection is blocked.**

```rust
#[test]
fn hydrated_markers_publish_while_enrichment_is_cancelled() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    let git = |args: &[&str]| {
        assert!(std::process::Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@t"]).args(args)
            .current_dir(path).status().unwrap().success());
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["commit", "-q", "--allow-empty", "-m", "init"]);
    git(&["checkout", "-q", "-b", "feature/h"]);
    std::fs::write(path.join("h.txt"), "h").unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "h"]);
    git(&["checkout", "-q", "main"]);
    std::fs::write(path.join("b.txt"), "b").unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "b"]);
    git(&["merge", "-q", "--squash", "feature/h"]);
    git(&["commit", "-q", "-m", "squash h"]);
    let destination = String::from_utf8(std::process::Command::new("git").args(["rev-parse", "HEAD"])
        .current_dir(path).output().unwrap().stdout).unwrap().trim().to_string();

    let mut app = App::new(path.to_path_buf(), "main".into(), Config::default());
    let options = graph::GraphLoadOptions {
        base_branch: Some("main".into()),
        cache_root: app.cache_root.clone(),
        lineage_root: Some(app.lineage_root.clone()),
        ..graph::GraphLoadOptions::default()
    };
    graph::load_graph_with_squash_annotations(path, options).unwrap();

    app.spawn_graph_load(50, false);
    // Block fresh detection: every enrichment worker for this load sees itself cancelled.
    app.graph_generation_signal.store(u64::MAX, Ordering::Release);
    wait_until(|| { app.drain_channels(); app.graph.snapshot().is_some() });
    let commit = app.graph.snapshot().unwrap().commits.iter().find(|c| c.oid == destination).unwrap().clone();
    assert!(commit.is_possible_squash_merge(), "marker came from hydration, not enrichment");
}
```

- [ ] **Step 2: Run the tests to confirm they fail.** Run `cargo test -- warm_load_hydrates hydrated_lineage_survives hydration_skips same_pair_hydrates default_base_filters updater_refill_hydrates unwritable_lineage locked_lineage hydrated_markers_publish`. Expected: FAIL (no hydration).

- [ ] **Step 3: Implement.**

```rust
/// Hydrate stored relationships into a freshly loaded snapshot. Runs in the
/// loader/updater worker, read-only; any failure leaves existing annotations intact.
pub fn hydrate_lineage(repo_path: &Path, snapshot: &mut GraphSnapshot, options: &GraphLoadOptions) {
    let Some(root) = options.lineage_root.as_ref() else { return };
    let Some(base_branch) = resolve_base_name(repo_path, options.base_branch.as_deref()) else { return };
    let Some(store) = LineageStore::open_read_only(repo_path, root) else { return };
    let oids = snapshot.commits.iter().map(|commit| commit.oid.clone()).collect::<HashSet<_>>();
    let _span = tracing::info_span!("lineage_hydrate", oids = oids.len()).entered();
    let relationships = match store.lookup(&oids) {
        Ok(relationships) => relationships,
        Err(error) => {
            tracing::debug!(%error, "lineage lookup skipped");
            return;
        }
    };
    let relationships = relationships
        .into_iter()
        .filter(|r| r.destination_refs.iter().any(|name| name == &base_branch))
        .collect::<Vec<_>>();
    apply_squash_enrichment(snapshot, &relationships);
}

pub fn spawn_graph_loader(repo_path: PathBuf, options: GraphLoadOptions) -> Receiver<Result<GraphSnapshot, GraphLoadError>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = load_graph(&repo_path, options.clone()).map(|mut snapshot| {
            hydrate_lineage(&repo_path, &mut snapshot, &options);
            snapshot
        });
        let _ = tx.send(result);
    });
    rx
}
```

   In `load_graph_with_squash_annotations`, call `hydrate_lineage` right after `load_graph`.

   Add the same optional adapter in the existing `spawn_graph_updater` worker (`graph.rs:61`), without changing its channel or coupling lineage storage to topology internals:

```rust
let result = update_graph_incrementally(&repo_path, snapshot, &delta, options.clone())
    .map(|mut snapshot| {
        hydrate_lineage(&repo_path, &mut snapshot, &options);
        snapshot
    });
let _ = tx.send(GraphUpdateMsg { revision, options, delta, result });
```

   At the App carry-forward site (`app.rs:636-648`), replace A's assignment with `apply_squash_enrichment(&mut snapshot, &latest_relationships)`, where `latest_relationships` is collected from the current App snapshot under the existing `!self.graph_update_invalidates_enrichment` guard. P's merge handles duplicate endpoint records, both ref unions and stronger confidence. This is only in-memory work; never rehydrate in the App. Leave the backend carry-forward in `update_graph_incrementally` intact: it runs before worker hydration and validly preserves already loaded records. The adapter can be removed alongside the updater later, with no P003 dependency or lineage redesign.

```rust
if !self.graph_update_invalidates_enrichment {
    let latest_relationships = self.graph.snapshot().map(|current| {
        current.commits.iter()
            .flat_map(|commit| commit.relationships.iter().cloned())
            .collect::<Vec<_>>()
    }).unwrap_or_default();
    graph::apply_squash_enrichment(&mut snapshot, &latest_relationships);
}
```

- [ ] **Step 4: Run the tests to confirm they pass.** Run `cargo test`, then `cargo build`.

- [ ] **Step 5: Commit.** `git commit -m "feat: hydrate stored relationships before publishing the Graph snapshot"`.

### Task L4: Squash provenance on the source endpoint and the presentation boundary

**Files:**
- Modify: `src/ui/info_modal.rs`
- Test: `src/ui/info_modal.rs` tests, `tests/integration.rs`

- [ ] **Step 1: Write the failing tests.**
  - **`info_modal`, fuzzy:** a source commit with a fuzzy 91% `SquashMerge` relationship renders `Possibly Squash-merged Into` = `<short dest> (main, 91% similarity)`.
  - **`info_modal`, exact:** a source commit with an exact squash relationship renders `Squash-merged Into` = `<short dest> (main)`.
  - **Integration, `hydrated_fuzzy_lineage_does_not_change_branch_status`:**
    1. Create a real unmerged branch `feature/fuzzy` and record `test_branches(&tmpdir, &repo, "main")`'s `merge_status` for it as the baseline.
    2. Upsert a fuzzy 91% `SquashMerge` relationship whose destination is the `main` tip, with `source_refs = ["feature/fuzzy"]`.
    3. Call `test_branches` again and assert the status equals the baseline.
    4. Load the graph and assert the destination commit has `fuzzy_squash_match()` = `Some(FuzzySquashMatch { similarity_percent: 91 })` and `is_possible_squash_merge()` = `false`.

- [ ] **Step 2: Run the tests to confirm they fail.** Run `cargo test --lib -- info_modal`, then `cargo test -- hydrated_fuzzy_lineage`. The info-modal tests fail. The status test may already pass, which is expected: it pins a boundary.

- [ ] **Step 3: Implement**, after the cherry fields from P3:

```rust
for relationship in commit.relationships.iter()
    .filter(|r| r.kind == RelationshipKind::SquashMerge && r.source_oid == commit.oid)
{
    let (label, detail) = match relationship.matching {
        RelationshipMatch::Exact => ("Squash-merged Into", relationship.destination_refs.join(", ")),
        RelationshipMatch::Fuzzy { similarity_percent } => (
            "Possibly Squash-merged Into",
            format!("{}, {similarity_percent}% similarity", relationship.destination_refs.join(", ")),
        ),
    };
    fields.push(InfoField { label, value: format!("{} ({detail})", short_oid(&relationship.destination_oid)) });
}
```

   Graph row glyphs stay as they are. Add a source-details fixture with `destination_refs == ["develop", "main"]` and assert both names render once in sorted order. Source and destination colors are task #054 (depends on #065); connectors are #055 (depends on #065 and #054, already fixed).

- [ ] **Step 4: Run the tests to confirm they pass.** Run `cargo test`, then `cargo build`.

- [ ] **Step 5: Commit.** `git commit -m "feat: show squash provenance on Graph source commits"`.

### Task L5: Docs, timing, acceptance

**Files:**
- Modify: `CLAUDE.md` (Architecture → Git backend list, Data Flow step 1)
- Modify: `src/git/graph.rs` (spans)

- [ ] **Step 1: Add timing spans** on the default target with `tracing::info_span!(…).entered()`. L2 and L3 already added `lineage_persist` and `lineage_hydrate`; add these two:
  - `graph_squash_relationships` (fields `jobs`) around `compute_squash_relationships`
  - `graph_cherry_relationships` (fields `tips`, `destinations_scanned`, `incomplete`) around `compute_cherry_pick_relationships`

  The default subscriber (`src/main.rs:362-382`, filter `git_branch_manager=debug`) records `time.busy` and `time.idle`.

- [ ] **Step 2: Document.**
  - In the `CLAUDE.md` Git backend list, add `lineage`.
  - In Data Flow step 1, add: "Graph loader and updater workers hydrate stored squash and cherry-pick relationships for the resolved base from the durable lineage store (`git::lineage`, under the data directory, never pruned or cleared by `R`) before publishing; enrichment upserts new relationships and only ever adds to them."

- [ ] **Step 3: Measure on this repository.** Record the results in the commit body.
  1. Run `cargo build`. Create one new temporary benchmark directory with separate `data` and `cache` subdirectories. Set explicit `GBM_DATA_DIR=<temporary>/data` and `GBM_CACHE_DIR=<temporary>/cache` for both runs, plus `GBM_DEBUG=<temporary>/cold.log` or `<temporary>/warm.log` to capture separate logs outside those stores. Do not delete or modify the user's real durable lineage files.
  2. Run `cargo run` with those isolated roots, open Graph, and note the time until markers appear. The initially empty roots make this the cold case.
  3. Quit and run `cargo run` again with the same isolated roots. Markers should appear with the structural snapshot. This is the warm case.
  4. Record `time.busy` for the four spans in both runs from the debug log.
  5. State honestly that warm markers are immediate only for *stored* relationships.

- [ ] **Step 4: Full verification.** Run `cargo test`, `cargo clippy --all-targets`, and `cargo build`. `git diff --cached Cargo.toml` must show no version line.

- [ ] **Step 5: Commit.** `git commit -m "docs: document relationship lineage and add enrichment timing spans"`.

**Slice L acceptance:** every requirement in the trace table is covered. Squash-merge into `main`, then make a separate version-bump commit (minor: new feature).

---

## Slice D — ct/graph-fuzzy-pair-cache

Cut this from `main` after slice A merges.

### Task D1: Skip already-scored fuzzy pairs

**Files:**
- Modify: `src/git/cache.rs` (the `fuzzy_pair` table and on-demand methods)
- Modify: `src/git/graph.rs` (fuzzy loop)
- Modify: `src/git/fuzzy_match.rs`: add `pub const FUZZY_SCORING_VERSION: u32 = 1;` with the doc line "bump whenever `score`, `classify`, or the thresholds change"
- Test: `src/git/cache.rs`, `src/git/graph.rs`, `tests/integration.rs`

**Interfaces:**
- Consumes: A1 `SourceTip`.
- Produces: `BranchCache::lookup_fuzzy_pairs(&self, keys: &[String]) -> HashMap<String, Option<u8>>` and `BranchCache::store_fuzzy_pairs(&self, entries: &[(String, Option<u8>)])`. The key is `format!("{destination}:{merge_base}:{tip}:v{FUZZY_SCORING_VERSION}")`.

- [ ] **Step 1: Write the failing tests.**
  - **`cache.rs`, round trip:** store and look up a set that includes `None` (scored, no match).
  - **`cache.rs`, missing table:** define a fixture here independently of P: create a temporary SQLite file with only `CREATE TABLE branch_cache (base_branch TEXT, branch_name TEXT, merge_status TEXT, commit_hash TEXT);`, close the connection, and use `BranchCache::load_from_path` on it. Looking up fuzzy keys returns an empty map and creates no table. P need not exist or have merged.
  - **Graph regression, `fuzzy_pair_cache_skips_rescoring_and_preserves_result`:**
    1. Use the scenario-20 fuzzy fixture plus an unrelated textual source/base pair that classifies to `None`. Load the graph and run the squash detector twice with the same temporary cache root through a local test helper accepting an injected scoring closure.
    2. After the first run, open the cache file with `rusqlite::Connection::open` (find it with `std::fs::read_dir(tmpdir.cache_root())`).
    3. Assert the first scoring closure was called on misses, and `fuzzy_pair` has one row per eligible (base commit, diverged tip) pair, including a `NULL` percent. Exact pairs are excluded, consistently with A.
    4. Reset the local count for the second run and assert **zero score calls**, including for the cached `None` pair; also assert identical relationship records, `fuzzy_squash_match()` and sources. Result equality and row counts alone are insufficient proof that scoring was skipped.

- [ ] **Step 2: Run the tests to confirm they fail.** Run `cargo test -- fuzzy_pair`. Expected: compile errors.

- [ ] **Step 3: Implement.**
  - Add `CREATE TABLE IF NOT EXISTS fuzzy_pair (key TEXT PRIMARY KEY, percent INTEGER)` to `ensure_schema`.
  - Write the two methods with the same structure as P1's `lookup_commit_patch_ids` and `store_commit_patch_ids`: open their own connection, chunk by 400, and treat a missing table as misses. If P has not merged yet, write them out in full; this slice does not depend on P.
  - In the fuzzy loop, build all eligible pair keys after A's exact-pair suppression, call `lookup_fuzzy_pairs` once, use stored values for hits (`Some(&None)` means scored/no match), score only absent keys, and call `store_fuzzy_pairs` once at the end.
  - Factor a private `compute_possible_squash_updates_with_scorer` helper taking `&mut impl FnMut(&[u8], &[u8]) -> Option<u8>` for the score/classify result while retaining A's existing detector arguments and `Vec<GraphEnrichmentUpdate>` result. The production wrapper passes the existing scorer; the local `graph.rs` test helper runs `load_graph`, this detector and A's `apply_squash_enrichment` with a counting closure and temporary cache. The counter lives in that test invocation and never in global mutable state. If P has already merged, follow its detector rename and relationship-vector result, but D must compile against A alone without P's helper names, cherry detector or cache methods. Keep the integration result assertion as an additional end-to-end check.

- [ ] **Step 4: Run the tests to confirm they pass.** Run `cargo test`, then `cargo build`.

- [ ] **Step 5: Commit.** `git commit -m "perf: cache fuzzy squash pair scores by commit identity"`.

---

## Requirement and constraint trace

The original #053 requirements (R1–R7) and the added R8 are now owned by the lineage ticket (supersedes cancelled #053). K1–K9 are the user's added constraints.

| Item | Covered by |
| --- | --- |
| R1 document the problem | "Problem statement"; L5 measurement |
| R2 durable records with five scalar fields, OIDs as text, both ref unions | L1 schema and two ref child tables; A1 and P1 producers |
| R3 uniqueness key, indexes, outside the 60-day cleanup | L1 schema; `transient_cache_prune_and_clear_leave_lineage` |
| R4 persist exact and fuzzy squash and exact cherry-pick, keep the score | A1 pairs; P1 pairs; L2 write-back; L1 `repeated_upsert…` |
| R5 hydrate before loader/updater publication; upsert without duplicates; preserve ref snapshots and resolved-base membership | L3 worker adapters, `same_pair_hydrates_under_both_bases`, `default_base_filters_mixed_lineage`, `updater_refill_hydrates_before_publication`; L1 transactional ref unions and `stale_and_concurrent_upserts…`; P2 `merge_relationship` |
| R6 annotate destination and displayed source; survive deleted refs and pruned objects | P2 `apply_attaches…`; L3 `hydrated_lineage_survives…`; P3 and L4 details |
| R7 tests (migration replaced per user) | L1–L3 tests; P1 `commit_patch_ids_tolerate…`; `locked_lineage…` |
| R8 presentation boundary and timings | A2; L4 `hydrated_fuzzy_lineage_does_not_change_branch_status`; L5 |
| K1 GraphSnapshot and existing channels only, workers only | Global Constraints; L2 root pass-through; L3 loader and removable updater publication adapters; App carry-forward performs only in-memory merging; `:4983` gets `Some(lineage_root)` |
| K2 complete records, keyed by type and OIDs | L1 |
| K3 identity gaps; bounded, cancellable discovery; no invented or partial tuples | A1 exact-pair suppression and detector test; X1 mid-worker supersession; P1 per-yield bounds/cancellation (including merges), retained buckets; P2 `cherry_pick_repicks_keep_all_destinations` |
| K4 hydrate before publishing; both endpoints; missing refs and objects, including newly refilled endpoints | L3 loader/updater tests; P2 endpoint merge; L4 multi-destination-ref details |
| K5 enrichment never erases; stale publication rejected separately from upsert | P2 `enrichment_never_erases…`; L2 `stale_enrichment_persists_but_does_not_publish` |
| K6 separate store; prune- and R-safe; common-dir identity; explicit test root | L1; L2 `TestDir::lineage_root`, `GBM_DATA_DIR` |
| K7 exact vs fuzzy presentation; no deletion authority | A2; L4 |
| K8 listed tests, including warm hydration while detection is blocked | X1–L4; `hydrated_markers_publish_while_enrichment_is_cancelled`; D1 local scoring-call counts and independent missing-table fixture |
| K9 honest performance boundary | "Performance boundary"; L5 isolated-root measurements; B text-hunk scorer parity and nonblocking binary diagnostics |

## Open decisions (confirm before cutting branches)

1. **Orphaned lineage files.** Brainstorming chose "delete the lineage file only when its repository's common Git dir no longer exists". This plan stores the common dir in the `meta` table but does **not** auto-delete: a repo on an unmounted volume, or one that was moved, would lose history that can't be replaced. The files are small, and adding deletion later is a contained pruner change.
2. **Base for the first slices.** Local `main` vs `origin/main`; see the caveat under "Branching and slices".
