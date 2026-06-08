# Performance Diagnostics Loop

This is the repo-local run log for the measure/change/measure loop described in
`~/.claude/plans/gbm-performance-diagnosis.md`.

## Iteration 1: parallel worktree enrichment

Date: 2026-06-07 local / 2026-06-08 UTC

Commit before change: `65c66d5fbd48187b648a45945206992509bf342b`

Test repos:

- `/Users/chris.thompson/workspace/zenpayroll`: 28 local branches, 10 worktrees.
- `/Users/chris.thompson/workspace/gbm-zenpayroll`: 1 local branch, 16,604 remote refs.

### Baseline

Commands:

```sh
GBM_TIMING_LOG=/tmp/gbm-loop-baseline-zenpayroll-branches.log \
  /usr/bin/time -p ./target/release/git-branch-manager \
  --repo /Users/chris.thompson/workspace/zenpayroll --branches --color=never

GBM_TIMING_LOG=/tmp/gbm-loop-baseline-zenpayroll-worktrees.log \
  /usr/bin/time -p ./target/release/git-branch-manager \
  --repo /Users/chris.thompson/workspace/zenpayroll --worktrees --color=never

GBM_TIMING_LOG=/tmp/gbm-loop-baseline-gbm-zenpayroll-remotes.log \
  /usr/bin/time -p ./target/release/git-branch-manager \
  --repo /Users/chris.thompson/workspace/gbm-zenpayroll --remotes --color=never
```

Results:

| Repo / view | Wall time | Key span | Span time | Note |
| --- | ---: | --- | ---: | --- |
| `zenpayroll --branches` | 23.90s | `git::branch::list_branches` | 23.3s | `collect_branch_metadata` 13.6s; `detect_merged_branches` 9.70s |
| `zenpayroll --worktrees` | 28.18s | `git::worktree::enrich_worktrees_worker` | 28.1s | 10 worktrees processed serially; most `git status --porcelain` calls were 3.5-4.0s |
| `gbm-zenpayroll --remotes` | stopped | `git::branch::spawn_remote_enricher` | open | `list_remote_branches_phase1` closed in 255ms, then the serial 16,603-branch remote enricher did not finish within the bounded run |

### Decision

Changed one function: `git::worktree::enrich_worktrees`.

The measurement showed 10 independent worktree status/age probes running
serially. Callers already apply `WorktreeEnrichResult` by `index`, so result
arrival order does not affect behavior. The change keeps the same channel API and
uses a coordinator thread to spawn one worker per worktree entry, then joins the
workers before closing the `enrich_worktrees_worker` span.

### After

Commands:

```sh
GBM_TIMING_LOG=/tmp/gbm-loop-after-worktree-parallel-zenpayroll-worktrees.log \
  /usr/bin/time -p ./target/release/git-branch-manager \
  --repo /Users/chris.thompson/workspace/zenpayroll --worktrees --color=never

GBM_TIMING_LOG=/tmp/gbm-loop-after-worktree-parallel-zenpayroll-branches.log \
  /usr/bin/time -p ./target/release/git-branch-manager \
  --repo /Users/chris.thompson/workspace/zenpayroll --branches --color=never

GBM_TIMING_LOG=/tmp/gbm-loop-after-worktree-parallel-gbm-zenpayroll-remotes-60s.log \
  /usr/bin/time -p perl -e 'alarm shift; exec @ARGV' 60 \
  ./target/release/git-branch-manager \
  --repo /Users/chris.thompson/workspace/gbm-zenpayroll --remotes --color=never
```

Results:

| Repo / view | Before | After | Delta | Key span before | Key span after | Note |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| `zenpayroll --worktrees` | 28.18s | 17.74s | -37.0% | 28.1s | 17.4s | Improvement accepted; child spans overlap now, so compare wall time and parent worker span only |
| `zenpayroll --branches` | 23.90s | 22.58s | -5.5% | 23.3s | 22.4s | Guardrail only; code path is not touched by this change |
| `gbm-zenpayroll --remotes` | stopped | 60s cap | n/a | open | open | Guardrail only; remote enricher remains the next large bottleneck candidate |

### Validation

- `cargo test worktrees` passed.
- `rustfmt --check src/git/worktree.rs` passed.
- `git diff --check` passed.
- Global `cargo fmt --check` currently reports unrelated formatting changes in
  dump/render integration files, so this iteration did not apply global
  formatting churn.

## Iteration 2: bounded parallel remote enrichment

Date: 2026-06-07 local / 2026-06-08 UTC

Commit before attempted change: `4972351`

### Baseline

The relevant baseline/guardrail was the large remote repo:

```sh
GBM_TIMING_LOG=/tmp/gbm-loop-after-worktree-parallel-gbm-zenpayroll-remotes-60s.log \
  /usr/bin/time -p perl -e 'alarm shift; exec @ARGV' 60 \
  ./target/release/git-branch-manager \
  --repo /Users/chris.thompson/workspace/gbm-zenpayroll --remotes --color=never
```

Result:

| Repo / view | Wall time | Key span | Span time | Note |
| --- | ---: | --- | ---: | --- |
| `gbm-zenpayroll --remotes` | 60.01s cap | `git::branch::spawn_remote_enricher` | open | `list_remote_branches_phase1` closed in 1.48s; no output rows were produced before the cap |

### Attempt

Changed one function: `git::branch::spawn_remote_enricher`.

The attempted change replaced the serial remote enrichment loop with a bounded
worker pool. Each worker opened its own `Repository`, resolved the base OID once,
then popped remote branches from a shared queue and sent results by `full_ref`.
This preserved the existing out-of-order-safe channel contract.

### After

```sh
GBM_TIMING_LOG=/tmp/gbm-loop-after-remote-parallel-gbm-zenpayroll-remotes-60s.log \
  /usr/bin/time -p perl -e 'alarm shift; exec @ARGV' 60 \
  ./target/release/git-branch-manager \
  --repo /Users/chris.thompson/workspace/gbm-zenpayroll --remotes --color=never
```

Result:

| Repo / view | Before | After | Delta | Evidence |
| --- | ---: | ---: | ---: | --- |
| `gbm-zenpayroll --remotes` | 60.01s cap | 60.02s cap | no improvement | No output rows; no closed aggregate worker span; `time` reported much higher CPU/system time (`user 117.39`, `sys 193.80`) |

### Outcome

Rejected. The attempted change did not produce a measurable improvement under
the same 60s bounded run and increased resource usage. The code change was
reverted. The next remote iteration should use finer-grained diagnostics or a
different algorithmic approach instead of naive parallel libgit2 graph walks.

## Iteration 3: bulk remote ahead/behind and merged status

Date: 2026-06-07 local / 2026-06-08 UTC

Commit before change: `644dacd`

### Baseline

The baseline is the same bounded remote run from Iteration 2: the serial
`spawn_remote_enricher` did not close inside the 60s cap, and no rows were
rendered.

### Decision

Changed one function: `git::branch::spawn_remote_enricher`.

The failed worker-pool attempt showed that parallel libgit2 graph walks do not
solve the 16k-remote case. Local Git 2.50.1 can compute the same data in bulk:

- `git for-each-ref refs/remotes --format='%(refname:short)%09%(ahead-behind:<base-oid>)'`
- `git branch -r --merged <base-oid>`

Manual probes against `/Users/chris.thompson/workspace/gbm-zenpayroll` completed
in 11.39s and 7.58s respectively, so this iteration replaced the per-branch
libgit2 graph loop with those two bulk commands and parsed their output back into
the existing `RemoteEnrichResult` channel.

### After

```sh
GBM_TIMING_LOG=/tmp/gbm-loop-after-remote-bulk-gbm-zenpayroll-remotes-60s.log \
  /usr/bin/time -p perl -e 'alarm shift; exec @ARGV' 60 \
  ./target/release/git-branch-manager \
  --repo /Users/chris.thompson/workspace/gbm-zenpayroll --remotes --color=never
```

Results:

| Repo / view | Before | After | Delta | Evidence |
| --- | ---: | ---: | ---: | --- |
| `gbm-zenpayroll remote_enricher_worker` | >60s | 19.0s | accepted | `result_count=16602`, `ahead_behind_count=16604`, `merged_count=546`, `missing_ahead_behind_count=0` |
| `gbm-zenpayroll --remotes` full dump | 60s cap | 60s cap | still capped | The bottleneck moved to `squash_candidate` / `is_squash_merged`: 648 candidates consumed 39.8s before the cap; `spawn_squash_checker` had `candidate_count=16056` |
| `zenpayroll --branches` guardrail | 23.90s baseline | 24.23s | within variance | Guardrail only; branch path is not touched by this change |

### Validation

- `cargo test remote` passed.
- `rustfmt --check src/git/branch.rs` passed.
- `git diff --check` passed.

### Next bottleneck

Remote enrichment is no longer the limiting remote phase. The next candidate is
`git::squash_loader::spawn_squash_checker` / `git::merge_detection::is_squash_merged`
on remote branches: after bulk enrichment, the dump built 16,056 squash
candidates, all cache misses in the observed tail, with each squash check around
55-90ms.

## Iteration 4: bounded parallel squash checking

Date: 2026-06-07 local / 2026-06-08 UTC

Commit before attempted change: `2a46945`

### Baseline

The baseline is the bounded remote run after Iteration 3:

```sh
GBM_TIMING_LOG=/tmp/gbm-loop-after-remote-bulk-gbm-zenpayroll-remotes-60s.log \
  /usr/bin/time -p perl -e 'alarm shift; exec @ARGV' 60 \
  ./target/release/git-branch-manager \
  --repo /Users/chris.thompson/workspace/gbm-zenpayroll --remotes --color=never
```

Result:

| Repo / view | Wall time | Key span | Span time | Note |
| --- | ---: | --- | ---: | --- |
| `gbm-zenpayroll --remotes` | 60.01s cap | `git::squash_loader::squash_candidate` | 39.85s total busy | After remote enrichment closed in 19.0s, the serial squash checker completed 648 real squash checks before the cap; mean 61.5ms, p95 80.5ms |

### Attempt

Changed one function: `git::squash_loader::spawn_squash_checker`.

The attempted change kept cache mutation on the coordinator thread, split
cache misses into a queue, and used an eight-worker pool to run
`is_squash_merged` for uncached candidates. It preserved the existing output
channel contract by sending `SquashResult` values back from the coordinator.

### After

```sh
GBM_TIMING_LOG=/tmp/gbm-loop-after-squash-parallel-gbm-zenpayroll-remotes-60s.log \
  /usr/bin/time -p perl -e 'alarm shift; exec @ARGV' 60 \
  ./target/release/git-branch-manager \
  --repo /Users/chris.thompson/workspace/gbm-zenpayroll --remotes --color=never
```

Result:

| Repo / view | Before | After | Delta | Evidence |
| --- | ---: | ---: | ---: | --- |
| `gbm-zenpayroll --remotes` full dump | 60.01s cap | 60.02s cap | no improvement | Output remained empty before the cap |
| Real squash checks completed before cap | 648 | 293 | -54.8% | The new log separated 16,056 cheap cache-probe spans from 293 real worker spans at `src/git/squash_loader.rs:116` |
| Real squash-check mean | 61.5ms | 1052.8ms | +1612% | Summed worker busy time was 308.5s because the eight workers overlapped; elapsed throughput still regressed |
| Real squash-check p95 | 80.5ms | 1260ms | +1465% | `is_squash_merged` per-call cost rose sharply under the worker pool |

### Outcome

Rejected. The attempted change did not improve the bounded full dump, completed
fewer squash checks before the same cap, and increased CPU cost. The code change
was reverted.

### Validation

- `cargo test squash` passed for the attempted change.
- `cargo test remote` passed for the attempted change.
- `cargo build --release` passed for the attempted change.

## Iteration 5: bulk local upstream tracking counts

Date: 2026-06-07 local / 2026-06-08 UTC

Commit before attempted change: `3f95dba`

### Baseline

The local branch baseline was the latest `zenpayroll --branches` guardrail after
Iteration 3:

```sh
GBM_TIMING_LOG=/tmp/gbm-loop-after-remote-bulk-zenpayroll-branches.log \
  /usr/bin/time -p ./target/release/git-branch-manager \
  --repo /Users/chris.thompson/workspace/zenpayroll --branches --color=never
```

Result:

| Repo / view | Wall time | Key span | Span time | Note |
| --- | ---: | --- | ---: | --- |
| `zenpayroll --branches` | 24.23s | `git::branch::collect_branch_metadata` | 14.0s | `collect_branch_metadata_merge_base` was 13.85s; `collect_branch_metadata_ahead_behind_graph` was only 98.2ms |

### Attempt

Changed one function: `git::branch::collect_branch_metadata`.

The attempted change precomputed local upstream tracking counts with:

```sh
git for-each-ref refs/heads --format='%(refname:short)%09%(upstream:short)%09%(upstream:track)'
```

It then used the parsed ahead/behind counts for tracked, non-gone local branches
and kept the existing `repo.graph_ahead_behind` path as a fallback.

### After

```sh
GBM_TIMING_LOG=/tmp/gbm-loop-after-local-tracking-bulk-zenpayroll-branches.log \
  /usr/bin/time -p ./target/release/git-branch-manager \
  --repo /Users/chris.thompson/workspace/zenpayroll --branches --color=never

GBM_TIMING_LOG=/tmp/gbm-loop-after-local-tracking-bulk-gbm-zenpayroll-remotes-60s.log \
  /usr/bin/time -p perl -e 'alarm shift; exec @ARGV' 60 \
  ./target/release/git-branch-manager \
  --repo /Users/chris.thompson/workspace/gbm-zenpayroll --remotes --color=never
```

Results:

| Repo / view | Before | After | Delta | Evidence |
| --- | ---: | ---: | ---: | --- |
| `zenpayroll --branches` full dump | 24.23s | 22.83s | not accepted | The apparent wall-time drop was dominated by merge-base variance, not the changed ahead/behind path |
| `collect_branch_metadata_ahead_behind_graph` / replacement | 98.2ms | 77.9ms | -20.3ms | The changed path was too small to matter for total startup time |
| `collect_branch_metadata_merge_base` | 13.85s | 11.62s | unrelated variance | This remained the dominant local metadata cost |
| `gbm-zenpayroll --remotes` full dump | 60s cap | 60s cap | no improvement | Guardrail remained capped; output stayed empty |

### Outcome

Rejected. The attempted change replaced a small cost center and did not address
the current local branch bottleneck. The code change was reverted.

### Validation

- `cargo test parse_local_tracking_ahead_behind` passed for the attempted change.
- `cargo test branch` passed for the attempted change.
- `cargo test remote` passed for the attempted change.
- `rustfmt --check src/git/branch.rs` passed for the attempted change.
- `cargo build --release` passed for the attempted change.

### Next bottleneck

For `zenpayroll --branches`, the data points to
`git::branch::collect_branch_metadata`'s per-branch merge-base computation:
27 `collect_branch_metadata_merge_base` spans consumed 13.85s before the
attempt and 11.62s after the attempt. The next local iteration should target
that merge-base path, not ahead/behind.

## Iteration 6: single-revwalk merged-branch detection

Date: 2026-06-07 local / 2026-06-08 UTC

Commit before change: `621a4cf`

### Baseline

The baseline is still the latest accepted-code `zenpayroll --branches`
guardrail from Iteration 3:

```sh
GBM_TIMING_LOG=/tmp/gbm-loop-after-remote-bulk-zenpayroll-branches.log \
  /usr/bin/time -p ./target/release/git-branch-manager \
  --repo /Users/chris.thompson/workspace/zenpayroll --branches --color=never
```

Result:

| Repo / view | Wall time | Key span | Span time | Note |
| --- | ---: | --- | ---: | --- |
| `zenpayroll --branches` | 24.23s | `git::merge_detection::detect_merged_branches` | 9.99s | 27 `detect_merged_graph_descendant_of` calls consumed 9.98s |
| `zenpayroll --branches` | 24.23s | `git::branch::collect_branch_metadata_merge_base` | 13.85s | This is separate from merged-branch detection and remains a hotspot |

### Decision

Changed one function: `git::merge_detection::detect_merged_branches`.

The function previously called `repo.graph_descendant_of(base, branch_tip)` once
per candidate branch. This iteration builds a `HashSet` of commits reachable
from the base branch with one revwalk, then classifies each candidate branch tip
with an O(1) membership check. The function signature and callers are unchanged.

### After

```sh
GBM_TIMING_LOG=/tmp/gbm-loop-after-detect-merged-revwalk-zenpayroll-branches.log \
  /usr/bin/time -p ./target/release/git-branch-manager \
  --repo /Users/chris.thompson/workspace/zenpayroll --branches --color=never

GBM_TIMING_LOG=/tmp/gbm-loop-after-detect-merged-revwalk-gbm-zenpayroll-remotes-60s.log \
  /usr/bin/time -p perl -e 'alarm shift; exec @ARGV' 60 \
  ./target/release/git-branch-manager \
  --repo /Users/chris.thompson/workspace/gbm-zenpayroll --remotes --color=never
```

Results:

| Repo / view | Before | After | Delta | Evidence |
| --- | ---: | ---: | ---: | --- |
| `detect_merged_branches` | 9.99s | 15.4ms | -99.8% | New `detect_merged_revwalk` span closed in 15.0ms |
| `zenpayroll --branches` full dump | 24.23s | 14.90s | -38.5% | `list_branches` span moved from 24.0s to 13.7s |
| `collect_branch_metadata_merge_base` | 13.85s | 13.59s | unchanged | This is now the dominant local branch cost |
| `gbm-zenpayroll --remotes` full dump | 60s cap | 60s cap | unchanged | Guardrail remained capped during remote squash checking |

### Outcome

Accepted. The changed function produced a clear function-level and user-visible
improvement on `zenpayroll --branches`, with no new remote guardrail regression.

### Validation

- `cargo test merged_branch_detection` passed.
- `cargo test branch` passed.
- `rustfmt --check src/git/merge_detection.rs` passed.
- `cargo build --release` passed.
- `git diff --check` passed.

### Next bottleneck

For `zenpayroll --branches`, `git::branch::collect_branch_metadata` is now almost
entirely merge-base work: 27 `collect_branch_metadata_merge_base` spans consumed
13.59s in the accepted after-run.
