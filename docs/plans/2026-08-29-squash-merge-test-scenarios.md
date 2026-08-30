# Squash-Merge Detection: Test Scenario Plan

**Status:** Draft — test-case enumeration and one new algorithm option

**Date:** 2026-08-29

**Parent doc:** [`2026-08-29-squash-merge-algorithm-options.md`](2026-08-29-squash-merge-algorithm-options.md)

**Scope:** Concrete, buildable test scenarios for the Graph possible-squash marker
(Algorithm A, `git/merge_detection.rs`/`git/graph.rs`) and the Branches/Remotes
squash status (Algorithm B, `git/merge_detection.rs::is_squash_merged` +
`git/squash_loader.rs`), plus a new fuzzy/possible-match tier (Option 6) needed
to cover near-exact-but-not-identical content.

This doc is grounded in research findings (git docs, `git-delete-squashed`,
`git-trim`, `gh-purge`, `git-machete`, GitHub/GitLab/Bitbucket API behavior,
and empirical tests of `git patch-id --stable` on empty and whitespace-only
diffs) gathered while investigating the parent plan doc's options. Two gaps
identified during that research directly motivate this doc:

1. Neither existing detector has a test case for a squash merge that required
   **conflict resolution** — where the landed squash commit is not a clean
   concatenation of the branch's own commits, so its content differs from the
   branch's own aggregate diff even though the branch is, in the product
   sense, fully landed.
2. Both existing detectors require **exact** patch-id equality. There is
   currently no tier between "exact match" and "no match" for base content
   that is *almost* identical to the branch's changes (e.g. a squash commit
   plus one trivial edit, or the reverse). This doc adds **Option 6** to
   close that gap.

## Test infrastructure

All scenarios use the existing `tests/integration.rs` pattern:

- `setup_test_repo()` → `(TempDir, Repository)`, with all git operations run
  via explicit `current_dir()` (never process-global `set_current_dir`), so
  scenarios can run in parallel per the existing test-suite convention.
- Each scenario below lists the exact `git` construction steps as a recipe;
  the implementation task should translate each into a `std::process::Command`
  sequence (or `git2` calls where equivalent — e.g. plain commits — exist)
  inside a dedicated `#[test]` function.
- Each scenario states an **expected classification** per detector. Where the
  expected result is a **known, currently-accepted limitation** (a documented
  false negative/positive), the test should still assert today's actual
  behavior (not `#[ignore]` it) — these are characterization tests that pin
  down current behavior so a future algorithm change is a deliberate,
  visible diff in test output, not a silent regression. Only scenarios that
  require Option 6 (not yet implemented) should be written against the new
  API surface as part of the same implementation task, since Option 6 is in
  scope for this plan (see "Option 6" below) rather than deferred.
- Naming convention: `test_squash_<category>_<specific_case>`, matching the
  existing `test_squash` test-name grep pattern used in this repo's `cargo
  test test_squash` command.

## New: Option 6 — Fuzzy/similarity-based possible-match tier

Algorithms A and B are both **exact**-match: `git patch-id --stable` equality
(A) and `git cherry`'s equivalent internal patch-id comparison (B). Per the
parent doc's product goal, this is correct for claim 2, "possible squash at
commit X" — but there is real-world content that is *almost* the same and
today produces no signal at all rather than a weaker "potential" signal.
Concretely: a squash commit with one extra trivial line changed after
resolving a conflict, or a branch that's 95% landed with one follow-up commit
still local.

### Algorithm sketch

For each `(branch aggregate diff, candidate base commit diff)` pair already
being compared under Algorithm A or B:

1. **Cheap pre-filter.** Compare `git diff --stat`-derived file sets and
   total added/removed line counts. Skip full comparison unless the file sets
   overlap by at least some threshold (e.g. ≥50% Jaccard on touched-file
   paths) and the added/removed line counts are within an order of magnitude
   of each other. This bounds the cost of the O(n×m) sweep the existing
   detectors already perform — fuzzy scoring is only run on pairs that
   already look plausible.
2. **Line-level similarity.** Parse both unified diffs into normalized
   added-line and removed-line multisets (trim, drop pure-blank lines,
   respect `--stable`-style whitespace insensitivity so this tier isn't
   *stricter* than the exact one it extends). Compute a Jaccard similarity:
   `|intersection| / |union|` over the combined added+removed line sets.
3. **Classification:**
   - `similarity == 1.0` and file sets identical → this is just Algorithm
     A/B's existing exact case; Option 6 defers to it (no duplicate signal).
   - `similarity >= 0.85` (tunable; needs the test matrix below to pick a
     real threshold) → **possible squash merge (fuzzy)**, reported with the
     numeric similarity score so the UI can distinguish it from an exact
     possible-squash marker (e.g. a dimmer marker or a "~92% match" label).
   - Below threshold → no signal, same as today.
4. **Renames/binary files are excluded from the line-level set** and handled
   as a separate boolean "file set overlap" signal only — do not attempt
   line-level similarity on binary diffs.

This tier is explicitly **additive**: it never downgrades an existing exact
match, and it never claims "proven" or even unqualified "possible" — the UI
label must carry the fuzzy/approximate qualifier so users don't mistake a
95%-similar unrelated commit for a real match.

### Open design questions to resolve during implementation

- Exact similarity threshold — start with the test matrix below and pick a
  value that clears the "duplicate trivial commit" false-positive scenario
  (13) while catching the "near-exact match" scenarios (20a–20d).
- Whether the fuzzy tier runs in the same background pass as A/B or as a
  separate, lower-priority enrichment step (bounded worker, per the parent
  doc's "Loading and UI direction" section) given its higher cost.
- Whether Branches/Remotes (Algorithm B) surfaces a fuzzy status at all, or
  whether this tier is Graph-only (Algorithm A) since it needs a specific
  candidate commit to report a similarity score against.

## Scenario matrix

Each entry: **Setup** (git recipe), **Expect A** (Graph marker), **Expect B**
(Branches/Remotes status), **Expect Option 6** (fuzzy tier, where relevant).

### 1. Baseline: single-commit clean squash

**Setup:** Branch with one commit off `base`; squash the same diff onto
`base` as a single commit (simulate via `commit-tree` with the branch's tree
and `base` as parent, or literally `git merge --squash && git commit`).
**Expect A:** Candidate base commit flagged `is_possible_squash_merge`.
**Expect B:** `SquashMerged`.
**Expect Option 6:** N/A (exact match; Option 6 defers).

### 2. Baseline: multi-commit branch squashed into one base commit

**Setup:** Branch with 3+ commits touching multiple files off `base`; squash
all onto `base` as a single commit with the branch tip's tree.
**Expect A:** Flagged, matching the branch's full aggregate diff.
**Expect B:** `SquashMerged`.

### 3. Regular merge commit — must NOT be flagged as squash

**Setup:** Branch merged into `base` via a normal two-parent `git merge`
(no squash).
**Expect A:** Not flagged as possible-squash (this is a regular merge,
already excluded per the parent doc's "Strengths" list for Algorithm A).
**Expect B:** Not `SquashMerged` (regular-merge detection should classify
this separately; verify it doesn't fall through to the squash path).

### 4. Branch itself contains a merge commit, then is squash-merged

**Setup:** Branch has a merge commit within its own history (e.g. merged a
sub-branch into it) before being squash-merged into `base`.
**Expect A:** Flagged — the branch's aggregate diff (merge-base → tip) is
still a flat diff regardless of internal topology; patch-id comparison
should not care that the branch's own history has merges.
**Expect B:** `SquashMerged`, for the same reason.
**Note:** confirms Algorithm A/B operate on tree diffs, not commit topology,
inside the branch itself — worth an explicit assertion since it's easy to
assume merge commits anywhere break detection.

### 5. Partial landing via individual cherry-picks (not a full squash)

**Setup:** Branch has 3 commits; only 2 of the 3 are cherry-picked
individually onto `base` (not squashed, not all 3).
**Expect A:** Not flagged — the branch's full aggregate diff doesn't match
any single base commit's diff (only a subset landed).
**Expect B:** Not `SquashMerged` — same reason; `git cherry` on the
synthetic full-branch commit won't find an equivalent single patch.
**Note:** This is a real product gap (partial coverage isn't reported at
all) called out in the parent doc's Option 4 ("report partial coverage when
only part of a branch has landed"). This test documents the current
all-or-nothing behavior as a known limitation, not a bug to fix here.

### 6. Reordered commits before squash (patch-id order-insensitivity)

**Setup:** Branch has commits A, B (different files); squash lands as a
single commit. Separately construct the branch's own diff computation from
a hunk order that differs from the base commit's literal diff ordering
(e.g. via `-O<orderfile>` on one side) to confirm `--stable` order
insensitivity holds in this codebase's actual invocation, not just in
isolation (this was empirically confirmed for whitespace directly; order
insensitivity should get its own dedicated assertion since it's the more
surprising of the two `--stable` guarantees).
**Expect A / B:** Still flagged/`SquashMerged` — order must not matter.

### 7. Rebased branch, then squash-merged

**Setup:** Branch created off `base`; `base` advances; branch is rebased
onto new `base` tip; then squashed onto `base`.
**Expect A / B:** Flagged/`SquashMerged`, using the *post-rebase*
merge-base. Confirms merge-base resolution uses current history, not a
stale ancestor.

### 8. Overlapping changes with conflict resolution during squash

This is the first new category requested for this plan.

**Setup (8a — resolution changes only the branch's own lines):** `base` and
`branch` both modify the same file but different lines, no true conflict at
the git level, but the squash author also fixed up a small adjacent
formatting issue while resolving. Squash commit's diff against `base`
therefore contains the branch's real changes **plus** a few extra
resolution-only lines not present in the branch's own diff.
**Expect A:** Not flagged under exact patch-id match (the added resolution
lines break equality) — document as a known false negative for Algorithm A.
**Expect B:** Not `SquashMerged`, same reason.
**Expect Option 6:** Should surface as **possible squash merge (fuzzy)**
given high line-level similarity and full file-set overlap — this is the
primary scenario Option 6 exists to catch. Assert the computed similarity
score lands above the chosen threshold.

**Setup (8b — true conflicting hunks, manually resolved):** `base` and
`branch` modify the *same* lines. Simulate the squash by manually
constructing the resolved content (what a human would produce resolving the
conflict) as the squash commit's tree, rather than a mechanical merge.
**Expect A/B:** Not flagged (false negative, same as 8a but with a lower
expected similarity since more lines differ).
**Expect Option 6:** Depends on how much the resolution changed relative to
the branch's own diff — assert whichever side of the threshold this
concrete recipe lands on, and use it to help calibrate the threshold rather
than assuming a specific classification in advance.

**Setup (8c — `git merge-tree` confirmation check):** Using the same setup
as 8a/8b, run `git merge-tree --write-tree base branch` (the *original*,
un-squashed branch) against **current** `base` (which already contains the
resolved squash) and compare the resulting tree to `base`'s own tree.
**Expect:** This is a test of the parent doc's Option 3 `merge-tree`
confirmation signal, not Option 6. Record whether the merge-tree result
equals `base`'s tree (i.e., merging the stale branch in today produces no
further changes) — this may succeed even when exact patch-id match (8a/8b)
fails, since `merge-tree` re-simulates the merge against current base rather
than comparing static diffs. This scenario is included here because it's the
conflict-resolution case where the two approaches (patch-id vs merge-tree)
are most likely to disagree, matching the parent doc's "A true, B false"
disagreement-visibility principle.

### 9. Binary file changes in the squash

**Setup:** Branch adds/modifies a binary file (e.g. a small fixed-content
blob); squash lands the same binary change.
**Expect A/B:** Flagged/`SquashMerged` if patch-id handles `--binary`
consistently (per this codebase's existing use of `--binary --full-index`).
This test exists because the correctness envelope research found binary-diff
patch-id behavior **undocumented at the git-scm level** — treat this as an
empirical determination, not an assumed-safe path, and record the actual
result rather than only asserting the desired one.

### 10. File renames and mode changes

**Setup (10a):** Branch renames a file with no content change; squash lands
the rename.
**Setup (10b):** Branch renames a file *and* changes its content; squash
lands both.
**Setup (10c):** Branch changes a file's executable bit only.
**Expect A/B:** Verify against this codebase's actual diff-option flags
(rename detection on/off, `-M`/`-C` presence). Research found git disables
rename detection *inside* `git patch-id`'s own internal computation
regardless of caller config — but this codebase computes `git diff` first
and pipes to `git patch-id`, so the `git diff` step's own rename-detection
config (default-on since Git 2.9) still applies to the diff text itself
before patch-id ever sees it. Assert actual behavior for all three cases;
do not assume renames are handled without a direct test.

### 11. Whitespace-only / line-ending-only changes

**Setup:** Branch changes only trailing whitespace / line endings on
otherwise-identical content; squash lands the identical whitespace change.
**Expect A/B:** Flagged/`SquashMerged` — confirmed empirically in this
research pass that `git patch-id --stable` ignores whitespace differences
entirely. Also add a **negative** variant: base's independent commit made
only an *unrelated* whitespace change (not the branch's change) — must NOT
be flagged, since both would otherwise produce a degenerate near-empty diff.

### 12. Empty / net-zero branch

**Setup:** Branch has commits that fully cancel out (e.g. adds then removes
the same content), so branch-tip tree equals merge-base tree.
**Expect A/B:** This is the scenario flagged by empirical testing in this
research pass: `git patch-id --stable` on an empty diff produces **no
output at all**, not a defined hash. Construct a *second*, unrelated
empty-diff base commit in the same test and confirm whether the current
implementation's comparison logic treats two empty patch-ids as equal
(a real false-positive risk) or explicitly special-cases empty diffs as
non-matching. Whichever the current code does, assert it explicitly — this
is exactly the kind of silent behavior the parent doc's cache section warns
about ("failed, ambiguous, and empty-patch results should not be treated as
durable positive squash evidence").

### 13. Reverted branch

**Setup:** Branch adds a commit, then a second commit that reverts it
(net-zero from `base`'s perspective, but the commits still exist).
**Expect A/B:** Should not be flagged as squash-merged into any specific
base commit content-wise, but is trivially "already integrated" (branch
adds no content) — this exercises the parent doc's `already_integrated`
OR-claim vs. `possible_squash(commit)` AND-claim distinction directly. Two
separate assertions: (a) branch is safe-to-delete / already-integrated, (b)
no specific base commit is flagged as *the* squash point, since none
exists.

### 14. Duplicate / independently recreated patch (false-positive risk)

**Setup:** Branch makes a one-line change. Independently, a *later, unrelated*
commit on `base` makes the exact same one-line textual change to the same
file (simulating two developers writing the identical fix).
**Expect A/B:** Will be flagged/`SquashMerged` — this is the **documented,
accepted false positive** per the correctness-envelope research (content
equivalence, not historical provenance). Assert it happens, with a comment
citing why: this is inherent to patch-id-based detection and not a bug.
Use this same fixture to calibrate Option 6's threshold: confirm this
trivial one-line duplicate produces `similarity == 1.0` for that single
file, and think about whether file-set-overlap gating (only the one file,
tiny diff) should suppress a *fuzzy* (as opposed to exact) label for very
small diffs where coincidental collision is more likely — record the
outcome rather than assuming a specific mitigation is needed.

### 15. Multiple merge bases / criss-cross history

**Setup:** Construct a criss-cross merge: two branches that have merged each
other, so `base` and `branch` have two co-equal merge bases. Run
`git merge-base --all base branch` to confirm more than one base exists in
the fixture before proceeding.
**Expect:** Document which merge-base `git merge-base` (no `--all`) actually
picks in this codebase's invocation, and whether that choice changes the
aggregate-diff computation's correctness for this specific fixture. Per the
research, this selection is officially unspecified — so the test's job is to
pin down current behavior (which base gets used, and whether the resulting
classification is still correct for this fixture) rather than to assert a
"correct" choice that git itself doesn't guarantee.

### 16. Shallow / out-of-window history (Graph `max_count` boundary)

**Setup:** Construct a base history longer than the Graph's configured
`max_count`, with the actual squash-landing commit older than the window.
**Expect A:** Not flagged — Algorithm A fails closed outside its displayed
window, per the parent doc's documented "Strengths." Assert no false
positive and no panic/error, just silent non-detection.
**Expect B:** Should still detect it (B is documented as normally searching
beyond the Graph display window) — this is the concrete regression test for
the parent doc's "A false, B true" disagreement case.

### 17. Local base vs. remote base divergence

**Setup:** Branch is squash-merged into `origin/main`, but the local `main`
has not fetched/merged that commit yet (local `main` is behind).
**Expect:** Detection against local `main` → not flagged. Detection against
`origin/main` (remote base) → flagged. This directly tests the "local
base versus remote base divergence" bullet in the parent doc's research
matrix and the loader's documented local+remote dual-base behavior.

### 18. Branch advances after being cached as squash-merged

**Setup:** Branch is squash-merged and detected as `SquashMerged`; branch
then gets a new commit on top (still locally, un-pushed and unmerged).
**Expect:** Re-running detection after the new commit should no longer
report `SquashMerged` for the branch's *current* tip, since its aggregate
diff has changed. This is the concrete regression test for the parent doc's
explicitly named cache limitation ("a branch that later advances can retain
stale status"). If the current cache does retain stale status, assert that
today, with a comment marking it as the known bug the parent doc already
flags — don't silently pass a wrong expectation.

### 19. Large history / many branch tips (performance characterization)

**Setup:** Programmatically generate a base history with a large number of
commits (e.g. 500+) and many diverged local branches (e.g. 50+), most
unrelated to any squash.
**Expect:** Not a correctness assertion — record wall-clock time and
subprocess count for Algorithm A and B against this fixture as a baseline.
Mark this test `#[ignore]` by default (expensive) and runnable explicitly,
consistent with how expensive fixtures are typically excluded from the
default `cargo test` run in this repo.

### 20. Near-exact match (Option 6 target scenarios)

**20a — squash + trivial follow-up folded together:** Branch's real changes
land in the squash commit, plus one small unrelated line (e.g. a version
bump) that was folded in during the squash but was never part of the
branch's own commits.
**Expect A/B:** Not flagged (exact match fails).
**Expect Option 6:** Flagged as possible squash (fuzzy) with high similarity.

**20b — squash omits a trivial branch change:** The squash commit landed
slightly *less* than the branch's full diff (e.g. the author dropped a
debug line during squash).
**Expect A/B:** Not flagged.
**Expect Option 6:** Flagged as fuzzy match; confirms the similarity
computation is symmetric (handles both superset and subset cases).

**20c — auto-formatter ran during squash:** Squash commit's diff contains
the branch's real change plus many trivial formatting-tool line changes
across the same files (e.g. a linter reformatted touched lines).
**Expect A/B:** Not flagged.
**Expect Option 6:** This is the hardest calibration case — high file
overlap, lower line-level similarity due to formatting noise. Use this
fixture specifically to decide whether the threshold needs a formatting-
aware normalization pass (e.g. re-run through the same normalization
`git patch-id --stable` already does for whitespace) before Jaccard
scoring, rather than assuming raw line comparison is sufficient.

**20d — coincidentally similar but unrelated commits (negative control):**
Two commits that touch the same file and have moderate line overlap by
coincidence, but are not the same logical change (different function,
same file, similar boilerplate).
**Expect Option 6:** Must NOT be flagged — this is the fuzzy tier's own
false-positive control. If it is flagged, the threshold is too low and needs
revisiting before this tier ships.

### 21. Initial Graph render latency while enrichment is deliberately delayed

**Setup:** Not a detection-correctness test — a UX/architecture test per the
parent doc's "Loading and UI direction" section. Simulate a slow squash
enrichment pass (e.g. inject an artificial delay in a test harness around
the annotation step) and assert the Graph's structural snapshot (DAG, refs,
commit summaries) is published and renderable *before* squash annotation
completes, not blocked on it.
**Note:** This scenario tests the parent doc's Follow-up project task ("Fix
Graph loading so structural rendering is not blocked by squash-merge
enrichment") rather than the detection algorithms themselves. Include it in
the same implementation task only if that decoupling work has landed by the
time this task runs; otherwise, record it as a currently-failing
characterization test with a comment pointing back to that follow-up task.

## Implementation task scope

A single follow-up task should:

1. Read this plan file in full.
2. Add one `#[test]` function per numbered scenario above (sub-scenarios
   like 8a/8b/8c and 20a–20d each get their own test) to `tests/integration.rs`,
   following the existing `setup_test_repo()` / explicit-`current_dir()`
   conventions.
3. Implement Option 6 (the fuzzy/similarity tier) as new code in
   `git/merge_detection.rs` (or a new sibling module, e.g.
   `git/fuzzy_match.rs`, if the scoring logic is substantial enough to
   warrant separation) sufficient to make the scenario-20 tests exercise
   real production code rather than being purely aspirational.
4. For scenarios documenting known, accepted limitations (5, 8a/8b under
   exact match, 14, 18 if unfixed, 15's unspecified-base-choice), assert
   **today's actual behavior**, with a comment citing the limitation and a
   back-reference to this plan doc section — do not leave them unimplemented
   or `#[ignore]`d.
5. Run `cargo test test_squash` and `cargo build` and report results,
   including the calibrated similarity threshold chosen for Option 6 and
   the reasoning (which scenario(s) it was tuned against).
