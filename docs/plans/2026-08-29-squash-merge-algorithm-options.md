# Squash-Merge Detection Algorithm Options

**Status:** Research and design plan

**Date:** 2026-08-29

**Scope:** Possible squash-merge indicators in the Graph tab and squash-merge
status in the Branches and Remotes views.

## Goal

Make squash-merge detection useful without blocking the initial Graph render,
while keeping its result honest about what Git can and cannot prove.

The product should distinguish three different claims:

1. **Already integrated:** merging the branch would add no net content to the
   selected base.
2. **Possible squash at commit X:** the branch's aggregate change matches a
   particular base commit and the surrounding topology supports that
   interpretation.
3. **Proven squash:** retained forge or pull-request metadata identifies the
   squash operation and its resulting commit.

Git history alone generally supports the first claim and can provide evidence
for the second. It cannot reliably establish the third when identical changes
could have been produced independently.

## Current implementations

### A. Graph possible-squash marker

Owner: `src/git/graph.rs::annotate_possible_squash_merges`

The Graph loader computes a stable patch ID for each eligible one-parent base
commit (`parent -> commit`) and for each displayed, diverged local branch
(`merge-base -> branch tip`). Equal patch IDs mark the matching base commit's
`GraphCommit::is_possible_squash_merge` flag. The implementation uses
`--binary`, `--full-index`, `--no-ext-diff`, `--no-textconv`, and
`git patch-id --stable`.

Strengths:

- Returns a candidate base commit, which is necessary for a Graph marker.
- Excludes regularly merged, ambiguous, empty, and otherwise ineligible
  relationships.
- Fails closed when the relevant history is outside the displayed
  `max_count` window.
- Does not depend on the Branches status cache.

Limitations:

- Its bounded candidate set lowers recall.
- It currently annotates the snapshot before returning it, so the Graph stays
  in `Loading graph...` until patch matching finishes.
- Equal patch IDs are evidence of equal normalized changes, not proof of
  historical provenance.

### B. Branches and Remotes squash status

Owners: `src/git/merge_detection.rs::is_squash_merged` and
`src/git/squash_loader.rs::spawn_squash_checker`

The detector finds the branch merge base, creates a synthetic one-parent
commit whose tree is the branch tip's tree, and runs:

```text
git cherry <base> <synthetic-branch-commit>
```

A `-` result means Git found a patch-equivalent non-merge commit in the base
history. The loader runs this against local and remote bases and persists the
result through `BranchCache`.

Strengths:

- Answers a useful branch-level content question.
- Normally searches beyond the Graph display window.
- Uses Git's native `cherry`/patch-equivalence behavior.
- Works for Branches and Remotes through a shared loader.

Limitations:

- Returns only a Boolean status, not the matching base commit OID.
- Can match an unrelated commit with the same normalized patch.
- Does not by itself prove a squash or identify a split/partial landing.
- The current cache treats `SquashMerged` as permanent, so a branch that later
  advances can retain stale status.

## Algorithm options

### Option 1: Keep the two detectors separate

Retain A for the Graph's per-commit marker and B for Branches/Remotes status.
Improve only their individual boundaries and tests.

This has the lowest implementation risk, but it preserves different scopes,
different Git invocation details, and different cache behavior.

### Option 2: Make Algorithm B return matching OIDs

Extend the branch-level detector to return the base commit or commits whose
patch IDs match the branch's aggregate patch. This would allow an unbounded
version of A to act as a Graph fallback.

Required decisions:

- Whether to scan all base non-merge commits or use a configurable bound.
- How to report duplicate matching patch IDs.
- Whether to use the same explicit diff options as A for binary, rename, and
  external-filter behavior.
- How to represent “integrated but no unique candidate” in the UI.

### Option 3: Layered hybrid detector

Use cheap topology and tree checks before patch matching, then preserve the
distinction between branch-level integration and commit-level attribution:

```text
proven_squash = retained PR/forge metadata identifies the operation

already_integrated =
    ancestor or identical tree
 OR merge-tree produces the base tree without new branch changes
 OR branch-level patch-equivalence succeeds

possible_squash(commit) =
    valid diverged topology
AND aggregate branch patch matches commit's parent-to-commit patch
AND merge-tree confirms that the branch adds no tree changes to that commit
```

This is not a binary OR across all signals. OR is appropriate for the broad
`already_integrated` claim; AND and explicit candidate identity are needed for
the more specific Graph claim.

`git merge-tree --write-tree` is a content/merge simulation signal, not proof
that a squash happened. It should therefore confirm or classify content
integration rather than independently create a squash marker.

### Option 4: Indexed patch and coverage analysis

Build a per-repository index of patch IDs for relevant base commits. Use it to

- map an aggregate branch patch to candidate base OIDs;
- detect individual cherry-picked commits;
- report partial coverage when only part of a branch has landed; and
- avoid repeatedly launching Git subprocesses for unchanged OID pairs.

This improves performance and makes richer status output possible, but it does
not change patch-ID false-positive semantics. Empty patches, duplicate IDs,
renames, binary files, and diff-option parity need explicit tests.

### Option 5: Forge metadata first

When a GitHub pull request and its retained merge metadata are available, use
that relationship before Git-only inference. GitHub documents squash-and-merge
as producing one squashed commit. When metadata is unavailable, fall back to
the heuristic statuses above and retain the “possible” label.

References:

- [GitHub pull-request merge methods](https://docs.github.com/en/pull-requests/collaborating-with-pull-requests/about-pull-request-merges)
- [GitHub pull-request API](https://docs.github.com/en/rest/pulls/pulls)

### Options that should remain supplemental

- [`git range-diff`](https://git-scm.com/docs/git-range-diff) is useful for
  comparing rewritten commit series, but is not a many-to-one squash detector.
- Commit subject/message matching is not reliable evidence.
- JGit or libgit2 patch-ID APIs could reduce subprocess overhead, but do not
  improve inference semantics without changing the surrounding algorithm.

## Combination semantics

| Signal | What it establishes | Combination rule |
| --- | --- | --- |
| Ancestry or equal trees | No remaining content difference | OR for `already_integrated`; not proof of squash |
| `merge-tree` produces the base/target tree | A simulated merge adds no net tree changes | OR for content integration; AND for candidate confirmation |
| Aggregate patch ID matches a base commit | A likely specific squash-equivalent commit | Required for `possible_squash(commit)` |
| `git cherry` finds an equivalent base patch | A branch-level equivalent patch exists | Fallback or branch-level status only unless it returns OIDs |
| PR/forge metadata | Recorded integration relationship | Preferred authoritative result |

The following disagreements should remain visible during research rather than
being collapsed by OR:

- **A false, B true:** B may have found a match outside the Graph window, but
  Graph does not know which row to mark.
- **A true, B false:** investigate base selection, local/remote scope, cache
  freshness, and diff-option differences.
- **Both true:** stronger evidence for content equivalence, but still not
  historical proof without metadata.

## Cache options

The existing Branches cache is keyed around branch/ref names and commit hashes
and stores user-facing `MergeStatus` values. It is not sufficient by itself for
Graph attribution because Graph needs candidate OIDs and must account for a
moving base and displayed-history bounds.

Research whether to use a separate Graph cache or extend the cache schema. A
safe Graph cache entry should include at least:

```text
repository identity
algorithm/version
base tip OID
branch tip OID
merge-base OID
candidate base commit OID
patch/diff option profile
result and failure reason
```

Positive and negative results should be invalidated when any relevant OID or
algorithm option changes. Failed, ambiguous, and empty-patch results should
not be treated as durable positive squash evidence. Cache access must remain
safe while the asynchronous Graph loader and Branches/Remotes loaders share a
repository cache location.

## Loading and UI direction

The initial Graph snapshot should be published as soon as structural graph
loading completes. Squash annotation should then run as enrichment:

1. Render the DAG, refs, and commit summaries immediately.
2. Start squash analysis in a bounded background worker.
3. Stream or apply annotation updates to the current Graph snapshot.
4. Discard results belonging to an obsolete reload generation.
5. Keep the marker labeled as possible unless stronger metadata exists.

The enrichment state should be independent from the structural
`Loading graph...` state so a slow or failed squash check cannot hide an
otherwise usable graph.

## Research and test matrix

Before choosing an implementation, measure both correctness and latency for:

- one-commit and multi-commit squash merges;
- regular merges and merge commits in the branch;
- cherry-picks, reordered commits, and rebases;
- overlapping changes and conflict-resolution edits;
- binary files, renames, mode changes, and whitespace-only changes;
- empty, reverted, and net-zero branches;
- duplicate or independently recreated patches;
- multiple merge bases and shallow/out-of-window history;
- local base versus remote base divergence;
- branch advancement after a cached positive result;
- large histories with many displayed branch tips;
- initial Graph render latency while enrichment is deliberately delayed.

Record for each case:

- the expected product claim (`integrated`, `possible`, `proven`, or
  `unmerged`);
- candidate commit OIDs, if any;
- false-positive and false-negative behavior;
- subprocess count and wall-clock time;
- cache hits, misses, invalidations, and stale-result protection.

## Recommended starting direction

1. Decouple Graph rendering from squash enrichment.
2. Keep Algorithm A as the Graph attribution baseline because it returns a
   specific candidate and fails conservatively.
3. Keep Algorithm B for Branches/Remotes branch-level status, but repair its
   cache invalidation and consider returning matching OIDs.
4. Add `merge-tree` as a content-equivalence confirmation, not as a standalone
   squash proof.
5. Prototype a separate OID-based Graph cache only after the asynchronous
   ownership and reload-generation behavior is settled.

## Follow-up project tasks

- Fix Graph loading so structural rendering is not blocked by squash-merge
  enrichment.
- Add cache support for Graph possible-squash analysis with OID-aware
  invalidation.

