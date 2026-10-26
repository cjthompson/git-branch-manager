use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use super::{record_deleted_remote_refs, RemoteRefs};
use crate::git::remote_resolve;
use crate::git::{operations, worktree};
use crate::types::{BranchAction, FailureCause, OperationResult, ProgressUpdate};

// ---- Action Execution (runs on background thread) ----

#[allow(clippy::too_many_arguments)]
fn execute_branch_name_cascade(
    action: BranchAction,
    item_names: &[String],
    repo_path: &Path,
    remote: Option<&str>,
    prog_tx: &Sender<ProgressUpdate>,
    cancel_flag: &Arc<AtomicBool>,
    partial_delete_risk: &Arc<AtomicBool>,
    remote_refs: &RemoteRefs,
) -> Vec<OperationResult> {
    let repo = match git2::Repository::open(repo_path) {
        Ok(repo) => repo,
        Err(error) => {
            return vec![OperationResult {
                branch_name: String::new(),
                action,
                success: false,
                message: format!("Failed to open repo: {error}"),
                failure: None,
            }];
        }
    };

    let all_worktrees = match worktree::try_list_worktrees(repo_path) {
        Ok(worktrees) => worktrees,
        Err(error) => {
            return item_names
                .iter()
                .map(|name| {
                    OperationResult::failure(
                        name,
                        action,
                        FailureCause::Other {
                            raw_message: error.clone(),
                        },
                        format!("Cannot inspect worktrees for {name}: {error}"),
                    )
                })
                .collect();
        }
    };

    let force = action == BranchAction::DeleteBranchAndRemoveWorktreeForce;
    let remove_remote = action == BranchAction::DeleteBranchAndRemoveWorktreeRemote;
    // Resolved before any local deletion: deleting a branch drops its config.
    let remote_plan = if remove_remote {
        operations::resolve_remote_deletes(repo_path, remote, item_names)
    } else {
        Vec::new()
    };
    let mut results = Vec::new();
    let mut locally_deleted = Vec::new();

    for (index, name) in item_names.iter().enumerate() {
        if cancel_flag.load(Ordering::Relaxed) {
            results.push(OperationResult {
                branch_name: String::new(),
                action,
                success: false,
                message: "Cancelled by user".into(),
                failure: None,
            });
            break;
        }
        let _ = prog_tx.send(ProgressUpdate {
            completed: index,
            total: item_names.len(),
            current_item: name.clone(),
        });

        let Some(worktree) = all_worktrees
            .iter()
            .find(|worktree| worktree.branch.as_deref() == Some(name.as_str()))
        else {
            // The worktree may have disappeared after the pre-flight. Still
            // attempt the branch deletion; its own safe/force checks remain
            // authoritative and classify any race for the Results overlay.
            let delete_result = if force {
                operations::delete_local_force(&repo, name)
            } else {
                operations::delete_local(&repo, name)
            };
            if delete_result.success {
                locally_deleted.push(name.clone());
            }
            results.push(delete_result);
            continue;
        };

        if worktree.is_main {
            results.push(OperationResult::failure(
                name,
                action,
                FailureCause::CheckedOutInWorktree {
                    worktree_path: worktree.path.clone(),
                    is_main: true,
                },
                format!(
                    "Cannot remove the primary worktree at {}",
                    worktree.path.display()
                ),
            ));
            continue;
        }

        let remove_result = if force {
            operations::force_remove_worktree(
                repo_path,
                &worktree.path,
                (index, item_names.len()),
                prog_tx,
                cancel_flag,
                partial_delete_risk,
            )
        } else {
            operations::remove_worktree(
                repo_path,
                &worktree.path,
                (index, item_names.len()),
                prog_tx,
                cancel_flag,
                partial_delete_risk,
            )
        };
        let removed = remove_result.success;
        results.push(remove_result);

        if removed {
            let delete_result = if force {
                operations::delete_local_force(&repo, name)
            } else {
                operations::delete_local(&repo, name)
            };
            if delete_result.success {
                locally_deleted.push(name.clone());
            }
            results.push(delete_result);
        }
    }

    if remove_remote && !locally_deleted.is_empty() && !cancel_flag.load(Ordering::Relaxed) {
        let _ = prog_tx.send(ProgressUpdate {
            completed: locally_deleted.len(),
            total: item_names.len(),
            current_item: "Deleting remote branches...".into(),
        });
        let planned: Vec<(String, Result<String, String>)> = remote_plan
            .into_iter()
            .filter(|(name, _)| locally_deleted.contains(name))
            .collect();
        let remote_results = operations::delete_remotes_planned(repo_path, &planned, cancel_flag);
        record_deleted_remote_refs(remote_refs, &planned, &remote_results);
        results.extend(remote_results);
    }

    results
}

#[allow(clippy::too_many_arguments)]
pub(super) fn execute_worktree_remove(
    action: BranchAction,
    item_names: &[String],
    repo_path: &Path,
    total: usize,
    prog_tx: &Sender<ProgressUpdate>,
    cancel_flag: &Arc<AtomicBool>,
    partial_delete_risk: &Arc<AtomicBool>,
    results: &mut Vec<OperationResult>,
) {
    let force = action == BranchAction::WorktreeForceRemove;
    for (i, path_str) in item_names.iter().enumerate() {
        if cancel_flag.load(Ordering::Relaxed) {
            break;
        }
        let wt_path = PathBuf::from(path_str);
        let result = if force {
            operations::force_remove_worktree(
                repo_path,
                &wt_path,
                (i, total),
                prog_tx,
                cancel_flag,
                partial_delete_risk,
            )
        } else {
            operations::remove_worktree(
                repo_path,
                &wt_path,
                (i, total),
                prog_tx,
                cancel_flag,
                partial_delete_risk,
            )
        };
        results.push(result);
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn execute_worktree_remove_and_delete_branch(
    action: BranchAction,
    item_names: &[String],
    repo_path: &Path,
    total: usize,
    prog_tx: &Sender<ProgressUpdate>,
    cancel_flag: &Arc<AtomicBool>,
    partial_delete_risk: &Arc<AtomicBool>,
    results: &mut Vec<OperationResult>,
    remote: Option<&str>,
    remote_refs: &RemoteRefs,
) -> Option<Vec<OperationResult>> {
    let repo = match git2::Repository::open(repo_path) {
        Ok(r) => r,
        Err(e) => {
            return Some(vec![OperationResult {
                branch_name: String::new(),
                action,
                success: false,
                message: format!("Failed to open repo: {e}"),
                failure: None,
            }]);
        }
    };
    let mut locally_deleted = Vec::new();
    let remove_remote = action == BranchAction::WorktreeRemoveAndDeleteBranchRemote;
    let mut remote_plan: Vec<(String, Result<String, String>)> = Vec::new();
    for (i, path_str) in item_names.iter().enumerate() {
        if cancel_flag.load(Ordering::Relaxed) {
            results.push(OperationResult {
                branch_name: String::new(),
                action,
                success: false,
                message: "Cancelled by user".into(),
                failure: None,
            });
            break;
        }

        let wt_path = PathBuf::from(path_str);
        let canonical_wt_path = std::fs::canonicalize(&wt_path).ok();

        // Look up the branch checked out in this worktree BEFORE removing
        // it -- removal deregisters the worktree, so `git worktree list`
        // can no longer tell us what branch it held.
        let all_wts = worktree::list_worktrees(repo_path);
        let branch_name = all_wts
            .into_iter()
            .find(|w| {
                if let Ok(wt_canonical) = std::fs::canonicalize(&w.path) {
                    if let Some(ref cwp) = canonical_wt_path {
                        return wt_canonical == *cwp;
                    }
                }
                w.path == wt_path
            })
            .and_then(|w| w.branch);

        let remove_result = operations::remove_worktree(
            repo_path,
            &wt_path,
            (i, total),
            prog_tx,
            cancel_flag,
            partial_delete_risk,
        );
        let removed = remove_result.success;
        results.push(remove_result);

        if removed {
            if let Some(name) = branch_name {
                if remove_remote {
                    remote_plan.push((
                        name.clone(),
                        remote_resolve::resolve_delete_for(&repo, remote, &name),
                    ));
                }
                let delete_result = operations::delete_local(&repo, &name);
                if delete_result.success {
                    locally_deleted.push(name);
                }
                results.push(delete_result);
            }
        }
    }
    if remove_remote && !locally_deleted.is_empty() {
        let _ = prog_tx.send(ProgressUpdate {
            completed: locally_deleted.len(),
            total,
            current_item: "Deleting remote branches...".into(),
        });
        let planned: Vec<(String, Result<String, String>)> = remote_plan
            .into_iter()
            .filter(|(name, _)| locally_deleted.contains(name))
            .collect();
        let remote_results = operations::delete_remotes_planned(repo_path, &planned, cancel_flag);
        record_deleted_remote_refs(remote_refs, &planned, &remote_results);
        results.extend(remote_results);
    }
    None
}

#[allow(clippy::too_many_arguments)]
pub(super) fn execute_delete_branch_cascade(
    action: BranchAction,
    item_names: &[String],
    repo_path: &Path,
    remote: Option<&str>,
    prog_tx: &Sender<ProgressUpdate>,
    cancel_flag: &Arc<AtomicBool>,
    partial_delete_risk: &Arc<AtomicBool>,
    results: &mut Vec<OperationResult>,
    remote_refs: &RemoteRefs,
) {
    results.extend(match action {
        BranchAction::DeleteBranchAndRemoveWorktree
        | BranchAction::DeleteBranchAndRemoveWorktreeForce
        | BranchAction::DeleteBranchAndRemoveWorktreeRemote => execute_branch_name_cascade(
            action,
            item_names,
            repo_path,
            remote,
            prog_tx,
            cancel_flag,
            partial_delete_risk,
            remote_refs,
        ),
        _ => unreachable!("matched new delete action"),
    });
}
