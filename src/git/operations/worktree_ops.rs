use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Sender;

use super::{cancelled, git_cmd};
use crate::git::status::detect_working_tree_status;
use crate::git::worktree_delete;
use crate::types::{BranchAction, OperationResult, ProgressUpdate};
use git2::Repository;
use tracing::instrument;

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
