use std::path::Path;
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
    repo_path: &Path,
    branch_name: &str,
) -> Vec<OperationResult> {
    let mut results = Vec::new();

    // Delete local
    let local_result = delete_local(repo, branch_name);
    let local_success = local_result.success;
    results.push(local_result);

    // Delete remote (only attempt if local succeeded)
    if local_success {
        let remote_result = delete_remote(repo_path, branch_name);
        results.push(remote_result);
    }

    results
}

/// Checkout a branch, optionally stashing and unstashing dirty working tree changes.
pub fn checkout_branch(repo_path: &Path, branch_name: &str, stash: bool) -> OperationResult {
    if stash {
        let output = Command::new("git")
            .current_dir(repo_path)
            .args(["stash", "push", "-m", "git-bm auto-stash"])
            .output();
        if let Ok(o) = &output
            && !o.status.success()
        {
            return OperationResult {
                branch_name: branch_name.to_string(),
                action: BranchAction::Checkout,
                success: false,
                message: format!(
                    "Stash failed: {}",
                    String::from_utf8_lossy(&o.stderr).trim()
                ),
            };
        }
    }

    let checkout = Command::new("git")
        .current_dir(repo_path)
        .args(["checkout", branch_name])
        .output();

    let result = match checkout {
        Ok(o) if o.status.success() => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::Checkout,
            success: true,
            message: format!("Checked out {}", branch_name),
        },
        Ok(o) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::Checkout,
            success: false,
            message: format!(
                "Checkout failed: {}",
                String::from_utf8_lossy(&o.stderr).trim()
            ),
        },
        Err(e) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::Checkout,
            success: false,
            message: format!("Failed to run git: {}", e),
        },
    };

    if stash && result.success {
        let _ = Command::new("git")
            .current_dir(repo_path)
            .args(["stash", "pop"])
            .output();
    }

    result
}

/// Fetch from all remotes.
pub fn fetch(repo_path: &Path) -> OperationResult {
    run_fetch_cmd(repo_path, false)
}

/// Fetch from all remotes with --prune (removes stale tracking refs).
pub fn fetch_prune(repo_path: &Path) -> OperationResult {
    run_fetch_cmd(repo_path, true)
}

fn run_fetch_cmd(repo_path: &Path, prune: bool) -> OperationResult {
    let mut args = vec!["fetch"];
    if prune {
        args.push("--prune");
    }
    let action = if prune {
        BranchAction::FetchPrune
    } else {
        BranchAction::Fetch
    };
    match Command::new("git")
        .current_dir(repo_path)
        .args(&args)
        .output()
    {
        Ok(output) if output.status.success() => OperationResult {
            branch_name: String::new(),
            action,
            success: true,
            message: if prune {
                "Fetched with prune".into()
            } else {
                "Fetched".into()
            },
        },
        Ok(output) => OperationResult {
            branch_name: String::new(),
            action,
            success: false,
            message: format!(
                "Fetch failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        },
        Err(e) => OperationResult {
            branch_name: String::new(),
            action,
            success: false,
            message: format!("Failed to run git: {}", e),
        },
    }
}

/// Delete a branch from the remote using git CLI.
fn delete_remote(repo_path: &Path, branch_name: &str) -> OperationResult {
    match Command::new("git")
        .current_dir(repo_path)
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
