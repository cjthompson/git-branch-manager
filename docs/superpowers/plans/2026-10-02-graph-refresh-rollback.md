# Graph Refresh Repair Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove the unnecessary incremental topology engine while preserving selective automatic Graph freshness and responsive result publication.

**Architecture:** Reuse the existing full Graph loader in background workers. Retain inexpensive before/after ref observations to select relevant mutations, and replace the incremental updater queue with a small coalescing gate. Keep viewport-preserving publication, configured upstream resolution, and fetch/primary-check ownership behavior.

**Tech Stack:** Rust, git2, existing Git CLI helpers, ratatui, standard-library mpsc channels, existing tracing subscriber.

**Spec:** [Proposed graph refresh and lineage design](../specs/2026-10-02-graph-refresh-and-lineage-design.md). Read the original #047 and #053 requirements and the rollback boundary there before execution. This plan covers three independently shippable Graph repairs; Branches responsiveness, worktree concurrency, and #053 are independent follow-up PRs.

## Delivery and acceptance

The user approved three high-priority fix tickets grouped by technical responsibility. Tasks 1 and 2 can ship independently; Task 3 depends on both. Each ticket includes its own tests, timing evidence, and review documentation. Task 3 owns the final integrated responsiveness capture. The constraints below describe the completed repair; intermediate tickets must preserve existing freshness and ownership behavior.

For each ticket, use `pre-push-check` before a PR push and again after PR creation. Run relevant regression tests, `cargo fmt --check`, `cargo clippy`, `cargo build`, and `git diff --check`. Keep local development version changes out of commits. Record baseline failures separately. Acceptance, completion, merge, and a separate stable version bump remain later actions.

## Global Constraints

- No repository traversal, Git subprocess, or SQLite operation runs while publishing a Graph result on the UI thread.
- A completed mutation refreshes an already-loaded Graph when its visible refs, topology, HEAD, worktree ownership, or local upstream metadata changed.
- Unchanged operations and unrelated hidden remote ref changes do not refresh Graph.
- Structural refresh uses the existing background full loader, with at most one retained structural worker result channel and one coalesced follow-up request.
- A mutation observed during a structural load prevents publishing that load as current and schedules a fresh load using the latest options.
- Fetch cancellation and primary-check overlay ownership remain correct.
- The existing transient cache, configured upstream selection, fuzzy presentation, and primary-branch code-check behavior remain compatible.
- Feature-branch version edits remain local; acceptance, merge, and a separate stable version bump are later actions.

## Review Focus

Step numbers in this section refer to Task 3. The local-push observation fixture belongs to Task 2 and is extended with selective invalidation assertions in Task 3.

- Push advances an upstream remote-tracking ref even with remotes hidden; refresh visible local tracking metadata, but ignore unrelated hidden remotes. Pin with the delta tests in Step 1 and the local-push integration case in Step 4.
- A mutation arrives during initial load or a settings-driven reload; discard the obsolete publication and coalesce one fresh worker load. Pin with the gate tests and controlled channel test in Steps 1 and 4.
- A canceled fetch partially changes refs before a primary check opens; publish refresh without reopening fetch results or stealing primary cancellation ownership. Preserve and adapt the existing canceled-fetch tests in Step 4.
- The selected OID survives a full worker reload while another view is active; retain viewport and navigation state. Pin with the presentation test in Step 4.
- Custom refs, stash roots, and multiple remotes exist; use the established full loader and its fallback rather than a second topology implementation. Pin with a real stash fixture in Step 4.

---

### Task 1: Remove repository scanning from Graph publication

**Type:** fix. **Priority:** high. **Dependencies:** none.

This PR removes the introduced UI-thread regression while retaining the existing updater.

**Files:** `src/app.rs` (the Graph structural-result drain, write-only state field, initialization, and scheduler assignment), App tests, and `docs/reviews/2026-10-02-ui-stall-regression-review.md`.

- [ ] Remove the synchronous `GraphRepositoryState::capture` call when publishing a structural result, including successful and failed loads.
- [ ] Remove the write-only `graph_repository_state` field, its initialization, and assignments. Retain worker observations and all existing Graph freshness behavior.
- [ ] Preserve result/enrichment generations, fetch cancellation, and primary-check overlay and cancellation ownership.
- [ ] Add focused success/error publication regression coverage. Rerun relevant Graph/fetch ownership tests. Verify there are no remaining references to `graph_repository_state`.
- [ ] Record monotonic publication timings through the existing tracing subscriber, including process ID, generation, stage, timestamps, and elapsed duration. Confirm the Graph publication path performs no repository traversal, Git subprocess, or SQLite operation. Report measured timing and remaining Branches stalls separately.
- [ ] Complete this ticket's build, formatting, lint, review, and acceptance checks described under Delivery and acceptance. Update the review document with the exact validation boundary.

### Task 2: Make Graph mutation observations avoid history traversal

**Type:** fix. **Priority:** high. **Dependencies:** none. It may be stacked after Task 1 for straightforward review.

This PR reduces worker observation cost while preserving the existing incremental updater. Observation counts are not consumed by that updater; displayed counts are calculated separately by `collect_ref_data`.

**Files:** `src/git/graph.rs` (`GraphRepositoryState::capture_with_head`, repository-state and updater tests), applicable App/integration fixtures, and review documentation. Verify compatibility with job and fetch callers in `src/job_queue.rs` and `src/app.rs` without replacing their ownership wiring.

- [ ] Observe canonical ref OIDs, HEAD, additional roots, worktree ownership, and configured upstream full ref names. Remove `graph_ahead_behind` from observations; store `None` in the tracking count slots and document that invariant.
- [ ] Keep actual displayed ahead/behind calculations in `collect_ref_data`. Preserve before/after worker observation ordering, partial canceled operations, and linked-worktree HEAD ownership.

Replace the tracking loop in `GraphRepositoryState::capture_with_head` with canonical upstream names and no graph walk:

```rust
for branch in repo.branches(Some(git2::BranchType::Local))
    .map_err(|error| error.message().to_string())?
{
    let (branch, _) = branch.map_err(|error| error.message().to_string())?;
    let Some(name) = branch.name()
        .map_err(|error| error.message().to_string())?
        .map(str::to_owned) else { continue };
    let Ok(upstream) = branch.upstream() else { continue };
    let Some(upstream_name) = upstream.get().name().map(str::to_owned)
        else { continue };
    state.tracking.insert(name, (upstream_name, None, None));
}
```

Keep ref OID, HEAD, additional-root, and worktree ownership observation in workers. Keep actual ahead/behind calculations in the existing structural loader's `collect_ref_data`, where they populate displayed counts. The `tracking` tuple's two count slots intentionally remain `None` in observations to minimize API churn; document this invariant beside the field.

- [ ] Add repository-state tests for canonical configured upstream names, absent observation counts, unchanged observations, and ownership identity. Prove the existing updater still displays correct tracking counts after both local-tip and upstream-tip movements.
- [ ] Add a temporary bare-remote push fixture using per-command `current_dir`: push with upstream, capture state, commit and push a new tip, then capture state again. Assert that the canonical upstream remote-tracking OID moved. Task 3 extends this fixture with `affects_graph(false)` assertions.
- [ ] Measure worker observation time while evaluating responsiveness, and report the exact boundary. Complete this ticket's focused tests, build, formatting, lint, review, and acceptance checks. Update review documentation without claiming the final scheduler repair is complete.

### Task 3: Replace incremental topology updates with selective background refresh

**Type:** fix. **Priority:** high. **Dependencies:** Tasks 1 and 2.

This is one independently shippable scheduler replacement PR. Keep selective invalidation, the coalescing gate, App integration, topology removal, and their regression coverage together. Tasks 1 and 2 already remove the UI capture and expensive observation counts.

**Files:**

- Create: `src/view/graph_refresh.rs` — pure coalescing policy and its unit tests.
- Modify: `src/view/mod.rs` — export `graph_refresh`.
- Modify: `src/app.rs:571-711, 1194-1284, 4923-4994` — replace the incremental drain/scheduler, retain background fetch observations and ownership guards.
- Modify: `src/git/graph.rs:53-74, 359-534` — remove updater messages/worker/topology algorithm and consume the lightweight observations from Task 2.
- Modify: `src/view/graph.rs:146-175, 260-390` — retain viewport-preserving snapshot publication; remove obsolete repository-delta patch methods once all callers are gone.
- Modify: `src/job_queue.rs:248-269` — keep worker observations and completion ordering; remove only interfaces rendered unused by the replacement.
- Test: existing App graph/fetch/primary-check tests, `tests/integration.rs`, and unit tests in `src/git/graph.rs` / `src/view/graph_refresh.rs`.
- Update: `docs/reviews/2026-10-02-ui-stall-regression-review.md` with the final validation boundary after acceptance.

**Interfaces:**

- Consume existing `GraphRepositoryState`, `GraphRepositoryDelta::between`, `GraphLoadOptions`, `GraphSnapshot`, `GraphEnrichmentMsg`, and `graph::spawn_graph_loader(PathBuf, GraphLoadOptions)`.
- Produce `GraphRepositoryDelta::affects_graph(&self, include_remotes: bool) -> bool`.
- Produce `GraphRefreshGate::request(&mut self, has_snapshot: bool, loading: bool) -> bool` and `GraphRefreshGate::take_follow_up(&mut self) -> bool`.
- Keep `App::request_graph_update(Result<GraphRepositoryDelta, String>)` as the existing job/fetch entry point; its implementation schedules a full background refresh instead of incremental topology.
- Keep `GraphState::apply_incremental_result(Result<GraphSnapshot, GraphLoadError>)` as the current presentation method; its preserved viewport behavior is useful for full worker snapshots too.

- [ ] **Step 1: Write failing tests for selective invalidation and coalescing**

Add these tests to `src/git/graph.rs`'s existing test module. They use existing domain structs without Git subprocesses:

```rust
#[test]
fn graph_refresh_ignores_unchanged_observations() {
    let state = GraphRepositoryState::default();
    let delta = GraphRepositoryDelta::between(state.clone(), state);
    assert!(!delta.affects_graph(false));
    assert!(!delta.affects_graph(true));
}

#[test]
fn graph_refresh_distinguishes_hidden_remote_and_upstream_moves() {
    let id = GraphRefId {
        kind: GraphRefKind::RemoteBranch,
        full_name: "refs/remotes/origin/topic".into(),
    };
    let mut before = GraphRepositoryState::default();
    before.refs.insert(id.clone(), "old".into());
    let mut after = before.clone();
    after.refs.insert(id.clone(), "new".into());
    let hidden = GraphRepositoryDelta::between(before.clone(), after.clone());
    assert!(!hidden.affects_graph(false));
    assert!(hidden.affects_graph(true));

    before.tracking.insert("topic".into(), (id.full_name.clone(), None, None));
    after.tracking = before.tracking.clone();
    let upstream = GraphRepositoryDelta::between(before, after);
    assert!(upstream.affects_graph(false));
}

#[test]
fn graph_refresh_preserves_head_worktree_and_upstream_invalidation() {
    let before = GraphRepositoryState::default();
    let mut after = before.clone();
    after.head_ref = Some("refs/heads/topic".into());
    assert!(GraphRepositoryDelta::between(before.clone(), after).affects_graph(false));
    let mut after = before.clone();
    after.worktrees.insert("/repo/wt".into(), Some("topic".into()));
    assert!(GraphRepositoryDelta::between(before.clone(), after).affects_graph(false));
    let mut after = before.clone();
    after.tracking.insert("topic".into(), ("refs/remotes/upstream/topic".into(), None, None));
    assert!(GraphRepositoryDelta::between(before, after).affects_graph(false));
}
```

Create `src/view/graph_refresh.rs` with the test module below before adding its production type, and export the module from `src/view/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::GraphRefreshGate;

    #[test]
    fn unopened_graph_stays_unloaded() {
        let mut gate = GraphRefreshGate::default();
        assert!(!gate.request(false, false));
        assert!(!gate.take_follow_up());
    }

    #[test]
    fn idle_loaded_graph_starts_refresh() {
        let mut gate = GraphRefreshGate::default();
        assert!(gate.request(true, false));
        assert!(!gate.take_follow_up());
    }

    #[test]
    fn mutations_during_initial_load_coalesce() {
        let mut gate = GraphRefreshGate::default();
        assert!(!gate.request(false, true));
        assert!(!gate.request(false, true));
        assert!(gate.take_follow_up());
        assert!(!gate.take_follow_up());
    }
}
```

- [ ] **Step 2: Verify the new tests fail for the missing policy**

Run `cargo test --lib graph_refresh -- --test-threads=1`. Expect missing `GraphRefreshGate` / `affects_graph` errors. This failure pins the required behavior before the replacement is added.

- [ ] **Step 3: Implement the policy and replace the topology machinery**

Implement the gate in `src/view/graph_refresh.rs`:

```rust
#[derive(Debug, Default)]
pub struct GraphRefreshGate {
    pending: bool,
}

impl GraphRefreshGate {
    pub fn request(&mut self, has_snapshot: bool, loading: bool) -> bool {
        if loading {
            self.pending = true;
            false
        } else {
            has_snapshot
        }
    }

    pub fn take_follow_up(&mut self) -> bool {
        std::mem::take(&mut self.pending)
    }
}
```

Add this method to `GraphRepositoryDelta`:

```rust
pub fn affects_graph(&self, include_remotes: bool) -> bool {
    if self.head_changed || self.worktrees_changed || self.tracking_changed
        || (include_remotes && self.additional_roots_changed)
    {
        return true;
    }
    let upstreams: HashSet<&str> = self.before.tracking.values()
        .chain(self.after.tracking.values())
        .map(|(name, _, _)| name.as_str())
        .collect();
    self.added_refs.iter().map(|(id, _)| id)
        .chain(self.removed_refs.iter().map(|(id, _)| id))
        .chain(self.moved_refs.iter().map(|(id, _, _)| id))
        .any(|id| id.kind != GraphRefKind::RemoteBranch
            || include_remotes || upstreams.contains(id.full_name.as_str()))
}
```

Task 2 supplies the lightweight worker observations consumed below. Preserve that observation invariant and keep displayed counts in the structural loader.

Add `graph_refresh_gate: GraphRefreshGate` to `App`, initialize with `Default::default()`, and replace the existing scheduler body with:

```rust
fn request_graph_update(&mut self, delta: Result<graph::GraphRepositoryDelta, String>) {
    let relevant = delta.as_ref()
        .map(|delta| delta.affects_graph(self.graph.includes_remotes()))
        .unwrap_or(true);
    if relevant && self.graph_refresh_gate.request(
        self.graph.snapshot().is_some(),
        self.graph.is_loading() || self.graph_rx.is_some(),
    ) {
        self.reload_graph();
    }
}
```

In the structural result drain, after assigning its generation and before publication, replace the synchronous capture and staged-delta reconciliation with:

```rust
if self.graph_refresh_gate.take_follow_up() {
    self.reload_graph();
    continue;
}
self.graph.apply_incremental_result(result);
```

Keep the existing structural-result enrichment spawn and generation check. Remove the separate incremental result drain, `GraphUpdateMsg`, `spawn_graph_updater`, `update_graph_incrementally`, its owned-topology builder and `topological_commit_order`, and the unused `graph_repository_state` field. Remove `graph_update_rx`, `graph_revision`, `pending_graph_deltas`, `graph_update_invalidates_enrichment`, `staged_graph_snapshot`, and `reconciling_initial_graph` after their callers are replaced. Remove `GraphState::apply_ref_delta` and `apply_repository_delta` if no remaining public callers exist; preserve `apply_incremental_result` for presentation.

Leave Graph in the affected-view action map so pushes can reach the observation scheduler. `refresh_after_job` continues refreshing the other affected views, then calls `request_graph_update` for potentially Graph-affecting actions. Its delta policy suppresses irrelevant or unchanged refreshes. Preserve the existing `op_rx` and `remote_fetch_rx` completion wiring, dismissed-fetch state, and `clear_cancel_flag_unless_primary_code_check_owns_it`; do not replace those blocks wholesale with parent code.

- [ ] **Step 4: Add and adapt end-to-end regression coverage**

Convert the current incremental-specific App tests to assert the same final graph contents via the background loader. Keep actual repository mutation and the existing deadline helper; replace waits on `graph_update_rx` with `!app.graph.is_loading() && app.graph_rx.is_none()`. Delete assertions requiring that a full load never occur; the accepted requirement is fresh content without UI blocking. Retain the tests for automatic/modal/canceled fetch completion and initial-load races.

Add the controlled-channel case to the existing App test module:

```rust
#[test]
fn graph_refresh_coalesces_a_mutation_before_publication() {
    let mut app = graph_app(vec![graph_ref(
        "feature/pending", graph::GraphRefKind::LocalBranch,
    )]);
    let snapshot = app.graph.snapshot().unwrap().clone();
    let previous_generation = app.graph_generation;
    let (tx, rx) = mpsc::channel();
    app.graph_rx = Some(rx);
    app.graph.begin_load(app.graph.max_count(), app.graph.includes_remotes());
    app.active_view = ViewId::Worktrees;
    app.request_graph_update(Err("observation failed after mutation".into()));
    tx.send(Ok(snapshot)).unwrap();
    app.drain_channels();
    assert!(app.graph_generation > previous_generation);
    assert!(app.graph.is_loading());
    assert_eq!(app.active_view, ViewId::Worktrees);
}
```

Extend existing `observed_commit_change_updates_loaded_graph_without_full_reload_or_navigation` by selecting a surviving commit and recording its OID plus `commit_offset()` and `horizontal_offset()` before the mutation. After the worker result, assert the same selected OID, offsets, active view, max-count, and include-remotes value. Rename it to `observed_commit_change_refreshes_loaded_graph_without_navigation`; input is now a full worker snapshot.

For the existing graph integration fixture, create a real stash before a mutation:

```rust
std::fs::write(dir.join("stash-input.txt"), "uncommitted\n").unwrap();
let status = std::process::Command::new("git")
    .args(["stash", "push", "--include-untracked", "-m", "refresh fixture"])
    .current_dir(dir).status().unwrap();
assert!(status.success());
let snapshot = graph::load_graph(dir, graph::GraphLoadOptions {
    include_remotes: true,
    cache_root: cache::CacheRoot::at(cache_dir.path()),
    ..graph::GraphLoadOptions::default()
}).unwrap();
assert!(!snapshot.commits.is_empty());
```

Here `dir` is the fixture repository path and `cache_dir` is a `tempfile::tempdir()` declared in that test. Compare the refreshed result with this same established full loader result. Keep the configured-upstream test in `src/git/graph.rs`; the rollback must not reinstate origin-name guessing.

Extend Task 2's temporary bare-remote local-push fixture to assert `affects_graph(false)` is true when the canonical upstream OID moves. The pure tests in Step 1 pin unchanged observations and unrelated hidden-remote cases. Keep all Git commands scoped to temporary paths with explicit `current_dir`.

Retain `late_canceled_fetch_completion_preserves_primary_check_ownership`, `cancelled_modal_fetch_keeps_and_applies_partial_observation_without_reopening_overlay`, and the pending-mutation/new-options test. These protect behavior introduced after the commit being selectively reverted.

- [ ] **Step 5: Run focused checks and a real responsiveness capture**

Run sequentially:

```sh
cargo test --lib graph_refresh -- --test-threads=1
cargo test --lib repository_state -- --test-threads=1
cargo test --bin git-branch-manager relevant_action -- --test-threads=1
cargo test --bin git-branch-manager graph_refresh -- --test-threads=1
cargo test --bin git-branch-manager fetch_completion -- --test-threads=1
cargo test --bin git-branch-manager late_canceled_fetch -- --test-threads=1
cargo test --lib -- --test-threads=1
cargo test --bin git-branch-manager -- --test-threads=1
cargo test --test integration -- --test-threads=1
cargo test --test primary_ref --test primary_code_content -- --test-threads=1
cargo fmt --check
cargo clippy
cargo build
git diff --check
```

Record named baseline failures separately; do not turn an unrelated baseline failure into a passing full-suite claim. Ensure no removed updater references remain in production by checking `rg -n 'graph_update_rx|GraphUpdateMsg|update_graph_incrementally|graph_repository_state' src`.

Add monotonic `Instant` timings around the structural drain and worker graph load using the existing tracing subscriber. Reproduce startup, a non-Graph-view mutation, canceled fetch, and repeated mutations in the large repository that stalled. Include stage, process ID, graph generation, start/end timestamps, and elapsed duration in the log. The UI publication path must contain no Git/SQLite I/O; input must continue while the structural worker is active. Report actual timings rather than inferring CPU use from `time.busy`. The old Branches refresh can still block and must be attributed separately.

- [ ] **Step 6: Review and commit the independently working repair**

Use `pre-push-check` before any PR push and after PR creation. Verify the change preserves #047 while removing the topology engine and that every Review Focus case has coverage. Check staged Cargo files contain no version hunk. Commit only after the user accepts execution and the required checks pass; do not merge, complete tasks, or bump the stable version until acceptance.

```sh
git add src/app.rs src/git/graph.rs src/job_queue.rs src/view/mod.rs src/view/graph.rs src/view/graph_refresh.rs tests/integration.rs
git diff --cached Cargo.toml Cargo.lock
git commit -m "fix: refresh Graph in background without redundant history scans"
```

## Self review and follow-up boundaries

Tasks 1 and 2 own immediate publication and worker-observation regressions respectively. Task 3's policy tests cover unchanged work, hidden upstream movements, HEAD/worktree/upstream changes, and coalescing. App/loader tests own navigation, selection, settings races, partial fetch cancellation, primary ownership, and stash roots. The spec's eight graph-repair constraints map to Steps 3–5. No requirement of #053 is claimed complete by this PR.

The next independent repair is the synchronous Branches refresh, because it explains the measured 26.7-second freeze even after the new graph work is removed. Cap worktree status concurrency in another independently working change. Task #053 requires its own detector/storage/hydration implementation plan after the paired-identity and discovery-boundary decisions in the spec are accepted.
