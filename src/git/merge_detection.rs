use crate::types::{BranchInfo, MergeStatus};
use git2::Repository;
use std::collections::HashSet;
use std::path::Path;
use std::process::Command;
use tracing::{field, info_span, instrument, Span};

/// Detect which branches have been regular-merged into the base branch using git2.
/// Modifies branch merge_status in place from Unmerged to Merged where applicable.
#[instrument(
    skip(repo, branches),
    fields(
        base_branch,
        branch_count = branches.len(),
        base_oid = field::Empty,
        base_lookup_result = field::Empty,
        candidate_count = field::Empty,
        checked_count = field::Empty,
        skipped_base_count = field::Empty,
        skipped_current_count = field::Empty,
        find_branch_error_count = field::Empty,
        missing_target_count = field::Empty,
        merged_count = field::Empty,
        unmerged_count = field::Empty,
    )
)]
/// Returns the set of all commit OIDs reachable from base, so callers can reuse it
/// for merge-base computation without a second traversal.
pub fn detect_merged_branches(
    repo: &Repository,
    base_branch: &str,
    branches: &mut [BranchInfo],
) -> anyhow::Result<HashSet<git2::Oid>> {
    let span = Span::current();
    let base_branch_ref = match info_span!("detect_merged_branches_base_lookup", base_branch)
        .in_scope(|| repo.find_branch(base_branch, git2::BranchType::Local))
    {
        Ok(branch) => branch,
        Err(err) => {
            span.record("base_lookup_result", "find_branch_error");
            return Err(err.into());
        }
    };
    let base_ref = match base_branch_ref.get().target() {
        Some(oid) => {
            let base_oid = oid.to_string();
            span.record("base_oid", base_oid.as_str());
            span.record("base_lookup_result", "success");
            oid
        }
        None => {
            span.record("base_lookup_result", "missing_target");
            return Err(anyhow::anyhow!("base branch has no target"));
        }
    };

    let candidates: Vec<(usize, git2::Oid)> = branches
        .iter()
        .enumerate()
        .filter(|(_, b)| !b.is_base && !b.is_current)
        .filter_map(|(i, b)| {
            repo.find_branch(&b.name, git2::BranchType::Local)
                .and_then(|br| {
                    br.get()
                        .target()
                        .ok_or_else(|| git2::Error::from_str("no target"))
                })
                .ok()
                .map(|oid| (i, oid))
        })
        .collect();

    let skipped_base_count = branches.iter().filter(|b| b.is_base).count();
    let skipped_current_count = branches.iter().filter(|b| b.is_current).count();
    let candidate_count = branches
        .iter()
        .filter(|b| !b.is_base && !b.is_current)
        .count();
    let find_branch_error_count = candidate_count.saturating_sub(candidates.len());
    let missing_target_count = 0usize;

    span.record("candidate_count", candidate_count);
    span.record("skipped_base_count", skipped_base_count);
    span.record("skipped_current_count", skipped_current_count);
    span.record("find_branch_error_count", find_branch_error_count);
    span.record("missing_target_count", missing_target_count);

    if candidates.is_empty() {
        span.record("checked_count", 0u64);
        span.record("merged_count", 0u64);
        span.record("unmerged_count", 0u64);
        return Ok(HashSet::new());
    }

    // Build a set of all commits reachable from base in one revwalk pass.
    // This replaces N calls to graph_descendant_of (each O(history)) with
    // one O(history) traversal + N O(1) hash lookups.
    let reachable = info_span!(
        "detect_merged_revwalk",
        base_oid = %base_ref,
        reachable_count = field::Empty,
    )
    .in_scope(|| -> anyhow::Result<HashSet<git2::Oid>> {
        let revwalk_span = Span::current();
        let mut revwalk = repo.revwalk()?;
        revwalk.set_sorting(git2::Sort::NONE)?;
        revwalk.push(base_ref)?;
        let mut set = HashSet::new();
        for oid_result in &mut revwalk {
            if let Ok(oid) = oid_result {
                set.insert(oid);
            }
        }
        revwalk_span.record("reachable_count", set.len() as u64);
        Ok(set)
    })?;

    let mut merged_count = 0usize;
    let mut unmerged_count = 0usize;
    for (i, branch_oid) in &candidates {
        if reachable.contains(branch_oid) {
            merged_count += 1;
            branches[*i].merge_status = MergeStatus::Merged;
        } else {
            unmerged_count += 1;
        }
    }
    span.record("checked_count", candidates.len() as u64);
    span.record("merged_count", merged_count);
    span.record("unmerged_count", unmerged_count);
    Ok(reachable)
}

/// Detect if a branch was squash-merged into the base branch using git CLI.
/// Uses commit-tree + cherry to check if the branch's tree content already exists in base.
#[instrument(skip(repo_path))]
pub fn is_squash_merged(
    repo_path: &Path,
    base_branch: &str,
    branch_name: &str,
    commit_hash: Option<&str>,
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

    // Step 1: find merge-base
    let ancestor = match git(&["merge-base", base_branch, branch_name]) {
        Some(a) if !a.is_empty() => a,
        _ => return false,
    };

    // Step 2: create temp commit-tree
    let tree_spec = if let Some(hash) = commit_hash {
        format!("{hash}^{{tree}}")
    } else {
        format!("{branch_name}^{{tree}}")
    };
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
