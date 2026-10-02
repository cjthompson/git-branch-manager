# Primary-branch code check repair implementation plan

> For agentic workers: use the project-tasks scout/executor workflow. The parent owns version changes and acceptance.

**Goal:** Complete task #039's Actions-pane check for selected committed ref changes, using the ahead/longer/newer candidate among the configured primary branch's local and origin refs. Report historical evidence explicitly rather than asserting current retention.

**Architecture:** Keep backend comparison in `git/merge_detection.rs`, results in `types.rs`, request lifecycle in `app.rs`, and wording in `ui/info_modal.rs`. Use the source repository for history metadata and an isolated object-backed repository for content comparison. Temporary merge blobs stay in a memory backend; a synthetic attributes index makes comparisons independent of local merge rules. Preserve existing automatic squash/cherry detection.

**Tech stack:** Rust, git2 0.21, Ratatui, existing background mpsc channels.

## Contract and approach

The original #002 and successor #039 both say "anywhere" but do not define current retention. The user clarified that primary selection should prefer whichever local or origin candidate is longest/newest. The result will explicitly identify historical evidence and will not claim historical matches prove current retention.

Historical evidence can include changes later reverted. Compare the selected ref's aggregate change relative to its single shared ancestor with the chosen primary snapshot. A primary-history snapshot matches when applying that aggregate change causes no conflict and adds no changes to the snapshot. A nonmatch cannot establish absence of semantically rewritten content.

For a current-tip interpretation, ancestry cannot prove retained content, and an ancestral selected ref's merge base is itself. Current retention would need an explicit source baseline or an honest unknown outcome rather than a guessed feature boundary.

Ancestry alone is fast but misses manual integration; patch IDs also miss combined or split integration and can ignore source merge resolutions. Use ancestry followed by content comparison. Do not infer a squash or cherry-pick mechanism from content evidence.

## Backend and domain result

- [x] Resolve exactly refs/heads/<configured-base> and refs/remotes/origin/<configured-base>, snapshotting both canonical refs/OIDs. Prefer a descendant over its ancestor. If divergent, prefer greater reachable commit count, then newer tip committer timestamp, then local on an exact tie. Use whichever candidate exists; propagate broken-ref errors. Ignore same-name tags and check cancellation during long walks.
- [x] Carry the chosen primary canonical ref/OID through successful and failed content checks and display the evidence in the result modal. Subsequent checks use its immutable OID, never re-resolve its name.
- [x] Capture the old API's manual/combined-integration failure and unlanded merge-resolution false positive before replacing the backend. Add coverage for split integration, historical matches after revert, partial integration, invalid refs, and namespace collisions.
- [x] Replace the feature-local result with `Merged`, `ContentEquivalent { commit_oid: String }`, and `NotFound`; derive Debug, Clone, PartialEq, Eq.
- [x] Change `check_ref_code_in_primary` to accept `repo`, `&ResolvedPrimaryRef`, `selected_ref`, and optional `&AtomicBool`; remove its unused filesystem path argument.
- [x] Resolve primary with the candidate selector above and selected with `revparse_single(selected_ref)?.peel_to_commit()`; retain immutable OIDs.
- [x] Check cancellation before doing work. Return `Merged` on equality or selected ancestry. Require exactly one `merge_bases` entry otherwise and report contextual errors for missing/ambiguous shared history.
- [x] Walk all reachable primary commits with TOPOLOGICAL | TIME sorting. Load ancestor and selected trees once; skip duplicate historical tree OIDs.
- [x] Compare each historical snapshot in the isolated comparison repository without touching the actual index or object store. Use a read-only object-directory alternate, memory-only writes, isolated config, and a synthetic text-attributes index. Recreate its blob after each memory-backend reset. Keep history traversal on the source repository to preserve shallow boundaries. The comparison is:

```rust
let merged = comparison_repo.merge_trees(&ancestor_tree, &history_tree, &selected_tree, None)?;
if !merged.has_conflicts() {
    let mut options = git2::DiffOptions::new();
    options.ignore_filemode(false).ignore_submodules(false);
    let diff = comparison_repo.diff_tree_to_index(
        Some(&history_tree), Some(&merged), Some(&mut options),
    )?;
    if diff.deltas().len() == 0 {
        return Ok(PrimaryBranchCodeMatch::ContentEquivalent {
            commit_oid: history_commit.id().to_string(),
        });
    }
}
```

- [x] Check cancellation on every iteration, propagate traversal/merge/diff errors with context, and return NotFound after exhausting available snapshots. Keep existing squash/cherry helper behavior unchanged.

## UI and lifecycle

- [x] Prefix only the Remotes code-check target with `refs/remotes/`; keep RemoteBranchInfo's abbreviated full_ref contract and Graph navigation unchanged.
- [x] Start the check through the existing cancelable Executing overlay and cancellation flag. Reset result-copy and scroll state, set return_view, and keep work off the UI thread.
- [x] Drop the dedicated result receiver on Escape; canceled/late results must not replace subsequent overlays. Surface unexpected receiver disconnects rather than leaving progress stuck.
- [x] Track scanner receiver and its cancellation token together in a feature-owned request object. Compare token identity with the active Executing cancellation token before presenting results or clearing shared state: job failures can replace a scanner overlay, then the user can start another operation. A late scanner result must not replace that new Executing overlay or clear its token. Preserve full backend error context in the result.
- [x] Show ancestry as historical reachability, content matches with a matched commit ID and explanation that the ref need not be merged, NotFound as conservative non-verification, and errors as failed checks. Keep result modals action-free.
- [x] Add independent app tests for canonical menus (including detached worktrees), dispatch, success/error delivery, cancellation/late results, and scroll/copy state reset. Add outcome wording coverage in info_modal.

## Integration coverage and documentation

- [x] Migrate existing canonical-ref coverage to new classifications, retain annotated tag/detached OID cases, and force nonidentical cherry-pick OIDs with fixed differing commit metadata or primary divergence.
- [x] Verify actual HEAD, refs, index bytes, and dirty tracked/untracked files remain unchanged. Cover preset cancellation, binary content and file modes. Inspect rename behavior and document conservative rewritten-content limits.
- [x] Document Actions availability, configured local primary history, historical inclusion versus current retention, conservative nonmatches, and committed-only worktree checks in README.

## Validation and acceptance

- [x] Run focused integration, binary app, and library modal regressions; format changed Rust files and check formatting.
- [x] Parent reviews the full feature diff and caller contracts, runs the full suite, cargo clippy, and required cargo build. Any baseline failures are reported separately with live main evidence.
- [x] Independent read-only verifier confirms requirements and correctness; address findings before presenting acceptance.
- [x] Leave Cargo.toml/Cargo.lock development-version edits local and out of commits. Leave task #039 in_progress until user acceptance; acceptance governs commit, completion record, and changelog update.

## Verification evidence

- Before backend replacement, manual integration with unrelated changes returned `Absent`, and unlanded source merge-resolution content falsely returned `CherryPickedEquivalent`. Both regressions were observed against the previous checker.
- The first seven content-check integration regressions pass after the replacement, including manual/split integration, partial integration, historical matches after revert, canonical refs, ambiguous source merge history, and repository state preservation.
- `cargo test --test primary_ref --test primary_code_content`: 14 tests pass. Covers local/origin selection, ref collisions and broken refs, snapshot movement, binary content, executable modes, rename, errors/cancellation, and a shallow boundary with its parent object absent.
- Live current-main baseline: full suite fails at `modal_keyboard_transition_preserves_single_target_recovery_across_resize`; baseline Clippy succeeds with existing warnings, and baseline formatting has existing differences. Final feature validation and independent static review are complete.

- Final validation: 464 library tests pass; bounded binary rerun has 105 passes and only the known main modal-recovery failure. Integration targets have 183 passes, one existing ignored test, and the unchanged 500-commit cap test filtered after passing on main; modal-shell 27, content-edge 5, and primary-ref 9 tests pass. Clippy succeeds with the same baseline warnings; cargo build succeeds at 0.12.4-dev5. Changed backend/types/modal and new test files pass rustfmt; git diff --check passes.
- Independent review has no remaining findings. Initial extra graph failures under concurrent test load disappear in the bounded final rerun. The Checkout position assertion was updated to select the named action.
- At the review checkpoint, no commit, merge, push, or task completion had occurred. Development-version edits were local and unstaged.

- [x] User accepted the result before committing, squash-merging, and completing task #039.
