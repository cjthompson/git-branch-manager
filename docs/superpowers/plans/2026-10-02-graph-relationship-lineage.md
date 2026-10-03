# Graph Relationship Lineage Implementation Plan (task #053)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Persist complete squash-merge and cherry-pick relationship records (lineage) per repository, hydrate them into the Graph before the structural snapshot is published, and keep them through ref deletion, object pruning, cache expiry, and the R-key cache clear.

**Architecture:** A new durable SQLite store (`src/git/lineage.rs`) keyed by the Git common directory lives under the application *data* directory, separate from the disposable `BranchCache`. Detection runs in the existing enrichment worker. It now keeps paired source and destination identities (`GraphRelationship`) and upserts accepted findings before publishing. The existing `spawn_graph_loader` worker hydrates stored relationships into the `GraphSnapshot` before sending it. Applying enrichment becomes a keep-everything merge, so fresh results add to hydrated history and never erase it.

**Tech Stack:** Rust 2021, `git2` 0.21 (`Diff::patchid`, `Revwalk`, `Oid::hash_object`), `rusqlite` 0.32 (bundled), `dirs` 6, `chrono`, ratatui, `tempfile` for tests.

**Spec:** `docs/superpowers/specs/2026-10-02-graph-refresh-and-lineage-design.md` (section "Task 053 requirements and judgment" and "Important implementation implications for 053"), plus `docs/reviews/2026-10-02-ui-stall-regression-review.md`. Executors read both. The squash-confidence model folded in here comes from `docs/plans/2026-09-22-squash-match-confidence.md` "Proposed fix 1".

---

## Problem statement (requirement 1)

Graph loading already caches patch-derived data: the `graph_patch` table in `BranchCache` stores `(old, new) → (patch_id, diff_text)`. It does not persist the *final* relationships.
- **Cherry-pick:** `compute_cherry_pick_updates` (`src/git/graph.rs:1047`) re-runs `git cherry` once per diverged tip on every load.
- **Squash:** `compute_possible_squash_updates` (`src/git/graph.rs:846`) rebuilds squash annotations only inside background enrichment.
- **Effect:** every relationship marker arrives in one batched message after both detectors finish, so the Graph has no markers for about 30 seconds on this repository. That figure is a recorded task observation, not a new benchmark; Task C6 measures it.

Three identity gaps prevent persisting complete tuples today:
1. **Exact squash** keeps source *names* (`PatchTarget::BranchTip { source_names }`) but drops the source tip OID.
2. **Fuzzy squash** reduces to the maximum percent (`branch_diffs: Vec<Vec<u8>>`), so the matching source is lost.
3. **`git cherry`** reports the source commit (`-` lines) but never the destination commit.

## Performance boundary (constraint 9)

Cached *positive* relationships give immediate markers on warm loads. A missing record does **not** prove that no relationship exists. Cold loads, newly created relationships, and new commits still require background detection, and this plan does not promise any particular speedup for them. This plan does not change Graph topology refresh: lineage supplies annotations only, and task #047's automatic refresh remains the mechanism for discovering changed refs. This plan also does not fix the synchronous Branches refresh, the UI-thread `GraphRepositoryState::capture`, or worktree concurrency. Those belong to `docs/superpowers/plans/2026-10-02-graph-refresh-rollback.md` and the other delivery units in the spec.

## Global Constraints

- **Integration surface (constraint 1):** integrate only through `GraphSnapshot`, `graph::spawn_graph_loader(PathBuf, GraphLoadOptions)`, `graph::spawn_possible_squash_enrichment`, `GraphEnrichmentMsg`, and `graph::apply_squash_enrichment` / `GraphState::apply_squash_enrichment`. Do **not** depend on `update_graph_incrementally`, `spawn_graph_updater`, `GraphUpdateMsg`, `GraphRepositoryState`, or `pending_graph_deltas`, which the rollback plan proposes to remove. Keep the names `GraphEnrichmentMsg.updates` and `GraphState::apply_squash_enrichment` so the drain call site at `src/app.rs:703` stays textually unchanged.
- **Workers only:** hydration, every SQLite access (lineage store and transient cache), detection, and every Git operation run in worker threads. Nothing new runs in `App::drain_channels`, `App::with_cache_root`, or the R-key handler (`App::clear_cache_and_refresh`). The App only stores a `LineageRoot` (a path) and an `Arc<AtomicU64>`.
- **No migration:** the lineage store is a new database file. The transient cache keeps its existing filename and schema. New transient tables are added with `CREATE TABLE IF NOT EXISTS` and queried by key, never loaded by `read_all`. This replaces the "migration" wording in requirement 7, per the user's instruction: "we don't need to do a migration because it's a temporary cache file. just create a new database with the right schema."
- **Record shape (constraint 2):** `type` (`sm` | `c-p`), `destination_hash`, `destination_ref`, `source_hash`, `source_ref`, nullable `similarity_percent` (`NULL` = exact). Full 40-hex OIDs stored as `TEXT`. Primary key `(type, destination_hash, source_hash)`. Indexes on `destination_hash` and on `source_hash`.
- **Never invent or persist partial tuples:** a record is written only when both OIDs came from the same detection pairing. A source with no discovered destination is not written.
- **Durable storage location (constraint 6):** `LineageRoot::from_env()` = `$GBM_DATA_DIR`, else `dirs::data_dir()/git-branch-manager`, else `std::env::temp_dir()/git-branch-manager`. The filename is `git-bm-lineage-<oid>.sqlite3`, where `<oid>` = `git2::Oid::hash_object(ObjectType::Blob, canonical_common_dir_bytes)`. Use this rather than `DefaultHasher`, which is not stable across Rust releases. `prune_stale_caches` and `BranchCache::clear` must never match or delete it.
- **Tests use explicit roots:** `GraphLoadOptions.lineage_root` is `Option<LineageRoot>` and defaults to `None`. Lineage opens only when a caller passes a root. `App::new` (test constructor), `TestDir`, and `manager_command` (`GBM_DATA_DIR`) use temporary directories.
- **Presentation (constraint 7):** exact and fuzzy stay distinct. Exact means `similarity_percent == 100` and gets the squash glyph. Fuzzy means `<= 99` and gets the `[fuzzy squash N%]` suffix and a `~N%` ref-pane suffix, never the exact glyph. Graph lineage never feeds `MergeStatus`, Branches merge columns, or delete eligibility.
- **Out of scope:** paired source/destination colors (task #054) and connectors (task #055). This plan only puts the data on both endpoints and shows text in the details panes.
- **Project rules:** run `cargo build` after every task. Per `CLAUDE.md`, the main agent alone sets `Cargo.toml` to `X.Y.Z-devN` locally and increments it on each code change. **Never stage or commit the version line**, and check `git diff --cached Cargo.toml` before every commit. Subagents never touch the version.
- **Cancellation:** detection scans check an `EnrichmentCancel` once per commit and once per job batch. It is cancelled when the App starts a newer graph load.
- **Scan bounds:** `MAX_CHERRY_SOURCE_COMMITS = 500` branch-side commits per tip and `MAX_CHERRY_DESTINATION_SCAN = 2_000` base-side commits per merge base. Exceeding a bound records nothing for the unmatched sources and logs `lineage.scan_incomplete`.

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

This is a `~/dev` repo with no CI, so no PRs are opened. Each slice is a `ct/` branch cut from the freshly fetched default branch (`git fetch && git symbolic-ref --short refs/remotes/origin/HEAD`). It is accepted by squash-merging into `main`, followed by a separate version-bump commit on `main` per `CLAUDE.md`. Slices that need an earlier slice are cut from `main` **after** that slice is merged, never from another `ct/` branch.

| Slice | Branch | Base | Contents |
| --- | --- | --- | --- |
| A | `ct/graph-squash-relationships` | `origin/main` (now) | Tasks A1–A2: `GraphRelationship` model, paired squash identities, fuzzy source names, `.floor()`. Ships the confidence plan's Fix 1. |
| C | `ct/graph-relationship-lineage` | `origin/main` **after A merges** | Tasks C1–C6: the full #053 delivery — durable store, git2 cherry-pick pairing, write-back, keep-everything merge, hydration, tests, docs. |
| D | `ct/graph-fuzzy-pair-cache` | `origin/main` **after A merges** | Task D1: transient fuzzy-pair score cache. Independent of C. |
| B | `ct/graph-git2-squash-patches` | `origin/main` **after C merges** | Task B1: switch squash `compute_patch` to the git2 helper introduced in C, gated on patch-id parity. |

**Dependency proofs (to be re-verified with `git diff --name-only` before each branch is cut):**
- **C needs A:** both modify `src/git/graph.rs`. C's hydration, `apply_relationships`, and `LineageStore` build and consume `GraphRelationship`, `RelationshipKind`, and `RelationshipMatch`, which A introduces. On today's `main`, C fails to build. *A changes only the in-memory squash identity and display model; it has value alone because the Graph starts showing fuzzy squash sources. If you would rather ship A and C as one slice, merge them; nothing else changes.*
- **D needs A:** both modify the fuzzy loop in `compute_possible_squash_updates` (`src/git/graph.rs`). D's pair key uses the per-pair `SourceTip { tip_oid, merge_base }` that A introduces, so D fails to build without A. D and C both touch `src/git/cache.rs` and `graph.rs`, but that is a conflict, not a dependency; they can merge in either order.
- **B needs C:** both modify `src/git/graph.rs`. B calls `git2_patch_id` / `git2_diff_text`, which C introduces. Writing B first would duplicate C's helper.

> **Base-branch caveat:** local `main` was 4 commits ahead of `origin/main` (`4bc8f7a`), and the spec, review, and rollback plan were untracked. A `ct/` branch cut from `origin/main` would miss them. Resolve this (push `main`, or the user decides otherwise) before cutting slice A.

## Review Focus

The input classes or failure modes most likely to bite a user that the spec implies but no requirement names. Each has a test in the owning task.

1. **The same commit is both a squash destination and a cherry-pick destination.** A single-commit branch squashed onto base has the same patch-id as its only commit. Both records must coexist (different `type`), and the display must not double-render or drop either. Test: `same_commit_records_squash_and_cherry_pick` (C2).
2. **A hydrated relationship whose `destination_ref` differs from the current `--base`.** Hydration only applies relationships whose `destination_ref` equals the current base branch, so `--base develop` doesn't show `main`'s squash history on a lane enrichment never inspects. Test: `hydration_skips_relationships_for_other_base` (C4).
3. **Two branches at the same tip.** One record per `(type, destination, source)`, but both names are kept in `lineage_source_ref` and both are shown. Test: `shared_tip_keeps_all_source_names` (C1).
4. **A long-lived branch whose merge base is thousands of commits back.** The destination scan stops at `MAX_CHERRY_DESTINATION_SCAN` and writes no partial tuple. Test: `cherry_scan_bound_writes_no_partial_record` (C2).
5. **The lineage database is locked or unwritable** (another process holds a write lock, or the directory is read-only). Detection still publishes in-session markers, and the loader still publishes the structural snapshot without hydration. Neither panics. Test: `unwritable_lineage_root_does_not_block_graph` (C4).

---

## File structure

| File | Slice | Responsibility |
| --- | --- | --- |
| `src/git/graph.rs` | A, C, D, B | `GraphRelationship` model and accessors, detectors, `apply_relationships`, hydration call, cancel token, git2 patch helpers |
| `src/git/lineage.rs` (new) | C | `LineageRoot`, `LineageStore`: durable schema, upsert, lookup |
| `src/git/mod.rs` | C | `pub mod lineage;` |
| `src/git/cache.rs` | C, D | on-demand transient tables `commit_patch_id` (C) and `fuzzy_pair` (D); `prune_stale_caches_at` made `pub(crate)` for tests |
| `src/git/fuzzy_match.rs` | A | `classify` uses `.floor()` |
| `src/view/graph.rs` | A, C | `GraphState::apply_squash_enrichment` signature follows the update type |
| `src/ui/graph_render.rs`, `src/ui/info_modal.rs`, `src/ui/commit_details.rs`, `src/ui/render.rs` | A, C | read accessors; fuzzy sources (A); cherry destination and source-end text (C) |
| `src/app.rs` | A, C | carry-forward copies `relationships`; `lineage_root` and `graph_generation_signal` fields; pass through to loader and enrichment |
| `src/main.rs` | C | pass `LineageRoot::from_env()` |
| `tests/integration.rs` | A, C, D | fixtures, `TestDir::lineage_root`, `GBM_DATA_DIR` in `manager_command`, new scenario tests |
| `CLAUDE.md` | C | Data Flow and Architecture note for lineage |

---

## Slice A — `ct/graph-squash-relationships`

### Task A1: Paired squash relationship model

**Files:**
- Modify: `src/git/graph.rs:33-41` (`GraphEnrichmentUpdate`), `:83-127` (`GraphCommit`, `FuzzySquashMatch`), `:846-1036` (`compute_possible_squash_updates`, `apply_squash_enrichment`), `:1235-1250` (`PatchTarget`)
- Modify: `src/git/fuzzy_match.rs:187-198` (`classify`)
- Modify: `src/app.rs:636-648` (carry-forward), every `GraphCommit { … }` fixture in `src/app.rs`, `src/view/graph.rs`, `src/ui/*.rs`, `tests/integration.rs`
- Test: `src/git/graph.rs` test module, `src/git/fuzzy_match.rs` tests, `tests/integration.rs`

**Interfaces:**
- Produces (used by A2, C, D, B):

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
    pub fn stronger(self, other: Self) -> Self;         // Exact wins, else max percent
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphRelationship {
    pub kind: RelationshipKind,
    pub matching: RelationshipMatch,
    pub destination_oid: String,
    pub destination_ref: String,
    pub source_oid: String,
    pub source_refs: Vec<String>,   // sorted, deduplicated
}
impl GraphRelationship {
    pub fn key(&self) -> (RelationshipKind, &str, &str);   // (kind, destination_oid, source_oid)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SquashMatchConfidence { pub sources: Vec<String>, pub similarity_percent: u8 }

impl GraphCommit {
    pub fn squash_match_confidence(&self) -> Option<SquashMatchConfidence>;
    pub fn is_possible_squash_merge(&self) -> bool;               // confidence == 100
    pub fn possible_squash_merge_sources(&self) -> Vec<String>;   // exact sources only
    pub fn fuzzy_squash_match(&self) -> Option<FuzzySquashMatch>; // best fuzzy, only when no exact
}
```

- `GraphCommit` loses `is_possible_squash_merge`, `possible_squash_merge_sources`, and `fuzzy_squash_match`, and gains `pub relationships: Vec<GraphRelationship>`. `is_cherry_picked_commit` stays a field in slice A.
- `GraphEnrichmentUpdate` becomes `{ oid: String, relationships: Vec<GraphRelationship>, is_cherry_picked_commit: bool }`.
- `FuzzySquashMatch { similarity_percent: u8 }` stays as the accessor's return type, so existing UI code changes are minimal.

- [ ] **Step 1: Write the failing unit tests** in `src/git/graph.rs`'s `#[cfg(test)] mod tests`:

```rust
fn squash(dest: &str, source: &str, names: &[&str], matching: RelationshipMatch) -> GraphRelationship {
    GraphRelationship {
        kind: RelationshipKind::SquashMerge,
        matching,
        destination_oid: dest.into(),
        destination_ref: "main".into(),
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
    let confidence = commit.squash_match_confidence().unwrap();
    assert_eq!(confidence, SquashMatchConfidence { sources: vec!["feature/high".into()], similarity_percent: 97 });
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

And in `src/git/fuzzy_match.rs` tests:

```rust
#[test]
fn classify_floors_so_fuzzy_never_reports_100() {
    let score = FuzzyScore { similarity: 0.996, file_overlap_ratio: 1.0, union_size: 1_000 };
    assert_eq!(classify(&score), Some(99));
}
```

`MIN_UNION_SIZE_FOR_FUZZY` must be ≤ 1,000 for this fixture; check the constant and raise `union_size` if it isn't.

- [ ] **Step 2: Run them and confirm they fail to compile**

Run: `cargo test --lib squash_confidence classify_floors`
Expected: compile errors for the missing `GraphRelationship`, `RelationshipMatch`, and `squash_match_confidence`.

- [ ] **Step 3: Add the model and accessors** to `src/git/graph.rs`, placed after `GraphCommit`:

```rust
impl RelationshipKind {
    pub fn code(self) -> &'static str {
        match self {
            Self::SquashMerge => "sm",
            Self::CherryPick => "c-p",
        }
    }
    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "sm" => Some(Self::SquashMerge),
            "c-p" => Some(Self::CherryPick),
            _ => None,
        }
    }
}

impl RelationshipMatch {
    pub fn similarity_percent(self) -> u8 {
        match self {
            Self::Exact => 100,
            Self::Fuzzy { similarity_percent } => similarity_percent,
        }
    }
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
            self.relationships.iter().filter(|relationship| {
                relationship.kind == RelationshipKind::SquashMerge
                    && relationship.destination_oid == self.oid
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

`RelationshipMatch::stronger` relies on `classify` never returning 100 for fuzzy matches (Step 5). Add this comment above it: `// Exact is 100 and fuzzy is floored to <= 99, so the larger percent is the stronger match.`

- [ ] **Step 4: Rewrite the squash producer to keep paired identities.** In `compute_possible_squash_updates`:
  1. Change `PatchTarget::BranchTip { source_names }` to `PatchTarget::BranchTip(SourceTip)`.
  2. Push `PatchTarget::BranchTip(SourceTip { tip_oid: tip.clone(), merge_base: merge_base.clone(), source_names })` in the tip loop.
  3. Replace `source_names_by_patch: HashMap<String, Vec<String>>` with `sources_by_patch: HashMap<String, Vec<SourceTip>>`.
  4. Replace `branch_diffs: Vec<Vec<u8>>` with `branch_diffs: Vec<(SourceTip, Vec<u8>)>`.
  5. Build relationships:

```rust
#[derive(Clone, Debug)]
struct SourceTip {
    tip_oid: String,
    merge_base: String,
    source_names: Vec<String>,
}

// exact tier
let mut relationships = Vec::<GraphRelationship>::new();
let mut exact_destinations = HashSet::<String>::new();
for (patch_id, sources) in &sources_by_patch {
    let Some(base_oids) = base_oids_by_patch.get(patch_id) else { continue };
    for destination in base_oids {
        exact_destinations.insert(destination.clone());
        for source in sources {
            relationships.push(GraphRelationship {
                kind: RelationshipKind::SquashMerge,
                matching: RelationshipMatch::Exact,
                destination_oid: destination.clone(),
                destination_ref: base_branch.clone(),
                source_oid: source.tip_oid.clone(),
                source_refs: source.source_names.clone(),
            });
        }
    }
}
// fuzzy tier: keep every accepted pair, not just the max
for (destination, base_diff) in &base_diffs {
    if exact_destinations.contains(destination) {
        continue;
    }
    for (source, branch_diff) in &branch_diffs {
        let Some(percent) = crate::git::fuzzy_match::score(branch_diff, base_diff)
            .and_then(|score| crate::git::fuzzy_match::classify(&score))
        else {
            continue;
        };
        relationships.push(GraphRelationship {
            kind: RelationshipKind::SquashMerge,
            matching: RelationshipMatch::Fuzzy { similarity_percent: percent },
            destination_oid: destination.clone(),
            destination_ref: base_branch.clone(),
            source_oid: source.tip_oid.clone(),
            source_refs: source.source_names.clone(),
        });
    }
}
```

  Then emit one `GraphEnrichmentUpdate` per base-lane commit, as today, with `relationships` = the entries whose `destination_oid == commit.oid`, and `is_cherry_picked_commit: false`. `apply_squash_enrichment` assigns `commit.relationships = update.relationships.clone()` and `commit.is_cherry_picked_commit = update.is_cherry_picked_commit`. Replacement is acceptable in slice A because nothing persists yet; slice C changes it to a merge. `merge_enrichment_updates` stays as is apart from the field rename. Cherry updates carry `relationships: Vec::new()`, and the `and_modify` only sets the cherry flag.

- [ ] **Step 5: Floor the fuzzy percent.** In `src/git/fuzzy_match.rs::classify`, replace `.round()` with `.floor()`. Update the `FuzzySquashMatch` doc comment in `graph.rs` from "rounded from the raw f32 score" to "floored from the raw f32 score; fuzzy is always <= 99".

- [ ] **Step 6: Migrate fixtures and readers.**
  - Fixtures: in every `GraphCommit { … }` literal, delete the three removed field lines and add `relationships: Vec::new(),` once. For fixtures that set `is_possible_squash_merge: true` with sources, or `fuzzy_squash_match: Some(..)`, build the equivalent `relationships` with the `squash(...)` helper pattern from Step 1, using the fixture's own OID as the destination. Find them with `rg -n "is_possible_squash_merge: true|fuzzy_squash_match: Some" src tests`.
  - Readers: replace `.is_possible_squash_merge` with `.is_possible_squash_merge()`, `.possible_squash_merge_sources` with `.possible_squash_merge_sources()`, and `.fuzzy_squash_match` with `.fuzzy_squash_match()`. Fix the borrow sites, such as `commit.fuzzy_squash_match.as_ref()` becoming `commit.fuzzy_squash_match()`. Writers like `enriched.commits[0].is_possible_squash_merge = true` (`src/app.rs:10583-10591`) become `commit.relationships = vec![...]` / `.clear()`.
  - Carry-forward `src/app.rs:640-646`: replace the three squash lines with `commit.relationships = old.relationships.clone();`.

- [ ] **Step 7: Run the whole suite**

Run: `cargo test` then `cargo clippy --all-targets` then `cargo build`
Expected: all pass. The squash-confidence unit tests from Step 1 pass, and existing exact-squash and fuzzy integration tests (`test_squash_scenario_20*`, the graph-patch cache tests) pass unchanged in meaning.

- [ ] **Step 8: Commit**

```bash
git add src tests
git diff --cached Cargo.toml   # must show no version change
git commit -m "refactor: keep paired squash identities in Graph relationships"
```

### Task A2: Show fuzzy squash sources

**Files:**
- Modify: `src/ui/graph_render.rs:193-230` (glyph, detail suffix), `:630-650` (`possible_squash_source_spans`), `:1466` (test)
- Modify: `src/ui/info_modal.rs:315-332`, `src/ui/commit_details.rs:53-63`
- Test: `src/ui/graph_render.rs`, `src/ui/info_modal.rs` test modules

**Interfaces:**
- Consumes: `GraphCommit::squash_match_confidence()`, `SquashMatchConfidence` from A1.
- Produces: no new public API.

- [ ] **Step 1: Write the failing tests.** Rename `ref_pane_hides_possible_squash_sources_without_an_exact_match` to `ref_pane_shows_fuzzy_squash_source_with_percent` and flip its assertion so that a commit with one fuzzy 97% relationship from `feature/login` renders a ref-pane span whose text is `format!("{} feature/login ~97%", symbols.status_squash_merged)`. Add an `info_modal` test asserting the fields `Possible Squash Merge (fuzzy)` = `97% similarity` and `Possible Squash Merge From` = `feature/login`. Keep the existing exact-match tests asserting `"{sym} feature/auth +1"` with no percent.

- [ ] **Step 2: Run them and confirm they fail**

Run: `cargo test --lib ref_pane_shows_fuzzy_squash_source_with_percent info_modal`
Expected: FAIL, because the ref pane returns no spans for fuzzy commits.

- [ ] **Step 3: Implement.** `possible_squash_source_spans` reads `commit.squash_match_confidence()`:

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

Leave the glyph rule unchanged: the exact glyph only when `is_possible_squash_merge()`. In `info_modal.rs`, add `Possible Squash Merge From` under the fuzzy branch, using `confidence.sources.join(", ")`.

- [ ] **Step 4: Run them and confirm they pass**

Run: `cargo test` then `cargo build`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src
git diff --cached Cargo.toml
git commit -m "feat: show fuzzy squash sources and confidence in Graph"
```

**Slice A acceptance:** in the TUI on `~/dev/claude-monitor`, the `42fe178` row's ref pane shows `≈ ct/fix-tui-quit-hang ~97%` (the confidence plan's Verification). Squash-merge `ct/graph-squash-relationships` into `main`, then make a separate version-bump commit.

---

## Slice C — `ct/graph-relationship-lineage` (task #053; cut from `main` after A merges)

### Task C1: Durable lineage store

**Files:**
- Create: `src/git/lineage.rs`
- Modify: `src/git/mod.rs` (add `pub mod lineage;`), `src/git/cache.rs` (make `prune_stale_caches_at` `pub(crate)`, and add a `#[cfg(test)]`-visible re-export if needed)
- Test: `src/git/lineage.rs` `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `GraphRelationship`, `RelationshipKind`, `RelationshipMatch` (A1).
- Produces:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineageRoot { path: PathBuf }
impl LineageRoot {
    pub fn from_env() -> Self;              // $GBM_DATA_DIR | dirs::data_dir()/git-branch-manager | temp
    pub fn at(path: impl Into<PathBuf>) -> Self;
    pub fn path(&self) -> &Path;
}

pub struct LineageStore { path: PathBuf }
impl LineageStore {
    /// None when `repo_path` is not a Git repository or the root cannot be created.
    pub fn open(repo_path: &Path, root: &LineageRoot) -> Option<Self>;
    pub fn path(&self) -> &Path;
    pub fn upsert(&self, relationships: &[GraphRelationship]) -> rusqlite::Result<()>;
    /// Every stored relationship whose destination or source OID is in `oids`.
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
        let status = Command::new("git").args(args).current_dir(dir).status().unwrap();
        assert!(status.success(), "git {args:?}");
    }

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q", "-b", "main"]);
        git(dir.path(), &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "init"]);
        dir
    }

    fn rel(kind: RelationshipKind, dest: char, source: char, refs: &[&str], matching: RelationshipMatch) -> GraphRelationship {
        GraphRelationship {
            kind,
            matching,
            destination_oid: dest.to_string().repeat(40),
            destination_ref: "main".into(),
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
        let root = tempfile::tempdir().unwrap();
        let root = LineageRoot::at(root.path());
        let store = LineageStore::open(repo.path(), &root).unwrap();
        store.upsert(&[
            rel(RelationshipKind::SquashMerge, 'a', 'b', &["feature/x"], RelationshipMatch::Fuzzy { similarity_percent: 91 }),
            rel(RelationshipKind::CherryPick, 'c', 'd', &["feature/y"], RelationshipMatch::Exact),
        ]).unwrap();
        drop(store);
        let reopened = LineageStore::open(repo.path(), &root).unwrap();
        let mut found = reopened.lookup(&oids(&['a', 'd'])).unwrap();
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
    fn linked_worktrees_share_and_clones_are_isolated() {
        let repo = repo();
        let root = tempfile::tempdir().unwrap();
        let root = LineageRoot::at(root.path());
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
        // Deliberately the same directory for both roots: the pruner must not match lineage files.
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
    fn non_repository_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        assert!(LineageStore::open(dir.path(), &LineageRoot::at(root.path())).is_none());
    }
}
```

- [ ] **Step 2: Run them and confirm they fail**

Run: `cargo test --lib lineage::`
Expected: compile errors, because `lineage` does not exist yet.

- [ ] **Step 3: Implement `src/git/lineage.rs`.**

```rust
//! Durable, repository-scoped squash/cherry-pick lineage. Unlike `BranchCache`,
//! this database is never pruned or cleared: it holds history that cannot be
//! recomputed once source refs are deleted and Git prunes their objects.

use crate::git::graph::{GraphRelationship, RelationshipKind, RelationshipMatch};
use rusqlite::{params, Connection};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

pub const LINEAGE_FILE_PREFIX: &str = "git-bm-lineage-";
const LOOKUP_CHUNK: usize = 400;

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

impl LineageStore {
    pub fn open(repo_path: &Path, root: &LineageRoot) -> Option<Self> {
        let repo = git2::Repository::open(repo_path).ok()?;
        let common_dir = fs::canonicalize(repo.commondir()).unwrap_or_else(|_| repo.commondir().to_path_buf());
        let common_dir_text = common_dir.to_string_lossy().into_owned();
        // A content hash is stable across Rust releases; DefaultHasher is not.
        let identity = git2::Oid::hash_object(git2::ObjectType::Blob, common_dir_text.as_bytes()).ok()?;
        fs::create_dir_all(root.path()).ok()?;
        let store = Self { path: root.path().join(format!("{LINEAGE_FILE_PREFIX}{identity}.sqlite3")) };
        let conn = store.connect().ok()?;
        conn.execute(
            "INSERT OR IGNORE INTO meta(key, value) VALUES ('common_dir', ?1)",
            params![common_dir_text],
        )
        .ok()?;
        Some(store)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn connect(&self) -> rusqlite::Result<Connection> {
        let conn = Connection::open(&self.path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;
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
             CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )?;
        Ok(conn)
    }

    pub fn upsert(&self, relationships: &[GraphRelationship]) -> rusqlite::Result<()> {
        if relationships.is_empty() {
            return Ok(());
        }
        let mut conn = self.connect()?;
        let tx = conn.transaction()?;
        let now = chrono::Utc::now().to_rfc3339();
        for relationship in relationships {
            let similarity = match relationship.matching {
                RelationshipMatch::Exact => None,
                RelationshipMatch::Fuzzy { similarity_percent } => Some(similarity_percent),
            };
            let primary_ref = relationship.source_refs.first().cloned().unwrap_or_default();
            tx.execute(
                "INSERT INTO lineage (type, destination_hash, destination_ref, source_hash, source_ref,
                                      similarity_percent, detected_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
                 ON CONFLICT (type, destination_hash, source_hash) DO UPDATE SET
                     destination_ref = excluded.destination_ref,
                     source_ref = CASE WHEN excluded.source_ref = '' THEN lineage.source_ref
                                       ELSE excluded.source_ref END,
                     similarity_percent = CASE
                         WHEN lineage.similarity_percent IS NULL OR excluded.similarity_percent IS NULL THEN NULL
                         ELSE MAX(lineage.similarity_percent, excluded.similarity_percent) END,
                     updated_at = excluded.updated_at",
                params![
                    relationship.kind.code(),
                    relationship.destination_oid,
                    relationship.destination_ref,
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
        }
        tx.commit()
    }

    pub fn lookup(&self, oids: &HashSet<String>) -> rusqlite::Result<Vec<GraphRelationship>> {
        if oids.is_empty() || !self.path.exists() {
            return Ok(Vec::new());
        }
        let conn = self.connect()?;
        let oids = oids.iter().collect::<Vec<_>>();
        let mut found = std::collections::HashMap::<(String, String, String), GraphRelationship>::new();
        for chunk in oids.chunks(LOOKUP_CHUNK) {
            let placeholders = vec!["?"; chunk.len()].join(",");
            let sql = format!(
                "SELECT l.type, l.destination_hash, l.destination_ref, l.source_hash, l.source_ref,
                        l.similarity_percent, group_concat(r.ref_name, char(10))
                 FROM lineage l
                 LEFT JOIN lineage_source_ref r
                   ON r.type = l.type AND r.destination_hash = l.destination_hash AND r.source_hash = l.source_hash
                 WHERE l.destination_hash IN ({placeholders}) OR l.source_hash IN ({placeholders})
                 GROUP BY l.type, l.destination_hash, l.source_hash"
            );
            let mut statement = conn.prepare(&sql)?;
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
                ))
            })?;
            for row in rows {
                let (code, destination, destination_ref, source, source_ref, similarity, names) = row?;
                let Some(kind) = RelationshipKind::from_code(&code) else { continue };
                let mut source_refs = names
                    .map(|joined| joined.split('\n').map(str::to_string).collect::<Vec<_>>())
                    .unwrap_or_default();
                if !source_ref.is_empty() {
                    source_refs.push(source_ref);
                }
                source_refs.sort();
                source_refs.dedup();
                let matching = similarity
                    .map(|similarity_percent| RelationshipMatch::Fuzzy { similarity_percent })
                    .unwrap_or(RelationshipMatch::Exact);
                found.insert(
                    (code, destination.clone(), source.clone()),
                    GraphRelationship { kind, matching, destination_oid: destination, destination_ref, source_oid: source, source_refs },
                );
            }
        }
        Ok(found.into_values().collect())
    }
}
```

Register `pub mod lineage;` in `src/git/mod.rs`. Make `prune_stale_caches_at` in `cache.rs` `pub(crate)`. Confirm `cache_database_name` (`cache.rs:142`) only accepts the `git-bm-repo-cache-` / `git-bm-cache-` prefixes; it already does, and the shared-directory test pins this.

- [ ] **Step 4: Run them and confirm they pass**

Run: `cargo test --lib lineage::` then `cargo build`
Expected: 6 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/git/lineage.rs src/git/mod.rs src/git/cache.rs
git diff --cached Cargo.toml
git commit -m "feat: add durable repository-scoped relationship lineage store"
```

This commit sits on the slice branch; the slice is accepted only after C6. C1 is not merged alone (constraint: no standalone schema).

### Task C2: git2 cherry-pick pairing with bounded, cancellable destination discovery

**Files:**
- Modify: `src/git/graph.rs:1038-1126` (replace `compute_cherry_pick_updates` with `compute_cherry_pick_relationships`), `:83-127` (remove the `is_cherry_picked_commit` field and add accessors)
- Modify: `src/git/cache.rs` (on-demand `commit_patch_id` table)
- Modify: UI readers of `is_cherry_picked_commit` (`graph_render.rs:205,224`, `info_modal.rs:326`, `commit_details.rs:53`), fixtures, and `src/app.rs:645`
- Test: `src/git/graph.rs` tests, `tests/integration.rs`

**Interfaces:**
- Consumes: A1 model; `BranchCache` (`load_for_base`, path).
- Produces:

```rust
#[derive(Clone, Debug)]
pub struct EnrichmentCancel { latest: Arc<AtomicU64>, generation: u64 }
impl EnrichmentCancel {
    pub fn new(latest: Arc<AtomicU64>, generation: u64) -> Self;
    pub fn never() -> Self;                     // for sync helpers and tests
    pub fn is_cancelled(&self) -> bool;         // latest.load(Acquire) != generation
}
pub const MAX_CHERRY_SOURCE_COMMITS: usize = 500;
pub const MAX_CHERRY_DESTINATION_SCAN: usize = 2_000;
const CHERRY_PATCH_ID_VERSION: u32 = 1;
pub(crate) fn git2_patch_id(repo: &git2::Repository, commit: git2::Oid) -> Option<String>;
fn compute_cherry_pick_relationships(
    repo_path: &Path, snapshot: &GraphSnapshot, base_branch: &str, base_tip: &str,
    cache: &BranchCache, cancel: &EnrichmentCancel,
) -> Vec<GraphRelationship>;

impl GraphCommit {
    pub fn is_cherry_picked_commit(&self) -> bool;          // CherryPick relationship with source_oid == oid
    pub fn cherry_pick_destinations(&self) -> Vec<&GraphRelationship>;  // this commit is the source
    pub fn cherry_pick_sources(&self) -> Vec<&GraphRelationship>;       // this commit is the destination
}

// cache.rs, on-demand (never in read_all):
impl BranchCache {
    pub fn lookup_commit_patch_ids(&self, keys: &[String]) -> HashMap<String, Option<String>>;
    pub fn store_commit_patch_ids(&self, entries: &[(String, Option<String>)]);
}
```

- [ ] **Step 1: Write the failing integration tests** in `tests/integration.rs`, next to `test_graph_cherry_pick_enrichment_marks_branch_commits`:

```rust
fn rev(dir: &std::path::Path, spec: &str) -> String {
    git_output(dir, &["rev-parse", spec]).trim().to_string()
}

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
    std::fs::write(dir.join("other.txt"), "o").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-q", "-m", "other"]);
    run_git(dir, &["cherry-pick", &source]);
    let destination = rev(dir, "HEAD");

    let snapshot = graph::load_graph_with_squash_annotations(dir, tmpdir.graph_options_for("main"))
        .expect("graph load");
    let source_commit = snapshot.commits.iter().find(|c| c.oid == source).unwrap();
    let destination_commit = snapshot.commits.iter().find(|c| c.oid == destination).unwrap();
    assert!(source_commit.is_cherry_picked_commit());
    let pair = &source_commit.cherry_pick_destinations()[0];
    assert_eq!(pair.destination_oid, destination);
    assert_eq!(pair.destination_ref, "main");
    assert_eq!(pair.source_refs, ["feature/pick"]);
    assert_eq!(destination_commit.cherry_pick_sources()[0].source_oid, source);
}

#[test]
fn cherry_pick_destinations_match_git_cherry() {
    // Rename, binary, and mode-change commits: the git2 pairing must mark the same
    // source set as `git cherry`, which the pairing replaces.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();
    std::fs::write(dir.join("rename-me.txt"), "contents\n".repeat(20)).unwrap();
    std::fs::write(dir.join("script.sh"), "echo hi\n").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-q", "-m", "seed"]);
    run_git(dir, &["checkout", "-q", "-b", "feature/mixed"]);
    run_git(dir, &["mv", "rename-me.txt", "renamed.txt"]);
    run_git(dir, &["commit", "-q", "-m", "rename"]);
    std::fs::write(dir.join("blob.bin"), [0u8, 159, 146, 150, 0, 1, 2]).unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-q", "-m", "binary"]);
    run_git(dir, &["update-index", "--chmod=+x", "script.sh"]);
    run_git(dir, &["commit", "-q", "-m", "mode"]);
    let picks = git_output(dir, &["rev-list", "--reverse", "main..feature/mixed"]);
    run_git(dir, &["checkout", "-q", "main"]);
    for oid in picks.lines() {
        run_git(dir, &["cherry-pick", oid]);
    }
    let expected = git_output(dir, &["cherry", "main", "feature/mixed"])
        .lines()
        .filter_map(|line| line.strip_prefix("- "))
        .map(str::to_string)
        .collect::<std::collections::BTreeSet<_>>();
    let snapshot = graph::load_graph_with_squash_annotations(dir, tmpdir.graph_options_for("main")).unwrap();
    let actual = snapshot.commits.iter()
        .filter(|c| c.is_cherry_picked_commit())
        .map(|c| c.oid.clone())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(actual, expected);
    assert_eq!(expected.len(), 3);
}

// Written in Task C3 Step 1: it needs `TestDir::lineage_root` and C3's write-back.
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
    let store = lineage::LineageStore::open(dir, &tmpdir.lineage_root()).unwrap();
    let stored = store.lookup(&[source.clone()].into_iter().collect()).unwrap();
    assert!(stored.iter().any(|r| r.kind == graph::RelationshipKind::CherryPick
        && r.source_oid == source && r.destination_oid == destination));
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
    run_git(dir, &["merge", "-q", "--squash", "feature/single"]);
    run_git(dir, &["commit", "-q", "-m", "squash single"]);
    let destination = rev(dir, "HEAD");
    let snapshot = graph::load_graph_with_squash_annotations(dir, tmpdir.graph_options_for("main")).unwrap();
    let commit = snapshot.commits.iter().find(|c| c.oid == destination).unwrap();
    assert!(commit.is_possible_squash_merge());
    assert_eq!(commit.cherry_pick_sources().len(), 1);
}
```

Add a helper to `TestDir`, next to `graph_options`:

```rust
fn graph_options_for(&self, base: &str) -> graph::GraphLoadOptions {
    let mut options = self.graph_options();
    options.base_branch = Some(base.to_string());
    options.max_count = 50;
    options
}
```

Unit tests in `src/git/graph.rs`:

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

And the bound test `cherry_scan_bound_writes_no_partial_record` (integration): build a branch whose cherry-picked commit's destination sits more than `MAX_CHERRY_DESTINATION_SCAN` commits below the base tip. Generating 2,000 empty commits is slow, so make the bound injectable for tests through a `pub(crate) fn compute_cherry_pick_relationships_with_bounds(…, source_limit, destination_limit)` wrapper, called from a `#[cfg(test)]` unit test in `graph.rs` with `destination_limit = 3` and 5 filler commits above the destination. Assert that the returned `Vec` is empty and that a cancelled `EnrichmentCancel` also returns an empty `Vec`.

- [ ] **Step 2: Run them and confirm they fail**

Run: `cargo test cherry_pick_relationships cherry_pick_destinations_match same_commit_records enrichment_cancel cherry_scan_bound`
Expected: compile errors for the missing `cherry_pick_destinations`, `EnrichmentCancel`, and `graph_options_for`.

- [ ] **Step 3: Implement the git2 helper and the cancel token** in `src/git/graph.rs`:

```rust
impl EnrichmentCancel {
    pub fn new(latest: Arc<AtomicU64>, generation: u64) -> Self { Self { latest, generation } }
    pub fn never() -> Self { Self { latest: Arc::new(AtomicU64::new(0)), generation: 0 } }
    pub fn is_cancelled(&self) -> bool { self.latest.load(Ordering::Acquire) != self.generation }
}

/// Stable patch ID of a single-parent commit's diff, computed in-process.
pub(crate) fn git2_patch_id(repo: &git2::Repository, oid: git2::Oid) -> Option<String> {
    let commit = repo.find_commit(oid).ok()?;
    if commit.parent_count() != 1 {
        return None;
    }
    let parent_tree = commit.parent(0).ok()?.tree().ok()?;
    let tree = commit.tree().ok()?;
    let diff = repo.diff_tree_to_tree(Some(&parent_tree), Some(&tree), None).ok()?;
    if diff.deltas().len() == 0 {
        return None;
    }
    diff.patchid(None).ok().map(|id| id.to_string())
}
```

- [ ] **Step 4: Add the on-demand transient table** in `src/git/cache.rs`. Add it to `ensure_schema`:

```sql
CREATE TABLE IF NOT EXISTS commit_patch_id (
    key      TEXT PRIMARY KEY,
    patch_id TEXT
);
```

Then add these methods. They open their own connection on `self.path` and never touch `read_all`:

```rust
pub fn lookup_commit_patch_ids(&self, keys: &[String]) -> HashMap<String, Option<String>> {
    let mut found = HashMap::new();
    let Ok(conn) = open_conn(&self.path) else { return found };
    for chunk in keys.chunks(400) {
        let sql = format!(
            "SELECT key, patch_id FROM commit_patch_id WHERE key IN ({})",
            vec!["?"; chunk.len()].join(",")
        );
        // A cache file written before this table existed has no table: treat as all misses.
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

Add a `cache.rs` unit test, `commit_patch_ids_tolerate_cache_written_before_table`: create a database with only `branch_cache` (via `Connection::open` and a `CREATE TABLE`), then assert that `lookup_commit_patch_ids` returns an empty map. Add another asserting a store→lookup round trip that includes a `None` value. This covers requirement 7's "compatibility with existing cache data".

- [ ] **Step 5: Implement `compute_cherry_pick_relationships`.** Algorithm:
  1. Open `git2::Repository` in the worker. Candidate tips are displayed commits carrying local non-base refs, in snapshot order, deduplicated by OID. No displayed merge base is required: destinations outside the window are found by the walks below, which `git cherry` also did. Keep an in-memory `HashMap<Oid, Option<String>>` of patch IDs for the whole run so overlapping base ranges across tips are computed once.
  2. **Sources:** for each tip, run a `Revwalk` with `push(tip)` and `hide(base_tip)`, skipping merges. These are the branch commits not in base, the same set `git cherry` lists. If more than `source_limit` commits come back, log `tracing::debug!(target: "lineage", tip, "lineage.scan_incomplete: source bound")` and skip this tip. A tip already contained in base yields nothing. Collect `source_oid → Vec<tip names>`.
  3. **Patch IDs:** look up keys `format!("{oid}:v{CHERRY_PATCH_ID_VERSION}")` with `cache.lookup_commit_patch_ids`, compute misses with `git2_patch_id`, and store misses with `store_commit_patch_ids` once per tip. Check `cancel.is_cancelled()` before each computation; on cancel, return the relationships completed so far (each one is complete).
  4. **Destinations:** for each tip, run a `Revwalk` with `push(base_tip)` and `hide(tip)`, skipping merges. These are the base commits not on the branch, as with `git cherry`'s upstream side. For each commit, check `cancel`, get its patch ID (memo, then cache, then compute), and match it against this tip's unresolved source patch IDs. Stop when every source patch ID has at least one destination, or after `destination_limit` commits; in the latter case log `lineage.scan_incomplete: destination bound`.
  5. **Emit** one `GraphRelationship { kind: CherryPick, matching: Exact, destination_oid, destination_ref: base_branch, source_oid, source_refs: sorted dedup tip names }` for every (source, destination) patch-ID match. Sources with no destination emit nothing. Persisted records may have endpoints outside the window; `apply_squash_enrichment` attaches only to displayed endpoints, and the store keeps the rest for later loads.

  Replace the `git cherry` subprocess and its `Command` / `Stdio` imports if they are no longer used. Remove the `is_cherry_picked_commit` field and add the three accessors:

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

  Readers change from `.is_cherry_picked_commit` to `.is_cherry_picked_commit()`. Fixtures delete `is_cherry_picked_commit: …` and express `true` with a cherry relationship whose `source_oid` is the fixture's OID. `src/app.rs:645` (the carry-forward line) is deleted, because `relationships` is already copied.

  Temporary bridge until C3: `GraphEnrichmentUpdate` drops `is_cherry_picked_commit`; cherry relationships are attached to their source commit's update. C3 replaces this per-OID update type entirely.

- [ ] **Step 6: Run them and confirm they pass**

Run: `cargo test` then `cargo build`
Expected: the new tests pass, and `test_graph_cherry_pick_enrichment_marks_branch_commits` still passes with `is_cherry_picked_commit()`.

- [ ] **Step 7: Commit**

```bash
git add src tests
git diff --cached Cargo.toml
git commit -m "feat: pair cherry-picked commits with their destination via git2 patch IDs"
```

### Task C3: Write-back, keep-everything merge, cancellation

**Files:**
- Modify: `src/git/graph.rs` (`GraphEnrichmentMsg`, `spawn_possible_squash_enrichment`, `load_graph_with_squash_annotations`, `apply_squash_enrichment`, `merge_enrichment_updates`, and `compute_possible_squash_updates` returning relationships)
- Modify: `src/view/graph.rs:202-209`
- Modify: `src/app.rs` (two enrichment spawn call sites at `:599` and `:667`, `spawn_graph_load`, new fields)
- Test: `src/git/graph.rs`, `src/app.rs` tests, `tests/integration.rs:7229-7380` (existing apply tests updated)

**Interfaces:**
- Consumes: C1 `LineageStore`, `LineageRoot`; C2 `EnrichmentCancel`, `compute_cherry_pick_relationships`.
- Produces:

```rust
pub struct GraphEnrichmentMsg { pub generation: u64, pub updates: Vec<GraphRelationship> }
pub fn compute_relationships(repo_path: &Path, snapshot: &GraphSnapshot, requested_base: Option<&str>,
                             cache_root: &CacheRoot, cancel: &EnrichmentCancel) -> Vec<GraphRelationship>;
pub fn apply_squash_enrichment(snapshot: &mut GraphSnapshot, updates: &[GraphRelationship]);  // keep-everything merge
pub fn spawn_possible_squash_enrichment(
    snapshot: GraphSnapshot, repo_path: PathBuf, requested_base: Option<String>, generation: u64,
    cache_root: CacheRoot, lineage_root: Option<LineageRoot>, latest_generation: Arc<AtomicU64>,
) -> Receiver<GraphEnrichmentMsg>;
// GraphState::apply_squash_enrichment(&mut self, updates: &[GraphRelationship]) -> bool
// App fields: lineage_root: LineageRoot, graph_generation_signal: Arc<AtomicU64>
```

- `GraphEnrichmentUpdate` and `merge_enrichment_updates` are deleted.
- `compute_possible_squash_updates` is renamed `compute_squash_relationships` and returns `Vec<GraphRelationship>`. Existing tests calling it (`tests/integration.rs:7231`) switch to `compute_relationships`.

- [ ] **Step 1: Write the failing tests.** In `src/git/graph.rs`:

```rust
#[test]
fn enrichment_never_erases_hydrated_relationships() {
    let hydrated = squash("d", "gone", &["deleted/branch"], RelationshipMatch::Exact);
    let mut snapshot = snapshot(vec![
        GraphCommit { oid: "d".into(), relationships: vec![hydrated.clone()], ..GraphCommit::default() },
    ]);
    apply_squash_enrichment(&mut snapshot, &[]);
    assert_eq!(snapshot.commits[0].relationships, [hydrated.clone()]);

    let fresh = squash("d", "t2", &["feature/new"], RelationshipMatch::Fuzzy { similarity_percent: 88 });
    apply_squash_enrichment(&mut snapshot, &[fresh.clone()]);
    assert_eq!(snapshot.commits[0].relationships.len(), 2);

    let renamed = GraphRelationship { source_refs: vec!["feature/renamed".into()], ..hydrated };
    apply_squash_enrichment(&mut snapshot, &[renamed]);
    let merged = snapshot.commits[0].relationships.iter().find(|r| r.source_oid == "gone").unwrap();
    assert_eq!(merged.source_refs, ["deleted/branch", "feature/renamed"]);
}

#[test]
fn apply_attaches_relationship_to_both_displayed_endpoints() {
    let relationship = squash("d", "s", &["feature/x"], RelationshipMatch::Exact);
    let mut snapshot = snapshot(vec![
        GraphCommit { oid: "d".into(), ..GraphCommit::default() },
        GraphCommit { oid: "s".into(), ..GraphCommit::default() },
    ]);
    apply_squash_enrichment(&mut snapshot, &[relationship.clone()]);
    assert_eq!(snapshot.commits[0].relationships, [relationship.clone()]);
    assert_eq!(snapshot.commits[1].relationships, [relationship]);
}
```

Add this helper to the test module (if `GraphRefCounts` lacks `Default`, construct it with its fields):

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
```

In `tests/integration.rs`:

```rust
#[test]
fn enrichment_persists_relationships_even_when_publication_is_stale() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();
    run_git(dir, &["checkout", "-q", "-b", "feature/sq"]);
    std::fs::write(dir.join("a.txt"), "a").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-q", "-m", "a"]);
    run_git(dir, &["checkout", "-q", "main"]);
    run_git(dir, &["merge", "-q", "--squash", "feature/sq"]);
    run_git(dir, &["commit", "-q", "-m", "squash sq"]);
    let destination = rev(dir, "HEAD");

    let lineage_root = tmpdir.lineage_root();
    let options = tmpdir.graph_options_for("main");
    let snapshot = graph::load_graph(dir, options.clone()).unwrap();
    let latest = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(1));
    let rx = graph::spawn_possible_squash_enrichment(
        snapshot, dir.to_path_buf(), Some("main".into()), 1, options.cache_root.clone(),
        Some(lineage_root.clone()), std::sync::Arc::clone(&latest),
    );
    let msg = rx.recv().expect("enrichment completes");
    // The App would now drop this message because a newer load started:
    latest.store(2, std::sync::atomic::Ordering::Release);
    assert_eq!(msg.generation, 1);
    let store = lineage::LineageStore::open(dir, &lineage_root).unwrap();
    let stored = store.lookup(&[destination.clone()].into_iter().collect()).unwrap();
    assert!(stored.iter().any(|r| r.kind == graph::RelationshipKind::SquashMerge && r.destination_oid == destination));
}

#[test]
fn cancelled_enrichment_publishes_nothing() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();
    let options = tmpdir.graph_options_for("main");
    let snapshot = graph::load_graph(dir, options.clone()).unwrap();
    let latest = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(9));
    let rx = graph::spawn_possible_squash_enrichment(
        snapshot, dir.to_path_buf(), Some("main".into()), 1, options.cache_root,
        Some(tmpdir.lineage_root()), latest,
    );
    assert!(rx.recv().is_err(), "a cancelled worker drops its sender without publishing");
}
```

Also add `cherry_pick_destination_outside_displayed_window_is_found` (its code is in Task C2 Step 1). Add `lineage` to the `use git_branch_manager::git::{…}` import list at the top of `tests/integration.rs`. Add a lineage root to `TestDir`: a `lineage_root: Option<tempfile::TempDir>` field created in `new`, an accessor `fn lineage_root(&self) -> lineage::LineageRoot`, inclusion in `preserve_tempdirs`, and `command.env("GBM_DATA_DIR", …)` in `manager_command`. `graph_options()` sets `options.lineage_root = Some(self.lineage_root())`.

- [ ] **Step 2: Run them and confirm they fail**

Run: `cargo test enrichment_never_erases apply_attaches enrichment_persists cancelled_enrichment`
Expected: compile errors, because the signatures don't match yet.

- [ ] **Step 3: Implement the merge.**

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
        current.destination_ref = incoming.destination_ref.clone();
        current.matching = current.matching.stronger(incoming.matching);
        current.source_refs.extend(incoming.source_refs.iter().cloned());
        current.source_refs.sort();
        current.source_refs.dedup();
    } else {
        existing.push(incoming.clone());
    }
}
```

- [ ] **Step 4: Implement the worker write-back and cancellation.**

```rust
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

pub fn spawn_possible_squash_enrichment(
    snapshot: GraphSnapshot, repo_path: PathBuf, requested_base: Option<String>, generation: u64,
    cache_root: CacheRoot, lineage_root: Option<LineageRoot>, latest_generation: Arc<AtomicU64>,
) -> Receiver<GraphEnrichmentMsg> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let cancel = EnrichmentCancel::new(latest_generation, generation);
        let updates = compute_relationships(&repo_path, &snapshot, requested_base.as_deref(), &cache_root, &cancel);
        // Persist before the staleness check: accepted findings are complete tuples
        // that remain true even when the UI no longer wants this generation.
        if let Some(root) = lineage_root.as_ref() {
            persist_relationships(&repo_path, root, &updates);
        }
        if cancel.is_cancelled() {
            return;
        }
        let _ = tx.send(GraphEnrichmentMsg { generation, updates });
    });
    rx
}

fn persist_relationships(repo_path: &Path, root: &LineageRoot, relationships: &[GraphRelationship]) {
    let Some(store) = LineageStore::open(repo_path, root) else { return };
    if let Err(error) = store.upsert(relationships) {
        tracing::warn!(target: "lineage", %error, "lineage upsert failed");
    }
}
```

Factor `resolve_base` out of the duplicated base and base-tip resolution at the top of both detectors. Its behavior is unchanged: requested base or `detect_base_branch`, then the displayed local-branch tip. `compute_squash_relationships` keeps the `load_patch_ids` worker pool unchanged, checks `cancel` before dispatching, and returns the relationships built in A1 Step 4 instead of per-OID updates. `load_graph_with_squash_annotations` becomes: `load_graph`, then hydration (C4), then `compute_relationships` with `EnrichmentCancel::never()`, then `persist_relationships` if `options.lineage_root` is set, then `apply_squash_enrichment`.

App wiring (`src/app.rs`):
  1. Add fields `lineage_root: lineage::LineageRoot` and `graph_generation_signal: Arc<AtomicU64>`.
  2. `with_cache_root` gains a trailing `lineage_root: lineage::LineageRoot` parameter. `main.rs` passes `lineage::LineageRoot::from_env()`; `App::new` (test) passes `LineageRoot::at(test_cache_root.path().join("lineage"))`.
  3. In `spawn_graph_load`, immediately after `self.graph_generation = …saturating_add(1)`, add `self.graph_generation_signal.store(self.graph_generation, Ordering::Release);`.
  4. At both `spawn_possible_squash_enrichment` call sites, append `Some(self.lineage_root.clone()), Arc::clone(&self.graph_generation_signal)`.

  The drain line `self.graph.apply_squash_enrichment(&msg.updates)` stays unchanged. `GraphState::apply_squash_enrichment` takes `&[GraphRelationship]`.

- [ ] **Step 5: Update the existing apply/stale tests** at `tests/integration.rs:7229-7380`. They construct `GraphEnrichmentUpdate`s and assert replacement. Convert them to `GraphRelationship` lists. Where a test asserted that an *empty* update **clears** markers, change the assertion to *retains*: that is the intended behavior change under constraint 5. Add the comment `// constraint 5: enrichment never erases relationships`.

- [ ] **Step 6: Run them and confirm they pass**

Run: `cargo test` then `cargo clippy --all-targets` then `cargo build`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src tests
git diff --cached Cargo.toml
git commit -m "feat: persist detected relationships and merge enrichment without erasing history"
```

### Task C4: Hydrate before publishing the structural snapshot

**Files:**
- Modify: `src/git/graph.rs` (`GraphLoadOptions` + `Default`, `spawn_graph_loader`, `load_graph_with_squash_annotations`, new `hydrate_lineage`)
- Modify: `src/app.rs:4649-4655`, `:4983-4990` (set `lineage_root`), and every `GraphLoadOptions { … }` literal without `..Default::default()` (add `lineage_root: None`)
- Test: `tests/integration.rs`, `src/app.rs` tests

**Interfaces:**
- Consumes: C1 store, C3 `apply_squash_enrichment`.
- Produces:

```rust
pub struct GraphLoadOptions { /* existing */ pub lineage_root: Option<LineageRoot> }   // Default: None
pub fn hydrate_lineage(repo_path: &Path, snapshot: &mut GraphSnapshot, options: &GraphLoadOptions);
```

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
    run_git(dir, &["merge", "-q", "--squash", "feature/hydrate"]);
    run_git(dir, &["commit", "-q", "-m", "squash hydrate"]);
    (source, rev(dir, "HEAD"))
}

#[test]
fn warm_load_hydrates_without_running_detection() {
    let (tmpdir, _repo) = setup_test_repo();
    let (source, destination) = squash_fixture(&tmpdir);
    // Populate lineage once through real detection.
    graph::load_graph_with_squash_annotations(tmpdir.path(), tmpdir.graph_options_for("main")).unwrap();
    // A structural load alone (no enrichment at all) now carries the markers.
    let rx = graph::spawn_graph_loader(tmpdir.path().to_path_buf(), tmpdir.graph_options_for("main"));
    let snapshot = rx.recv().unwrap().unwrap();
    let commit = snapshot.commits.iter().find(|c| c.oid == destination).unwrap();
    assert!(commit.is_possible_squash_merge());
    assert_eq!(commit.possible_squash_merge_sources(), ["feature/hydrate"]);
    let source_commit = snapshot.commits.iter().find(|c| c.oid == source);
    if let Some(source_commit) = source_commit {
        assert!(source_commit.relationships.iter().any(|r| r.destination_oid == destination));
    }
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
    assert_eq!(commit.relationships[0].source_oid, source);
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
        destination_ref: "develop".into(),
        source_oid: "1".repeat(40),
        source_refs: vec!["feature/elsewhere".into()],
    }]).unwrap();
    let rx = graph::spawn_graph_loader(tmpdir.path().to_path_buf(), tmpdir.graph_options_for("main"));
    let snapshot = rx.recv().unwrap().unwrap();
    let commit = snapshot.commits.iter().find(|c| c.oid == destination).unwrap();
    assert!(commit.relationships.iter().all(|r| r.destination_ref == "main"));
}

#[test]
fn unwritable_lineage_root_does_not_block_graph() {
    let (tmpdir, _repo) = setup_test_repo();
    let blocker = tmpdir.path().join("not-a-dir");
    std::fs::write(&blocker, "file").unwrap();     // create_dir_all fails on a file path
    let mut options = tmpdir.graph_options_for("main");
    options.lineage_root = Some(lineage::LineageRoot::at(blocker.join("lineage")));
    let snapshot = graph::spawn_graph_loader(tmpdir.path().to_path_buf(), options.clone()).recv().unwrap();
    assert!(snapshot.is_ok());
    assert!(graph::load_graph_with_squash_annotations(tmpdir.path(), options).is_ok());
}
```

App-level test in `src/app.rs` (constraint 8: "prove warm hydration works while fresh detection is blocked"):

```rust
#[test]
fn hydrated_markers_publish_while_enrichment_is_cancelled() {
    // Arrange a repo with a squash, populate lineage through the app's own root.
    let (dir, destination) = squash_repo_fixture();   // reuse or add next to existing graph fixtures
    let mut app = App::new(dir.path().to_path_buf(), "main".into(), Config::default());
    let options = graph::GraphLoadOptions {
        base_branch: Some("main".into()),
        cache_root: app.cache_root.clone(),
        lineage_root: Some(app.lineage_root.clone()),
        ..graph::GraphLoadOptions::default()
    };
    graph::load_graph_with_squash_annotations(dir.path(), options).unwrap();

    app.spawn_graph_load(50, false);
    // Block fresh detection: every enrichment worker for this load sees itself cancelled.
    app.graph_generation_signal.store(u64::MAX, Ordering::Release);
    wait_until(Duration::from_secs(10), || { app.drain_channels(); app.graph.snapshot().is_some() });
    let commit = app.graph.snapshot().unwrap().commits.iter().find(|c| c.oid == destination).unwrap().clone();
    assert!(commit.is_possible_squash_merge(), "marker came from hydration, not enrichment");
}
```

Use the existing deadline helper introduced in `598625c` ("replace fixed polling budgets with a deadline wait"); find it with `rg -n "fn wait_until|deadline" src/app.rs`. Use its real name and signature. Build `squash_repo_fixture` with the same git steps as `squash_fixture`, following the existing app-test fixture style near `src/app.rs:10463`.

- [ ] **Step 2: Run them and confirm they fail**

Run: `cargo test warm_load_hydrates hydrated_lineage_survives hydration_skips unwritable_lineage hydrated_markers_publish`
Expected: FAIL, because `lineage_root` is not a `GraphLoadOptions` field and `spawn_graph_loader` publishes no relationships.

- [ ] **Step 3: Implement.**

```rust
// GraphLoadOptions: add `pub lineage_root: Option<LineageRoot>`; Default sets `lineage_root: None`.

/// Hydrate stored relationships into a freshly loaded snapshot. Runs in the
/// loader worker; failures leave the snapshot unannotated rather than failing the load.
pub fn hydrate_lineage(repo_path: &Path, snapshot: &mut GraphSnapshot, options: &GraphLoadOptions) {
    let Some(root) = options.lineage_root.as_ref() else { return };
    let Some(store) = LineageStore::open(repo_path, root) else { return };
    let oids = snapshot.commits.iter().map(|commit| commit.oid.clone()).collect::<HashSet<_>>();
    let relationships = match store.lookup(&oids) {
        Ok(relationships) => relationships,
        Err(error) => {
            tracing::warn!(target: "lineage", %error, "lineage lookup failed");
            return;
        }
    };
    let relationships = relationships
        .into_iter()
        .filter(|r| options.base_branch.as_deref().map_or(true, |base| r.destination_ref == base))
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

In `load_graph_with_squash_annotations`, call `hydrate_lineage` right after `load_graph`. App: `spawn_graph_load` and the `:4983` options literal set `lineage_root: Some(self.lineage_root.clone())`. Mechanically add `lineage_root: None` to every remaining `GraphLoadOptions { … }` literal that lacks `..Default::default()`; find them with `rg -n "GraphLoadOptions \{" src tests`. Keep `TestDir::graph_options()` setting `Some(self.lineage_root())`.

- [ ] **Step 4: Run them and confirm they pass**

Run: `cargo test` then `cargo build`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src tests
git diff --cached Cargo.toml
git commit -m "feat: hydrate stored relationships before publishing the Graph snapshot"
```

### Task C5: Endpoint details and the presentation boundary

**Files:**
- Modify: `src/ui/info_modal.rs` (commit fields), `src/ui/commit_details.rs`
- Test: `src/ui/info_modal.rs` tests, `tests/integration.rs`

**Interfaces:**
- Consumes: `GraphCommit::cherry_pick_sources()`, `cherry_pick_destinations()`, and `relationships`.
- Produces: no new API.

- [ ] **Step 1: Write the failing tests.**
  - `info_modal`: a destination commit with one `CherryPick` relationship, from a source OID that is not in the snapshot, with `source_refs = ["feature/gone"]`, renders the field `Cherry-picked From` = `<short source> (feature/gone)`.
  - `info_modal`: a source commit with a `SquashMerge` fuzzy 91% relationship renders `Possibly Squash-merged Into` = `<short dest> (main, 91% similarity)`.
  - `info_modal`: a source commit with an exact squash relationship renders `Squash-merged Into` = `<short dest> (main)`.
  - Integration, constraint 7: `hydrated_fuzzy_lineage_does_not_change_branch_status`. Upsert a fuzzy `SquashMerge` relationship whose `source_refs = ["feature/fuzzy"]` for an unmerged real branch. Call `test_branches(&tmpdir, &repo, "main")` and assert the branch's `merge_status` is unchanged from a run with an empty lineage root, still `MergeStatus::Unmerged` or the fixture's baseline. Then load the graph and assert the destination commit has `fuzzy_squash_match()` = `Some(91)` and `is_possible_squash_merge()` = `false`.

- [ ] **Step 2: Run them and confirm they fail**

Run: `cargo test --lib info_modal` then `cargo test hydrated_fuzzy_lineage`
Expected: the info-modal tests FAIL because the fields are missing. The status test may already PASS; that is expected, because it pins a boundary rather than driving new code.

- [ ] **Step 3: Implement** the three fields in `info_modal.rs` commit fields, after the existing squash fields:

```rust
for relationship in commit.cherry_pick_sources() {
    fields.push(InfoField {
        label: "Cherry-picked From",
        value: format!("{} ({})", short_oid(&relationship.source_oid), relationship.source_refs.join(", ")),
    });
}
for relationship in commit.relationships.iter()
    .filter(|r| r.kind == RelationshipKind::SquashMerge && r.source_oid == commit.oid)
{
    let (label, detail) = match relationship.matching {
        RelationshipMatch::Exact => ("Squash-merged Into", relationship.destination_ref.clone()),
        RelationshipMatch::Fuzzy { similarity_percent } => (
            "Possibly Squash-merged Into",
            format!("{}, {similarity_percent}% similarity", relationship.destination_ref),
        ),
    };
    fields.push(InfoField { label, value: format!("{} ({detail})", short_oid(&relationship.destination_oid)) });
}
```

Use the module's existing short-OID helper; find it with `rg -n "fn short_oid" src/ui`. Add the same cherry line to `commit_details.rs`, styled with `theme.modal_secondary` / `theme.modal_commit` like the fuzzy line. The Graph row glyphs stay as they are; source and destination colors are task #054.

- [ ] **Step 4: Run them and confirm they pass**

Run: `cargo test` then `cargo build`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src tests
git diff --cached Cargo.toml
git commit -m "feat: show relationship provenance on both Graph endpoints"
```

### Task C6: Docs, timing, acceptance

**Files:**
- Modify: `CLAUDE.md` (Architecture → Git backend list, Data Flow step 1)
- Modify: `src/git/graph.rs` (tracing spans)

- [ ] **Step 1: Add timing spans** with `tracing::info_span!(target: "lineage", …)` around `hydrate_lineage` (fields `oids`, `found`), `compute_squash_relationships`, `compute_cherry_pick_relationships` (fields `sources`, `destinations_scanned`, `incomplete`), and `persist_relationships` (field `count`). Use `.entered()` guards. The existing subscriber (`src/main.rs:362-382`, default filter `git_branch_manager=debug`) records `time.busy` / `time.idle`.

- [ ] **Step 2: Document.** In `CLAUDE.md`:
  - Git backend list: add `lineage`.
  - Data Flow step 1: add one sentence — "The Graph loader hydrates stored squash and cherry-pick relationships from the durable lineage store (`git::lineage`, under the data directory, never pruned or cleared by `R`) before publishing; enrichment upserts new relationships and only ever adds to them."

- [ ] **Step 3: Measure on this repository.** These are not unit tests; record the results in the slice's final commit message body.
  1. `cargo build`, then remove this repo's lineage file under `~/Library/Application Support/git-branch-manager/`: run `ls` there and delete only `git-bm-lineage-*.sqlite3` for this repo.
  2. Run `cargo run`, open Graph, and note the wall time until markers appear. This is the cold load; expect it to be similar to today.
  3. Quit and run `cargo run` again. Markers should appear with the structural snapshot (warm load).
  4. From the debug log, record the `time.busy` of the four spans for both runs.
  5. State the result honestly: warm markers are immediate for *stored* relationships; anything new still waits for enrichment.

- [ ] **Step 4: Full verification**

Run: `cargo test`, `cargo clippy --all-targets`, `cargo build`
Expected: all pass. Check `git diff --cached Cargo.toml` shows no version line.

- [ ] **Step 5: Commit** — `git commit -m "docs: document relationship lineage and add enrichment timing spans"`

**Slice C acceptance:** all 7 requirements and 9 constraints are covered (see the trace table). Squash-merge into `main`, then a separate version-bump commit (minor: new feature).

---

## Slice D — `ct/graph-fuzzy-pair-cache` (cut from `main` after A merges)

### Task D1: Skip already-scored fuzzy pairs

**Files:**
- Modify: `src/git/cache.rs` (`fuzzy_pair` table, on-demand methods), `src/git/graph.rs` (fuzzy loop), `src/git/fuzzy_match.rs` (`pub const FUZZY_SCORING_VERSION: u32 = 1;`, plus a doc line saying to bump it whenever `score`, `classify`, or the thresholds change)
- Test: `src/git/cache.rs`, `tests/integration.rs`

**Interfaces:**
- Consumes: A1 `SourceTip`.
- Produces:
  - `BranchCache::lookup_fuzzy_pairs(&self, keys: &[String]) -> HashMap<String, Option<u8>>`
  - `BranchCache::store_fuzzy_pairs(&self, entries: &[(String, Option<u8>)])`
  - Key format: `format!("{destination}:{merge_base}:{tip}:v{FUZZY_SCORING_VERSION}")`.

- [ ] **Step 1: Write the failing tests.**
  - `cache.rs`: a round trip including `None` (scored, no match).
  - `cache.rs`: a lookup against a database lacking the table returns an empty map.
  - Integration, `fuzzy_pair_cache_skips_rescoring_and_preserves_result`: run `load_graph_with_squash_annotations` twice on the fuzzy fixture used by the scenario-20 tests. Between runs, assert the `fuzzy_pair` table holds one row per (base commit, diverged tip) pair. Use `rusqlite::Connection::open` on the cache file path; find it with `std::fs::read_dir(cache_root)`. Assert the second run yields the same `fuzzy_squash_match()` and sources.

- [ ] **Step 2: Run them and confirm they fail** — `cargo test fuzzy_pair`. Expected: compile errors.

- [ ] **Step 3: Implement.**
  - Add `CREATE TABLE IF NOT EXISTS fuzzy_pair (key TEXT PRIMARY KEY, percent INTEGER)` to `ensure_schema`, plus the two methods. Shape them like `lookup_commit_patch_ids` / `store_commit_patch_ids` in C2; if C has not merged yet, write them in full here with the same structure.
  - In the fuzzy loop, build every pair key first and call `lookup_fuzzy_pairs` once. Then, for a hit, use the stored `Option<u8>` without calling `score`. For a miss, score, push the result, and record `(key, percent)`. Call `store_fuzzy_pairs` once at the end.

- [ ] **Step 4: Run them and confirm they pass** — `cargo test`, `cargo build`.

- [ ] **Step 5: Commit** — `git commit -m "perf: cache fuzzy squash pair scores by commit identity"`

---

## Slice B — `ct/graph-git2-squash-patches` (cut from `main` after C merges)

### Task B1: In-process squash patch computation, gated on parity

**Files:**
- Modify: `src/git/graph.rs` (`compute_patch`, `GRAPH_DIFF_VERSION` 1 → 2)
- Test: `tests/integration.rs`

**Interfaces:**
- Consumes: the C2 `git2_patch_id` approach.
- Produces: `pub(crate) fn git2_diff_patch(repo: &git2::Repository, old: git2::Oid, new: git2::Oid) -> (Option<String>, Option<Vec<u8>>)`.

- [ ] **Step 1: Write the parity and benchmark test** `graph_git2_patch_matches_git_cli_patch_id`. Build commits that rename, add a binary file, change mode, and edit text. For each `(parent, commit)`, assert that `git2_diff_patch(...).0` equals the first field of `git diff --binary --full-index <parent> <commit> | git patch-id --stable`. Also assert that `fuzzy_match::score(git2_text, cli_text)` classifies as identical or near-identical; the fuzzy tier tokenizes text, so minor header differences are acceptable if the score is ≥ 0.99. **Stop condition:** if exact parity fails for any case, do not switch. Record the mismatching case in the task notes and end the slice; the user decides.

- [ ] **Step 2: Run it and confirm it fails** — `cargo test graph_git2_patch_matches`. Expected: compile error, because the function is missing.

- [ ] **Step 3: Implement `git2_diff_patch`.** Use `diff_tree_to_tree(old_tree, new_tree, DiffOptions::new().show_binary(true))`, `diff.patchid(None)`, and `diff.print(DiffFormat::Patch, …)`, appending `origin` + `content` bytes for `+`, `-`, and ` ` lines and raw content for headers. Make `compute_patch` call it with a per-worker `Repository::open`. Bump `GRAPH_DIFF_VERSION` to `2` so cached CLI-generated entries are not mixed in.

- [ ] **Step 4: Benchmark.** Using the C6 timing spans on this repo with a cold transient cache (R key, then reopen Graph), compare `compute_squash_relationships` `time.busy` before and after. Keep the switch only if it is at least as fast, per the user's rule: "prefer git2 operations over spawning git processes, if the git2 operation is at least the same performance". Otherwise, revert the call-site change and keep only the test, noting the numbers.

- [ ] **Step 5: Run the suite and commit** — `cargo test`, `cargo build`, then `git commit -m "perf: compute Graph squash patches in-process with git2"`.

---

## Requirement and constraint trace

| Item | Covered by |
| --- | --- |
| R1 document the problem | "Problem statement" section; C6 measurement |
| R2 durable records with the five fields, OIDs as text | C1 schema; C2/C3 producers |
| R3 uniqueness key, indexes, outside 60-day cleanup | C1 schema + `transient_cache_prune_and_clear_leave_lineage` |
| R4 persist exact and fuzzy squash and exact cherry-pick, keep the score | A1 pairs; C2 cherry pairs; C3 write-back; C1 `repeated_upsert…` |
| R5 hydrate early; upsert without duplicates; refresh ref snapshots | C4 hydration; C1 upsert/`shared_tip…`; C3 `merge_relationship` |
| R6 annotate destination and displayed source; survive deleted refs and pruned objects | C3 `apply_attaches…`; C4 `hydrated_lineage_survives…`; C5 details |
| R7 tests (reload, upserts, deleted refs and objects, isolation, retention, compatibility); "migration" replaced per user | C1 tests; C2 `commit_patch_ids_tolerate…`; C4 tests |
| K1 GraphSnapshot and existing channels only, workers only | Global Constraints; C3/C4 touch only loader, enrichment, apply, options |
| K2 complete records, key by type + OIDs | C1 |
| K3 identity gaps; bounded, cancellable discovery; no invented or partial tuples | A1 Step 4; C2 Steps 3–5 + bound/cancel tests |
| K4 hydrate before publishing; both endpoints; missing refs and objects | C4 |
| K5 enrichment never erases; stale publication rejected separately from upsert | C3 tests + worker order |
| K6 separate store; prune and R-key safe; common-dir identity; explicit test root | C1; `TestDir::lineage_root`; `GBM_DATA_DIR` |
| K7 exact vs fuzzy presentation; no deletion authority | A2; C5 `hydrated_fuzzy_lineage_does_not_change_branch_status` |
| K8 listed tests, including warm hydration with detection blocked | C1–C5; `hydrated_markers_publish_while_enrichment_is_cancelled` |
| K9 honest performance boundary | "Performance boundary" section; C6 Step 3 |

## Open decisions (from brainstorming; confirm before cutting branches)

1. **Orphaned lineage files.** Brainstorming chose "delete the lineage file only when its repository's common Git dir no longer exists". This plan stores the common dir in the `meta` table but does **not** auto-delete, because a repo on an unmounted volume or a moved repo would lose irreplaceable history. Files are small. Adding the deletion later is a contained pruner change.
2. **Base for slice A.** Local `main` vs `origin/main`; see the caveat under "Branching and slices".
