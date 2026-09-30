# Share cache data across linked worktrees

**Date:** 2026-09-29
**Status:** Design — approved, pending spec review

## Context

`BranchCache::load` currently hashes the path passed by its caller. The app
passes the worktree directory, so each linked worktree gets a different SQLite
file even though the worktrees share the same Git object database and common
refs. This duplicates reusable ahead/behind, merge-base, and Graph patch data.

The cache also stores merge statuses and a `base_tip` shortcut marker. Those
values depend on the selected base branch. Sharing the current schema unchanged
could let worktrees using different `--base` values overwrite or reuse each
other's status entries. Integration tests also create temporary repositories;
their persistent cache files outlive the temporary directories.

## Goal

- Use one SQLite cache file for a Git repository's common directory. Linked
  worktrees share it; separate clones keep separate files.
- Reuse repository-wide values keyed by immutable Git object IDs.
- Keep branch merge-status data independent for each selected base branch.
- Prevent temporary test repositories from leaving files in the user's
  persistent cache directory.
- Remove abandoned persistent cache databases after 60 days without use.

## Design

Resolve the repository's common Git directory and hash that canonical path for
the cache filename. Use a new filename prefix for this repository-scoped cache
so it cannot accidentally open an existing worktree-path database with the old
schema. This makes the main worktree and linked worktrees resolve to the same
database without sharing data between independent clones. If Git cannot resolve
the common directory, retain the caller-path identity as a safe fallback; this
may miss sharing but must not prevent the app from opening.

Keep the ahead/behind, merge-base, and Graph patch tables shared in that
database. Their keys use commit OIDs (or OID pairs), so their computed values
do not depend on which worktree requested them.

Bind each `BranchCache` used for status work to the selected base branch. Every
base-sensitive caller passes that branch when loading the cache, and the scope
travels with the cache object into async squash, cherry, and diagnostic work.
Persist branch-status rows by `(base_branch, branch_name)` and store a separate
base-tip marker for each base branch. A lookup or write uses the bound base
scope; the fast-path check compares that base branch's recorded tip. This keeps
one physical database per repository while avoiding cross-base status reuse.
OID-keyed tables do not receive this scope. Existing in-process writers
continue to use SQLite WAL transactions and the current busy timeout.

The cache-clear action continues to clear the repository database, including
all base scopes and OID-keyed values.

Run a best-effort cache sweep once at application startup, before opening
persistent cache databases. Restrict it to direct children of the configured
cache root whose names match the app's legacy `git-bm-cache-*.sqlite3` or new
`git-bm-repo-cache-*.sqlite3` database patterns. Expire a database 60 days
after its last use; refresh its modification time when it is loaded, including
read-only cache hits. Remove its matching `-wal` and `-shm` files at the same
time. Ignore filesystem cleanup errors so pruning cannot prevent startup.
If a file's age cannot be determined or its removal fails, leave it for a later
sweep.

The cache contents are derived. Expiring a database only causes later cache
misses and recomputation. A repository used within the retention window keeps
its cache even if it did not write new entries.

Integration tests must direct cache writes to a test-owned temporary cache root
outside the repository worktree, never the user's OS cache directory. Pass this
location explicitly through cache-consuming calls and background workers;
avoid a process-global environment override because tests run in parallel.
Linked worktrees created by one test use that test's same cache root. On normal
`TestDir` drop, remove both the test repository and its cache root, including
SQLite WAL/SHM sidecars. When `GBM_KEEP_TEST_REPOS` is set, preserve both for
inspection.

## Existing cache files

The new common-directory identity will not read or migrate entries from
existing worktree-path cache files. Existing legacy files remain until they
age past the 60-day retention window or are removed manually; the startup sweep
recognizes both legacy and new filename patterns.

## Components and touch points

- `src/git/cache.rs`: resolve the common directory, scope merge-status and
  base-tip storage by base branch, and accept an explicit cache root while
  retaining the OS cache directory as the production default. Add the
  age-based sweep and refresh a database's last-used timestamp on load.
- `src/main.rs`, `src/app.rs`, `src/git/branch.rs`, `src/git/diagnostics.rs`,
  `src/git/graph.rs`, and `src/dump.rs`: pass the selected base branch for
  status-aware cache handles and pass cache location through internal loads.
  Async squash/cherry workers keep using the already scoped handle they
  receive.
- `src/git/cache.rs`: preserve repository-wide OID-keyed reuse without adding
  base-branch scope to those tables.
- `tests/integration.rs`: exercise a main worktree plus a linked worktree,
  verify shared database identity and independent status scopes for different
  bases, give each `TestDir` a temporary cache root, pass it through every
  cache-using path, and remove it when the test repository drops.

## Testing

- A cache-path test proves that the main worktree and a linked worktree resolve
  to the same SQLite file, while a separate clone resolves to another file.
- Cache tests prove that OID-keyed values are visible from either worktree and
  branch-status rows and base-tip markers remain isolated by base branch.
- Integration tests assert their database paths are beneath the test's temp
  cache root and outside the user's OS cache directory.
- A `TestDir` lifecycle test proves normal teardown removes the temporary cache
  root and WAL/SHM sidecars, while `GBM_KEEP_TEST_REPOS` preserves them.
- Pruning tests use a controlled clock to prove files at 59 days remain, files
  past 60 days and their sidecars are removed, recently loaded read-only caches
  are retained, and unrelated files are untouched.
- Run the focused Rust tests and the repository-required `cargo build` during
  implementation.

## Delivery

1. Stop integration tests from leaving persistent cache files. This cleanup is
   useful on its own and can ship independently.
2. Add 60-day cleanup for current and legacy app cache files. This can ship
   independently and starts aging out existing path-keyed files.
3. Change the cache identity to the common Git directory and add base-scoped
   status storage. These changes must ship together so a shared database never
   exposes unscoped status rows.

## Out of scope

- One global SQLite file for every Git repository.
- Migrating cached entries from worktree-path databases into repository-scoped
  databases.
- Changing merge-detection or Graph patch algorithms.
