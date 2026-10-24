mod local;
mod remote;
mod squash_message;
mod sync;
mod worktree_ops;

pub use local::{
    checkout_branch, cherry_pick_local, delete_local, delete_local_force,
    delete_locals_with_progress, fetch, fetch_prune, fetch_sync,
};
pub use remote::{
    checkout_remote_branch, cherry_pick_remote, delete_remotes_batch,
    delete_remotes_batch_for_remote, delete_remotes_with_progress,
    delete_remotes_with_progress_for_remote, fetch_remote, merge_remote_into_current, pull_remote,
};
pub use sync::{
    fast_forward, force_push_branch, merge_branch, pull_branch, push_branch, rebase_branch,
};
pub use worktree_ops::{create_worktree, force_remove_worktree, remove_worktree};

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::Duration;

use crate::types::{BranchAction, OperationResult, ProgressUpdate};
use std::path::Path;
use std::process::{Command, Stdio};

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
