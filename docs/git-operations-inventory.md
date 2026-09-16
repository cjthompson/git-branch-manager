# Git operation inventory

This document records common, user-facing Git operations that can be performed
on commits, branches, tags, remote-tracking branches, arbitrary refs, and
worktrees, then compares them with the operations currently exposed by
git-branch-manager.

The inventory is intentionally bounded to normal interactive Git workflows.
It does not attempt to enumerate every plumbing command or every option of
every porcelain command. `Direct` means the application exposes a selectable
action. `Partial` means a related action exists but its target, strategy, or
scope is narrower than the usual Git operation. `Display-only` means the
application shows related information without offering the mutation. `Internal`
means Git performs the operation as part of another action, but the operation
is not independently selectable. `Missing` means no user-facing operation was
found.

The app also runs Git reads internally to load refs, determine merge status and
ahead/behind state, inspect worktree cleanliness, and enrich the Graph. Those
reads support the UI but are not independent user actions; the matrix calls
that distinction out where it affects coverage.

## Current action surfaces

The Graph `ENTER` menu is ref-driven. It expands the actions for each local
branch, remote branch, or tag attached to the selected commit; a commit with
no matching ref has no operation rows. With multiple refs, the menu prefixes
each row with its ref name and removes the single-letter shortcuts.

| Surface | Current user-visible actions | Important boundary |
| --- | --- | --- |
| Graph `ENTER` on a local branch ref | Checkout; Delete local; Delete local + remote; Force-delete local; Fast-forward; Push; Force push; Pull; Merge into base; Squash merge into base; Rebase onto base; Create worktree; Open PR in browser | Actions belong to the attached branch ref, not to the selected commit as an arbitrary object. |
| Graph `ENTER` on a remote branch ref | Checkout remote; Delete remote branch; Delete remote + local; Fetch remote; Pull remote; Merge into current; Cherry-pick latest; View PR in browser | Cherry-pick is limited to the tip of the selected remote branch. |
| Graph `ENTER` on a tag ref | Delete tag; Delete tag local + remote; Push tag | No create, move, inspect/verify, checkout, or compare-tag action. |
| Branches view | The local-branch actions above, plus selected-item shortcuts for Checkout, Delete local, Delete local + remote, Push, Fetch, and Fetch + prune | Branch actions generally operate on the selected branch or the configured base branch. |
| Remotes view | The remote-branch actions above, plus selected-item shortcuts for Delete remote, Checkout remote, Fetch remote, and Fetch + prune | Remote configuration itself is not exposed. |
| Tags view | Delete tag; Delete tag + remote; Push tag; Fetch; Fetch + prune | Tag creation and tag inspection are not exposed as operations. |
| Worktrees view | Remove worktree; Force remove worktree; Fetch; Fetch + prune | Worktree move, lock, prune, repair, and detached-at-commit creation are not exposed. |
| Confirmation/recovery paths | Force-delete local; remove worktree + delete branch; force-remove worktree + delete branch; remove worktree + delete branch local + remote | These are recovery/cascade paths, not ordinary Graph or item-menu rows. |
| Global/view controls | Fetch all; Fetch + prune; cancel queued/running jobs; clear cache; reload/recheck data | Cancel, cache, and reload controls are application controls rather than Git operations. |

`Open PR in browser` and `View PR in browser` are included above to make the
full menu inventory explicit, but they are integration actions rather than Git
operations.

## Comparison matrix

### Commit inspection and history

| Target | Typical operation | Typical Git form | App coverage | Current behavior or gap |
| --- | --- | --- | --- | --- |
| Repository/ref | Load refs and derive branch relationships | `git for-each-ref`, `git merge-base`, `git cherry` | Internal | Used for list loading, merge-status detection, and Graph enrichment; there is no user-invoked operation for these queries. |
| Commit | View commit metadata | `git show --no-patch`, `git log` | Display-only | The Graph and commit info modal show commit metadata. |
| Commit | Browse commit history and topology | `git log --graph` | Display-only | The Graph displays the history and can load older commits. |
| Commit | View the commit patch | `git show <commit>` | Missing | No action opens a commit diff or patch view. |
| Commit | Compare a commit with its parent | `git diff <commit>^ <commit>` | Missing | No commit-level diff action. |
| Commit/ref | Compare two arbitrary commits or refs | `git diff <a> <b>` | Missing | No arbitrary source/target comparison. |
| Commit/ref | Show changed files and statistics | `git diff --stat`, `git show --stat` | Missing | Related status data is used internally, but no commit diff/stat action is exposed. |
| Commit/ref | Find branches/tags containing or pointing at a commit | `git branch --contains`, `git tag --points-at` | Display-only | Attached refs are shown in the Graph; there is no general contains/points-at query. |
| Commit/ref | Inspect the object or raw contents | `git cat-file`, `git ls-tree` | Missing | No object/tree/blob inspection action. |
| Commit/ref | Verify a commit signature | `git verify-commit` | Missing | Signature status is not exposed. |
| Commit/ref | Inspect or compare a commit range | `git log A..B`, `git range-diff` | Missing | No range selection or range-diff view. |
| File/line in a commit | Show blame/line history | `git blame`, `git log -L` | Missing | No file browser or blame action. |

### Commit creation and history editing

| Target | Typical operation | Typical Git form | App coverage | Current behavior or gap |
| --- | --- | --- | --- | --- |
| Working tree/index | Stage or unstage changes | `git add`, `git restore --staged` | Missing | The app does not expose an index/staging view. |
| Working tree/index | Create a commit | `git commit` | Missing | No commit message editor or commit action. |
| Commit | Amend the current commit | `git commit --amend` | Missing | No amend action. |
| Commit | Reword, edit, split, reorder, fix up, or squash commits | `git rebase -i` | Missing | The app's squash merge creates a merge result; it is not interactive history editing. |
| Commit/range | Cherry-pick one commit or a range | `git cherry-pick <commit>...` | Partial | `Cherry-pick latest` operates only on the tip of a selected remote branch. |
| Commit/range | Revert one commit or a range | `git revert <commit>...` | Missing | No revert action. |
| Merge commit | Revert a merge commit | `git revert -m <parent> <merge>` | Missing | No merge-parent selection or merge-revert action. |
| Commit/range | Apply a patch | `git apply`, `git am` | Missing | No patch import/apply workflow. |
| Commit/range | Export patches | `git format-patch`, `git bundle` | Missing | No patch or bundle export. |
| Repository history | Bisect a regression | `git bisect start/good/bad/skip` | Missing | No bisect session management. |
| Commit | Attach or inspect notes | `git notes` | Missing | No notes workflow. |

### Local branches and ref pointers

| Target | Typical operation | Typical Git form | App coverage | Current behavior or gap |
| --- | --- | --- | --- | --- |
| Commit/ref | Create a branch at an arbitrary commit or ref | `git branch <name> <start-point>` | Missing | No branch-creation action from a selected commit or ref. |
| Branch | Switch/check out a local branch | `git switch <name>`, `git checkout <name>` | Partial | Checkout is available for a local branch, subject to current/base/worktree guards; there is no general commit checkout. |
| Commit | Detach `HEAD` at an arbitrary commit | `git switch --detach <commit>` | Missing | No detached-commit checkout action. |
| Branch | Rename a branch | `git branch -m` | Missing | No local or remote rename workflow. |
| Branch | Copy a branch | `git branch -c` | Missing | No branch-copy action. |
| Branch | Delete safely | `git branch -d` | Partial | `Delete local` is available, but its safety behavior is mediated by the app's merge-status/preflight flow rather than exposing a separate `-d` choice. |
| Branch | Delete forcibly | `git branch -D` | Direct | `Force-delete local` is available from the menu and as a recovery action. |
| Branch | Delete local and corresponding remote branch | `git push <remote> --delete <branch>` plus local deletion | Direct (constrained) | `Delete local + remote` is available when a tracking remote exists. |
| Branch | Set or change upstream | `git push -u`, `git branch --set-upstream-to` | Partial | A normal Push can set the upstream; there is no explicit upstream configuration action. |
| Branch | Unset upstream | `git branch --unset-upstream` | Missing | No action. |
| Branch | Set branch description/configuration | `git branch --edit-description`, `git config branch.*` | Missing | No branch metadata editor. |
| Branch | Merge into a chosen destination | `git merge <source>` | Partial | Merge is exposed for a branch, but the destination is the configured base branch rather than an arbitrary user-selected ref. |
| Branch | Squash-merge into a chosen destination | `git merge --squash <source>` | Partial | Squash merge is exposed only into the configured base branch and follows the app's current-worktree workflow. |
| Branch | Rebase onto a chosen target | `git rebase <target>` | Partial | Rebase is exposed onto the configured base branch; arbitrary target selection is not available from the action menu. |
| Branch/ref | Fast-forward or update a ref without checkout | `git fetch <remote> <src>:<dst>`, `git update-ref` | Partial | Fast-forward is available for a non-current tracked branch, but the implementation uses the configured `origin` and branch name. |
| Branch/ref | Reset a branch to a commit | `git reset`, `git update-ref` | Missing | No soft, mixed, or hard reset action. |
| Branch/ref | Force-move a branch pointer | `git update-ref`, `git branch -f` | Missing | No arbitrary ref movement. |
| Branch/ref | Compare branch reachability or divergence | `git merge-base`, `git rev-list`, `git log A..B` | Display-only | Ahead/behind information is shown for tracking branches; arbitrary pair comparison is not available. |

### Remote synchronization and remote configuration

| Target | Typical operation | Typical Git form | App coverage | Current behavior or gap |
| --- | --- | --- | --- | --- |
| Repository | Fetch all remotes | `git fetch --all` | Direct | Available as Fetch all. |
| Repository | Fetch and prune stale remote-tracking refs | `git fetch --all --prune` | Direct | Available as Fetch + prune. |
| Remote | Fetch one remote | `git fetch <remote>` | Direct (constrained) | Fetch remote is available from a selected remote branch; there is no separate remote-configuration screen. |
| Remote/refspec | Fetch a chosen refspec, shallow range, or tag set | `git fetch <remote> <refspec>` | Partial | Fetch is available, but arbitrary refspec/depth/filter selection is not. |
| Local tracking branch | Pull with fast-forward-only behavior | `git pull --ff-only` | Partial | Pull is available for a behind tracked branch, but current behavior is narrower than general pull and uses the app's fast-forward path. |
| Local tracking branch | Pull by merge | `git pull --no-rebase` | Missing | No pull strategy choice. |
| Local tracking branch | Pull by rebase | `git pull --rebase` | Missing | No pull strategy choice. |
| Local branch | Push to its upstream | `git push` | Direct (constrained) | Push is available when the branch is ahead or needs an upstream. |
| Local branch | Force-push safely | `git push --force-with-lease` | Direct (constrained) | Force push is exposed for the app's ahead-and-behind case; arbitrary refspec/lease selection is not. |
| Ref/refspec | Push a chosen ref or refspec | `git push <remote> <src>:<dst>` | Missing | No arbitrary source/destination ref mapping. |
| Remote branch | Delete a remote branch | `git push <remote> --delete <branch>` | Direct | Delete remote branch is available. |
| Remote-tracking refs | Prune stale tracking refs | `git remote prune <remote>` | Partial | Fetch + prune provides the repository-wide form; there is no standalone per-remote prune action. |
| Remote | Add, remove, rename, or inspect a remote | `git remote add/remove/rename/show` | Missing | Remote names and URLs are not configurable in the app. |
| Remote | Change remote URL or push URL | `git remote set-url` | Missing | No remote URL editor. |
| Remote | Fetch/push all refs as a mirror | `git push --mirror`, matching refspecs | Missing | No mirror synchronization. |

### Tags

| Target | Typical operation | Typical Git form | App coverage | Current behavior or gap |
| --- | --- | --- | --- | --- |
| Tag | List tags and the commits they name | `git tag`, `git show-ref --tags` | Display-only | Tags are listed with associated information. |
| Commit/ref | Create a lightweight tag | `git tag <name> <commit>` | Missing | No tag-creation action. |
| Commit/ref | Create an annotated or signed tag | `git tag -a/-s <name> <commit>` | Missing | No message/signing flow. |
| Tag | Move or retarget a tag | `git tag -f <name> <commit>` | Missing | No tag update action. |
| Tag | Delete a local tag | `git tag -d <name>` | Direct | Delete tag is available. |
| Tag | Delete a remote tag | `git push <remote> --delete <tag>` | Direct (combined) | Delete tag local + remote is available. |
| Tag | Push a tag | `git push <remote> <tag>` | Direct | Push tag is available. |
| Tag | Fetch tags | `git fetch --tags` | Partial | Fetch all may update tags, but there is no explicit tag-only fetch action. |
| Tag | Verify a tag signature or object | `git verify-tag`, `git show <tag>` | Missing | No verification or tag-target inspection action. |
| Tag/ref | Checkout, diff, merge, or compare a tag as an arbitrary ref | `git switch <tag>`, `git diff <tag>`, `git merge <tag>` | Missing | Tags in the Graph menu currently offer deletion and push only. |

### Arbitrary refs and recovery

| Target | Typical operation | Typical Git form | App coverage | Current behavior or gap |
| --- | --- | --- | --- | --- |
| Any ref | List and inspect refs | `git for-each-ref`, `git show-ref` | Display-only | The app displays selected categories of refs, not a general ref browser. |
| Any ref | Create, move, or delete a ref | `git update-ref` | Missing | No arbitrary-ref editor. |
| Any ref | Inspect ref history | `git reflog` | Missing | No reflog viewer. |
| Any ref | Recover a prior ref value | `git reflog`, `git update-ref` | Missing | No recovery workflow for lost commits or moved refs. |
| Repository | Check object/database integrity | `git fsck` | Missing | No object integrity check. |
| Repository | Garbage-collect, repack, or expire reflogs | `git gc`, `git repack` | Missing | No repository maintenance actions. |
| Repository | Clean untracked or ignored files | `git clean` | Missing | No destructive working-tree cleanup action. |
| Working tree | Show status and changed files | `git status`, `git diff` | Display-only / Internal | Worktree cleanliness is read internally and shown in worktree data; there is no full status/diff browser. |
| Working tree | Stash changes | `git stash push` | Missing | No stash creation action. |
| Stash | List, inspect, apply, pop, branch, drop, or clear | `git stash list/show/apply/pop/drop/clear/branch` | Missing | No stash management. |
| In-progress operation | Continue, skip, or abort merge/rebase/cherry-pick/revert | `git merge --continue/--abort`, etc. | Missing | No conflict-state controller or continuation menu. |
| Conflict state | Launch conflict resolution or rerere support | `git mergetool`, `git rerere` | Missing | No conflict-resolution integration. |

### Worktrees

| Target | Typical operation | Typical Git form | App coverage | Current behavior or gap |
| --- | --- | --- | --- | --- |
| Repository | List worktrees and their state | `git worktree list` | Display-only | Worktrees are listed with branch, path, and cleanliness information. |
| Branch | Create a worktree for a branch | `git worktree add <path> <branch>` | Direct | Create worktree is available from a local branch. |
| Commit/ref | Create a detached worktree at an arbitrary commit/ref | `git worktree add --detach <path> <commit>` | Missing | Creation is branch-oriented; no detached commit target. |
| Worktree | Remove a clean worktree | `git worktree remove <path>` | Direct | Remove worktree is available, with guards for the main and dirty worktrees. |
| Worktree | Force-remove a worktree | `git worktree remove --force <path>` | Direct | Force remove worktree is available. |
| Worktree + branch | Remove worktree and delete its branch | Worktree removal plus branch deletion | Direct (combined) | Available from the worktree menu and delete recovery path. |
| Worktree + branch + remote | Remove worktree and delete local and remote branch | Combined `worktree remove` and remote deletion | Direct (combined) | Available when the branch is clean, non-base, and has a remote; recovery also has a cascade form. |
| Worktree | Move a worktree | `git worktree move` | Missing | No move action. |
| Worktree | Lock or unlock a worktree | `git worktree lock/unlock` | Missing | No lock state control. |
| Repository | Prune stale worktree metadata | `git worktree prune` | Missing | No standalone worktree metadata cleanup. |
| Worktree | Repair a worktree registration | `git worktree repair` | Missing | No repair action. |

## Main findings

The current app is strong on branch lifecycle and synchronization for a
configured base branch, plus basic worktree cleanup. The largest gaps are not
additional variants of the existing delete/push menu; they are missing target
and object models:

- There is no generic commit action layer. A selected commit can only inherit
  actions from attached refs, so commit diff, revert, cherry-pick, branch-at-
  commit, tag-at-commit, and detached-worktree workflows are unavailable.
- There is no arbitrary-ref target model. Reset, ref movement, ref comparison,
  reflog recovery, tag retargeting, and refspec-oriented synchronization cannot
  be represented by the current branch/tag/remote menu targets.
- History editing and working-tree authoring are absent. Amend, interactive
  rebase, staging, commit creation, stash management, conflict continuation,
  and bisect are not exposed.
- Several existing labels describe broader Git concepts than the current
  implementation supports. In particular, Pull is a constrained
  fast-forward path; Merge and Rebase use fixed application destinations; and
  Cherry-pick latest selects only a remote branch tip.
- Worktree lifecycle is only partially covered. Add/remove and deletion
  cascades exist, while move, lock, prune, repair, and detached-at-commit
  creation do not.

## Source locations used for the comparison

- [`src/types.rs`](../src/types.rs) — `BranchAction` variants and labels.
- [`src/app.rs`](../src/app.rs) — Graph ref-driven menu construction, branch,
  remote, tag, and worktree menus, view-level actions, and recovery paths.
- [`src/ui/help.rs`](../src/ui/help.rs) — view-specific keyboard actions.
- [`src/git/operations.rs`](../src/git/operations.rs) — fetch, fast-forward,
  pull, push, merge, and rebase behavior.
- [`src/git/tags.rs`](../src/git/tags.rs) — tag deletion and push behavior.
- [`src/job_queue.rs`](../src/job_queue.rs) — dispatch and combined/cascade
  action behavior.
- [`docs/rewrite-requirements.md`](rewrite-requirements.md) — existing Git
  operation API requirements for the current application model.
