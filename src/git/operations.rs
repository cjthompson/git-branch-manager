use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::Duration;

use crate::git::worktree;
use crate::types::{BranchAction, FailureCause, OperationResult, ProgressUpdate};
use git2::Repository;
use std::path::Path;
use std::process::{Command, Stdio};
use tracing::instrument;

use super::status::detect_working_tree_status;
use super::worktree_delete;

fn git_cmd(repo_path: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.current_dir(repo_path)
        .stdin(Stdio::null())
        .env("GIT_TERMINAL_PROMPT", "0");
    cmd
}

fn cancelled(branch_name: &str, action: BranchAction) -> OperationResult {
    OperationResult {
        branch_name: branch_name.to_string(),
        action,
        success: false,
        message: "Cancelled".into(),
        failure: None,
    }
}

fn run_with_progress<F>(
    item_names: &[String],
    action: BranchAction,
    prog_tx: &Sender<ProgressUpdate>,
    cancel: &AtomicBool,
    mut op: F,
) -> Vec<OperationResult>
where
    F: FnMut(&str) -> OperationResult,
{
    let total = item_names.len();
    let mut results = Vec::with_capacity(total);
    for (i, name) in item_names.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            results.push(cancelled(name, action));
            break;
        }
        let _ = prog_tx.send(ProgressUpdate {
            completed: i,
            total,
            current_item: name.clone(),
        });
        results.push(op(name));
    }
    results
}

fn run_git_cancellable(
    cmd: &mut Command,
    cancel: &AtomicBool,
) -> Option<std::io::Result<std::process::Output>> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return Some(Err(e)),
    };
    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            return None;
        }
        match child.try_wait() {
            Ok(Some(_)) => return Some(child.wait_with_output()),
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Some(Err(e)),
        }
    }
}

/// Classify a `find_branch` failure into a typed [`FailureCause`].
fn classify_branch_error(err: &git2::Error) -> FailureCause {
    if err.code() == git2::ErrorCode::NotFound {
        FailureCause::BranchNotFound
    } else {
        FailureCause::Other {
            raw_message: err.message().to_string(),
        }
    }
}

fn command_failure_text(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    } else {
        stderr
    }
}

fn classify_delete_command_error(repo_path: &Path, branch_name: &str, raw: &str) -> FailureCause {
    let lower = raw.to_ascii_lowercase();
    let checked_out_hint = lower.contains("used by worktree")
        || lower.contains("checked out")
        || lower.contains("checked-out");

    if checked_out_hint {
        // Use the caller-excluding lookup: if `branch_name` is checked out
        // in *this* worktree (repo_path itself), that's not a recoverable
        // "checked out elsewhere" case — it falls through to `Other` below,
        // preserving the raw git message.
        match worktree::try_other_worktree_for_branch(repo_path, branch_name) {
            Ok(Some(worktree)) => {
                return FailureCause::CheckedOutInWorktree {
                    worktree_path: worktree.path,
                    is_main: worktree.is_main,
                };
            }
            Ok(None) => {}
            Err(lookup_error) => {
                return FailureCause::Other {
                    raw_message: format!("{raw} (worktree lookup failed: {lookup_error})"),
                };
            }
        }
    }

    if lower.contains("not fully merged") || lower.contains("not merged") {
        FailureCause::NotMerged
    } else if lower.contains("not found") || lower.contains("does not exist") {
        FailureCause::BranchNotFound
    } else {
        FailureCause::Other {
            raw_message: raw.to_string(),
        }
    }
}

fn delete_failure_result(
    branch_name: &str,
    action: BranchAction,
    cause: FailureCause,
    fallback_message: impl Into<String>,
) -> OperationResult {
    if matches!(&cause, FailureCause::BranchNotFound) {
        OperationResult {
            branch_name: branch_name.to_string(),
            action,
            // An already-deleted branch satisfies the user's requested end
            // state. Keep the typed cause for diagnostics, but do not turn a
            // harmless race into failure UX.
            success: true,
            message: format!("Branch {branch_name} already gone"),
            failure: Some(cause),
        }
    } else {
        OperationResult::failure(branch_name, action, cause, fallback_message)
    }
}

fn delete_local_with_mode(
    repo: &Repository,
    branch_name: &str,
    force: bool,
) -> OperationResult {
    let action = if force {
        BranchAction::DeleteLocalForce
    } else {
        BranchAction::DeleteLocal
    };
    let verb = if force { "Force-deleted" } else { "Deleted" };
    let failure_verb = if force { "force-delete" } else { "delete" };

    match repo.find_branch(branch_name, git2::BranchType::Local) {
        Ok(_) => {
            let Some(repo_path) = repo.workdir() else {
                return OperationResult::failure(
                    branch_name,
                    action,
                    FailureCause::Other {
                        raw_message: "cannot delete a branch from a bare repository".into(),
                    },
                    format!("Failed to {failure_verb} {branch_name}: bare repository"),
                );
            };
            let flag = if force { "-D" } else { "-d" };
            match git_cmd(repo_path).args(["branch", flag, branch_name]).output() {
                Ok(output) if output.status.success() => OperationResult::success(
                    branch_name,
                    action,
                    format!("{verb} {branch_name}"),
                ),
                Ok(output) => {
                    let raw = command_failure_text(&output);
                    let cause = classify_delete_command_error(repo_path, branch_name, &raw);
                    delete_failure_result(
                        branch_name,
                        action,
                        cause,
                        format!("Failed to {failure_verb} {branch_name}: {raw}"),
                    )
                }
                Err(error) => OperationResult::failure(
                    branch_name,
                    action,
                    FailureCause::Other {
                        raw_message: error.to_string(),
                    },
                    format!("Failed to {failure_verb} {branch_name}: {error}"),
                ),
            }
        }
        Err(e) => {
            let cause = classify_branch_error(&e);
            delete_failure_result(
                branch_name,
                action,
                cause,
                format!("Failed to {failure_verb} {branch_name}: {e}"),
            )
        }
    }
}

/// Delete a local branch with Git's safe `-d` merge check.
#[instrument(skip(repo))]
pub fn delete_local(repo: &Repository, branch_name: &str) -> OperationResult {
    delete_local_with_mode(repo, branch_name, false)
}

/// Force-delete a local branch with Git's `-D` override for the merge check.
#[instrument(skip(repo))]
pub fn delete_local_force(repo: &Repository, branch_name: &str) -> OperationResult {
    delete_local_with_mode(repo, branch_name, true)
}

#[instrument(skip(repo, repo_path))]
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

    let result = git_cmd(repo_path).args(["checkout", branch_name]).output();

    if stash {
        let _ = git_cmd(repo_path).args(["stash", "pop"]).output();
    }

    match result {
        Ok(out) if out.status.success() => OperationResult::success(
            branch_name,
            action,
            format!("Checked out {branch_name}"),
        ),
        Ok(out) => OperationResult {
            branch_name: branch_name.to_string(),
            action,
            success: false,
            message: String::from_utf8_lossy(&out.stderr).trim().to_string(),
            failure: None,
        },
        Err(e) => OperationResult {
            branch_name: branch_name.to_string(),
            action,
            success: false,
            message: e.to_string(),
            failure: None,
        },
    }
}

#[instrument(skip(repo_path, cancel))]
pub fn fetch(repo_path: &Path, cancel: &AtomicBool) -> OperationResult {
    run_fetch_cmd(repo_path, false, cancel)
}

#[instrument(skip(repo_path, cancel))]
pub fn fetch_prune(repo_path: &Path, cancel: &AtomicBool) -> OperationResult {
    run_fetch_cmd(repo_path, true, cancel)
}

#[instrument(skip(repo_path))]
pub fn fetch_sync(repo_path: &Path) -> bool {
    let out = git_cmd(repo_path).args(["fetch", "--all"]).output();
    matches!(out, Ok(o) if o.status.success())
}

fn run_fetch_cmd(repo_path: &Path, prune: bool, cancel: &AtomicBool) -> OperationResult {
    let mut args = vec!["fetch", "--all"];
    if prune {
        args.push("--prune");
    }
    let action = if prune {
        BranchAction::FetchPrune
    } else {
        BranchAction::Fetch
    };

    match run_git_cancellable(git_cmd(repo_path).args(&args), cancel) {
        None => cancelled("", action),
        Some(Ok(out)) if out.status.success() => OperationResult::success(
            String::new(),
            action,
            "Fetched all remotes".to_string(),
        ),
        Some(Ok(out)) => OperationResult {
            branch_name: String::new(),
            action,
            success: false,
            message: String::from_utf8_lossy(&out.stderr).trim().to_string(),
            failure: None,
        },
        Some(Err(e)) => OperationResult {
            branch_name: String::new(),
            action,
            success: false,
            message: e.to_string(),
            failure: None,
        },
    }
}

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

#[instrument(skip(repo_path))]
pub fn checkout_remote_branch(repo_path: &Path, remote: &str, short_name: &str) -> OperationResult {
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
        Ok(o) if o.status.success() => OperationResult::success(
            short_name,
            BranchAction::CheckoutRemote,
            format!("Checked out {short_name} tracking {remote}/{short_name}"),
        ),
        Ok(o) => OperationResult {
            branch_name: short_name.to_string(),
            action: BranchAction::CheckoutRemote,
            success: false,
            message: String::from_utf8_lossy(&o.stderr).trim().to_string(),
            failure: None,
        },
        Err(e) => OperationResult {
            branch_name: short_name.to_string(),
            action: BranchAction::CheckoutRemote,
            success: false,
            message: e.to_string(),
            failure: None,
        },
    }
}

#[instrument(skip(repo_path, branch_names, cancel), fields(count = branch_names.len()))]
pub fn delete_remotes_batch(
    repo_path: &Path,
    branch_names: &[String],
    cancel: &AtomicBool,
) -> Vec<OperationResult> {
    delete_remotes_batch_for_remote(repo_path, "origin", branch_names, cancel)
}

#[instrument(skip(repo_path, branch_names, cancel), fields(count = branch_names.len()))]
pub fn delete_remotes_batch_for_remote(
    repo_path: &Path,
    remote: &str,
    branch_names: &[String],
    cancel: &AtomicBool,
) -> Vec<OperationResult> {
    if branch_names.is_empty() {
        return vec![];
    }

    // Try batch delete first
    let mut args = vec!["push", remote, "--delete"];
    let refs: Vec<&str> = branch_names.iter().map(|s| s.as_str()).collect();
    args.extend(&refs);

    match run_git_cancellable(git_cmd(repo_path).args(&args), cancel) {
        None => {
            return branch_names
                .iter()
                .map(|name| cancelled(name, BranchAction::DeleteRemoteBranch))
                .collect()
        }
        Some(Ok(o)) if o.status.success() => {
            return branch_names
                .iter()
                .map(|name| {
                    OperationResult::success(
                        name,
                        BranchAction::DeleteRemoteBranch,
                        format!("Deleted remote {name}"),
                    )
                })
                .collect()
        }
        Some(_) => {} // fall through to individual deletes
    }

    // Fallback to individual deletes
    branch_names
        .iter()
        .map(|name| delete_remote(repo_path, remote, name, cancel))
        .collect()
}

fn delete_remote(
    repo_path: &Path,
    remote: &str,
    branch_name: &str,
    cancel: &AtomicBool,
) -> OperationResult {
    match run_git_cancellable(
        git_cmd(repo_path).args(["push", remote, "--delete", branch_name]),
        cancel,
    ) {
        None => cancelled(branch_name, BranchAction::DeleteRemoteBranch),
        Some(Ok(o)) if o.status.success() => OperationResult::success(
            branch_name,
            BranchAction::DeleteRemoteBranch,
            format!("Deleted remote {branch_name}"),
        ),
        Some(Ok(o)) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::DeleteRemoteBranch,
            success: false,
            message: String::from_utf8_lossy(&o.stderr).trim().to_string(),
            failure: None,
        },
        Some(Err(e)) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::DeleteRemoteBranch,
            success: false,
            message: e.to_string(),
            failure: None,
        },
    }
}

pub fn delete_locals_with_progress(
    repo: &Repository,
    branch_names: &[String],
    action: BranchAction,
    prog_tx: &Sender<ProgressUpdate>,
    cancel: &AtomicBool,
) -> Vec<OperationResult> {
    run_with_progress(branch_names, action, prog_tx, cancel, |name| {
        delete_local(repo, name)
    })
}

pub fn delete_remotes_with_progress(
    repo_path: &Path,
    branch_names: &[String],
    prog_tx: &Sender<ProgressUpdate>,
    cancel: &AtomicBool,
) -> Vec<OperationResult> {
    delete_remotes_with_progress_for_remote(repo_path, "origin", branch_names, prog_tx, cancel)
}

pub fn delete_remotes_with_progress_for_remote(
    repo_path: &Path,
    remote: &str,
    branch_names: &[String],
    prog_tx: &Sender<ProgressUpdate>,
    cancel: &AtomicBool,
) -> Vec<OperationResult> {
    run_with_progress(
        branch_names,
        BranchAction::DeleteRemoteBranch,
        prog_tx,
        cancel,
        |name| delete_remote(repo_path, remote, name, cancel),
    )
}

#[instrument(skip(repo_path, cancel))]
pub fn fetch_remote(repo_path: &Path, remote: &str, cancel: &AtomicBool) -> Vec<OperationResult> {
    vec![
        match run_git_cancellable(git_cmd(repo_path).args(["fetch", remote]), cancel) {
            None => cancelled(remote, BranchAction::FetchRemote),
            Some(Ok(o)) if o.status.success() => OperationResult::success(
                remote,
                BranchAction::FetchRemote,
                format!("Fetched {remote}"),
            ),
            Some(Ok(o)) => OperationResult {
                branch_name: remote.to_string(),
                action: BranchAction::FetchRemote,
                success: false,
                message: String::from_utf8_lossy(&o.stderr).trim().to_string(),
                failure: None,
            },
            Some(Err(e)) => OperationResult {
                branch_name: remote.to_string(),
                action: BranchAction::FetchRemote,
                success: false,
                message: e.to_string(),
                failure: None,
            },
        },
    ]
}

#[instrument(skip(repo_path, cancel))]
pub fn pull_remote(
    repo_path: &Path,
    remote: &str,
    short_name: &str,
    cancel: &AtomicBool,
) -> Vec<OperationResult> {
    let refspec = format!("{short_name}:{short_name}");
    vec![
        match run_git_cancellable(git_cmd(repo_path).args(["fetch", remote, &refspec]), cancel) {
            None => cancelled(short_name, BranchAction::PullRemote),
            Some(Ok(o)) if o.status.success() => OperationResult::success(
                short_name,
                BranchAction::PullRemote,
                format!("Pulled {remote}/{short_name}"),
            ),
            Some(Ok(o)) => OperationResult {
                branch_name: short_name.to_string(),
                action: BranchAction::PullRemote,
                success: false,
                message: String::from_utf8_lossy(&o.stderr).trim().to_string(),
                failure: None,
            },
            Some(Err(e)) => OperationResult {
                branch_name: short_name.to_string(),
                action: BranchAction::PullRemote,
                success: false,
                message: e.to_string(),
                failure: None,
            },
        },
    ]
}

#[instrument(skip(repo_path))]
pub fn merge_remote_into_current(
    repo_path: &Path,
    full_ref: &str,
    short_name: &str,
) -> Vec<OperationResult> {
    let out = git_cmd(repo_path).args(["merge", full_ref]).output();
    vec![match out {
        Ok(o) if o.status.success() => OperationResult::success(
            short_name,
            BranchAction::MergeRemoteIntoCurrent,
            format!("Merged {full_ref} into current"),
        ),
        _ => {
            let _ = git_cmd(repo_path).args(["merge", "--abort"]).output();
            OperationResult {
                branch_name: short_name.to_string(),
                action: BranchAction::MergeRemoteIntoCurrent,
                success: false,
                message: "Merge conflict \u{2014} aborted".to_string(),
                failure: None,
            }
        }
    }]
}

#[instrument(skip(repo_path))]
pub fn cherry_pick_remote(
    repo_path: &Path,
    full_ref: &str,
    short_name: &str,
) -> Vec<OperationResult> {
    let out = git_cmd(repo_path).args(["cherry-pick", full_ref]).output();
    vec![match out {
        Ok(o) if o.status.success() => OperationResult::success(
            short_name,
            BranchAction::CherryPickRemote,
            format!("Cherry-picked {full_ref}"),
        ),
        _ => {
            let _ = git_cmd(repo_path).args(["cherry-pick", "--abort"]).output();
            OperationResult {
                branch_name: short_name.to_string(),
                action: BranchAction::CherryPickRemote,
                success: false,
                message: "Cherry-pick conflict \u{2014} aborted".to_string(),
                failure: None,
            }
        }
    }]
}

#[instrument(skip(repo_path))]
pub fn create_worktree(repo_path: &Path, branch_name: &str) -> OperationResult {
    let sanitized = branch_name.replace('/', "-");
    let wt_path = repo_path.join(".worktrees").join(&sanitized);
    let wt_str = wt_path.to_string_lossy();

    let out = git_cmd(repo_path)
        .args(["worktree", "add", &wt_str, branch_name])
        .output();

    match out {
        Ok(o) if o.status.success() => OperationResult::success(
            branch_name,
            BranchAction::Worktree,
            format!("Created worktree at {wt_str}"),
        ),
        Ok(o) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::Worktree,
            success: false,
            message: String::from_utf8_lossy(&o.stderr).trim().to_string(),
            failure: None,
        },
        Err(e) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::Worktree,
            success: false,
            message: e.to_string(),
            failure: None,
        },
    }
}

#[instrument(skip(repo_path, worktree_path, prog_tx, cancel, partial_delete_risk))]
pub fn remove_worktree(
    repo_path: &Path,
    worktree_path: &Path,
    outer: (usize, usize),
    prog_tx: &Sender<ProgressUpdate>,
    cancel: &AtomicBool,
    partial_delete_risk: &AtomicBool,
) -> OperationResult {
    remove_worktree_impl(
        repo_path,
        worktree_path,
        false,
        outer,
        prog_tx,
        cancel,
        partial_delete_risk,
    )
}

#[instrument(skip(repo_path, worktree_path, prog_tx, cancel, partial_delete_risk))]
pub fn force_remove_worktree(
    repo_path: &Path,
    worktree_path: &Path,
    outer: (usize, usize),
    prog_tx: &Sender<ProgressUpdate>,
    cancel: &AtomicBool,
    partial_delete_risk: &AtomicBool,
) -> OperationResult {
    remove_worktree_impl(
        repo_path,
        worktree_path,
        true,
        outer,
        prog_tx,
        cancel,
        partial_delete_risk,
    )
}

/// Shared orchestration body for [`remove_worktree`]/[`force_remove_worktree`]:
/// validates the target, then delegates the actual filesystem work to
/// `git::worktree_delete`'s primitives, streaming per-file progress ticks
/// while keeping `ProgressUpdate.completed`/`.total` in the caller's
/// worktree-batch units (`outer`) rather than file units -- `job_queue`'s
/// cross-job aggregate math depends on that.
fn remove_worktree_impl(
    repo_path: &Path,
    worktree_path: &Path,
    force: bool,
    outer: (usize, usize),
    prog_tx: &Sender<ProgressUpdate>,
    cancel: &AtomicBool,
    partial_delete_risk: &AtomicBool,
) -> OperationResult {
    let action = if force {
        BranchAction::WorktreeForceRemove
    } else {
        BranchAction::WorktreeRemove
    };
    let wt_str = worktree_path.to_string_lossy();
    let (outer_completed, outer_total) = outer;

    // Refuse to ever delete the main worktree -- compare canonicalized paths
    // so a relative/symlinked `worktree_path` can't slip past a naive `==`.
    let repo_canonical =
        std::fs::canonicalize(repo_path).unwrap_or_else(|_| repo_path.to_path_buf());
    let worktree_canonical =
        std::fs::canonicalize(worktree_path).unwrap_or_else(|_| worktree_path.to_path_buf());
    if repo_canonical == worktree_canonical {
        return OperationResult {
            branch_name: wt_str.to_string(),
            action,
            success: false,
            message: "Refusing to remove the main worktree".to_string(),
            failure: None,
        };
    }

    let repo = match Repository::open(repo_path) {
        Ok(r) => r,
        Err(e) => {
            return OperationResult {
                branch_name: wt_str.to_string(),
                action,
                success: false,
                message: format!("Failed to open repo: {e}"),
                failure: None,
            }
        }
    };

    // Must resolve the `Worktree` handle before any deletion happens --
    // `find_worktree_for_path` canonicalizes `worktree_path`, which requires
    // it to still exist on disk.
    let Some(wt) = worktree_delete::find_worktree_for_path(&repo, worktree_path) else {
        return OperationResult {
            branch_name: wt_str.to_string(),
            action,
            success: false,
            message: format!("{wt_str} is not a registered worktree"),
            failure: None,
        };
    };

    if !force {
        let status = detect_working_tree_status(worktree_path);
        if !status.is_clean() {
            return OperationResult {
                branch_name: wt_str.to_string(),
                action,
                success: false,
                message: "Worktree has uncommitted changes — use force remove".to_string(),
                failure: None,
            };
        }
    }

    let _ = prog_tx.send(ProgressUpdate {
        completed: outer_completed,
        total: outer_total,
        current_item: format!("{wt_str} — scanning"),
    });

    let file_total = worktree_delete::count_files(worktree_path);
    let batch_size = (file_total / 200).max(1);
    let mut done = 0usize;

    let outcome =
        worktree_delete::delete_recursive(worktree_path, cancel, partial_delete_risk, |rel_path| {
            done += 1;
            if done.is_multiple_of(batch_size) || done == file_total {
                let _ = prog_tx.send(ProgressUpdate {
                    completed: outer_completed,
                    total: outer_total,
                    current_item: format!("{wt_str} — {done}/{file_total}: {}", rel_path.display()),
                });
            }
        });

    match outcome {
        worktree_delete::DeleteOutcome::Cancelled => cancelled(&wt_str, action),
        worktree_delete::DeleteOutcome::Error(e) => OperationResult {
            branch_name: wt_str.to_string(),
            action,
            success: false,
            message: format!("{done}/{file_total} files removed ({e})"),
            failure: None,
        },
        worktree_delete::DeleteOutcome::Completed => match worktree_delete::prune_admin(&wt) {
            Ok(()) => OperationResult {
                branch_name: wt_str.to_string(),
                action,
                success: true,
                message: format!("Removed worktree {wt_str}"),
                failure: None,
            },
            Err(e) => OperationResult {
                branch_name: wt_str.to_string(),
                action,
                success: false,
                message: format!(
                    "Removed worktree files for {wt_str} but failed to prune git metadata ({e}) — run `git worktree prune`"
                ),
                failure: None,
            },
        },
    }
}
