use std::io::Write;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::AtomicBool;

use super::{cancelled, git_cmd, run_git_cancellable, squash_message::squash_commit_message};
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

    let squash_message = if squash {
        match squash_commit_message(repo_path, branch_name, base) {
            Ok(message) => Some(message),
            Err(error) => {
                return vec![OperationResult {
                    branch_name: branch_name.to_string(),
                    action,
                    success: false,
                    message: format!("Failed to prepare squash commit message: {error:#}"),
                    failure: None,
                }];
            }
        }
    } else {
        None
    };

    let old_stash = if squash && stash {
        stash_ref(repo_path)
    } else {
        None
    };
    if stash {
        let mut push_args = vec!["stash", "push"];
        if squash {
            push_args.push("--include-untracked");
        }
        push_args.extend(["-m", "gbm-auto-stash"]);
        let _ = git_cmd(repo_path).args(&push_args).output();
    }
    // Squash pops only a stash this call created, never a pre-existing one.
    let should_pop = stash && (!squash || stash_ref(repo_path) != old_stash);

    // Checkout base
    let co = git_cmd(repo_path).args(["checkout", base]).output();
    if !matches!(&co, Ok(o) if o.status.success()) {
        if should_pop {
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

    let mut result = match out {
        Ok(o) if o.status.success() => {
            if squash {
                let commit_result = squash_message
                    .as_deref()
                    .ok_or_else(|| "squash commit message was not prepared".to_string())
                    .and_then(|message| commit_squash(repo_path, message));
                match commit_result {
                    Ok(()) => OperationResult::success(
                        branch_name,
                        action,
                        format!("Merged {branch_name} into {base}"),
                    ),
                    Err(error) => OperationResult {
                        branch_name: branch_name.to_string(),
                        action,
                        success: false,
                        message: format!("Failed to create squash commit: {error}"),
                        failure: None,
                    },
                }
            } else {
                OperationResult::success(
                    branch_name,
                    action,
                    format!("Merged {branch_name} into {base}"),
                )
            }
        }
        _ => {
            // `merge --squash` sets no MERGE_HEAD, so `merge --abort` fails there.
            // Without a saved stash, `reset --merge` would discard a dirty tree's staged edits.
            let reset_safe = !stash || should_pop;
            let message = if squash && !reset_safe {
                "Merge conflict \u{2014} not reset; stash was not created, working tree left as is"
            } else {
                let abort_args: &[&str] = if squash {
                    &["reset", "--merge"]
                } else {
                    &["merge", "--abort"]
                };
                let _ = git_cmd(repo_path).args(abort_args).output();
                "Merge conflict \u{2014} aborted"
            };
            OperationResult {
                branch_name: branch_name.to_string(),
                action,
                success: false,
                message: message.to_string(),
                failure: None,
            }
        }
    };

    if should_pop {
        let pop = git_cmd(repo_path).args(["stash", "pop"]).output();
        if squash && !matches!(&pop, Ok(o) if o.status.success()) {
            result.message.push_str(
                " \u{2014} stash restore conflicted; your changes remain in stash@{0} (gbm-auto-stash)",
            );
        }
    }
    vec![result]
}

fn stash_ref(repo_path: &Path) -> Option<String> {
    let out = git_cmd(repo_path)
        .args(["rev-parse", "-q", "--verify", "refs/stash"])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn commit_squash(repo_path: &Path, message: &str) -> Result<(), String> {
    let mut child = git_cmd(repo_path)
        .args(["commit", "--cleanup=verbatim", "--file=-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "failed to open git commit stdin".to_string())?;
    let message = message.to_owned();
    let writer = std::thread::spawn(move || stdin.write_all(message.as_bytes()));
    let output = child
        .wait_with_output()
        .map_err(|error| error.to_string())?;
    writer
        .join()
        .map_err(|_| "failed while writing squash commit message".to_string())?
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if stderr.is_empty() {
            format!("git commit exited with {}", output.status)
        } else {
            stderr
        })
    }
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
