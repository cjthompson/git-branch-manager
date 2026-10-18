use crate::git::fuzzy_match;
use crate::types::SquashConfidence;
use std::path::Path;
use std::process::Command;
use tracing::instrument;

/// Detect if a branch was squash-merged into the base branch using git CLI.
/// Uses commit-tree + cherry to check if the branch's tree content already exists in base.
///
/// `merge_base` is the branch's already-known merge base with `base_branch` (the
/// value computed once from the in-memory reachable set in `fill_merge_base_commits`).
/// When `Some`, it is used directly and the per-call `git merge-base` subprocess is
/// skipped entirely — this avoids re-walking history for every candidate, which is
/// catastrophic on branches whose merge base is far back or absent (disjoint
/// histories force `git merge-base` to walk the full graph). When `None`, we fall
/// back to computing it via `git merge-base` (used by the remote path, which does
/// not precompute merge bases).
#[instrument(skip(repo_path))]
pub fn is_squash_merged(
    repo_path: &Path,
    base_branch: &str,
    branch_name: &str,
    commit_hash: Option<&str>,
    merge_base: Option<&str>,
) -> bool {
    let git = |args: &[&str]| -> Option<String> {
        let out = Command::new("git")
            .args(args)
            .current_dir(repo_path)
            .stdin(std::process::Stdio::null())
            .output()
            .ok()?;
        if out.status.success() {
            Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
        } else {
            None
        }
    };

    let branchish = commit_hash.unwrap_or(branch_name);

    // Step 1: find merge-base. Prefer the precomputed value; only shell out when absent.
    let ancestor = match merge_base {
        Some(mb) if !mb.is_empty() => mb.to_string(),
        _ => match git(&["merge-base", base_branch, branchish]) {
            Some(a) if !a.is_empty() => a,
            _ => return false,
        },
    };

    // Step 2: create temp commit-tree
    let tree_spec = format!("{branchish}^{{tree}}");
    let temp_commit = match git(&["commit-tree", &tree_spec, "-p", &ancestor, "-m", "_"]) {
        Some(c) if !c.is_empty() => c,
        _ => return false,
    };

    // Step 3: cherry check
    match git(&["cherry", base_branch, &temp_commit]) {
        Some(result) => result.starts_with('-'),
        None => false,
    }
}

/// True when every commit unique to `branch_name` relative to `merge_base`
/// has already landed in `base_branch` via individual cherry-picks.
///
/// Unlike `is_squash_merged`, this needs no synthetic `commit-tree`: `git
/// cherry <base> <branchish> <limit>` already walks every real commit between
/// `limit` (exclusive) and `branchish`, prefixing each line with `-` (its
/// patch-id is already reachable from `base_branch`) or `+` (it is not).
/// Every non-empty line must start with `-` for the branch to count as fully
/// cherry-picked; a single `+` (or any git failure) fails closed to `false`,
/// mirroring `is_squash_merged`'s fail-closed semantics.
#[instrument(skip(repo_path))]
pub fn is_cherry_picked(
    repo_path: &Path,
    base_branch: &str,
    branch_name: &str,
    commit_hash: Option<&str>,
    merge_base: Option<&str>,
) -> bool {
    let git = |args: &[&str]| -> Option<String> {
        let out = Command::new("git")
            .args(args)
            .current_dir(repo_path)
            .stdin(std::process::Stdio::null())
            .output()
            .ok()?;
        if out.status.success() {
            Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
        } else {
            None
        }
    };

    let branchish = commit_hash.unwrap_or(branch_name);

    let ancestor = match merge_base {
        Some(mb) if !mb.is_empty() => mb.to_string(),
        _ => match git(&["merge-base", base_branch, branchish]) {
            Some(a) if !a.is_empty() => a,
            _ => return false,
        },
    };

    match git(&["cherry", base_branch, branchish, &ancestor]) {
        Some(result) => {
            for line in result.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if !line.starts_with('-') {
                    return false;
                }
            }
            // Empty output (no lines to evaluate) means there are no commits
            // unique to the branch in the (tip, merge-base] window — every
            // commit reachable from branchish via that window has its
            // patch-id already in upstream. Counts as fully cherry-picked.
            true
        }
        None => false,
    }
}

/// Confirm that replaying `branchish` onto `base_branch` produces base's tree.
/// Fail closed on conflicts, missing refs, unsupported git, or other failures.
fn merge_tree_confirms(repo_path: &Path, base_branch: &str, branchish: &str) -> bool {
    let git = |args: &[&str]| -> Option<String> {
        let out = Command::new("git")
            .args(args)
            .current_dir(repo_path)
            .stdin(std::process::Stdio::null())
            .output()
            .ok()?;
        if out.status.success() {
            Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
        } else {
            None
        }
    };

    let base_tree = match git(&["rev-parse", &format!("{base_branch}^{{tree}}")]) {
        Some(t) if !t.is_empty() => t,
        _ => return false,
    };
    let written_tree = match git(&["merge-tree", "--write-tree", base_branch, branchish]) {
        Some(t) if !t.is_empty() => t,
        _ => return false,
    };
    written_tree == base_tree
}

/// Compute a raw diff compatible with `fuzzy_match::score`.
fn compute_diff(repo_path: &Path, merge_base: &str, refish: &str) -> Option<Vec<u8>> {
    let out = Command::new("git")
        .current_dir(repo_path)
        .args([
            "diff",
            "--binary",
            "--full-index",
            "--no-ext-diff",
            "--no-textconv",
            merge_base,
            refish,
            "--",
        ])
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() || out.stdout.is_empty() {
        return None;
    }
    Some(out.stdout)
}

/// Return a confidence classification for content that is likely already
/// represented in the base branch when exact patch-id matching is absent.
#[instrument(skip(repo_path))]
pub fn likely_squash_merged(
    repo_path: &Path,
    base_branch: &str,
    branch_name: &str,
    commit_hash: Option<&str>,
    merge_base: Option<&str>,
) -> Option<SquashConfidence> {
    let git = |args: &[&str]| -> Option<String> {
        let out = Command::new("git")
            .args(args)
            .current_dir(repo_path)
            .stdin(std::process::Stdio::null())
            .output()
            .ok()?;
        if out.status.success() {
            Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
        } else {
            None
        }
    };

    let branchish = commit_hash.unwrap_or(branch_name);
    let ancestor = match merge_base {
        Some(mb) if !mb.is_empty() => {
            match git(&["rev-parse", "--verify", &format!("{mb}^{{commit}}")]) {
                Some(a) if !a.is_empty() => a,
                _ => return None,
            }
        }
        _ => match git(&["merge-base", base_branch, branchish]) {
            Some(a) if !a.is_empty() => a,
            _ => return None,
        },
    };

    if merge_tree_confirms(repo_path, base_branch, branchish) {
        return Some(SquashConfidence::MergeTreeConfirmed);
    }

    let branch_diff = compute_diff(repo_path, &ancestor, branchish)?;
    let base_diff = compute_diff(repo_path, &ancestor, base_branch)?;
    let fscore = fuzzy_match::score(&branch_diff, &base_diff)?;
    let similarity_percent = fuzzy_match::classify(&fscore)?;
    Some(SquashConfidence::FuzzyMatch { similarity_percent })
}
