use crate::types::{MergeStatus, WorkingTreeStatus, WorktreeEnrichResult, WorktreeInfo};
use chrono::{DateTime, TimeZone, Utc};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::mpsc::{self, Receiver};
use tracing::{field, info_span, instrument, Span};

fn git_command_output(dir: &Path, args: &[&str]) -> Option<Output> {
    let span = info_span!(
        "git_command",
        dir = ?dir,
        command = "git",
        args = ?args,
        exit_code = field::Empty,
        stdout_bytes = field::Empty,
        stderr_bytes = field::Empty,
        success = field::Empty,
        result_state = field::Empty,
    );
    let output = {
        let _entered = span.enter();
        Command::new("git")
            .args(args)
            .current_dir(dir)
            .stdin(std::process::Stdio::null())
            .env("GIT_OPTIONAL_LOCKS", "0")
            .output()
    };

    match output {
        Ok(output) if output.status.success() => {
            span.record(
                "exit_code",
                output.status.code().map(i64::from).unwrap_or(-1),
            );
            span.record("stdout_bytes", output.stdout.len() as u64);
            span.record("stderr_bytes", output.stderr.len() as u64);
            span.record("success", true);
            span.record("result_state", "success");
            Some(output)
        }
        Ok(output) => {
            span.record(
                "exit_code",
                output.status.code().map(i64::from).unwrap_or(-1),
            );
            span.record("stdout_bytes", output.stdout.len() as u64);
            span.record("stderr_bytes", output.stderr.len() as u64);
            span.record("success", false);
            span.record("result_state", "nonzero_exit");
            None
        }
        Err(_) => {
            span.record("success", false);
            span.record("result_state", "spawn_error");
            None
        }
    }
}

fn git_out(dir: &Path, args: &[&str]) -> String {
    git_command_output(dir, args)
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// List all worktrees for the repository using `git worktree list --porcelain`.
#[instrument(
    skip(repo_path),
    fields(
        repo_path = ?repo_path,
        command = "git",
        args = ?["worktree", "list", "--porcelain"],
        stdout_bytes = field::Empty,
        parsed_worktree_count = field::Empty,
        parse_result = field::Empty,
        result_state = field::Empty,
    )
)]
pub fn list_worktrees(repo_path: &Path) -> Vec<WorktreeInfo> {
    let span = Span::current();
    let output = match git_command_output(repo_path, &["worktree", "list", "--porcelain"]) {
        Some(output) => output,
        None => {
            span.record("stdout_bytes", 0);
            span.record("parsed_worktree_count", 0);
            span.record("parse_result", "skipped");
            span.record("result_state", "command_failed");
            return vec![];
        }
    };
    span.record("stdout_bytes", output.stdout.len() as u64);

    let output = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if output.is_empty() {
        span.record("parsed_worktree_count", 0);
        span.record("parse_result", "empty");
        span.record("result_state", "empty");
        return vec![];
    }

    let parse_span = info_span!(
        "list_worktrees_parse",
        stdout_bytes = output.len() as u64,
        parsed_worktree_count = field::Empty,
        result_state = field::Empty,
    );
    let _parse_entered = parse_span.enter();

    let mut worktrees = Vec::new();
    let mut current_path: Option<PathBuf> = None;
    let mut current_hash = String::new();
    let mut current_branch: Option<String> = None;
    let mut is_first = true;

    for line in output.lines() {
        if let Some(path_str) = line.strip_prefix("worktree ") {
            // Flush previous entry
            if let Some(path) = current_path.take() {
                let path_for_span = path.clone();
                let branch_name = current_branch
                    .clone()
                    .unwrap_or_else(|| "(detached)".to_string());
                let wt = info_span!(
                    "list_worktrees_parse_entry",
                    path = ?path_for_span,
                    branch = branch_name.as_str(),
                    head = current_hash.as_str(),
                    is_main = is_first,
                )
                .in_scope(|| {
                    build_worktree(
                        path,
                        std::mem::take(&mut current_hash),
                        current_branch.take(),
                        is_first,
                    )
                });
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
        let path_for_span = path.clone();
        let branch_name = current_branch
            .clone()
            .unwrap_or_else(|| "(detached)".to_string());
        let wt = info_span!(
            "list_worktrees_parse_entry",
            path = ?path_for_span,
            branch = branch_name.as_str(),
            head = current_hash.as_str(),
            is_main = is_first,
        )
        .in_scope(|| build_worktree(path, current_hash, current_branch, is_first));
        worktrees.push(wt);
    }

    parse_span.record("parsed_worktree_count", worktrees.len() as u64);
    parse_span.record("result_state", "success");
    span.record("parsed_worktree_count", worktrees.len() as u64);
    span.record("parse_result", "success");
    span.record("result_state", "success");
    worktrees
}

/// Spawn a background thread that enriches worktrees with working tree status and age.
#[instrument(skip(worktrees), fields(count = worktrees.len()))]
pub fn enrich_worktrees(worktrees: Vec<WorktreeInfo>) -> Receiver<WorktreeEnrichResult> {
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        let worker_count = worktrees.len();
        let worker_span = info_span!(
            "enrich_worktrees_worker",
            count = worker_count,
            worker_count,
            join_error_count = field::Empty,
        );
        let _worker_entered = worker_span.enter();
        let mut handles = Vec::with_capacity(worker_count);
        for (index, wt) in worktrees.into_iter().enumerate() {
            let tx = tx.clone();
            handles.push(std::thread::spawn(move || {
                let (wt_status, age_date) = info_span!(
                    "enrich_worktree_entry",
                    index,
                    path = ?wt.path,
                    branch = wt.branch.as_deref().unwrap_or("(detached)"),
                    is_main = wt.is_main,
                )
                .in_scope(|| status_and_age(&wt.path));
                let _ = tx.send(WorktreeEnrichResult {
                    index,
                    wt_status,
                    age_date,
                });
            }));
        }
        drop(tx);

        let mut join_error_count = 0usize;
        for handle in handles {
            if handle.join().is_err() {
                join_error_count += 1;
            }
        }
        worker_span.record("join_error_count", join_error_count);
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

#[instrument(skip(dir), fields(path = ?dir))]
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

#[instrument(skip(dir), fields(path = ?dir))]
fn head_commit_date(dir: &Path) -> DateTime<Utc> {
    let output = git_out(dir, &["log", "-1", "--format=%ct", "HEAD"]);
    output
        .parse::<i64>()
        .ok()
        .and_then(|ts| Utc.timestamp_opt(ts, 0).single())
        .unwrap_or_else(Utc::now)
}
