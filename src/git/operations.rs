use std::process::Command;

use git2::{BranchType, Repository};

use crate::types::{BranchAction, OperationResult};

/// Delete a local branch using git2.
pub fn delete_local(repo: &Repository, branch_name: &str) -> OperationResult {
    match repo.find_branch(branch_name, BranchType::Local) {
        Ok(mut branch) => match branch.delete() {
            Ok(()) => OperationResult {
                branch_name: branch_name.to_string(),
                action: BranchAction::DeleteLocal,
                success: true,
                message: "Deleted local branch".to_string(),
            },
            Err(e) => OperationResult {
                branch_name: branch_name.to_string(),
                action: BranchAction::DeleteLocal,
                success: false,
                message: format!("Failed to delete: {}", e),
            },
        },
        Err(e) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::DeleteLocal,
            success: false,
            message: format!("Branch not found: {}", e),
        },
    }
}

/// Delete a branch both locally and from the remote.
///
/// Returns a result for each sub-operation (local delete, remote delete).
pub fn delete_local_and_remote(
    repo: &Repository,
    branch_name: &str,
) -> Vec<OperationResult> {
    let mut results = Vec::new();

    // Delete local
    let local_result = delete_local(repo, branch_name);
    let local_success = local_result.success;
    results.push(local_result);

    // Delete remote (only attempt if local succeeded)
    if local_success {
        let remote_result = delete_remote(branch_name);
        results.push(remote_result);
    }

    results
}

/// Delete a branch from the remote using git CLI.
fn delete_remote(branch_name: &str) -> OperationResult {
    match Command::new("git")
        .args(["push", "origin", "--delete", branch_name])
        .output()
    {
        Ok(output) if output.status.success() => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::DeleteLocalAndRemote,
            success: true,
            message: "Deleted remote branch".to_string(),
        },
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            OperationResult {
                branch_name: branch_name.to_string(),
                action: BranchAction::DeleteLocalAndRemote,
                success: false,
                message: format!("Failed to delete remote: {}", stderr.trim()),
            }
        }
        Err(e) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::DeleteLocalAndRemote,
            success: false,
            message: format!("Failed to run git push: {}", e),
        },
    }
}
