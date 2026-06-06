use crate::types::{MergeStatus, WorkingTreeStatus, WorktreeEnrichResult, WorktreeInfo};
use chrono::{DateTime, TimeZone, Utc};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver};
use tracing::instrument;

fn git_out(dir: &Path, args: &[&str]) -> String {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .stdin(std::process::Stdio::null())
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// List all worktrees for the repository using `git worktree list --porcelain`.
#[instrument(skip(repo_path))]
pub fn list_worktrees(repo_path: &Path) -> Vec<WorktreeInfo> {
    let output = git_out(repo_path, &["worktree", "list", "--porcelain"]);
    if output.is_empty() {
        return vec![];
    }

    let mut worktrees = Vec::new();
    let mut current_path: Option<PathBuf> = None;
    let mut current_hash = String::new();
    let mut current_branch: Option<String> = None;
    let mut is_first = true;

    for line in output.lines() {
        if let Some(path_str) = line.strip_prefix("worktree ") {
            // Flush previous entry
            if let Some(path) = current_path.take() {
                let wt = build_worktree(
                    path,
                    std::mem::take(&mut current_hash),
                    current_branch.take(),
                    is_first,
                );
                worktrees.push(wt);
                is_first = false;
            }
            current_path = Some(PathBuf::from(path_str));
            current_hash.clear();
            current_branch = None;
        } else if let Some(hash) = line.strip_prefix("HEAD ") {
            current_hash = hash[..7.min(hash.len())].to_string();
        } else if let Some(branch_ref) = line.strip_prefix("branch ") {
            current_branch = branch_ref
                .strip_prefix("refs/heads/")
                .map(|s| s.to_string());
        } else if line == "detached" {
            current_branch = None;
        }
    }

    // Don't forget the last entry
    if let Some(path) = current_path {
        let wt = build_worktree(path, current_hash, current_branch, is_first);
        worktrees.push(wt);
    }

    worktrees
}

/// Spawn a background thread that enriches worktrees with working tree status and age.
#[instrument(skip(worktrees), fields(count = worktrees.len()))]
pub fn enrich_worktrees(worktrees: Vec<WorktreeInfo>) -> Receiver<WorktreeEnrichResult> {
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        for (index, wt) in worktrees.iter().enumerate() {
            let (wt_status, age_date) = status_and_age(&wt.path);
            if tx
                .send(WorktreeEnrichResult {
                    index,
                    wt_status,
                    age_date,
                })
                .is_err()
            {
                return;
            }
        }
    });

    rx
}

fn build_worktree(
    path: PathBuf,
    commit_hash: String,
    branch: Option<String>,
    is_main: bool,
) -> WorktreeInfo {
    WorktreeInfo {
        path,
        branch,
        is_main,
        commit_hash,
        wt_status: WorkingTreeStatus::clean(),
        age_date: Utc::now(),
        merge_status: MergeStatus::Unmerged,
        ahead: None,
        behind: None,
        pr: None,
    }
}

fn status_and_age(dir: &Path) -> (WorkingTreeStatus, DateTime<Utc>) {
    let output = git_out(dir, &["status", "--porcelain"]);
    let mut has_staged = false;
    let mut has_unstaged = false;
    let mut has_untracked = false;

    for line in output.lines() {
        let bytes = line.as_bytes();
        if bytes.len() < 2 {
            continue;
        }
        let index = bytes[0];
        let work = bytes[1];

        if index == b'?' {
            has_untracked = true;
            continue;
        }
        if index != b' ' && index != b'?' {
            has_staged = true;
        }
        if work != b' ' && work != b'?' {
            has_unstaged = true;
        }
    }

    let status = WorkingTreeStatus {
        has_staged,
        has_unstaged,
        has_untracked,
    };
    let age = head_commit_date(dir);
    (status, age)
}

fn head_commit_date(dir: &Path) -> DateTime<Utc> {
    let output = git_out(dir, &["log", "-1", "--format=%ct", "HEAD"]);
    output
        .parse::<i64>()
        .ok()
        .and_then(|ts| Utc.timestamp_opt(ts, 0).single())
        .unwrap_or_else(Utc::now)
}
