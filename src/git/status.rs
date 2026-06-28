use crate::types::WorkingTreeStatus;
use git2::{Repository, StatusOptions};
use tracing::instrument;

#[instrument(skip(repo))]
pub fn detect_working_tree_status(repo: &Repository) -> WorkingTreeStatus {
    let mut opts = StatusOptions::new();
    opts.include_untracked(true).recurse_untracked_dirs(false);

    let statuses = match repo.statuses(Some(&mut opts)) {
        Ok(s) => s,
        Err(_) => return WorkingTreeStatus::clean(),
    };

    let mut has_staged = false;
    let mut has_modified = false;
    let mut has_untracked = false;

    for entry in statuses.iter() {
        let s = entry.status();
        if s.intersects(
            git2::Status::INDEX_NEW
                | git2::Status::INDEX_MODIFIED
                | git2::Status::INDEX_DELETED
                | git2::Status::INDEX_RENAMED
                | git2::Status::INDEX_TYPECHANGE,
        ) {
            has_staged = true;
        }
        if s.intersects(
            git2::Status::WT_MODIFIED
                | git2::Status::WT_DELETED
                | git2::Status::WT_RENAMED
                | git2::Status::WT_TYPECHANGE,
        ) {
            has_modified = true;
        }
        if s.contains(git2::Status::WT_NEW) {
            has_untracked = true;
        }
    }

    WorkingTreeStatus {
        has_staged,
        has_modified,
        has_untracked,
    }
}
