use std::process::Command;

use anyhow::Result;
use git2::{BranchType, Repository};

use crate::types::{BranchInfo, MergeStatus};

/// Detect branches that were merged via regular merge (commit is ancestor of base).
pub fn detect_merged_branches(
    repo: &Repository,
    base_branch: &str,
    branches: &mut [BranchInfo],
) -> Result<()> {
    let base_ref = repo.find_branch(base_branch, BranchType::Local)?;
    let base_oid = base_ref.get().peel_to_commit()?.id();

    for branch in branches.iter_mut() {
        if branch.is_base || branch.is_current {
            continue;
        }

        let branch_ref = match repo.find_branch(&branch.name, BranchType::Local) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let branch_oid = match branch_ref.get().peel_to_commit() {
            Ok(c) => c.id(),
            Err(_) => continue,
        };

        // If the branch tip is an ancestor of the base tip, it's merged
        if repo.graph_descendant_of(base_oid, branch_oid).unwrap_or(false) {
            branch.merge_status = MergeStatus::Merged;
        }
    }

    Ok(())
}

/// Detect branches that were squash-merged into the base branch.
///
/// Uses git CLI commands since git2 doesn't expose commit-tree or cherry.
pub fn detect_squash_merged_branches(base_branch: &str, branches: &mut [BranchInfo]) {
    for branch in branches.iter_mut() {
        if branch.merge_status != MergeStatus::Unmerged || branch.is_base || branch.is_current {
            continue;
        }

        if is_squash_merged(base_branch, &branch.name) {
            branch.merge_status = MergeStatus::SquashMerged;
        }
    }
}

/// Check if a single branch was squash-merged into the base branch.
fn is_squash_merged(base_branch: &str, branch_name: &str) -> bool {
    // Step 1: git merge-base <base> <branch>
    let ancestor = match Command::new("git")
        .args(["merge-base", base_branch, branch_name])
        .output()
    {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        }
        _ => return false,
    };

    // Step 2: git commit-tree <branch>^{tree} -p <ancestor> -m _
    let tree_spec = format!("{}^{{tree}}", branch_name);
    let temp_commit = match Command::new("git")
        .args(["commit-tree", &tree_spec, "-p", &ancestor, "-m", "_"])
        .output()
    {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        }
        _ => return false,
    };

    // Step 3: git cherry <base> <temp_commit>
    match Command::new("git")
        .args(["cherry", base_branch, &temp_commit])
        .output()
    {
        Ok(output) if output.status.success() => {
            let result = String::from_utf8_lossy(&output.stdout);
            result.trim().starts_with('-')
        }
        _ => false,
    }
}
