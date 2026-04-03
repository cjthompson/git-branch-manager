use crate::types::PrMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};

/// Spawn a background thread that fetches PR info from GitHub.
pub fn spawn_pr_loader(repo_path: PathBuf) -> Receiver<PrMap> {
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        let prs = super::github::fetch_open_prs(&repo_path);
        let _ = tx.send(prs);
    });

    rx
}
