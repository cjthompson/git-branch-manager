use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use super::github::PrMap;

/// Spawn a background thread that fetches PR data from GitHub.
/// Returns a Receiver that will receive exactly one PrMap when the fetch completes.
pub fn spawn_pr_loader(repo_path: PathBuf) -> Receiver<PrMap> {
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        let prs = super::github::fetch_open_prs(&repo_path);
        let _ = tx.send(prs);
    });

    rx
}
