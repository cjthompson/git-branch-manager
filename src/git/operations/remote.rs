use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Sender;

use std::collections::BTreeMap;

use super::{cancelled, git_cmd, run_git_cancellable, run_with_progress};
use crate::git::remote_resolve::{self, unresolved_result};
use crate::types::{BranchAction, OperationResult, ProgressUpdate};
use git2::Repository;
use tracing::instrument;

/// Resolved before local deletion, which drops each branch's config.
pub fn resolve_remote_deletes(
    repo_path: &Path,
    explicit: Option<&str>,
    branch_names: &[String],
) -> Vec<(String, Result<String, String>)> {
    if let Some(remote) = explicit {
        return branch_names
            .iter()
            .map(|name| (name.clone(), Ok(remote.to_owned())))
            .collect();
    }
    match Repository::open(repo_path) {
        Ok(repo) => branch_names
            .iter()
            .map(|name| (name.clone(), remote_resolve::resolve_delete(&repo, name)))
            .collect(),
        Err(error) => branch_names
            .iter()
            .map(|name| (name.clone(), Err(format!("Failed to open repo: {error}"))))
            .collect(),
    }
}

/// Batch-delete remote branches from a plan made by [`resolve_remote_deletes`].
/// Entries that failed to resolve become failure results and are not deleted.
pub fn delete_remotes_planned(
    repo_path: &Path,
    plan: &[(String, Result<String, String>)],
    cancel: &AtomicBool,
) -> Vec<OperationResult> {
    let mut results = Vec::new();
    let mut by_remote: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for (name, target) in plan {
        match target {
            Ok(remote) => by_remote
                .entry(remote.as_str())
                .or_default()
                .push(name.clone()),
            Err(message) => results.push(unresolved_result(
                name,
                BranchAction::DeleteRemoteBranch,
                message.clone(),
            )),
        }
    }
    for (remote, names) in by_remote {
        results.extend(delete_remotes_batch_for_remote(
            repo_path, remote, &names, cancel,
        ));
    }
    results
}

/// Per-branch progress variant of [`delete_remotes_planned`].
pub fn delete_remotes_with_progress_planned(
    repo_path: &Path,
    plan: &[(String, Result<String, String>)],
    prog_tx: &Sender<ProgressUpdate>,
    cancel: &AtomicBool,
) -> Vec<OperationResult> {
    let names: Vec<String> = plan.iter().map(|(name, _)| name.clone()).collect();
    run_with_progress(
        &names,
        BranchAction::DeleteRemoteBranch,
        prog_tx,
        cancel,
        |name| match plan.iter().find(|(planned, _)| planned == name) {
            Some((_, Ok(remote))) => delete_remote(repo_path, remote, name, cancel),
            Some((_, Err(message))) => {
                unresolved_result(name, BranchAction::DeleteRemoteBranch, message.clone())
            }
            None => unresolved_result(
                name,
                BranchAction::DeleteRemoteBranch,
                "No remote resolved".into(),
            ),
        },
    )
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
    delete_remotes_planned(
        repo_path,
        &resolve_remote_deletes(repo_path, None, branch_names),
        cancel,
    )
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

pub fn delete_remotes_with_progress(
    repo_path: &Path,
    branch_names: &[String],
    prog_tx: &Sender<ProgressUpdate>,
    cancel: &AtomicBool,
) -> Vec<OperationResult> {
    delete_remotes_with_progress_planned(
        repo_path,
        &resolve_remote_deletes(repo_path, None, branch_names),
        prog_tx,
        cancel,
    )
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
