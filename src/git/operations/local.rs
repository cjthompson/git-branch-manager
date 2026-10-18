use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Sender;

use super::{cancelled, git_cmd, run_git_cancellable, run_with_progress};
use crate::git::worktree;
use crate::types::{BranchAction, FailureCause, OperationResult, ProgressUpdate};
use git2::Repository;
use tracing::instrument;

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
