use crate::types::PrMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use tracing::{field, info_span, instrument};

/// Spawn a background thread that fetches PR info from GitHub.
#[instrument(fields(repo_path = ?repo_path, result_count = field::Empty))]
pub fn spawn_pr_loader(repo_path: PathBuf) -> Receiver<PrMap> {
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        let worker_span = info_span!(
            "spawn_pr_loader_worker",
            repo_path = ?repo_path,
            result_count = field::Empty,
        );
        let prs = worker_span.in_scope(|| super::github::fetch_open_prs(&repo_path));
        worker_span.record("result_count", prs.len() as u64);
        let _ = tx.send(prs);
    });

    rx
}
