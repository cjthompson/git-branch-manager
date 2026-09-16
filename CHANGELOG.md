# Changelog

## 2026-09-16

### Plan: Graph Actions-Menu Availability Correction (P010)
- Full verification matrix: cargo build clean, cargo test 660 passed (1 ignored), cargo clippy clean, cargo fmt clean on all P010-touched files (#ui, #git, #worktree)
- Add Req 5 regression tests in tests/integration.rs (5 scenarios: current, linked-worktree, no-upstream, dirty, base-worktree; capability gating + dispatch-path assertions via inject_running_for_test; 412 insertions) (#ui, #git, #worktree)
- Audit help.rs, status_bar.rs, and other UI/non-UI files for stale gating prose — no user-facing changes needed (matches the plan's prediction; list_render bulk checkbox and Step 5 bulk filter are consistent on is_pinned semantics) (#ui, #git, #worktree)
- Resolve and pass the worktree-aware dispatch path from App (enqueue_or_start_with_remote gains dispatch_path parameter; App::dispatch_path_for resolves Merge/Squash to base worktree and Rebase to branch worktree; both Confirm overlay call sites updated) (#ui, #git, #worktree)
- Add dispatch_path plumbing to JobQueue (ActionJob gains dispatch_path: Option<PathBuf>; JobQueue::start resolves via unwrap_or_else fallback to self.repo_path; 11 construction sites set dispatch_path: None) (#ui, #git, #worktree)
- Refactor bulk multi-select key handlers (d/D/p) for parity: parameterized get_selected_branch_names_for helper; delete preserves !is_pinned() preflight UX, delete-local-and-remote adds no-remote check, push uses capability::can_push strict (#ui, #git, #worktree)
- Refactor build_branch_menu_for to call capability evaluators (src/app.rs:2552-2758, row_from helper, loading-gate closure); restore can_push tracked-ahead gate and can_pull non-current-behind gate; 46 unit tests in git::capability (#ui, #git, #worktree)
- Implement each capability evaluator with focused unit tests (src/git/capability.rs: 13 can_* evaluators, 43 unit tests, all clippy-clean) (#ui, #git, #worktree)
- Add App::worktree_presence_for and App::worktree_presence_for_branch_or_base helpers (src/app.rs:3056, :3074) bridging WorktreeInfo state to capability::WorktreePresence (#ui, #git, #worktree)

## 2026-09-15

### Plan: Graph Actions-Menu Availability Correction (P010)
- Add capability module (src/git/capability.rs) with WorktreePresence and Capability types plus 13 stub evaluator functions; wire pub mod capability into src/git/mod.rs (#ui, #git, #worktree)

### Tasks
- Inventory typical Git operations and compare app actions (#git, #ui)

## 2026-09-13

### Modal system redesign (P008)
- Reusable modal shell with pinned headers/footers, compact geometry, scrolling, and semantic theme roles
- Structured safe/recovery confirmation with a separate single-target destructive review
- Per-result Results accordion with selected-item recovery and dirty linked-worktree warnings
- Keyboard-selectable Settings and Filter actions, compact footer controls, and modal interaction regressions

### Fixes
- Visually distinguish unavailable Actions-menu rows (#ui, #p008)

### Likely squash-merge detection (P007)
- UI & safety surfacing

## 2026-09-12

### Likely squash-merge detection (P007)
- Concurrency precedence guard
- Wire likely_squash_merged into squash_loader
- Add merge_detection::likely_squash_merged heuristic function
- Types & exhaustive-match wiring

### Plan: Fix modal layout, contrast, and key-hint styling (P006)
- Tier 1: must-fix overlay bugs (theme.dim fg, confirm.rs bracket, question line, executing.rs panic)
- Tier 2: cross-overlay consolidation (key_hint + block_panel helpers, refactor 5 sites, fix cursor-row bg hole)
- Tier 3: overlay-specific readability fixes (wrap estimation, secondary_text, abbreviate_path, results/filter/help/executing/settings polish)
- Tier 4: regression tests (theme.dim fg, key_hint + block_panel, build_delete_preflight, executing non-ASCII)

## 2026-09-11

### Tasks
- feat: mark cherry-picked commits (#cherry-pick, #merge-detection, #ui, #symbols)

## 2026-09-10

### Plan: Specific, Recoverable Branch-Delete Failures (P005)
- UX for "doesn't exist anymore"
- Worktree data dependency on Branches view
- Recovery actions — ! and r

### Fixes
- Address P005 typed-failure and worktree lookup review findings

## 2026-09-07

### Plan: Specific, Recoverable Branch-Delete Failures (P005)
- Pre-flight reason in Confirm overlay
- Failure surface — auto-open Results
- Pre-flight in delete_local
- New BranchAction variants
- Typed failure cause
- Hoist the worktree→branch lookup

## 2026-09-06

### Force default branch into column 0 of the Graph view (P003)
- Verify: cargo build / cargo test / cargo clippy / manual visual check of Graph view (#verification)

## 2026-09-04

### Force default branch into column 0 of the Graph view (P003)
- Bump Cargo.toml version to 0.9.0-dev3 and run cargo build (#release)
- Add source-code comment above the fallback banner in src/ui/graph_render.rs (#graph, #ui)

## 2026-09-03

### Force default branch into column 0 of the Graph view (P003)
- Add 3 integration tests for column-0 placement in tests/integration.rs (basic, special chars, diverged remote) (#graph, #tests)

## 2026-09-02

### Graph Commit Details: Author + Local-Timezone Date (P004)
- Final acceptance pass
- Modal renders Author + Date rows
- Cross-loader agreement test for author + date
- git CLI fallback loader populates author + author date
- gleisbau loader populates author + author date
- Add author + date fields to GraphCommit
- Refactor 25 test fixtures to use ..GraphCommit::default()
- Add #[derive(Default)] to GraphCommit
- Add format_local_absolute helper

### Force default branch into column 0 of the Graph view (P003)
- Plumb base_branch into gleisbau_settings() (#graph, #git)
- Add `regex = "1"` as direct dep in Cargo.toml (#graph, #deps)

## 2026-08-31

### Fixes
- Action model scrollbar does not scroll with keyboard or mouse (#ui, #keyboard, #mouse, #scroll)
- Do not block Graph rendering on squash-merge detection (#graph, #performance, #async, #ui)

### Tasks
- Add OID-aware cache for Graph squash-merge detection (#graph, #cache, #git, #performance)

## 2026-08-30

### Tasks
- Document Graph UX and verify regression coverage (#graph, #docs, #testing)

## 2026-08-29

### Squash-merge detection test scenarios (P002)
- Implement squash-merge detection test scenarios (#testing, #squash-merge)

### Tasks
- Add git log graph view with improved graph drawing (#ui, #git, #graph)
- Reuse existing branch and remote action menus from Graph (#graph, #actions, #tui)

## 2026-08-28

### Tasks
- Add Gleisbau-backed graph loading with Git fallback (#graph, #git)
- Add the first Graph tab with DAG and branch sidebar (#graph, #tui)

## 2026-08-27

### Graph LRT Pseudo-pane Implementation (P001)
- Render the responsive LRT pseudo-pane (#graph, #ui, #responsive)
- Update the Graph handoff and verify integrated behavior (#graph, #docs, #testing)

### Tasks
- Add focus-aware horizontal Graph scrolling (#graph, #ui)
- Add possible squash-merged commit indicator to Graph (#graph, #git, #ui)

## 2026-08-26

### Graph LRT Pseudo-pane Implementation (P001)
- Reduce Graph ref metadata and add linked-worktree data (#graph, #git)
- Add one-cell right-pane glyphs (#graph, #symbols, #ui)

### Fixes
- Remote column should show full remote ref and base branch info (#ui, #branches)
- Remote view: show full remote branch name in separate column (#ui, #remote)

### Tasks
- Info modal: tab to switch focus between INFO and ACTIONS; arrow keys navigate INFO items; enter/y copies value to clipboard (#ui, #modal, #keyboard)
- Add 'in sync' status for branches that exactly match base (#ui, #merge-detection)

## 2026-07-17

### Tasks
- Worktrees action menu: add option to delete worktree + checked-out branch (local, or local + remote) (#worktrees)

## 2026-07-14

### Tasks
- Upgrade git2 crate to 0.21.0 (#deps)
- Stretchy-column priority in the responsive table layout is now matched by column name ("Branch"/"Name"/"Path") instead of position, so a future column reorder can't silently break which column gets priority width; also adds a tracking-link symbol to SymbolSet for a follow-up task (#ui)
- A/B column now colors ahead and behind counts differently (green/yellow) instead of a single shared color, across all four themes (#ui)
- Worktree details' "Changed Files" section now itemizes staged files (previously only counted via a has_staged flag, not listed individually); a file both staged and further edited shows as two separate entries (#ui, #worktrees)
- Responsive column width now demotes Age, then Merge, then A/B+PR one tier at a time as the terminal narrows, instead of flipping every column to its compact form at once (#ui, #responsive-width)

## 2026-07-10

### Fixes
- Deleting multiple remote branches in the Remotes view now shows the same per-item progress bar as local branch deletion, via a shared delete_remotes_with_progress helper

### Tasks
- Details view: worktrees with modified or untracked files now show a "Changed Files" section listing each path with its modified/untracked kind
- Parallelize squash-merge detection loop, capped at 4 worker threads pulling from a shared queue, with cache reads/writes retained by a single owner thread

## 2026-07-07

### Tasks
- Change error text color in Results overlay from dark gray to a brighter color (e.g. red)

## 2026-07-06

### Fixes
- Default sort column/direction settings now take effect at runtime: applied at startup to both Branches and Remotes views, applied live when edited in the Settings overlay, and applied to the CLI dump path (`--branches`/`--remotes`). Audit of all six Settings rows confirmed Symbol set, Theme, Auto-fetch, and Load worktrees were already correctly wired (#settings, #sort, #branches)
- Sort mode is preserved across post-operation reloads (fetch, delete, push, etc.) on Remotes/Tags/Worktrees views; previously `refresh_after_operation` rebuilt the `ListState` via `ListState::empty()` which discarded `sort_column`/`sort_ascending`. Branches was already correct (#fetch, #sort, #state)
- Default sort now applies to all four views (Branches, Remotes, Tags, Worktrees), each tracked independently in both Config and runtime `ListState`. Settings overlay shows one merged sort row per view (e.g. "Branches sort: age (asc)"). Existing configs with the legacy top-level `sort_column`/`sort_asc` are migrated automatically to the Branches and Remotes per-view fields (#settings, #sort, #tags, #worktrees)

## 2026-06-29

### Fixes
- Push is now available for any local branch when a remote is configured, not just tracked branches; uses `--set-upstream` to create the tracking ref automatically (#branches, #push)
- Remote branches list now updates immediately after "delete local + remote" without requiring a fetch (#ui, #remotes)
- Results and Confirm modals now resize dynamically to fit content — long messages and key hints no longer cut off (#ui, #modal)
- Remote branches now inherit squash-merge status from local branch detection — squash-merged branches no longer show as unmerged in the Remotes view (#ui, #remotes, #merge-detection)

## 2026-06-25

### Fixes
- Refresh branch metadata after branch actions (#branches)

## 2026-06-23

### Fixes
- Diagnostics: cache-accuracy audit now verifies every local branch (running the real squash check per non-reachable branch) instead of only cached ones, and reports verified/skipped counts with reasons rather than a misleading "verified" tally (#diagnostics, #cache)
- Filter modal: keep open on selection; add ESC hint; rename section to Merge Status (#ui, #filter)
- Rename merge filter syntax `status:` to `merge:` (e.g. `merge:merged`) for consistency with the renamed Merge column (#ui, #filter)
- Add abbreviated versions for Merge and Status columns (responsive like Age); rename merge column to "Merge" for consistency across views (#ui, #columns, #responsive)
- Change Remote column in Branches view to show indicator symbol instead of branch name (#ui, #columns)
- Make Path column have priority over Branch column in Worktrees view (#ui, #columns, #worktrees)

### Tasks
- Add Diagnostics modal (F2) with cache-accuracy verification (#diagnostics, #cache)

## 2026-06-09

### Tasks
- Remotes view: widen Age when space allows and fall back to compact ages when the resolved cell is too narrow

## 2026-06-08

### Tasks
- Worktrees view: Path column shows the end of the path, and Branch gets more room with left-truncated ellipsis display when too narrow

## 2026-06-06

### Tasks
- Extract generic confirm_selected helper (collect_targets + open_confirm) (#refactor, #dryness)
- Reduce active-view sort dispatch repetition via generic list_state helpers (#refactor, #dryness)
- Extract branch-like summary logic from status bar (#refactor, #dryness)
- Extract shared cell renderers (age, status, ahead/behind, PR) (#refactor, #dryness)
- Extract shared filter token groups (#refactor, #dryness)
- Extract shared column comparators and builders (#refactor, #dryness)

## 2026-04-04

### Fixes
- Sort column cycling skips wrong columns and indicator is misaligned (#sorting, #bug)
- Settings: default sort column and sort direction cannot be changed (#settings, #bug)
- Add [base] tag and current branch indicator to branch names (#ui, #branches)

### Tasks
- Make all columns sortable (#sorting, #columns)
- Base branch must always be first in local and remote branch lists (#ui, #sorting)

## 2026-04-03

### Fixes
- Age column: full vs abbreviated format based on terminal width (#ui, #columns)
- Remote view: match original app layout and features (#ui, #remote)
- Tab then Shift-Tab does not return to previous view (#navigation, #bug)
- Add merged/squashed counts to status bar (#ui, #statusbar)
- Status column must show full merge status text (#ui, #columns)

### Tasks
- Add PR column to branch list views (#ui, #columns)
