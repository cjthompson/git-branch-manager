use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use chrono::{DateTime, TimeZone, Utc};
use git2::Repository;

use crate::types::{MergeStatus, WorkingTreeStatus, WorktreeEnrichResult, WorktreeInfo};

/// Run a git command in `dir`, return stdout as String.
fn git_out(dir: &Path, args: &[&str]) -> String {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// Parse `git worktree list --porcelain` output into a list of WorktreeInfo.
/// Uses `repo` to resolve the HEAD commit date for each worktree in-process.
fn parse_porcelain(output: &str, repo: &Repository) -> Vec<WorktreeInfo> {
    // First pass: collect raw worktree data (path, full_sha, branch, is_main)
    let mut raw_worktrees: Vec<(PathBuf, String, Option<String>, bool)> = Vec::new();
    let mut path: Option<PathBuf> = None;
    let mut sha = String::new();
    let mut branch: Option<String> = None;
    let mut is_main = true;

    for line in output.lines() {
        if line.is_empty() {
            if let Some(p) = path.take() {
                raw_worktrees.push((p, sha.clone(), branch.take(), is_main));
                is_main = false;
                sha.clear();
            }
        } else if let Some(rest) = line.strip_prefix("worktree ") {
            path = Some(PathBuf::from(rest));
        } else if let Some(rest) = line.strip_prefix("HEAD ") {
            sha = rest.to_string(); // keep full SHA for commit lookup
        } else if let Some(rest) = line.strip_prefix("branch refs/heads/") {
            branch = Some(rest.to_string());
        }
        // "detached" line — branch stays None
    }

    // flush last block (no trailing blank line in some git versions)
    if let Some(p) = path {
        raw_worktrees.push((p, sha, branch, is_main));
    }

    raw_worktrees
        .into_iter()
        .map(|(p, sha, branch, is_main)| {
            let age_date = git2::Oid::from_str(&sha)
                .ok()
                .and_then(|oid| repo.find_commit(oid).ok())
                .map(|c| {
                    let ts = c.committer().when().seconds();
                    Utc.timestamp_opt(ts, 0).single().unwrap_or_else(Utc::now)
                })
                .unwrap_or_else(Utc::now);
            let short_sha = sha.chars().take(7).collect();
            build_worktree(p, short_sha, branch, is_main, age_date)
        })
        .collect()
}

fn build_worktree(
    path: PathBuf,
    commit_hash: String,
    branch: Option<String>,
    is_main: bool,
    age_date: DateTime<Utc>,
) -> WorktreeInfo {
    WorktreeInfo {
        path,
        branch,
        is_main,
        commit_hash,
        wt_status: WorkingTreeStatus::clean(),
        age_date,
        merge_status: MergeStatus::Unmerged,
        ahead: None,
        behind: None,
        pr: None,
    }
}

/// Compute working tree status and age for a worktree directory.
///
/// Runs `git status --porcelain` in `dir`. If dirty, stats the listed files to
/// find the newest mtime. If clean, reads HEAD commit date via git log.
fn status_and_age(dir: &Path) -> (WorkingTreeStatus, DateTime<Utc>) {
    let status_out = git_out(dir, &["status", "--porcelain"]);

    let mut has_staged = false;
    let mut has_unstaged = false;
    let mut has_untracked = false;
    let mut dirty_paths: Vec<PathBuf> = Vec::new();

    for line in status_out.lines() {
        if line.len() < 3 {
            continue;
        }
        let x = line.chars().next().unwrap_or(' ');
        let y = line.chars().nth(1).unwrap_or(' ');
        let file = {
            let raw = line[3..].trim();
            // Porcelain v1 renames: "new_name -> old_name" — take the destination (new name)
            raw.split(" -> ").next().unwrap_or(raw)
        };

        if x == '?' && y == '?' {
            has_untracked = true;
            // Skip untracked directories (trailing slash) — stat-ing a dir gives
            // the dir's own mtime, not the newest file inside it.
            if !file.ends_with('/') {
                dirty_paths.push(dir.join(file));
            }
        } else {
            let mut pushed = false;
            if x != ' ' && x != '?' {
                has_staged = true;
                dirty_paths.push(dir.join(file));
                pushed = true;
            }
            if y != ' ' && y != '?' {
                has_unstaged = true;
                if !pushed {
                    dirty_paths.push(dir.join(file));
                }
            }
        }
    }

    let wt_status = WorkingTreeStatus { has_staged, has_unstaged, has_untracked };

    let age_date = if wt_status.is_clean() {
        head_commit_date(dir)
    } else {
        newest_mtime(&dirty_paths).unwrap_or_else(|| head_commit_date(dir))
    };

    (wt_status, age_date)
}

fn head_commit_date(dir: &Path) -> DateTime<Utc> {
    let out = git_out(dir, &["log", "-1", "--format=%ct", "HEAD"]);
    out.trim()
        .parse::<i64>()
        .ok()
        .and_then(|ts| Utc.timestamp_opt(ts, 0).single())
        .unwrap_or_else(Utc::now)
}

fn newest_mtime(paths: &[PathBuf]) -> Option<DateTime<Utc>> {
    paths
        .iter()
        .filter_map(|p| {
            std::fs::metadata(p)
                .ok()
                .and_then(|m| m.modified().ok())
                .map(DateTime::<Utc>::from)
        })
        .max()
}

/// List all worktrees for the repo rooted at `repo_path`.
/// Returns phase-1 data only (merge status, ahead/behind, and PR are not populated).
/// HEAD commit dates are resolved in-process via git2 — no subprocess per worktree.
pub fn list_worktrees(repo_path: &Path) -> Vec<WorktreeInfo> {
    let output = git_out(repo_path, &["worktree", "list", "--porcelain"]);
    let Ok(repo) = Repository::open(repo_path) else {
        return Vec::new();
    };
    parse_porcelain(&output, &repo)
}

/// Spawn a background thread that computes `wt_status` and `age_date` for each
/// worktree and sends one [`WorktreeEnrichResult`] per worktree via the returned
/// channel.
///
/// The channel closes naturally when the thread finishes (Sender is dropped).
pub fn enrich_worktrees(worktrees: Vec<WorktreeInfo>) -> Receiver<WorktreeEnrichResult> {
    let (tx, rx) = mpsc::channel::<WorktreeEnrichResult>();

    thread::spawn(move || {
        for (index, worktree) in worktrees.iter().enumerate() {
            let (wt_status, age_date) = status_and_age(&worktree.path);

            if tx
                .send(WorktreeEnrichResult { index, wt_status, age_date })
                .is_err()
            {
                break; // Receiver dropped (app exited)
            }
        }
    });

    rx
}
