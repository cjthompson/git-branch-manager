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

/// Fast-forward a local branch to match its remote tracking branch without checking it out.
pub fn fast_forward(repo_path: &Path, branch_name: &str) -> OperationResult {
    match Command::new("git")
        .current_dir(repo_path)
        .args([
            "fetch",
            "origin",
            &format!("{}:{}", branch_name, branch_name),
        ])
        .output()
    {
        Ok(o) if o.status.success() => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::FastForward,
            success: true,
            message: "Fast-forwarded to remote".into(),
        },
        Ok(o) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::FastForward,
            success: false,
            message: format!(
                "Cannot fast-forward: {}",
                String::from_utf8_lossy(&o.stderr).trim()
            ),
        },
        Err(e) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::FastForward,
            success: false,
            message: format!("Failed: {}", e),
        },
    }
}

/// Merge a branch into the base branch (regular or squash merge).
///
/// If `stash` is true, working tree changes are stashed before the operation and popped after.
/// On merge conflict the merge is aborted and the original branch is checked back out.
pub fn merge_branch(
    repo_path: &Path,
    branch_name: &str,
    base: &str,
    squash: bool,
    stash: bool,
) -> Vec<OperationResult> {
    let mut results = Vec::new();
    let action = if squash {
        BranchAction::SquashMerge
    } else {
        BranchAction::Merge
    };

    if stash {
        let o = Command::new("git")
            .current_dir(repo_path)
            .args(["stash", "push", "-m", "git-bm auto-stash"])
            .output();
        if let Ok(o) = &o
            && !o.status.success()
        {
            results.push(OperationResult {
                branch_name: branch_name.to_string(),
                action,
                success: false,
                message: "Stash failed".into(),
            });
            return results;
        }
    }

    // Checkout base
    let co = Command::new("git")
        .current_dir(repo_path)
        .args(["checkout", base])
        .output();
    if let Ok(o) = &co
        && !o.status.success()
    {
        results.push(OperationResult {
            branch_name: branch_name.to_string(),
            action,
            success: false,
            message: format!("Checkout {} failed", base),
        });
        if stash {
            let _ = Command::new("git")
                .current_dir(repo_path)
                .args(["stash", "pop"])
                .output();
        }
        return results;
    }

    // Merge
    let mut args = vec!["merge"];
    if squash {
        args.push("--squash");
    }
    args.push(branch_name);
    let merge_out = Command::new("git").current_dir(repo_path).args(&args).output();
    match merge_out {
        Ok(o) if o.status.success() => {
            if squash {
                let _ = Command::new("git")
                    .current_dir(repo_path)
                    .args(["commit", "-m", &format!("Squash merge {}", branch_name)])
                    .output();
            }
            results.push(OperationResult {
                branch_name: branch_name.to_string(),
                action,
                success: true,
                message: if squash {
                    "Squash merged".into()
                } else {
                    "Merged".into()
                },
            });
        }
        Ok(o) => {
            let _ = Command::new("git")
                .current_dir(repo_path)
                .args(["merge", "--abort"])
                .output();
            results.push(OperationResult {
                branch_name: branch_name.to_string(),
                action,
                success: false,
                message: format!(
                    "Merge failed: {}",
                    String::from_utf8_lossy(&o.stderr).trim()
                ),
            });
        }
        Err(e) => results.push(OperationResult {
            branch_name: branch_name.to_string(),
            action,
            success: false,
            message: format!("Failed: {}", e),
        }),
    }

    if stash {
        let _ = Command::new("git")
            .current_dir(repo_path)
            .args(["stash", "pop"])
            .output();
    }
    results
}

/// Rebase a branch onto the base branch.
///
/// Checks out the branch, runs `git rebase <base>`, and on conflict aborts.
/// If `stash` is true, working tree changes are stashed before and popped after.
pub fn rebase_branch(
    repo_path: &Path,
    branch_name: &str,
    base: &str,
    stash: bool,
) -> Vec<OperationResult> {
    let mut results = Vec::new();

    if stash {
        let o = Command::new("git")
            .current_dir(repo_path)
            .args(["stash", "push", "-m", "git-bm auto-stash"])
            .output();
        if let Ok(o) = &o
            && !o.status.success()
        {
            results.push(OperationResult {
                branch_name: branch_name.to_string(),
                action: BranchAction::Rebase,
                success: false,
                message: "Stash failed".into(),
            });
            return results;
        }
    }

    let co = Command::new("git")
        .current_dir(repo_path)
        .args(["checkout", branch_name])
        .output();
    if let Ok(o) = &co
        && !o.status.success()
    {
        results.push(OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::Rebase,
            success: false,
            message: "Checkout failed".into(),
        });
        if stash {
            let _ = Command::new("git")
                .current_dir(repo_path)
                .args(["stash", "pop"])
                .output();
        }
        return results;
    }

    let rebase = Command::new("git")
        .current_dir(repo_path)
        .args(["rebase", base])
        .output();
    match rebase {
        Ok(o) if o.status.success() => {
            results.push(OperationResult {
                branch_name: branch_name.to_string(),
                action: BranchAction::Rebase,
                success: true,
                message: format!("Rebased onto {}", base),
            });
        }
        Ok(o) => {
            let _ = Command::new("git")
                .current_dir(repo_path)
                .args(["rebase", "--abort"])
                .output();
            results.push(OperationResult {
                branch_name: branch_name.to_string(),
                action: BranchAction::Rebase,
                success: false,
                message: format!(
                    "Rebase conflicts: {}",
                    String::from_utf8_lossy(&o.stderr).trim()
                ),
            });
        }
        Err(e) => results.push(OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::Rebase,
            success: false,
            message: format!("Failed: {}", e),
        }),
    }

    if stash {
        let _ = Command::new("git")
            .current_dir(repo_path)
            .args(["stash", "pop"])
            .output();
    }
    results
}

/// Create a git worktree for the given branch under `.worktrees/<sanitized-name>`.
pub fn create_worktree(repo_path: &Path, branch_name: &str) -> OperationResult {
    let sanitized = branch_name.replace('/', "-");
    let worktree_path = repo_path.join(".worktrees").join(&sanitized);
    match Command::new("git")
        .current_dir(repo_path)
        .args([
            "worktree",
            "add",
            worktree_path.to_str().unwrap_or(""),
            branch_name,
        ])
        .output()
    {
        Ok(o) if o.status.success() => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::Worktree,
            success: true,
            message: format!("Worktree at {}", worktree_path.display()),
        },
        Ok(o) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::Worktree,
            success: false,
            message: format!(
                "Failed: {}",
                String::from_utf8_lossy(&o.stderr).trim()
            ),
        },
        Err(e) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::Worktree,
            success: false,
            message: format!("Failed: {}", e),
        },
    }
}

/// Push a branch to its remote tracking branch.
pub fn push_branch(repo_path: &Path, branch_name: &str) -> OperationResult {
    match Command::new("git")
        .current_dir(repo_path)
        .args(["push", "origin", branch_name])
        .output()
    {
        Ok(o) if o.status.success() => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::Push,
            success: true,
            message: "Pushed to remote".into(),
        },
        Ok(o) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::Push,
            success: false,
            message: format!(
                "Push failed: {}",
                String::from_utf8_lossy(&o.stderr).trim()
            ),
        },
        Err(e) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::Push,
            success: false,
            message: format!("Failed to run git: {}", e),
        },
    }
}

/// Force push a branch to its remote tracking branch using --force-with-lease.
pub fn force_push_branch(repo_path: &Path, branch_name: &str) -> OperationResult {
    match Command::new("git")
        .current_dir(repo_path)
        .args(["push", "--force-with-lease", "origin", branch_name])
        .output()
    {
        Ok(o) if o.status.success() => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::ForcePush,
            success: true,
            message: "Force pushed to remote".into(),
        },
        Ok(o) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::ForcePush,
            success: false,
            message: format!(
                "Force push failed: {}",
                String::from_utf8_lossy(&o.stderr).trim()
            ),
        },
        Err(e) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::ForcePush,
            success: false,
            message: format!("Failed to run git: {}", e),
        },
    }
}

/// Pull (fast-forward) a branch from its remote tracking branch.
///
/// For branches that are not currently checked out, uses `git fetch origin branch:branch`.
/// For the current branch, uses `git pull --ff-only`.
pub fn pull_branch(repo_path: &Path, branch_name: &str, is_current: bool) -> OperationResult {
    if is_current {
        match Command::new("git")
            .current_dir(repo_path)
            .args(["pull", "--ff-only"])
            .output()
        {
            Ok(o) if o.status.success() => OperationResult {
                branch_name: branch_name.to_string(),
                action: BranchAction::Pull,
                success: true,
                message: "Pulled from remote".into(),
            },
            Ok(o) => OperationResult {
                branch_name: branch_name.to_string(),
                action: BranchAction::Pull,
                success: false,
                message: format!(
                    "Pull failed: {}",
                    String::from_utf8_lossy(&o.stderr).trim()
                ),
            },
            Err(e) => OperationResult {
                branch_name: branch_name.to_string(),
                action: BranchAction::Pull,
                success: false,
                message: format!("Failed to run git: {}", e),
            },
        }
    } else {
        match Command::new("git")
            .current_dir(repo_path)
            .args([
                "fetch",
                "origin",
                &format!("{}:{}", branch_name, branch_name),
            ])
            .output()
        {
            Ok(o) if o.status.success() => OperationResult {
                branch_name: branch_name.to_string(),
                action: BranchAction::Pull,
                success: true,
                message: "Pulled from remote".into(),
            },
            Ok(o) => OperationResult {
                branch_name: branch_name.to_string(),
                action: BranchAction::Pull,
                success: false,
                message: format!(
                    "Pull failed: {}",
                    String::from_utf8_lossy(&o.stderr).trim()
                ),
            },
            Err(e) => OperationResult {
                branch_name: branch_name.to_string(),
                action: BranchAction::Pull,
                success: false,
                message: format!("Failed to run git: {}", e),
            },
        }
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
