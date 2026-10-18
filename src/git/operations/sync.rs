use std::path::Path;
use std::sync::atomic::AtomicBool;

use super::{cancelled, git_cmd, run_git_cancellable};
use crate::types::{BranchAction, OperationResult};
use tracing::instrument;

#[instrument(skip(repo_path, cancel))]
pub fn fast_forward(repo_path: &Path, branch_name: &str, cancel: &AtomicBool) -> OperationResult {
    let refspec = format!("{branch_name}:{branch_name}");
    match run_git_cancellable(
        git_cmd(repo_path).args(["fetch", "origin", &refspec]),
        cancel,
    ) {
        None => cancelled(branch_name, BranchAction::FastForward),
        Some(Ok(o)) if o.status.success() => OperationResult::success(
            branch_name,
            BranchAction::FastForward,
            format!("Fast-forwarded {branch_name}"),
        ),
        Some(Ok(o)) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::FastForward,
            success: false,
            message: String::from_utf8_lossy(&o.stderr).trim().to_string(),
            failure: None,
        },
        Some(Err(e)) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::FastForward,
            success: false,
            message: e.to_string(),
            failure: None,
        },
    }
}

#[instrument(skip(repo_path, cancel))]
pub fn pull_branch(
    repo_path: &Path,
    branch_name: &str,
    is_current: bool,
    cancel: &AtomicBool,
) -> OperationResult {
    if is_current {
        match run_git_cancellable(git_cmd(repo_path).args(["pull", "--ff-only"]), cancel) {
            None => cancelled(branch_name, BranchAction::Pull),
            Some(Ok(o)) if o.status.success() => OperationResult::success(
                branch_name,
                BranchAction::Pull,
                format!("Pulled {branch_name}"),
            ),
            Some(Ok(o)) => OperationResult {
                branch_name: branch_name.to_string(),
                action: BranchAction::Pull,
                success: false,
                message: String::from_utf8_lossy(&o.stderr).trim().to_string(),
                failure: None,
            },
            Some(Err(e)) => OperationResult {
                branch_name: branch_name.to_string(),
                action: BranchAction::Pull,
                success: false,
                message: e.to_string(),
                failure: None,
            },
        }
    } else {
        fast_forward(repo_path, branch_name, cancel)
    }
}

#[instrument(skip(repo_path, cancel))]
pub fn push_branch(repo_path: &Path, branch_name: &str, cancel: &AtomicBool) -> OperationResult {
    match run_git_cancellable(
        git_cmd(repo_path).args(["push", "--set-upstream", "origin", branch_name]),
        cancel,
    ) {
        None => cancelled(branch_name, BranchAction::Push),
        Some(Ok(o)) if o.status.success() => OperationResult::success(
            branch_name,
            BranchAction::Push,
            format!("Pushed {branch_name}"),
        ),
        Some(Ok(o)) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::Push,
            success: false,
            message: String::from_utf8_lossy(&o.stderr).trim().to_string(),
            failure: None,
        },
        Some(Err(e)) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::Push,
            success: false,
            message: e.to_string(),
            failure: None,
        },
    }
}

#[instrument(skip(repo_path, cancel))]
pub fn force_push_branch(
    repo_path: &Path,
    branch_name: &str,
    cancel: &AtomicBool,
) -> OperationResult {
    match run_git_cancellable(
        git_cmd(repo_path).args(["push", "--force-with-lease", "origin", branch_name]),
        cancel,
    ) {
        None => cancelled(branch_name, BranchAction::ForcePush),
        Some(Ok(o)) if o.status.success() => OperationResult::success(
            branch_name,
            BranchAction::ForcePush,
            format!("Force pushed {branch_name}"),
        ),
        Some(Ok(o)) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::ForcePush,
            success: false,
            message: String::from_utf8_lossy(&o.stderr).trim().to_string(),
            failure: None,
        },
        Some(Err(e)) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::ForcePush,
            success: false,
            message: e.to_string(),
            failure: None,
        },
    }
}

#[instrument(skip(repo_path))]
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
            failure: None,
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
            OperationResult::success(
                branch_name,
                action,
                format!("Merged {branch_name} into {base}"),
            )
        }
        _ => {
            let _ = git_cmd(repo_path).args(["merge", "--abort"]).output();
            OperationResult {
                branch_name: branch_name.to_string(),
                action,
                success: false,
                message: "Merge conflict \u{2014} aborted".to_string(),
                failure: None,
            }
        }
    };

    if stash {
        let _ = git_cmd(repo_path).args(["stash", "pop"]).output();
    }
    vec![result]
}

#[instrument(skip(repo_path))]
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

    let co = git_cmd(repo_path).args(["checkout", branch_name]).output();
    if !matches!(&co, Ok(o) if o.status.success()) {
        if stash {
            let _ = git_cmd(repo_path).args(["stash", "pop"]).output();
        }
        return vec![OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::Rebase,
            success: false,
            message: format!("Failed to checkout {branch_name}"),
            failure: None,
        }];
    }

    let out = git_cmd(repo_path).args(["rebase", base]).output();
    let result = match out {
        Ok(o) if o.status.success() => OperationResult::success(
            branch_name,
            BranchAction::Rebase,
            format!("Rebased {branch_name} onto {base}"),
        ),
        _ => {
            let _ = git_cmd(repo_path).args(["rebase", "--abort"]).output();
            OperationResult {
                branch_name: branch_name.to_string(),
                action: BranchAction::Rebase,
                success: false,
                message: "Rebase conflict \u{2014} aborted".to_string(),
                failure: None,
            }
        }
    };

    if stash {
        let _ = git_cmd(repo_path).args(["stash", "pop"]).output();
    }
    vec![result]
}
