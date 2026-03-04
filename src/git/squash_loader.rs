use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use crate::types::SquashResult;
use super::merge_detection::is_squash_merged;

/// Spawn a background thread that checks each candidate branch for squash-merge
/// status and sends results back one-by-one.
///
/// The channel closes naturally when the thread completes (Sender is dropped).
pub fn spawn_squash_checker(
    repo_path: PathBuf,
    base_branch: String,
    candidates: Vec<String>,
) -> Receiver<SquashResult> {
    let (tx, rx) = mpsc::channel::<SquashResult>();

    thread::spawn(move || {
        for branch_name in candidates {
            let squash_merged = is_squash_merged(&repo_path, &base_branch, &branch_name);
            if tx
                .send(SquashResult {
                    branch_name,
                    is_squash_merged: squash_merged,
                })
                .is_err()
            {
                break; // Receiver dropped (app exited)
            }
        }
    });

    rx
}
