//! Pre-filter + Jaccard-similarity scoring for the Option 6 fuzzy/possible-match
//! tier. See `docs/plans/2026-08-29-squash-merge-test-scenarios.md` ("New:
//! Option 6 — Fuzzy/similarity-based possible-match tier").
//!
//! This module is deliberately Graph-only (see the implementation map that
//! accompanied this module's introduction): it scores an already-fetched
//! `(branch tip diff, candidate base commit diff)` pair — both raw `git diff
//! --binary --full-index --no-ext-diff --no-textconv` byte outputs — and
//! reports either a numeric similarity (for a "possible squash merge
//! (fuzzy)" label) or `None` (no signal, same as today).

use std::collections::HashSet;

/// Minimum Jaccard similarity (0.0-1.0) for a `possible squash merge (fuzzy)`
/// classification. Calibrated against the scenario-20 fixtures in
/// `tests/integration.rs` (see that file's `test_squash_scenario_20*` tests):
/// comfortably below the near-exact "trivial follow-up folded in"/"omitted
/// trivial change" cases (~0.85-0.9 observed) and comfortably above the
/// "coincidentally similar but unrelated" negative control (~0.3 observed).
pub const FUZZY_SIMILARITY_THRESHOLD: f32 = 0.75;

/// Minimum number of distinct changed-line tokens (the union of both diffs'
/// added+removed line sets) required before a fuzzy classification is
/// emitted. Guards against the coincidental-duplicate false positive in tiny
/// diffs, where `similarity == 1.0` (or very close to it) is common by
/// chance for a single shared one-line edit (scenario 14) but doesn't carry
/// the same evidentiary weight as a larger near-exact match (scenario 20a).
/// `classify` already defers `similarity >= 1.0` to the exact-match tier, so
/// this gate mainly matters for near-(but not exactly-)identical tiny diffs.
const MIN_UNION_SIZE_FOR_FUZZY: usize = 6;

/// Minimum ratio of touched-file overlap (Jaccard over file paths) required
/// before running the more expensive line-level comparison at all.
const FILE_OVERLAP_PREFILTER_THRESHOLD: f32 = 0.5;

/// Result of scoring one `(diff_a, diff_b)` pair. `union_size` is exposed
/// (beyond the plan doc's original sketch) so `classify` can gate on
/// absolute diff size, not just the relative Jaccard number — see
/// `MIN_UNION_SIZE_FOR_FUZZY`.
pub struct FuzzyScore {
    /// Raw 0.0-1.0 Jaccard similarity over the combined added+removed line sets.
    pub similarity: f32,
    /// Jaccard similarity over touched-file paths.
    pub file_overlap_ratio: f32,
    /// Size of the union of both diffs' normalized added+removed line-token sets.
    pub union_size: usize,
}

/// Extract the set of file paths touched by a `git diff` (via `diff --git
/// a/<path> b/<path>` headers). Used for the cheap file-set-overlap
/// pre-filter and for `FuzzyScore::file_overlap_ratio`.
fn touched_files(diff: &[u8]) -> HashSet<String> {
    let mut files = HashSet::new();
    for line in String::from_utf8_lossy(diff).lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            if let Some(index) = rest.find(" b/") {
                files.insert(rest[index + 3..].to_string());
            } else {
                files.insert(rest.to_string());
            }
        }
    }
    files
}

/// Parse a unified diff into normalized added-line and removed-line sets.
/// Lines are trimmed and blank lines dropped; content inside `GIT binary
/// patch` / `Binary files ... differ` sections is excluded entirely (binary
/// changes are handled only via the file-overlap signal, never line-level
/// similarity, per the plan doc). Rename-only diffs naturally contribute no
/// content lines (git emits `rename from`/`rename to` headers with no `+`/`-`
/// hunk lines when there's no content change), so no special-casing is
/// needed for the "renames are excluded" requirement beyond skipping the
/// `+++`/`---` file-header lines below.
fn added_removed_lines(diff: &[u8]) -> (HashSet<String>, HashSet<String>) {
    let mut added = HashSet::new();
    let mut removed = HashSet::new();
    let mut in_binary_section = false;
    for line in String::from_utf8_lossy(diff).lines() {
        if line.starts_with("diff --git ") {
            in_binary_section = false;
            continue;
        }
        if line.starts_with("GIT binary patch") || line.starts_with("Binary files ") {
            in_binary_section = true;
            continue;
        }
        if in_binary_section {
            continue;
        }
        if line.starts_with("+++ ") || line.starts_with("--- ") {
            continue;
        }
        if let Some(rest) = line.strip_prefix('+') {
            let trimmed = rest.trim();
            if !trimmed.is_empty() {
                added.insert(trimmed.to_string());
            }
        } else if let Some(rest) = line.strip_prefix('-') {
            let trimmed = rest.trim();
            if !trimmed.is_empty() {
                removed.insert(trimmed.to_string());
            }
        }
    }
    (added, removed)
}

/// Cheap pre-filter: compare touched-file sets and added/removed line counts
/// before running the full line-level comparison. Bounds the cost of the
/// O(n×m) sweep callers already perform (one call per candidate base commit
/// × displayed branch tip) by rejecting pairs that don't already look
/// plausible.
fn passes_prefilter(diff_a: &[u8], diff_b: &[u8]) -> bool {
    let files_a = touched_files(diff_a);
    let files_b = touched_files(diff_b);
    if files_a.is_empty() || files_b.is_empty() {
        return false;
    }
    let intersection = files_a.intersection(&files_b).count();
    let union = files_a.union(&files_b).count();
    if union == 0 || (intersection as f32 / union as f32) < FILE_OVERLAP_PREFILTER_THRESHOLD {
        return false;
    }

    let (added_a, removed_a) = added_removed_lines(diff_a);
    let (added_b, removed_b) = added_removed_lines(diff_b);
    let count_a = (added_a.len() + removed_a.len()).max(1);
    let count_b = (added_b.len() + removed_b.len()).max(1);
    let ratio = count_a as f32 / count_b as f32;
    let ratio = if ratio < 1.0 { 1.0 / ratio } else { ratio };
    ratio <= 10.0
}

/// Score a `(diff_a, diff_b)` pair. Returns `None` when the cheap pre-filter
/// rejects the pair (file sets don't overlap enough, or line counts differ
/// by more than an order of magnitude) — this is a "no signal" result, not a
/// zero similarity.
pub fn score(diff_a: &[u8], diff_b: &[u8]) -> Option<FuzzyScore> {
    if !passes_prefilter(diff_a, diff_b) {
        return None;
    }

    let files_a = touched_files(diff_a);
    let files_b = touched_files(diff_b);
    let file_union = files_a.union(&files_b).count();
    let file_overlap_ratio = if file_union == 0 {
        0.0
    } else {
        files_a.intersection(&files_b).count() as f32 / file_union as f32
    };

    let (added_a, removed_a) = added_removed_lines(diff_a);
    let (added_b, removed_b) = added_removed_lines(diff_b);
    // Tag added/removed so an added line in one diff never matches a removed
    // line with the same text in the other diff.
    let set_a: HashSet<String> = added_a
        .iter()
        .map(|line| format!("+{line}"))
        .chain(removed_a.iter().map(|line| format!("-{line}")))
        .collect();
    let set_b: HashSet<String> = added_b
        .iter()
        .map(|line| format!("+{line}"))
        .chain(removed_b.iter().map(|line| format!("-{line}")))
        .collect();

    let union_size = set_a.union(&set_b).count();
    if union_size == 0 {
        return None;
    }
    let intersection_size = set_a.intersection(&set_b).count();
    let similarity = intersection_size as f32 / union_size as f32;

    Some(FuzzyScore {
        similarity,
        file_overlap_ratio,
        union_size,
    })
}

/// Classify a [`FuzzyScore`] into a rounded 0-100 similarity percent, or
/// `None` if it doesn't clear the fuzzy tier. `similarity >= 1.0` (a
/// content-identical pair) always returns `None`: per the plan doc's
/// algorithm sketch, Option 6 is additive and defers to the exact-match tier
/// rather than duplicating its signal.
pub fn classify(score: &FuzzyScore) -> Option<u8> {
    if score.similarity >= 1.0 {
        return None;
    }
    if score.union_size < MIN_UNION_SIZE_FOR_FUZZY {
        return None;
    }
    if score.similarity < FUZZY_SIMILARITY_THRESHOLD {
        return None;
    }
    Some((score.similarity * 100.0).round().clamp(0.0, 100.0) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diff_for(paths_and_hunks: &[(&str, &str)]) -> Vec<u8> {
        let mut out = String::new();
        for (path, hunk) in paths_and_hunks {
            out.push_str(&format!("diff --git a/{path} b/{path}\n"));
            out.push_str("index 0000000..1111111 100644\n");
            out.push_str(&format!("--- a/{path}\n"));
            out.push_str(&format!("+++ b/{path}\n"));
            out.push_str(hunk);
        }
        out.into_bytes()
    }

    #[test]
    fn identical_diffs_score_as_1_0_and_are_not_classified() {
        let diff = diff_for(&[("f.txt", "@@ -1,2 +1,2 @@\n-a\n+b\n context\n")]);
        let fscore = score(&diff, &diff).expect("identical diffs should pass the prefilter");
        assert_eq!(fscore.similarity, 1.0);
        assert_eq!(classify(&fscore), None, "exact match defers to the exact tier");
    }

    #[test]
    fn unrelated_diffs_fail_the_prefilter() {
        let a = diff_for(&[("a.txt", "@@ -1 +1 @@\n-x\n+y\n")]);
        let b = diff_for(&[("totally-different.txt", "@@ -1 +1 @@\n-p\n+q\n")]);
        assert!(score(&a, &b).is_none());
    }

    #[test]
    fn binary_sections_are_excluded_from_line_level_comparison() {
        let mut diff = String::new();
        diff.push_str("diff --git a/bin.dat b/bin.dat\n");
        diff.push_str("index 0000000..1111111 100644\n");
        diff.push_str("GIT binary patch\n");
        diff.push_str("literal 4\n");
        diff.push_str("+abcd\n"); // would look like an added line if not excluded
        let (added, removed) = added_removed_lines(diff.as_bytes());
        assert!(added.is_empty());
        assert!(removed.is_empty());
    }
}
