# Proposed graph refresh and lineage design

This assessment compares task #047 and task #053 against local `main` at `598625c8b2c4709eab7da4f6453f539e198c686e`. It builds on the [UI stall review](../../reviews/2026-10-02-ui-stall-regression-review.md). Task requirements were read again from task-db on 2026-10-02 using project `github.com-personal/cjthompson/git-branch-manager`; no task records were changed.

**Recommendation:** selectively revert the incremental topology updater introduced by `4bc8f7a`, preserve #047's automatic background Graph refresh, and retain lightweight worker observations to avoid unnecessary refreshes. Implement #053 independently to hydrate persistent relationship markers. Neither that rollback nor #053 fixes the preexisting synchronous branch reload or worktree concurrency; those need separate repairs.

## Original task 047 requirements and judgment

The four recorded requirements are:

1. “Source task: github.com-personal/cjthompson/git-branch-manager#024 — Job completion only refreshes return_view; other views an action mutates go stale.”
2. “The listed Branches, Remotes, and Worktrees refresh cases already exist; add Graph refresh when a completed action changes refs or metadata shown by an already-loaded Graph, even if another view initiated the job.”
3. “Identify only Graph-affecting actions and avoid unnecessary Graph refreshes.”
4. “Add a regression test for Graph becoming stale after an action completes from another view.”

The task requires freshness, selective invalidation, and a regression test. It does not require an incremental topology algorithm, a ban on full background loads, or traversing history before and after each operation.

| Requirement | `1518390`, original #047 implementation | `4bc8f7a`, later replacement at current main |
| --- | --- | --- |
| Refresh an already-loaded Graph after an action from another view | Implemented through the existing background graph loader. | Implemented through observed deltas and the incremental updater. |
| Preserve dependent Branches, Remotes, and Worktrees refreshes | Preserved. | Preserved; their preexisting synchronous Branches path remains. |
| Avoid unnecessary Graph refreshes | Uses an action allowlist and existing-snapshot guard. It excludes push on an incorrect general assumption about local remote-tracking refs. It can also reload after an eligible action that made no change. | Avoids loading an entirely unopened Graph in the scheduler, but performs observations for jobs anyway. Its action map includes every push and remote deletion; unchanged deltas still enter metadata refresh work. The unused UI observation and discarded ancestry pass are unnecessary work. |
| Regression test for another view initiating a mutation | Present. | Present, with more tests for fetching, stale enrichment, cancellation, and publication. Small fixtures do not establish responsiveness. |

Both implementations satisfy the central freshness case. The replacement exceeds the required scope and violates the intended restraint through redundant work. Restoring the original implementation unchanged would restore a policy gap, so use a selective rollback plus corrected invalidation rather than a blanket `git revert`.

### Push policy must use actual local observations

The original allowlist says remote-only writes leave local remote-tracking refs unchanged. A local bare-remote experiment contradicted that assumption: after `git push origin main`, `refs/remotes/origin/main` changed to the newly pushed commit. A push can therefore change visible Graph refs or tracking counts.

Use observed canonical ref identities and OIDs to decide whether a push affects the current Graph. A changed remote ref is relevant when remotes are displayed or when it is an upstream of a displayed local branch. An unchanged push and changes to unrelated hidden remote refs should not trigger a Graph reload. Preserve local branch, tag, HEAD, worktree, and upstream-configuration invalidation.

## Task 053 requirements and judgment

The seven recorded requirements are:

1. “Document the existing problem: current graph loading persists diff-derived patch data, but it recomputes cherry-pick detection and does not persist final squash/cherry relationships. Relationship annotations arrive only after background enrichment completes, leaving the graph without markers for roughly 30 seconds on this repository.”
2. “Add durable repository-scoped lineage records with fields: type (sm for squash merge or c-p for cherry-pick), destination_hash (full destination commit OID), destination_ref (destination ref name captured at detection), source_hash (full original source commit OID), and source_ref (original branch/ref name captured at detection). Store OIDs as text so records remain useful after source refs are deleted and Git prunes source objects.”
3. “Use a stable uniqueness key based on relationship type plus full destination and source OIDs, and index destination_hash and source_hash for graph lookup. Keep this historical lineage outside the existing 60-day idle cache cleanup so it survives ordinary cache expiry.”
4. “Persist accepted exact and fuzzy squash relationships and exact cherry-pick relationships when detection completes. Preserve a fuzzy squash similarity score when one exists, so rendering can show the same confidence after restart.”
5. “On repository load, hydrate cached lineage early enough for graph markers and relationships to render before any expensive fresh detection runs. Fresh detection should upsert records and refresh branch/ref snapshots without duplicating relationships.”
6. “Use cached lineage to annotate the destination commit and, when the source commit is present in the loaded graph, its source commit. A missing source ref or pruned source object must not erase the destination's historical provenance.”
7. “Add migration and focused tests covering cache reload, repeat upserts, deleted source refs or unavailable source objects, per-repository isolation, retention beyond transient-cache pruning, and compatibility with existing cache data.”

The current implementation does not satisfy #053. `BranchCache` stores patch IDs and diff bytes, rather than final relationship records. `compute_cherry_pick_updates` runs `git cherry` again. `compute_possible_squash_updates` and fuzzy scoring produce annotations only after enrichment. The roughly 30-second marker delay is a recorded task observation, not a new benchmark from this investigation.

| Problem | Background refresh from #047 | Incremental updater in `4bc8f7a` | Persistent lineage from #053 |
| --- | --- | --- | --- |
| Ref and commit topology changes after an operation | Solves this. | Solves this with additional machinery. | Does not solve this. Cached relationships do not discover new refs or commits. |
| Markers missing during enrichment after restart | Does not solve this. | Preserves some in-memory annotations during a session; does not persist them. | Directly addresses this through early hydration. |
| Source refs deleted or source objects pruned | Fresh detection loses its source inputs. | Does not create durable provenance. | Directly addresses this through stored source/destination OIDs and ref snapshots. |
| New UI-thread repository scan | Existing background loader avoids the new scan. | Introduces the scan at `app.rs:586`. | Does not remove that call. Database hydration must itself stay in workers. |
| Preexisting 26.7-second synchronous Branches refresh | Remains. | Remains. | Remains. |
| Excessive concurrent worktree status scans | Remains. | Remains, with additional graph work. | Does not bound workers. |

**#053 is the better solution for persistent relationship markers, but it is a complement to #047.** It justifies treating lineage persistence as a separate subsystem rather than relying on incremental snapshots to retain relationships. It does not justify reverting automatic Graph freshness. Recommend reverting the unnecessary topology engine, preserving automatic refresh, and implementing lineage separately.

Cached positive relationships permit immediate annotations on warm loads. Absence of a record is not proof that no relationship exists. Cold loads and newly created relationships still require background detection. Merely persisting positive records does not eliminate all fresh detection or guarantee a particular speedup.

### Important implementation implications for 053

Persist relationship tuples, not the current display flags:

- Exact squash detection must retain the source tip OID together with its names; `PatchTarget::BranchTip` currently retains names while its result path discards source identity.
- Fuzzy detection must retain each accepted source/destination pair and score; the current maximum-percent reduction loses which source produced the score.
- `git cherry` reports the original source commit with a minus marker; it does not provide the destination commit OID. Map that source patch to destination commits before writing a complete cherry-pick record. Define an explicit cancellable discovery boundary for destinations outside the displayed window; never invent a destination or persist a partial tuple as complete lineage.
- Store one record per `(type, destination_hash, source_hash)`. Upsert ref snapshots without erasing previously known provenance when a live ref disappears. Store a nullable similarity score, preserving the distinction between exact and fuzzy classification.
- Hydrate both visible source and destination commits from stored OIDs without resolving the source object. Preserve “possible” or fuzzy presentation; a stored similarity match must not become a claim of definite merge or authorize a destructive action.
- Store lineage in a separate per-repository database under application data, using the Git common directory as identity. Do not put it in the deletable `BranchCache` file: both 60-day pruning and the R-key cache clear can remove that database. Use an explicit temporary lineage root in tests, and leave the existing transient schema compatible.
- Send hydrated snapshots through the existing worker channel. A stale fresh-enrichment batch must merge/upsert accepted lineage rather than clear hydrated historical relationships with default false flags.

The paired-identity gap means #053 needs detector changes and hydration integration, rather than a schema-only PR. Ship working persistence and displayed hydration together. Relationship connectors and paired colors belong to the existing dependent tasks #054 and #055, not to this repair.

## Options and rollback boundaries

| Option | Benefit | Cost or regression | Judgment |
| --- | --- | --- | --- |
| Repair the current incremental engine | Removing the unused UI scan and discarded pass is small. | Retains a second topology engine, full observations, known root-handling failures, and additional lifecycle state not required by #047. | Reasonable emergency patch; weaker long-term simplification. |
| Selectively revert incremental topology and use background full loads | Removes the direct UI regression and redundant ancestry engine, reuses established loaders, and keeps #047. | Structural loads may still be expensive; preserve selection, coalesce requests, and hydrate lineage separately. | Recommended. |
| Revert `4bc8f7a` literally | Removes introduced scans and updater code. | Does not apply cleanly over later primary-check and test changes. Also loses useful upstream selection, cancellation, and view-state behavior unless preserved. | Use only as a starting reference in isolation. |
| Revert both `1518390` and `4bc8f7a`, or the entire reviewed range | Removes automatic refresh work. | Reintroduces stale Graph behavior and discards unrelated features. #053 does not replace freshness. | Reject. |

Keep these behaviors while removing the topology engine:

1. Refresh after relevant mutations from another view without navigating the user.
2. Use the configured upstream in Graph tracking metadata rather than a same-name origin guess.
3. Preserve selected OID, vertical/horizontal viewport, graph options, and pending jump target across refreshes; keep the existing presentation behavior of `GraphState::apply_incremental_result` even when its input is a full worker snapshot.
4. Retain fetch receivers through cancellation, refresh after partial mutations, and preserve later primary-check cancellation/overlay ownership.
5. Drop stale enrichment generations and coalesce invalidations during an in-flight structural load.

The observations retained for invalidation must read ref OIDs, HEAD, upstream names, and worktree ownership in a worker. They must not calculate ahead/behind or commit reachability. OID movement already supplies the invalidation signal; display counts are computed by the actual graph loader.

## Independent delivery units

1. **Graph refresh repair:** selectively remove the incremental engine, make invalidation cheap, coalesce full background loads, and retain the behaviors above. The [implementation plan](../plans/2026-10-02-graph-refresh-rollback.md) covers this PR.
2. **Branches responsiveness:** move `refresh_branches` traversal, candidate creation, and cache loading into a worker; publish owned results and ignore replaced receivers. This directly addresses the measured multi-second post-operation freeze. Reuse the startup phase-one loading pattern; bound publication work on the UI thread.
3. **Worktree concurrency:** replace one status thread per worktree with a shared queue capped at four workers; stop taking new work when the receiver closes. This caps that source of contention while preserving streamed results. It does not prove the existing four-worker squash/cherry pools need smaller limits; profile those separately.
4. **Task #053:** deliver persistent paired lineage plus early hydration in a separate PR. It can proceed independently of the topology rollback because both use `GraphSnapshot` and the existing loader/enrichment channels. All seven requirements must be satisfied before completing #053.

The first three units are independently shippable fixes. Task #053 is a feature with its own detector/storage/hydration acceptance gate. Do not combine a bare migration or empty cache API into a separate PR.

## Requirements for the graph refresh repair

- No repository traversal, Git subprocess, or SQLite operation runs while publishing a Graph result on the UI thread.
- A completed mutation refreshes an already-loaded Graph when its visible refs, topology, HEAD, worktree ownership, or local upstream metadata changed.
- Unchanged operations and unrelated hidden remote ref changes do not refresh Graph.
- Structural refresh uses the existing background full loader, with at most one retained structural worker result channel and one coalesced follow-up request.
- A mutation observed during a structural load prevents publishing that load as current and schedules a fresh load using the latest options.
- Fetch cancellation and primary-check overlay ownership remain correct.
- The existing transient cache, configured upstream selection, fuzzy presentation, and primary-branch code-check behavior remain compatible.
- Feature-branch version edits remain local; acceptance, merge, and a separate stable version bump are later actions.

## Investigation evidence and limits

A reverse-apply check of `4bc8f7a` against current `main` failed at `src/app.rs:1102`. In a disposable source snapshot, reversing the patch produced four rejected App hunks. Resolving those while preserving the later primary-check ownership guard produced a compiling rollback prototype. Both original #047 tests passed, as did `late_canceled_fetch_completion_preserves_primary_check_ownership`.

This is feasibility evidence for rollback, not validation of the entire recommended selective implementation. The prototype is in `/private/tmp/gbm-revert-evaluation.kPzMcE`; it was not applied to the working checkout or committed. The recommendation additionally preserves upstream/viewport behavior and corrects push invalidation, which that literal rollback prototype does not fully cover.

The push observation experiment used a local bare remote and `/usr/bin/git` and verified that a push advanced the local remote-tracking ref. The actual repository build passed after the experiment. No profile measured the new capture duration, resource saturation, or the performance of a completed #053 implementation. The final acceptance run must reproduce the real stalls with timestamped monotonic stage timings.
