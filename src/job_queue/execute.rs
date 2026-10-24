use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use super::cascade;
use crate::git::{operations, tags};
use crate::types::{BranchAction, OperationResult, ProgressUpdate};

#[allow(clippy::too_many_arguments)]
pub(super) fn execute_action_with_remote(
    action: BranchAction,
    item_names: &[String],
    repo_path: &Path,
    base_branch: &str,
    needs_stash: bool,
    remote: Option<&str>,
    prog_tx: &Sender<ProgressUpdate>,
    cancel_flag: &Arc<AtomicBool>,
    partial_delete_risk: &Arc<AtomicBool>,
) -> Vec<OperationResult> {
    let total = item_names.len();
    let mut results = Vec::new();

    match action {
        BranchAction::DeleteLocal
        | BranchAction::DeleteLocalForce
        | BranchAction::DeleteLocalAndRemote => {
            let repo = match git2::Repository::open(repo_path) {
                Ok(r) => r,
                Err(e) => {
                    return vec![OperationResult {
                        branch_name: String::new(),
                        action,
                        success: false,
                        message: format!("Failed to open repo: {e}"),
                        failure: None,
                    }];
                }
            };
            let mut locally_deleted = Vec::new();
            for (i, name) in item_names.iter().enumerate() {
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
                    completed: i,
                    total,
                    current_item: name.clone(),
                });
                let result = if action == BranchAction::DeleteLocalForce {
                    operations::delete_local_force(&repo, name)
                } else {
                    operations::delete_local(&repo, name)
                };
                if result.success {
                    locally_deleted.push(name.clone());
                }
                results.push(result);
            }
            if action == BranchAction::DeleteLocalAndRemote && !locally_deleted.is_empty() {
                let _ = prog_tx.send(ProgressUpdate {
                    completed: locally_deleted.len(),
                    total,
                    current_item: "Deleting remote branches...".into(),
                });
                let remote_name = remote.unwrap_or("origin");
                results.extend(operations::delete_remotes_batch_for_remote(
                    repo_path,
                    remote_name,
                    &locally_deleted,
                    cancel_flag,
                ));
            }
        }
        BranchAction::Checkout => {
            if let Some(name) = item_names.first() {
                let repo = match git2::Repository::open(repo_path) {
                    Ok(r) => r,
                    Err(e) => {
                        return vec![OperationResult {
                            branch_name: name.clone(),
                            action,
                            success: false,
                            message: format!("Failed to open repo: {e}"),
                            failure: None,
                        }];
                    }
                };
                results.push(operations::checkout_branch(
                    &repo,
                    repo_path,
                    name,
                    needs_stash,
                ));
            }
        }
        BranchAction::FastForward => {
            if let Some(name) = item_names.first() {
                results.push(operations::fast_forward(repo_path, name, cancel_flag));
            }
        }
        BranchAction::Push => {
            for (i, name) in item_names.iter().enumerate() {
                if cancel_flag.load(Ordering::Relaxed) {
                    break;
                }
                let _ = prog_tx.send(ProgressUpdate {
                    completed: i,
                    total,
                    current_item: name.clone(),
                });
                results.push(operations::push_branch(repo_path, name, cancel_flag));
            }
        }
        BranchAction::ForcePush => {
            if let Some(name) = item_names.first() {
                results.push(operations::force_push_branch(repo_path, name, cancel_flag));
            }
        }
        BranchAction::Pull => {
            if let Some(name) = item_names.first() {
                // Assume not current for context menu; pull_branch handles it
                results.push(operations::pull_branch(repo_path, name, false, cancel_flag));
            }
        }
        BranchAction::Merge | BranchAction::SquashMerge => {
            if let Some(name) = item_names.first() {
                let squash = action == BranchAction::SquashMerge;
                results.extend(operations::merge_branch(
                    repo_path,
                    name,
                    base_branch,
                    squash,
                    needs_stash,
                ));
            }
        }
        BranchAction::Rebase => {
            if let Some(name) = item_names.first() {
                results.extend(operations::rebase_branch(
                    repo_path,
                    name,
                    base_branch,
                    needs_stash,
                ));
            }
        }
        BranchAction::Worktree => {
            if let Some(name) = item_names.first() {
                results.push(operations::create_worktree(repo_path, name));
            }
        }
        BranchAction::DeleteTag | BranchAction::DeleteTagAndRemote => {
            let repo = match git2::Repository::open(repo_path) {
                Ok(r) => r,
                Err(e) => {
                    return vec![OperationResult {
                        branch_name: String::new(),
                        action,
                        success: false,
                        message: format!("Failed to open repo: {e}"),
                        failure: None,
                    }];
                }
            };
            let tag_names: Vec<String> = item_names.to_vec();
            results.extend(tags::delete_tags_batch(&repo, &tag_names));
            if action == BranchAction::DeleteTagAndRemote {
                let successfully_deleted: Vec<String> = results
                    .iter()
                    .filter(|r| r.success)
                    .map(|r| r.branch_name.clone())
                    .collect();
                if !successfully_deleted.is_empty() {
                    results.extend(tags::delete_remote_tags_batch(
                        repo_path,
                        &successfully_deleted,
                    ));
                }
            }
        }
        BranchAction::PushTag => {
            for name in item_names {
                results.push(tags::push_tag(repo_path, name));
            }
        }
        BranchAction::DeleteRemoteBranch => {
            let remote_name = remote.unwrap_or("origin");
            results.extend(operations::delete_remotes_with_progress_for_remote(
                repo_path,
                remote_name,
                item_names,
                prog_tx,
                cancel_flag,
            ));
        }
        BranchAction::DeleteRemoteAndLocal => {
            if let Some(name) = item_names.first() {
                let remote_name = remote.unwrap_or("origin");
                let remote_results = operations::delete_remotes_batch_for_remote(
                    repo_path,
                    remote_name,
                    std::slice::from_ref(name),
                    cancel_flag,
                );
                results.extend(remote_results);
                if let Ok(repo) = git2::Repository::open(repo_path) {
                    let local_result = operations::delete_local(&repo, name);
                    results.push(OperationResult {
                        action: BranchAction::DeleteRemoteAndLocal,
                        ..local_result
                    });
                }
            }
        }
        BranchAction::CheckoutRemote => {
            if let Some(name) = item_names.first() {
                let remote_name = remote.unwrap_or("origin");
                results.push(operations::checkout_remote_branch(
                    repo_path,
                    remote_name,
                    name,
                ));
            }
        }
        BranchAction::FetchRemote => {
            if !item_names.is_empty() {
                let remote_name = remote.unwrap_or("origin");
                results.extend(operations::fetch_remote(
                    repo_path,
                    remote_name,
                    cancel_flag,
                ));
            }
        }
        BranchAction::PullRemote => {
            if let Some(name) = item_names.first() {
                let remote_name = remote.unwrap_or("origin");
                results.extend(operations::pull_remote(
                    repo_path,
                    remote_name,
                    name,
                    cancel_flag,
                ));
            }
        }
        BranchAction::MergeRemoteIntoCurrent => {
            if let Some(name) = item_names.first() {
                let remote_name = remote.unwrap_or("origin");
                let full_ref = format!("{remote_name}/{name}");
                results.extend(operations::merge_remote_into_current(
                    repo_path, &full_ref, name,
                ));
            }
        }
        BranchAction::CherryPickRemote => {
            if let Some(name) = item_names.first() {
                let remote_name = remote.unwrap_or("origin");
                let full_ref = format!("{remote_name}/{name}");
                results.extend(operations::cherry_pick_remote(repo_path, &full_ref, name));
            }
        }
        BranchAction::CherryPickLocal => {
            if let Some(name) = item_names.first() {
                results.extend(operations::cherry_pick_local(repo_path, name));
            }
        }
        BranchAction::CherryPickCommit => {
            if let Some(oid) = item_names.first() {
                results.push(operations::cherry_pick_commit(repo_path, oid));
            }
        }
        BranchAction::WorktreeRemove | BranchAction::WorktreeForceRemove => {
            cascade::execute_worktree_remove(
                action,
                item_names,
                repo_path,
                total,
                prog_tx,
                cancel_flag,
                partial_delete_risk,
                &mut results,
            );
        }
        BranchAction::WorktreeRemoveAndDeleteBranch
        | BranchAction::WorktreeRemoveAndDeleteBranchRemote => {
            if let Some(early) = cascade::execute_worktree_remove_and_delete_branch(
                action,
                item_names,
                repo_path,
                total,
                prog_tx,
                cancel_flag,
                partial_delete_risk,
                &mut results,
            ) {
                return early;
            }
        }
        BranchAction::DeleteBranchAndRemoveWorktree
        | BranchAction::DeleteBranchAndRemoveWorktreeForce
        | BranchAction::DeleteBranchAndRemoveWorktreeRemote => {
            cascade::execute_delete_branch_cascade(
                action,
                item_names,
                repo_path,
                remote,
                prog_tx,
                cancel_flag,
                partial_delete_risk,
                &mut results,
            );
        }
        BranchAction::Fetch | BranchAction::FetchPrune => {
            let result = if action == BranchAction::FetchPrune {
                operations::fetch_prune(repo_path, cancel_flag)
            } else {
                operations::fetch(repo_path, cancel_flag)
            };
            results.push(result);
        }
        BranchAction::ViewRemotePR => {
            // Handled in App::execute_menu_action, shouldn't reach here
        }
        BranchAction::JumpToGraph => {
            // Handled in App::execute_menu_action, shouldn't reach the job queue.
        }
        BranchAction::CheckPrimaryBranchCode => {
            // Handled in App::execute_menu_action, shouldn't reach the job queue.
        }
    }

    // Send final progress
    let _ = prog_tx.send(ProgressUpdate {
        completed: results.iter().filter(|r| r.success).count().min(total),
        total,
        current_item: "Done".to_string(),
    });

    results
}
