# UI stall regression review

Reviewed `c145d91836789ba3b9ae3117e16111dbf47134fe...598625c8b2c4709eab7da4f6453f539e198c686e` on 2026-10-02. The endpoint is the resolved local `main`; the base is its ancestor. The range contains 26 commits and changes 43 files.

The range introduces the synchronous graph-state scan identified in the log review and adds avoidable background Git work. The synchronous branch refresh and one-thread-per-worktree design were already present at the base. The new graph work can contribute to stalls, but the available logs do not measure its individual contribution.

All source line ranges below refer to the reviewed `main` unless explicitly labeled otherwise. The companion [introduced changes diff](2026-10-02-ui-stall-introduced-changes.diff) contains exact, unedited hunks from `4bc8f7a` against its parent, including their original old/new line numbers. Excerpts below select relevant added lines; ellipses omit unrelated context.

## Attribution of the three original issues

| Original issue | Contribution from this range | Introducing commit and task association |
| --- | --- | --- |
| 1. Synchronous branch refresh after operations | The blocking branch traversal and refresh callers already existed. The range changes cache loading but does not introduce this traversal. | Existing at the base; no new task attribution for this mechanism. |
| 2. Synchronous graph-state capture in the UI channel drain | Introduced. Every completed structural graph load now performs an uncached repository scan before the UI can continue. | `4bc8f7a371a574f999016aab77685132fc42a882`, “Automatically update Graph after Git actions.” Related feature task: #047, with the qualification below. |
| 3. Background concurrency and contention | The existing worktree and squash/cherry worker counts remain. Graph refreshes after actions from other views, repeated repository captures, and a discarded ancestry pass add work that can compete with those workers. Actual contention is unmeasured. | `1518390` implements task #047; `4bc8f7a` replaces its full-refresh path and adds the scans and discarded pass. The original concurrency design predates the range. |

Task #047 is “Refresh Graph when completed jobs change its refs,” in P002, succeeding #024. Its requirements explicitly say the Branches, Remotes, and Worktrees refresh cases already exist and ask to avoid unnecessary Graph refreshes. Commit `1518390de1816d74f950d8dedcb8081cdcdadbc5` has that exact task title. The later `4bc8f7a` commit explicitly says it supersedes the full-refresh implementation from `1518390`. This establishes the feature relationship; neither the later commit message nor the task record returned by `task get` explicitly assigns `4bc8f7a` to a numbered task. There is no confirmed separate task number for that later implementation.

## Introduced UI thread scan

**Priority: P1.** `4bc8f7a` added a call to `GraphRepositoryState::capture` inside `App::drain_channels`. It runs whenever the structural graph result arrives, including an error result, before publishing the result or processing more input.

The call scans refs, calculates ahead/behind separately for every local branch with an upstream, and runs `git worktree list`. These operations execute synchronously on the UI thread. The capture has no ahead/behind cache and no cancellation or time budget. Different branches with the same pair of tips also repeat the traversal.

The captured value is assigned to `graph_repository_state`, but that field has no reads in the reviewed code. Its other occurrences are its declaration, initialization, and another assignment in `request_graph_update`. The added UI-thread scan therefore produces no consumed result at this revision.

Files and ranges:

- [src/app.rs](../../src/app.rs:571), lines **571–596**: structural-result drain; added blocking call at **586**.
- [src/git/graph.rs](../../src/git/graph.rs:193), lines **193–241**: new capture implementation; uncached ahead/behind loop at **222–232**, worktree subprocess at **237–239**.
- [src/app.rs](../../src/app.rs:4990), line **4990**: the other assignment to the unused field.

Relevant added code from the introducing diff:

```diff
             self.clear_toast();
+            self.graph_repository_state = graph::GraphRepositoryState::capture(&self.repo_path).ok();
+            let mut pending = std::mem::take(&mut self.pending_graph_deltas);
```

```diff
+        for branch in repo.branches(Some(git2::BranchType::Local)).map_err(|error| error.message().to_string())? {
...
+            let tracking = match (branch.get().target(), upstream.get().target()) {
+                (Some(local), Some(remote)) => repo.graph_ahead_behind(local, remote).ok().map(|(ahead, behind)| (Some(ahead.try_into().unwrap_or(u32::MAX)), Some(behind.try_into().unwrap_or(u32::MAX)))),
+                _ => None,
+            }.unwrap_or((None, None));
...
+        for worktree in crate::git::worktree::try_list_worktrees(repo_path)? {
```

Recommendation: remove the unused capture, or move any repository observation that is actually needed into the structural graph worker and return its owned result through the channel. Reuse OID-keyed ahead/behind results where needed.

## Introduced background work

### Graph refreshes after actions from other views

Task #047, implemented in `1518390`, changed the final Graph refresh condition from `origin == ViewId::Graph` to “Graph-affecting action and an existing Graph snapshot.” This starts a Graph load and subsequent enrichment after eligible jobs initiated from Branches, Remotes, Tags, or Worktrees as well. It can overlap the existing dependent-view loaders and detectors. This is intended stale-view correction, but it increases occasions for background Graph work.

The exact change in `src/app.rs` at commit `1518390`, lines **4493–4503**, is:

```diff
-        if origin == ViewId::Graph {
+        if action_affects_graph(action) && self.graph.snapshot().is_some() {
             self.refresh_view_data(ViewId::Graph);
         }
```

At reviewed `main`, `4bc8f7a` supersedes that full-refresh condition with the incremental scheduler: [src/app.rs](../../src/app.rs:4923), **4923–4940**, followed by enrichment scheduling at **661–670**. The original `1518390` condition is no longer present. This scheduling change does not introduce the per-worktree worker count or change squash/cherry pool limits.

### An ancestry pass whose result is discarded

**Priority: P2.** The new incremental updater clones all previously displayed commits, then filters them by checking reachability from resulting ref roots. Each check can walk repository history. It immediately clears the filtered records before filling them from a separate revision walk, so the first pass contributes no records to the final result.

For C displayed commits and R ref roots, the pass can perform up to C × R ancestry checks before roots are deduplicated. The display limit does not bound the history examined by each ancestry check. The later record-building loop does stop at `max_count`; this review does not claim that loop lacks a break.

Introducing commit: `4bc8f7a`; related task association is the qualified #047 relationship above. The commit message itself lists the discarded ancestry pass as remaining work.

File and ranges: [src/git/graph.rs](../../src/git/graph.rs:398), lines **398–404** for the discarded pass, **408–416** for root deduplication and clearing, and **418–436** for the separately bounded refill. The updater runs in a background thread at lines **61–74**, so this is added background work, rather than a direct synchronous UI block.

```diff
+    let mut records = previous.commits.clone();
+    records.retain(|commit| {
+        git2::Oid::from_str(&commit.oid).ok().is_some_and(|commit_oid| {
+            eligible_roots.iter().any(|root| *root == commit_oid || repository.graph_descendant_of(*root, commit_oid).unwrap_or(false))
+        })
+    });
...
+    records.clear();
```

Recommendation: remove the discarded pass and retain the single refill path. Measure the remaining traversal separately before claiming a particular improvement in stall duration.

### Repeated full captures around jobs and fetches

`4bc8f7a` also added before/after captures around every queued job and both fetch paths. Each capture repeats the uncached ahead/behind and worktree scan described above. The incremental updater performs another capture when refs change.

The job captures run even when Graph has never loaded and for actions whose resulting Graph update will be skipped. Operation results are sent only after the post-operation capture finishes. This can delay visible completion and queue advancement; those captures run in background threads, so they do not directly block input handling. Extra CPU and filesystem contention is a possible contribution to issue 3, not a measured conclusion from the logs.

Files and ranges, all introduced by `4bc8f7a`:

- [src/job_queue.rs](../../src/job_queue.rs:248), **248–269**: pre-job capture at **249**, post-job capture at **263**, result publication at **269**.
- [src/app.rs](../../src/app.rs:4788), **4788–4796**: automatic and remote-view fetch captures.
- [src/app.rs](../../src/app.rs:5019), **5019–5032**: modal fetch captures.
- [src/git/graph.rs](../../src/git/graph.rs:382), **382–389**: another capture in the incremental updater.

Representative job diff:

```diff
         std::thread::spawn(move || {
+            let before = GraphRepositoryState::capture_with_head(&repo_path, &graph_head_path);
...
+            let after = GraphRepositoryState::capture_with_head(&repo_path, &graph_head_path);
+            let graph_delta = match (before, after) {
+                (Ok(before), Ok(after)) => Ok(GraphRepositoryDelta::between(before, after)),
+                (Err(error), _) | (_, Err(error)) => Err(error),
+            };
+            let _ = graph_delta_tx.send(graph_delta);
             let _ = op_tx.send(results);
```

Recommendation: observe the minimal ref/HEAD/worktree state needed for deltas, avoid recomputing all tracking counts for every observation, and skip Graph-specific work when its result cannot be used. Preserve the before/after mutation contract when changing observation.

## Existing mechanisms retained by this range

### Synchronous branch refresh

At the base, `refresh_branches` already calls `list_branches_phase1` synchronously, calculates the same fingerprint without using unchanged inputs to skip work, and spawns the same squash/cherry checks afterward. The Branches refresh route from completed jobs also exists at the base.

Current ranges: [src/app.rs](../../src/app.rs:4800), **4800–4828**, and [src/git/branch.rs](../../src/git/branch.rs:77), **77–84**. The load call at current line **4821** is attributed by blame to `63552cc`, outside the requested range. The job refresh routing predates the range in `f38abb8` and `3111083`.

The range changes cache calls in this function through `1a7991e`, “share cache across repository worktrees”:

```diff
-        let new_cache = cache::BranchCache::load(&repo_path);
-        let cache_for_cherry = cache::BranchCache::load(&repo_path);
+        let new_cache =
+            cache::BranchCache::load_for_base(&repo_path, &base_branch, &self.cache_root);
+        let cache_for_cherry =
+            cache::BranchCache::load_for_base(&repo_path, &base_branch, &self.cache_root);
```

Those calls follow the blocking traversal; they do not introduce it. No log evidence attributes the observed multi-second refreshes to the new cache implementation.

### One worker per worktree

The worker count and per-entry spawn already exist at the base: [src/git/worktree.rs](../../src/git/worktree.rs:198), **198–229**. Blame attributes the parallel spawning to `d6f8152`, outside the range. The range makes a sorting-related identity correction in `daaf063`, task #051, “Apply saved default sorts to loaded list data”:

```diff
-        for (index, wt) in worktrees.into_iter().enumerate() {
+        for wt in worktrees {
             let tx = tx.clone();
             handles.push(std::thread::spawn(move || {
+                let path = wt.path.clone();
...
-                    index,
+                    path,
```

This changes result matching from a row index to a stable path. It does not increase worker count or change status scanning. `src/git/squash_loader.rs`, `src/git/cherry_loader.rs`, and `src/git/status.rs` have no diff in the requested range. Their worker limits and status command cannot be attributed to this range.

## Log evidence and limits

`/private/tmp/gbm-debug.log` spans multiple launches and revisions. The earlier timings establish that blocking branch refresh and slow worktree scans occurred, but do not benchmark the later graph changes:

| UTC time | Evidence | Debug log lines |
| --- | --- | --- |
| 2026-10-01 09:04:37 | UI `post_operation` / `sync_full` branch refresh: 26.7s; metadata collection: 20.3s; merge detection: 5.64s. | 33513–33515; 33093–33096; 33234–33238 |
| 2026-10-01 09:13:30 | UI branch refresh: 6.97s. | 45263–45265 |
| 2026-10-01 17:25:09 | UI branch refresh: 3.05s with `inputs_changed: false`. | 63453–63455 |
| 2026-10-01 09:12:13 | Worktree enrichment: 21 workers, about 198s elapsed. | 42351–42353 |

These examples precede `4bc8f7a`'s recorded commit time, 2026-10-02 03:16:48 UTC. They must not be described as measured regressions from that commit. The logs do not identify the running executable's SHA, so commit timestamps alone also cannot prove which uncommitted code was running.

The watchdog reports have no timestamps, process IDs, or active-operation context. Individual lines in a rising sequence are repeated observations of one pause. Its wall-clock-based 483.8s outlier can include sleep/resume or clock changes. Tracing `time.busy` here is elapsed span time, not CPU utilization. There is no resource profile establishing CPU or disk saturation.

## Verification and review boundary

The independent reviewer inspected the range across all 43 changed files, with surrounding loader/channel/job contracts, baseline implementations, relevant tests, and commit attribution. The confirmed findings were checked against history and source before this report was written. The requested review is diagnostic; application code, versions, task status, and Git history were not changed.

- `cargo test --bin git-branch-manager observed_commit_change_updates_loaded_graph_without_full_reload_or_navigation -- --test-threads=1`: one test passed. This checks graph publication, not latency or UI-thread safety.
- `cargo build`: passed. The full test suite was not run for this diagnostic review.
- The existing tests use small repositories and do not establish responsiveness on deep histories or many worktrees.
- Runtime attribution of the new graph work requires timing spans around capture/updater/drain operations or a sample of the UI thread during a reproduced stall.

The introduced UI-thread scan should be fixed. The added background work merits removal or reduction. The retained synchronous branch refresh and unbounded worktree concurrency require separate fixes even though this range did not introduce them.
