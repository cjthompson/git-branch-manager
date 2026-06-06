use crate::types::{BranchInfo, MergeStatus};
use git2::Repository;
use std::path::Path;
use std::process::Command;
use tracing::instrument;

/// Detect which branches have been regular-merged into the base branch using git2.
/// Modifies branch merge_status in place from Unmerged to Merged where applicable.
#[instrument(skip(repo, branches), fields(base_branch))]
pub fn detect_merged_branches(
    repo: &Repository,
    base_branch: &str,
    branches: &mut [BranchInfo],
) -> anyhow::Result<()> {
    let base_ref = repo
        .find_branch(base_branch, git2::BranchType::Local)?
        .get()
        .target()
        .ok_or_else(|| anyhow::anyhow!("base branch has no target"))?;

    for branch in branches.iter_mut() {
        if branch.is_base || branch.is_current {
            continue;
        }
        let branch_oid = match repo
            .find_branch(&branch.name, git2::BranchType::Local)
            .and_then(|b| {
                b.get()
                    .target()
                    .ok_or_else(|| git2::Error::from_str("no target"))
            }) {
            Ok(oid) => oid,
            Err(_) => continue,
        };

        if repo
            .graph_descendant_of(base_ref, branch_oid)
            .unwrap_or(false)
        {
            branch.merge_status = MergeStatus::Merged;
        }
    }
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
