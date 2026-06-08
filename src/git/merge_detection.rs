use crate::types::{BranchInfo, MergeStatus};
use git2::Repository;
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
        graph_error_count = field::Empty,
    )
)]
pub fn detect_merged_branches(
    repo: &Repository,
    base_branch: &str,
    branches: &mut [BranchInfo],
) -> anyhow::Result<()> {
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

    let mut candidate_count = 0usize;
    let mut checked_count = 0usize;
    let mut skipped_base_count = 0usize;
    let mut skipped_current_count = 0usize;
    let mut find_branch_error_count = 0usize;
    let mut missing_target_count = 0usize;
    let mut merged_count = 0usize;
    let mut unmerged_count = 0usize;
    let mut graph_error_count = 0usize;
    for branch in branches.iter_mut() {
        let branch_span = info_span!(
            "detect_merged_branch_candidate",
            branch_name = %branch.name,
            is_base = branch.is_base,
            is_current = branch.is_current,
            branch_tip = field::Empty,
            merge_status = field::Empty,
            result_state = field::Empty,
        );
        let _branch_enter = branch_span.enter();

        if branch.is_base {
            skipped_base_count += 1;
            branch_span.record("result_state", "skipped_base");
            continue;
        }
        if branch.is_current {
            skipped_current_count += 1;
            branch_span.record("result_state", "skipped_current");
            continue;
        }
        candidate_count += 1;

        let candidate_branch =
            match info_span!("detect_merged_find_branch", branch_name = %branch.name)
                .in_scope(|| repo.find_branch(&branch.name, git2::BranchType::Local))
            {
                Ok(branch) => branch,
                Err(_) => {
                    find_branch_error_count += 1;
                    branch_span.record("result_state", "find_branch_error");
                    continue;
                }
            };

        let branch_oid = match info_span!("detect_merged_target", branch_name = %branch.name)
            .in_scope(|| {
                candidate_branch
                    .get()
                    .target()
                    .ok_or_else(|| git2::Error::from_str("no target"))
            }) {
            Ok(oid) => oid,
            Err(_) => {
                missing_target_count += 1;
                branch_span.record("result_state", "missing_target");
                continue;
            }
        };
        let branch_oid_string = branch_oid.to_string();
        branch_span.record("branch_tip", branch_oid_string.as_str());
        checked_count += 1;

        match info_span!(
            "detect_merged_graph_descendant_of",
            branch_name = %branch.name,
            base_oid = %base_ref,
            branch_tip = %branch_oid,
        )
        .in_scope(|| repo.graph_descendant_of(base_ref, branch_oid))
        {
            Ok(true) => {
                merged_count += 1;
                branch.merge_status = MergeStatus::Merged;
                branch_span.record("merge_status", "merged");
                branch_span.record("result_state", "success");
            }
            Ok(false) => {
                unmerged_count += 1;
                branch_span.record("merge_status", "unmerged");
                branch_span.record("result_state", "success");
            }
            Err(_) => {
                graph_error_count += 1;
                branch_span.record("merge_status", "unmerged");
                branch_span.record("result_state", "graph_error");
            }
        }
    }
    span.record("candidate_count", candidate_count);
    span.record("checked_count", checked_count);
    span.record("skipped_base_count", skipped_base_count);
    span.record("skipped_current_count", skipped_current_count);
    span.record("find_branch_error_count", find_branch_error_count);
    span.record("missing_target_count", missing_target_count);
    span.record("merged_count", merged_count);
    span.record("unmerged_count", unmerged_count);
    span.record("graph_error_count", graph_error_count);
    Ok(())
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
