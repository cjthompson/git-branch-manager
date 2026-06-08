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

## Iteration 7: bounded merge-base lookup in branch metadata

Date: 2026-06-07 local / 2026-06-08 UTC

Commit before change: `eab83a2`

### Baseline

The baseline is the accepted after-run from Iteration 6:

```sh
GBM_TIMING_LOG=/tmp/gbm-loop-after-detect-merged-revwalk-zenpayroll-branches.log \
  /usr/bin/time -p ./target/release/git-branch-manager \
  --repo /Users/chris.thompson/workspace/zenpayroll --branches --color=never
```

Result:

| Repo / view | Wall time | Key span | Span time | Note |
| --- | ---: | --- | ---: | --- |
| `zenpayroll --branches` | 14.90s | `git::branch::collect_branch_metadata_merge_base` | 13.59s | 27 per-branch `repo.merge_base()` spans dominated the run |
| `zenpayroll --branches` | 14.90s | `git::merge_detection::detect_merged_branches` | 15.4ms | Already fixed in Iteration 6 |

### Decision

Changed one function: `git::branch::collect_branch_metadata`.

The function previously called `repo.merge_base(base, branch_tip)` for every
non-base local branch. This iteration builds one reachable set from the base
branch, then finds a branch's merge-base by walking that branch until it hits the
reachable set. Each branch walk is capped at 1,000 commits, matching the data:
`zenpayroll` had two shallow merge-base successes and 25 branches with no useful
intersection after very large graph walks. If the shared base revwalk cannot be
built, the old `repo.merge_base()` path remains as a fallback.

### After

```sh
GBM_TIMING_LOG=/tmp/gbm-loop-after-merge-base-bounded-zenpayroll-branches.log \
  /usr/bin/time -p ./target/release/git-branch-manager \
  --repo /Users/chris.thompson/workspace/zenpayroll --branches --color=never

GBM_TIMING_LOG=/tmp/gbm-loop-after-merge-base-bounded-gbm-zenpayroll-remotes-60s.log \
  /usr/bin/time -p perl -e 'alarm shift; exec @ARGV' 60 \
  ./target/release/git-branch-manager \
  --repo /Users/chris.thompson/workspace/gbm-zenpayroll --remotes --color=never
```

Results:

| Repo / view | Before | After | Delta | Evidence |
| --- | ---: | ---: | ---: | --- |
| `collect_branch_metadata_merge_base` | 13.59s | 78.9ms | -99.4% | 27 bounded spans; `merge_base_success_count=2`, `merge_base_limited_count=25` |
| `collect_branch_metadata` | 13.70s | 175ms | -98.7% | Includes a 53.7ms shared base revwalk |
| `list_branches` | 13.70s | 189ms | -98.6% | Branch graph work is no longer the dominant runtime |
| `zenpayroll --branches` full dump | 14.90s | 1.21s | -91.9% | Remaining wall time is mostly the failing PR lookup (`fetch_open_prs` 710ms) |
| `gbm-zenpayroll --remotes` full dump | 60s cap | 60s cap | unchanged | Guardrail remained capped during remote squash checking |

### Outcome

Accepted. The changed function removed the last multi-second local branch graph
span and made `zenpayroll --branches` effectively sub-second before PR lookup.

### Validation

- `cargo test branch` passed.
- `cargo test merged_branch_detection` passed.
- `cargo build --release` passed.
- `git diff --check` passed.

### Next bottleneck

For `zenpayroll --branches`, the remaining user-visible time is now mostly PR
lookup failure handling (`fetch_open_prs_checked` around 710ms in the accepted
after-run). For `gbm-zenpayroll --remotes`, the bottleneck remains remote squash
checking after remote enrichment.

## Iteration 8: pass candidate commits to squash checks

Date: 2026-06-07 local / 2026-06-08 UTC

Commit before change: `234e656`

### Baseline

Fresh current-state diagnostics before the attempted change:

```sh
GBM_TIMING_LOG=/tmp/gbm-loop-current-zenpayroll-branches.log \
  /usr/bin/time -p ./target/release/git-branch-manager \
  --repo /Users/chris.thompson/workspace/zenpayroll --branches --color=never

GBM_TIMING_LOG=/tmp/gbm-loop-current-zenpayroll-worktrees.log \
  /usr/bin/time -p ./target/release/git-branch-manager \
  --repo /Users/chris.thompson/workspace/zenpayroll --worktrees --color=never

GBM_TIMING_LOG=/tmp/gbm-loop-current-gbm-zenpayroll-remotes-60s.log \
  /usr/bin/time -p perl -e 'alarm shift; exec @ARGV' 60 \
  ./target/release/git-branch-manager \
  --repo /Users/chris.thompson/workspace/gbm-zenpayroll --remotes --color=never
```

Results:

| Repo / view | Wall time | Key span | Span time | Note |
| --- | ---: | --- | ---: | --- |
| `zenpayroll --branches` | 2.92s | `git::branch::list_branches` | 1.85s | `fetch_open_prs_checked` failed in 691ms; branch graph work stayed fast |
| `zenpayroll --worktrees` | 14.28s | `git::worktree::enrich_worktrees_worker` | 14.2s | Guardrail; still dominated by concurrent `git status --porcelain` calls |
| `gbm-zenpayroll --remotes` | 60.03s cap | `git::squash_loader::squash_candidate` | 38.44s total busy | 221 real squash checks completed before the cap; mean 173.9ms, p95 372ms |

### Decision

Changed one function: `git::squash_loader::spawn_squash_checker`.

The function receives candidates as `(branch_name, commit_hash)` pairs, but the
cache-miss path passed `None` into `is_squash_merged`, forcing Git to resolve the
branch ref again while constructing the temporary squash-check commit. This
iteration passes the existing candidate commit hash into `is_squash_merged`.
Cache keys, channel results, and status semantics are unchanged.

### After

```sh
GBM_TIMING_LOG=/tmp/gbm-loop-after-squash-commit-hash-zenpayroll-branches.log \
  /usr/bin/time -p ./target/release/git-branch-manager \
  --repo /Users/chris.thompson/workspace/zenpayroll --branches --color=never

GBM_TIMING_LOG=/tmp/gbm-loop-after-squash-commit-hash-zenpayroll-worktrees.log \
  /usr/bin/time -p ./target/release/git-branch-manager \
  --repo /Users/chris.thompson/workspace/zenpayroll --worktrees --color=never

GBM_TIMING_LOG=/tmp/gbm-loop-after-squash-commit-hash-gbm-zenpayroll-remotes-60s.log \
  /usr/bin/time -p perl -e 'alarm shift; exec @ARGV' 60 \
  ./target/release/git-branch-manager \
  --repo /Users/chris.thompson/workspace/gbm-zenpayroll --remotes --color=never
```

Results:

| Repo / view | Before | After | Delta | Evidence |
| --- | ---: | ---: | ---: | --- |
| `gbm-zenpayroll squash_candidate` completed checks | 221 | 254 | +14.9% | Same 60s cap; candidate count remained 16,056 |
| `gbm-zenpayroll squash_candidate` mean | 173.9ms | 155.0ms | -10.9% | Total busy time stayed around 39s because the capped run completed more checks |
| `gbm-zenpayroll squash_candidate` p95 | 372ms | 226ms | -39.2% | Same log parser and same candidate order |
| Common first 221 checks | 173.9ms mean | 149.6ms mean | -24.3ms/check | 117 checks improved, 100 worsened |
| `gbm-zenpayroll --remotes` full dump | 60.03s cap | 60.03s cap | unchanged | Still capped; this is a throughput improvement, not completion yet |
| `zenpayroll --branches` guardrail | 2.92s | 3.45s | within variance | Changed path is not used; PR lookup failure remained ~688ms |
| `zenpayroll --worktrees` guardrail | 14.28s | 12.52s | within variance | Changed path is not used |

### Outcome

Accepted. The full remote dump still does not complete inside 60 seconds, but
the targeted bottleneck completed more squash checks under the same cap and the
same overlapping branch set was faster on average. The change is small and keeps
all external behavior unchanged.

### Validation

- `cargo test squash` passed.
- `rustfmt src/git/squash_loader.rs --check` passed.
- `cargo build --release` passed.

### Next bottleneck

For `gbm-zenpayroll --remotes`, the next meaningful target is still
`git::merge_detection::is_squash_merged`: each cache-miss candidate still runs
multiple Git commands, and the full dump remains capped before all 16,056
candidates are checked.

## Iteration 9: use candidate commit for squash merge-base

Date: 2026-06-07 local / 2026-06-08 UTC

Commit before change: `62a0f2a`

### Baseline

The baseline is the accepted after-run from Iteration 8:

```sh
GBM_TIMING_LOG=/tmp/gbm-loop-after-squash-commit-hash-gbm-zenpayroll-remotes-60s.log \
  /usr/bin/time -p perl -e 'alarm shift; exec @ARGV' 60 \
  ./target/release/git-branch-manager \
  --repo /Users/chris.thompson/workspace/gbm-zenpayroll --remotes --color=never
```

Result:

| Repo / view | Wall time | Key span | Span time | Note |
| --- | ---: | --- | ---: | --- |
| `gbm-zenpayroll --remotes` | 60.03s cap | `git::squash_loader::squash_candidate` | 39.37s total busy | 254 real squash checks completed before the cap; mean 155.0ms, p95 226ms |

### Decision

Changed one function: `git::merge_detection::is_squash_merged`.

After Iteration 8, `spawn_squash_checker` passes the candidate commit hash into
`is_squash_merged`, but `is_squash_merged` still used the branch ref for
`git merge-base`. This iteration uses the provided commit hash for merge-base
and tree lookup when present. For a candidate snapshot, `git merge-base main
<commit>` is equivalent to resolving the branch ref first, and callers that do
not have a commit hash keep the old branch-name path.

### After

```sh
GBM_TIMING_LOG=/tmp/gbm-loop-after-squash-mergebase-commit-zenpayroll-branches.log \
  /usr/bin/time -p ./target/release/git-branch-manager \
  --repo /Users/chris.thompson/workspace/zenpayroll --branches --color=never

GBM_TIMING_LOG=/tmp/gbm-loop-after-squash-mergebase-commit-zenpayroll-worktrees.log \
  /usr/bin/time -p ./target/release/git-branch-manager \
  --repo /Users/chris.thompson/workspace/zenpayroll --worktrees --color=never

GBM_TIMING_LOG=/tmp/gbm-loop-after-squash-mergebase-commit-gbm-zenpayroll-remotes-60s.log \
  /usr/bin/time -p perl -e 'alarm shift; exec @ARGV' 60 \
  ./target/release/git-branch-manager \
  --repo /Users/chris.thompson/workspace/gbm-zenpayroll --remotes --color=never
```

Results:

| Repo / view | Before | After | Delta | Evidence |
| --- | ---: | ---: | ---: | --- |
| `gbm-zenpayroll squash_candidate` completed checks | 254 | 260 | +2.4% | Same 60s cap; candidate count remained 16,056 |
| `gbm-zenpayroll squash_candidate` mean | 155.0ms | 151.9ms | -2.0% | Small improvement on the completed checks |
| `gbm-zenpayroll squash_candidate` p95 | 226ms | 217ms | -4.0% | Same parser and same candidate order |
| Common first 254 checks | 155.0ms mean | 151.3ms mean | -3.7ms/check | 157 checks improved, 89 worsened |
| `gbm-zenpayroll --remotes` full dump | 60.03s cap | 60.04s cap | unchanged | Still capped; throughput improved slightly |
| `zenpayroll --branches` guardrail | 3.45s | 2.86s | within variance | Changed path is not used; PR lookup failure remained ~737ms |
| `zenpayroll --worktrees` guardrail | 12.52s | 12.92s | within variance | Changed path is not used |

### Outcome

Accepted. The improvement is small, but the same overlapping branch set was
faster on average and the capped run completed six additional squash checks.
The full remote dump still does not complete inside 60 seconds.

### Validation

- `cargo test squash` passed.
- `rustfmt src/git/merge_detection.rs --check` passed.
- `cargo build --release` passed.

### Next bottleneck

For `gbm-zenpayroll --remotes`, the remaining issue is not branch ref resolution
alone. The function still shells out multiple times per cache miss
(`merge-base`, `commit-tree`, `cherry`), so the next useful iteration should
target command count or a bulk squash-detection approach.

## Iteration 10: periodically save squash-cache progress

Date: 2026-06-07 local / 2026-06-08 UTC

Commit before change: `8f3d1a5`

### Baseline

The baseline is the accepted after-run from Iteration 9:

```sh
GBM_TIMING_LOG=/tmp/gbm-loop-after-squash-mergebase-commit-gbm-zenpayroll-remotes-60s.log \
  /usr/bin/time -p perl -e 'alarm shift; exec @ARGV' 60 \
  ./target/release/git-branch-manager \
  --repo /Users/chris.thompson/workspace/gbm-zenpayroll --remotes --color=never
```

Result:

| Repo / view | Wall time | Cache load | Key span | Note |
| --- | ---: | ---: | --- | --- |
| `gbm-zenpayroll --remotes` | 60.04s cap | 0 entries | 260 `squash_candidate` spans | All 260 were cache misses; mean 151.9ms, p95 217ms; no cache save occurred before the cap killed the process |

### Decision

Changed one function: `git::squash_loader::spawn_squash_checker`.

The remote squash checker only saved its `BranchCache` after processing all
candidates. In the large remote repo, the process is killed by the 60s
diagnostic cap before the final save, so repeated runs redo the same cache
misses. This iteration saves after every 200 new cache inserts, and saves any
dirty progress before returning if the receiver is dropped. The normal final
save remains unchanged.

### After

```sh
GBM_TIMING_LOG=/tmp/gbm-loop-after-squash-periodic-save-zenpayroll-branches.log \
  /usr/bin/time -p ./target/release/git-branch-manager \
  --repo /Users/chris.thompson/workspace/zenpayroll --branches --color=never

GBM_TIMING_LOG=/tmp/gbm-loop-after-squash-periodic-save-zenpayroll-worktrees.log \
  /usr/bin/time -p ./target/release/git-branch-manager \
  --repo /Users/chris.thompson/workspace/zenpayroll --worktrees --color=never

GBM_TIMING_LOG=/tmp/gbm-loop-after-squash-periodic-save-gbm-zenpayroll-remotes-60s-run1.log \
  /usr/bin/time -p perl -e 'alarm shift; exec @ARGV' 60 \
  ./target/release/git-branch-manager \
  --repo /Users/chris.thompson/workspace/gbm-zenpayroll --remotes --color=never

GBM_TIMING_LOG=/tmp/gbm-loop-after-squash-periodic-save-gbm-zenpayroll-remotes-60s-run2.log \
  /usr/bin/time -p perl -e 'alarm shift; exec @ARGV' 60 \
  ./target/release/git-branch-manager \
  --repo /Users/chris.thompson/workspace/gbm-zenpayroll --remotes --color=never
```

Results:

| Repo / view | Before | After | Delta | Evidence |
| --- | ---: | ---: | ---: | --- |
| Cold remote capped run | 260 candidates | 257 candidates | -1.2% | Run 1 started with 0 cache entries; periodic save wrote 200 entries in 0.36ms |
| Repeated remote capped run | 260 candidates | 411 candidates | +58.1% | Run 2 loaded 200 cache entries; 200 hits averaged 0.0029ms |
| Repeated remote cache misses | 260 misses | 211 misses | -18.8% | Run 2 skipped the first 200 previously checked candidates |
| Repeated remote full dump | 60.04s cap | 60.01s cap | unchanged | Still capped, but progresses farther and saves 400 entries by the next cap |
| `zenpayroll --branches` guardrail | 2.86s | 2.89s | within variance | Changed path is not used |
| `zenpayroll --worktrees` guardrail | 12.92s | 12.73s | within variance | Changed path is not used |

### Outcome

Accepted. The cold capped run is roughly neutral, but repeated capped runs now
reuse completed squash checks instead of starting over from an empty cache. This
directly improves the diagnostic loop and any interrupted long remote run.

### Validation

- `cargo test squash` passed.
- `rustfmt src/git/squash_loader.rs --check` passed.
- `cargo build --release` passed.

### Next bottleneck

The remote dump still does not finish inside 60 seconds. With cache progress now
persisted, the next iteration can either keep running until cache warmup exposes
the next phase, or reduce the per-miss command count in
`git::merge_detection::is_squash_merged`.

## Iteration 11: derive remote merged status from ahead count

Date: 2026-06-07 local / 2026-06-08 UTC

Commit before change: `a32b42a`

### Baseline

The baseline is the accepted repeated run from Iteration 10:

```sh
GBM_TIMING_LOG=/tmp/gbm-loop-after-squash-periodic-save-gbm-zenpayroll-remotes-60s-run2.log \
  /usr/bin/time -p perl -e 'alarm shift; exec @ARGV' 60 \
  ./target/release/git-branch-manager \
  --repo /Users/chris.thompson/workspace/gbm-zenpayroll --remotes --color=never
```

Result:

| Repo / view | Wall time | Key span | Span time | Note |
| --- | ---: | --- | ---: | --- |
| `gbm-zenpayroll --remotes` | 60.01s cap | `git::branch::remote_enricher_worker` | 18.5s | `for-each-ref ... ahead-behind` 11.2s plus `branch -r --merged` 7.30s |
| `gbm-zenpayroll --remotes` | 60.01s cap | `git::squash_loader::squash_candidate` | 411 candidates | 200 cache hits, 211 misses |

### Decision

Changed one function: `git::branch::spawn_remote_enricher`.

The worker already runs `git for-each-ref refs/remotes
--format='%(refname:short)%09%(ahead-behind:<base>)'`. A remote ref with
`ahead == 0` has no commits not reachable from the base commit, which is the same
merged condition needed by this view. A direct probe on `gbm-zenpayroll` matched
exactly: 546 refs with `ahead == 0`, 546 refs from `git branch -r --merged
<base>`, and no differences in either direction. This iteration removes the
second Git command and derives `MergeStatus::Merged` from `ahead == Some(0)`.

### After

```sh
GBM_TIMING_LOG=/tmp/gbm-loop-after-remote-merged-from-ahead-zenpayroll-branches.log \
  /usr/bin/time -p ./target/release/git-branch-manager \
  --repo /Users/chris.thompson/workspace/zenpayroll --branches --color=never

GBM_TIMING_LOG=/tmp/gbm-loop-after-remote-merged-from-ahead-zenpayroll-worktrees.log \
  /usr/bin/time -p ./target/release/git-branch-manager \
  --repo /Users/chris.thompson/workspace/zenpayroll --worktrees --color=never

GBM_TIMING_LOG=/tmp/gbm-loop-after-remote-merged-from-ahead-gbm-zenpayroll-remotes-60s.log \
  /usr/bin/time -p perl -e 'alarm shift; exec @ARGV' 60 \
  ./target/release/git-branch-manager \
  --repo /Users/chris.thompson/workspace/gbm-zenpayroll --remotes --color=never
```

Results:

| Repo / view | Before | After | Delta | Evidence |
| --- | ---: | ---: | ---: | --- |
| `remote_enricher_worker` | 18.5s | 12.2s | -34.1% | Removed the 7.30s `branch -r --merged` command |
| Remote merged count | 546 | 546 | unchanged | Derived from `ahead == 0`; direct pre-change equivalence probe found no differences |
| Remote enricher Git commands | 2 | 1 | -1 command | Remaining command is the existing `for-each-ref ... ahead-behind` |
| `gbm-zenpayroll --remotes` full dump | 60.01s cap | 60.02s cap | unchanged | Still capped during squash checking |
| `gbm-zenpayroll squash_candidate` progress | 411 candidates | 597 candidates | +45.3% | After run loaded 400 cache entries, so this combines cache progress with the faster remote enricher |
| `zenpayroll --branches` guardrail | 2.89s | 2.95s | within variance | Changed path is not used |
| `zenpayroll --worktrees` guardrail | 12.73s | 13.62s | within variance | Changed path is not used |

### Outcome

Accepted. The changed function removes a redundant multi-second Git command from
the remote-enrichment phase while preserving the observed merged-status result
set on the large remote repo and passing remote tests.

### Validation

- Direct equivalence probe on `gbm-zenpayroll`: `ahead == 0` count 546,
  `branch -r --merged` count 546, no set differences.
- `cargo test remote` passed.
- `rustfmt src/git/branch.rs --check` passed.
- `cargo build --release` passed.

### Next bottleneck

The fixed remote-enrichment cost is lower, so the remaining capped time is again
remote squash checking. The latest run loaded 400 cached squash results but did
not save the next 197 misses before the cap, so the cache-save interval may now
be too coarse for the faster remote-enrichment phase.

## Iteration 12: lower squash-cache save interval

Date: 2026-06-07 local / 2026-06-08 UTC

Commit before change: `016a289`

### Baseline

The baseline is the accepted after-run from Iteration 11:

```sh
GBM_TIMING_LOG=/tmp/gbm-loop-after-remote-merged-from-ahead-gbm-zenpayroll-remotes-60s.log \
  /usr/bin/time -p perl -e 'alarm shift; exec @ARGV' 60 \
  ./target/release/git-branch-manager \
  --repo /Users/chris.thompson/workspace/gbm-zenpayroll --remotes --color=never
```

Result:

| Repo / view | Wall time | Cache load | Key span | Note |
| --- | ---: | ---: | --- | --- |
| `gbm-zenpayroll --remotes` | 60.02s cap | 400 entries | 597 `squash_candidate` spans | 400 cache hits, 197 misses; no cache save occurred because the 200-insert interval was not reached |

### Decision

Changed one function: `git::squash_loader::spawn_squash_checker`.

Iteration 11 made remote enrichment faster, which left enough time for 197 new
squash misses before the cap. With the previous 200-entry save interval, those
197 checks were lost when the process was killed. This iteration lowers the
periodic cache-save interval from 200 to 100 inserts so capped runs persist
progress sooner. The normal final save and receiver-drop save behavior are
unchanged.

### After

```sh
GBM_TIMING_LOG=/tmp/gbm-loop-after-squash-save-100-zenpayroll-branches.log \
  /usr/bin/time -p ./target/release/git-branch-manager \
  --repo /Users/chris.thompson/workspace/zenpayroll --branches --color=never

GBM_TIMING_LOG=/tmp/gbm-loop-after-squash-save-100-zenpayroll-worktrees.log \
  /usr/bin/time -p ./target/release/git-branch-manager \
  --repo /Users/chris.thompson/workspace/zenpayroll --worktrees --color=never

GBM_TIMING_LOG=/tmp/gbm-loop-after-squash-save-100-gbm-zenpayroll-remotes-60s-run1.log \
  /usr/bin/time -p perl -e 'alarm shift; exec @ARGV' 60 \
  ./target/release/git-branch-manager \
  --repo /Users/chris.thompson/workspace/gbm-zenpayroll --remotes --color=never

GBM_TIMING_LOG=/tmp/gbm-loop-after-squash-save-100-gbm-zenpayroll-remotes-60s-run2.log \
  /usr/bin/time -p perl -e 'alarm shift; exec @ARGV' 60 \
  ./target/release/git-branch-manager \
  --repo /Users/chris.thompson/workspace/gbm-zenpayroll --remotes --color=never
```

Results:

| Repo / view | Before | After | Delta | Evidence |
| --- | ---: | ---: | ---: | --- |
| First capped run cache saves | none | 500 and 600 entries | improved | Run 1 loaded 400 entries and saved twice before the cap |
| First capped run progress | 597 candidates | 605 candidates | +1.3% | Run 1 had 400 hits and 205 misses |
| Repeated capped run cache load | 400 entries | 600 entries | +200 entries | Run 2 loaded the entries saved by run 1 |
| Repeated capped run progress | 597 candidates | 784 candidates | +31.3% | Run 2 had 600 hits and 184 misses |
| Repeated remote full dump | 60.02s cap | 60.02s cap | unchanged | Still capped during squash checking |
| `zenpayroll --branches` guardrail | 2.95s | 2.88s | within variance | Changed path is not used |
| `zenpayroll --worktrees` guardrail | 13.62s | 14.29s | within variance | Changed path is not used |

### Outcome

Accepted. Lowering the interval preserves progress that was previously lost
under the 60s cap, and the next run starts 200 cache entries farther ahead. The
full remote dump is still capped, but repeated runs now advance more reliably.

### Validation

- `cargo test squash` passed.
- `rustfmt src/git/squash_loader.rs --check` passed.
- `cargo build --release` passed.

### Next bottleneck

The remaining remote time is still uncached squash detection. With the cache now
advancing in 100-entry chunks, subsequent diagnostics can continue warming the
cache or target the per-miss `is_squash_merged` command sequence.
