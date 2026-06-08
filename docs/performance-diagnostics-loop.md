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

