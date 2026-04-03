use crate::types::{BranchAction, OperationResult};
use git2::Repository;
use std::path::Path;
use std::process::{Command, Stdio};

fn git_cmd(repo_path: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.current_dir(repo_path)
        .stdin(Stdio::null())
        .env("GIT_TERMINAL_PROMPT", "0");
    cmd
}

pub fn delete_local(repo: &Repository, branch_name: &str) -> OperationResult {
    match repo.find_branch(branch_name, git2::BranchType::Local) {
        Ok(mut branch) => match branch.delete() {
            Ok(()) => OperationResult {
                branch_name: branch_name.to_string(),
                action: BranchAction::DeleteLocal,
                success: true,
                message: format!("Deleted {branch_name}"),
            },
            Err(e) => OperationResult {
                branch_name: branch_name.to_string(),
                action: BranchAction::DeleteLocal,
                success: false,
                message: format!("Failed to delete {branch_name}: {e}"),
            },
        },
        Err(e) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::DeleteLocal,
            success: false,
            message: format!("Branch not found: {e}"),
        },
    }
}

pub fn checkout_branch(
    repo: &Repository,
    repo_path: &Path,
    branch_name: &str,
    stash: bool,
) -> OperationResult {
    let action = BranchAction::Checkout;
    let _ = repo; // repo kept in signature for future use / consistency

    if stash {
        let _ = git_cmd(repo_path)
            .args(["stash", "push", "-m", "gbm-auto-stash"])
            .output();
    }

    let result = git_cmd(repo_path)
        .args(["checkout", branch_name])
        .output();

    if stash {
        let _ = git_cmd(repo_path).args(["stash", "pop"]).output();
    }

    match result {
        Ok(out) if out.status.success() => OperationResult {
            branch_name: branch_name.to_string(),
            action,
            success: true,
            message: format!("Checked out {branch_name}"),
        },
        Ok(out) => OperationResult {
            branch_name: branch_name.to_string(),
            action,
            success: false,
            message: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        },
        Err(e) => OperationResult {
            branch_name: branch_name.to_string(),
            action,
            success: false,
            message: e.to_string(),
        },
    }
}

pub fn fetch(repo_path: &Path) -> OperationResult {
    run_fetch_cmd(repo_path, false)
}

pub fn fetch_prune(repo_path: &Path) -> OperationResult {
    run_fetch_cmd(repo_path, true)
}

pub fn fetch_sync(repo_path: &Path) -> bool {
    let out = git_cmd(repo_path).args(["fetch", "--all"]).output();
    matches!(out, Ok(o) if o.status.success())
}

fn run_fetch_cmd(repo_path: &Path, prune: bool) -> OperationResult {
    let mut args = vec!["fetch", "--all"];
    if prune {
        args.push("--prune");
    }
    let action = if prune {
        BranchAction::FetchPrune
    } else {
        BranchAction::Fetch
    };

    match git_cmd(repo_path).args(&args).output() {
        Ok(out) if out.status.success() => OperationResult {
            branch_name: String::new(),
            action,
            success: true,
            message: "Fetched all remotes".to_string(),
        },
        Ok(out) => OperationResult {
            branch_name: String::new(),
            action,
            success: false,
            message: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        },
        Err(e) => OperationResult {
            branch_name: String::new(),
            action,
            success: false,
            message: e.to_string(),
        },
    }
}

pub fn fast_forward(repo_path: &Path, branch_name: &str) -> OperationResult {
    let out = git_cmd(repo_path)
        .args(["fetch", "origin", &format!("{branch_name}:{branch_name}")])
        .output();

    match out {
        Ok(o) if o.status.success() => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::FastForward,
            success: true,
            message: format!("Fast-forwarded {branch_name}"),
        },
        Ok(o) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::FastForward,
            success: false,
            message: String::from_utf8_lossy(&o.stderr).trim().to_string(),
        },
        Err(e) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::FastForward,
            success: false,
            message: e.to_string(),
        },
    }
}

pub fn pull_branch(repo_path: &Path, branch_name: &str, is_current: bool) -> OperationResult {
    if is_current {
        let out = git_cmd(repo_path).args(["pull", "--ff-only"]).output();
        match out {
            Ok(o) if o.status.success() => OperationResult {
                branch_name: branch_name.to_string(),
                action: BranchAction::Pull,
                success: true,
                message: format!("Pulled {branch_name}"),
            },
            Ok(o) => OperationResult {
                branch_name: branch_name.to_string(),
                action: BranchAction::Pull,
                success: false,
                message: String::from_utf8_lossy(&o.stderr).trim().to_string(),
            },
            Err(e) => OperationResult {
                branch_name: branch_name.to_string(),
                action: BranchAction::Pull,
                success: false,
                message: e.to_string(),
            },
        }
    } else {
        fast_forward(repo_path, branch_name)
    }
}

pub fn push_branch(repo_path: &Path, branch_name: &str) -> OperationResult {
    let out = git_cmd(repo_path)
        .args(["push", "origin", branch_name])
        .output();

    match out {
        Ok(o) if o.status.success() => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::Push,
            success: true,
            message: format!("Pushed {branch_name}"),
        },
        Ok(o) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::Push,
            success: false,
            message: String::from_utf8_lossy(&o.stderr).trim().to_string(),
        },
        Err(e) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::Push,
            success: false,
            message: e.to_string(),
        },
    }
}

pub fn force_push_branch(repo_path: &Path, branch_name: &str) -> OperationResult {
    let out = git_cmd(repo_path)
        .args(["push", "--force-with-lease", "origin", branch_name])
        .output();

    match out {
        Ok(o) if o.status.success() => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::ForcePush,
            success: true,
            message: format!("Force pushed {branch_name}"),
        },
        Ok(o) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::ForcePush,
            success: false,
            message: String::from_utf8_lossy(&o.stderr).trim().to_string(),
        },
        Err(e) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::ForcePush,
            success: false,
            message: e.to_string(),
        },
    }
}

pub fn merge_branch(
    repo_path: &Path,
    branch_name: &str,
    base: &str,
    squash: bool,
    stash: bool,
) -> Vec<OperationResult> {
    let action = if squash {
        BranchAction::SquashMerge
    } else {
        BranchAction::Merge
    };

    if stash {
        let _ = git_cmd(repo_path)
            .args(["stash", "push", "-m", "gbm-auto-stash"])
            .output();
    }

    // Checkout base
    let co = git_cmd(repo_path).args(["checkout", base]).output();
    if !matches!(&co, Ok(o) if o.status.success()) {
        if stash {
            let _ = git_cmd(repo_path).args(["stash", "pop"]).output();
        }
        return vec![OperationResult {
            branch_name: branch_name.to_string(),
            action,
            success: false,
            message: format!("Failed to checkout {base}"),
        }];
    }

    let mut merge_args = vec!["merge"];
    if squash {
        merge_args.push("--squash");
    }
    merge_args.push(branch_name);

    let out = git_cmd(repo_path).args(&merge_args).output();

    let result = match out {
        Ok(o) if o.status.success() => {
            if squash {
                let _ = git_cmd(repo_path)
                    .args(["commit", "-m", &format!("Squash merge {branch_name}")])
                    .output();
            }
            OperationResult {
                branch_name: branch_name.to_string(),
                action,
                success: true,
                message: format!("Merged {branch_name} into {base}"),
            }
        }
        _ => {
            let _ = git_cmd(repo_path).args(["merge", "--abort"]).output();
            OperationResult {
                branch_name: branch_name.to_string(),
                action,
                success: false,
                message: "Merge conflict \u{2014} aborted".to_string(),
            }
        }
    };

    if stash {
        let _ = git_cmd(repo_path).args(["stash", "pop"]).output();
    }
    vec![result]
}

pub fn rebase_branch(
    repo_path: &Path,
    branch_name: &str,
    base: &str,
    stash: bool,
) -> Vec<OperationResult> {
    if stash {
        let _ = git_cmd(repo_path)
            .args(["stash", "push", "-m", "gbm-auto-stash"])
            .output();
    }

    let co = git_cmd(repo_path)
        .args(["checkout", branch_name])
        .output();
    if !matches!(&co, Ok(o) if o.status.success()) {
        if stash {
            let _ = git_cmd(repo_path).args(["stash", "pop"]).output();
        }
        return vec![OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::Rebase,
            success: false,
            message: format!("Failed to checkout {branch_name}"),
        }];
    }

    let out = git_cmd(repo_path).args(["rebase", base]).output();
    let result = match out {
        Ok(o) if o.status.success() => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::Rebase,
            success: true,
            message: format!("Rebased {branch_name} onto {base}"),
        },
        _ => {
            let _ = git_cmd(repo_path).args(["rebase", "--abort"]).output();
            OperationResult {
                branch_name: branch_name.to_string(),
                action: BranchAction::Rebase,
                success: false,
                message: "Rebase conflict \u{2014} aborted".to_string(),
            }
        }
    };

    if stash {
        let _ = git_cmd(repo_path).args(["stash", "pop"]).output();
    }
    vec![result]
}

pub fn checkout_remote_branch(
    repo_path: &Path,
    remote: &str,
    short_name: &str,
) -> OperationResult {
    let out = git_cmd(repo_path)
        .args([
            "checkout",
            "-b",
            short_name,
            "--track",
            &format!("{remote}/{short_name}"),
        ])
        .output();

    match out {
        Ok(o) if o.status.success() => OperationResult {
            branch_name: short_name.to_string(),
            action: BranchAction::CheckoutRemote,
            success: true,
            message: format!("Checked out {short_name} tracking {remote}/{short_name}"),
        },
        Ok(o) => OperationResult {
            branch_name: short_name.to_string(),
            action: BranchAction::CheckoutRemote,
            success: false,
            message: String::from_utf8_lossy(&o.stderr).trim().to_string(),
        },
        Err(e) => OperationResult {
            branch_name: short_name.to_string(),
            action: BranchAction::CheckoutRemote,
            success: false,
            message: e.to_string(),
        },
    }
}

pub fn delete_remotes_batch(repo_path: &Path, branch_names: &[String]) -> Vec<OperationResult> {
    if branch_names.is_empty() {
        return vec![];
    }

    // Try batch delete first
    let mut args = vec!["push", "origin", "--delete"];
    let refs: Vec<&str> = branch_names.iter().map(|s| s.as_str()).collect();
    args.extend(&refs);

    let out = git_cmd(repo_path).args(&args).output();
    if matches!(&out, Ok(o) if o.status.success()) {
        return branch_names
            .iter()
            .map(|name| OperationResult {
                branch_name: name.clone(),
                action: BranchAction::DeleteRemoteBranch,
                success: true,
                message: format!("Deleted remote {name}"),
            })
            .collect();
    }

    // Fallback to individual deletes
    branch_names
        .iter()
        .map(|name| delete_remote(repo_path, name))
        .collect()
}

fn delete_remote(repo_path: &Path, branch_name: &str) -> OperationResult {
    let out = git_cmd(repo_path)
        .args(["push", "origin", "--delete", branch_name])
        .output();

    match out {
        Ok(o) if o.status.success() => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::DeleteRemoteBranch,
            success: true,
            message: format!("Deleted remote {branch_name}"),
        },
        Ok(o) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::DeleteRemoteBranch,
            success: false,
            message: String::from_utf8_lossy(&o.stderr).trim().to_string(),
        },
        Err(e) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::DeleteRemoteBranch,
            success: false,
            message: e.to_string(),
        },
    }
}

pub fn fetch_remote(repo_path: &Path, remote: &str) -> Vec<OperationResult> {
    let out = git_cmd(repo_path).args(["fetch", remote]).output();
    vec![match out {
        Ok(o) if o.status.success() => OperationResult {
            branch_name: remote.to_string(),
            action: BranchAction::FetchRemote,
            success: true,
            message: format!("Fetched {remote}"),
        },
        Ok(o) => OperationResult {
            branch_name: remote.to_string(),
            action: BranchAction::FetchRemote,
            success: false,
            message: String::from_utf8_lossy(&o.stderr).trim().to_string(),
        },
        Err(e) => OperationResult {
            branch_name: remote.to_string(),
            action: BranchAction::FetchRemote,
            success: false,
            message: e.to_string(),
        },
    }]
}

pub fn pull_remote(repo_path: &Path, remote: &str, short_name: &str) -> Vec<OperationResult> {
    let out = git_cmd(repo_path)
        .args(["fetch", remote, &format!("{short_name}:{short_name}")])
        .output();

    vec![match out {
        Ok(o) if o.status.success() => OperationResult {
            branch_name: short_name.to_string(),
            action: BranchAction::PullRemote,
            success: true,
            message: format!("Pulled {remote}/{short_name}"),
        },
        Ok(o) => OperationResult {
            branch_name: short_name.to_string(),
            action: BranchAction::PullRemote,
            success: false,
            message: String::from_utf8_lossy(&o.stderr).trim().to_string(),
        },
        Err(e) => OperationResult {
            branch_name: short_name.to_string(),
            action: BranchAction::PullRemote,
            success: false,
            message: e.to_string(),
        },
    }]
}

pub fn merge_remote_into_current(
    repo_path: &Path,
    full_ref: &str,
    short_name: &str,
) -> Vec<OperationResult> {
    let out = git_cmd(repo_path).args(["merge", full_ref]).output();
    vec![match out {
        Ok(o) if o.status.success() => OperationResult {
            branch_name: short_name.to_string(),
            action: BranchAction::MergeRemoteIntoCurrent,
            success: true,
            message: format!("Merged {full_ref} into current"),
        },
        _ => {
            let _ = git_cmd(repo_path).args(["merge", "--abort"]).output();
            OperationResult {
                branch_name: short_name.to_string(),
                action: BranchAction::MergeRemoteIntoCurrent,
                success: false,
                message: "Merge conflict \u{2014} aborted".to_string(),
            }
        }
    }]
}

pub fn cherry_pick_remote(
    repo_path: &Path,
    full_ref: &str,
    short_name: &str,
) -> Vec<OperationResult> {
    let out = git_cmd(repo_path)
        .args(["cherry-pick", full_ref])
        .output();
    vec![match out {
        Ok(o) if o.status.success() => OperationResult {
            branch_name: short_name.to_string(),
            action: BranchAction::CherryPickRemote,
            success: true,
            message: format!("Cherry-picked {full_ref}"),
        },
        _ => {
            let _ = git_cmd(repo_path)
                .args(["cherry-pick", "--abort"])
                .output();
            OperationResult {
                branch_name: short_name.to_string(),
                action: BranchAction::CherryPickRemote,
                success: false,
                message: "Cherry-pick conflict \u{2014} aborted".to_string(),
            }
        }
    }]
}

pub fn create_worktree(repo_path: &Path, branch_name: &str) -> OperationResult {
    let sanitized = branch_name.replace('/', "-");
    let wt_path = repo_path.join(".worktrees").join(&sanitized);
    let wt_str = wt_path.to_string_lossy();

    let out = git_cmd(repo_path)
        .args(["worktree", "add", &wt_str, branch_name])
        .output();

    match out {
        Ok(o) if o.status.success() => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::Worktree,
            success: true,
            message: format!("Created worktree at {wt_str}"),
        },
        Ok(o) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::Worktree,
            success: false,
            message: String::from_utf8_lossy(&o.stderr).trim().to_string(),
        },
        Err(e) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::Worktree,
            success: false,
            message: e.to_string(),
        },
    }
}

pub fn remove_worktree(repo_path: &Path, worktree_path: &Path) -> OperationResult {
    let wt_str = worktree_path.to_string_lossy();
    let out = git_cmd(repo_path)
        .args(["worktree", "remove", &wt_str])
        .output();

    match out {
        Ok(o) if o.status.success() => OperationResult {
            branch_name: wt_str.to_string(),
            action: BranchAction::WorktreeRemove,
            success: true,
            message: format!("Removed worktree {wt_str}"),
        },
        Ok(o) => OperationResult {
            branch_name: wt_str.to_string(),
            action: BranchAction::WorktreeRemove,
            success: false,
            message: String::from_utf8_lossy(&o.stderr).trim().to_string(),
        },
        Err(e) => OperationResult {
            branch_name: wt_str.to_string(),
            action: BranchAction::WorktreeRemove,
            success: false,
            message: e.to_string(),
        },
    }
}

pub fn force_remove_worktree(repo_path: &Path, worktree_path: &Path) -> OperationResult {
    let wt_str = worktree_path.to_string_lossy();
    let out = git_cmd(repo_path)
        .args(["worktree", "remove", "--force", &wt_str])
        .output();

    match out {
        Ok(o) if o.status.success() => OperationResult {
            branch_name: wt_str.to_string(),
            action: BranchAction::WorktreeForceRemove,
            success: true,
            message: format!("Force removed worktree {wt_str}"),
        },
        Ok(o) => OperationResult {
            branch_name: wt_str.to_string(),
            action: BranchAction::WorktreeForceRemove,
            success: false,
            message: String::from_utf8_lossy(&o.stderr).trim().to_string(),
        },
        Err(e) => OperationResult {
            branch_name: wt_str.to_string(),
            action: BranchAction::WorktreeForceRemove,
            success: false,
            message: e.to_string(),
        },
    }
}
