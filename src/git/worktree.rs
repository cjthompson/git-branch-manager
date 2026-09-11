use crate::types::{
    BranchInfo, MergeStatus, WorkingTreeStatus, WorktreeEnrichResult, WorktreeInfo,
};
use chrono::{DateTime, TimeZone, Utc};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::mpsc::{self, Receiver};
use tracing::{field, info_span, instrument, Span};

fn git_command_output(dir: &Path, args: &[&str]) -> Result<Output, String> {
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
            Ok(output)
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
            Err(format!(
                "git {:?} exited with {}: {}",
                args,
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ))
        }
        Err(error) => {
            span.record("success", false);
            span.record("result_state", "spawn_error");
            Err(format!("failed to run git {:?}: {error}", args))
        }
    }
}

fn git_out(dir: &Path, args: &[&str]) -> String {
    git_command_output(dir, args)
        .ok()
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
pub fn try_list_worktrees(repo_path: &Path) -> Result<Vec<WorktreeInfo>, String> {
    let span = Span::current();
    let output = match git_command_output(repo_path, &["worktree", "list", "--porcelain"]) {
        Ok(output) => output,
        Err(error) => {
            span.record("stdout_bytes", 0);
            span.record("parsed_worktree_count", 0);
            span.record("parse_result", "skipped");
            span.record("result_state", "command_failed");
            return Err(error);
        }
    };
    span.record("stdout_bytes", output.stdout.len() as u64);

    let output = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if output.is_empty() {
        span.record("parsed_worktree_count", 0);
        span.record("parse_result", "empty");
        span.record("result_state", "empty");
        return Ok(vec![]);
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
    Ok(worktrees)
}

/// Best-effort compatibility wrapper for display paths that can recover from
/// a missing worktree list by rendering no rows. Delete pre-flight and worker
/// recovery paths use [`try_list_worktrees`] so command failures stay visible.
pub fn list_worktrees(repo_path: &Path) -> Vec<WorktreeInfo> {
    try_list_worktrees(repo_path).unwrap_or_default()
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

/// Copy each worktree's merge status from its checked-out branch.
///
/// Matches by short branch name. Detached-HEAD worktrees (`branch == None`) and
/// any worktree whose branch has no entry in `branches` keep their existing
/// default (`Unmerged`). Pure and synchronous — callers already hold the
/// enriched branch list, so this never recomputes merge detection.
pub fn apply_branch_merge_status(worktrees: &mut [WorktreeInfo], branches: &[BranchInfo]) {
    for wt in worktrees.iter_mut() {
        if let Some(name) = wt.branch.as_deref() {
            if let Some(b) = branches.iter().find(|b| b.name == name) {
                wt.merge_status = b.merge_status;
                // The base-branch worktree has no meaningful merge status (a
                // branch can't be merged into itself); the renderer blanks it.
                wt.is_base = b.is_base;
            }
        }
    }
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
        is_base: false,
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
    let status = super::status::detect_working_tree_status(dir);
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

/// Short names of every branch checked out in a non-**main** worktree — this
/// filters by `!is_main`, not by "not the caller's own worktree". If the
/// caller itself is running from a linked (non-main) worktree, that
/// worktree's own branch is still included here. Callers that need to
/// exclude the caller's own worktree specifically (e.g. destructive delete
/// pre-flight) should use [`try_other_worktree_for_branch`] instead.
pub fn branches_checked_out_in_worktrees(repo_path: &Path) -> HashSet<String> {
    list_worktrees(repo_path)
        .into_iter()
        .filter(|worktree| !worktree.is_main)
        .filter_map(|worktree| worktree.branch)
        .collect()
}

/// True when `worktree_path` refers to the same on-disk worktree as
/// `repo_path` — i.e. the worktree the caller itself is running from.
/// Canonicalizes both sides so relative segments, symlinks (e.g. macOS's
/// `/var` -> `/private/var`), and trailing slashes don't produce false
/// negatives. Falls back to a direct comparison if either path can't be
/// canonicalized (e.g. it no longer exists on disk).
pub fn is_current_worktree(repo_path: &Path, worktree_path: &Path) -> bool {
    match (repo_path.canonicalize(), worktree_path.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => repo_path == worktree_path,
    }
}

/// Find the worktree that has `branch` checked out, including the primary
/// worktree **and** the caller's own worktree at `repo_path` if it matches.
/// The `Result` is important for destructive pre-flight: an unavailable
/// worktree listing must not look like a branch is unowned. Use
/// [`try_other_worktree_for_branch`] when the caller's own worktree should
/// be excluded from the match (e.g. "is this checked out somewhere I could
/// act on", not "is this checked out at all").
pub fn try_worktree_for_branch(
    repo_path: &Path,
    branch: &str,
) -> Result<Option<WorktreeInfo>, String> {
    try_list_worktrees(repo_path).map(|worktrees| {
        worktrees
            .into_iter()
            .find(|worktree| worktree.branch.as_deref() == Some(branch))
    })
}

/// Find a worktree — other than the caller's own — that has `branch`
/// checked out. Unlike [`try_worktree_for_branch`], the worktree located at
/// `repo_path` itself is excluded, so this answers "is this branch checked
/// out somewhere else I could recover from", not "is this branch checked
/// out at all". Used by destructive delete pre-flight: a branch checked out
/// in the caller's *own* worktree isn't a recoverable-elsewhere case (you
/// can't remove the worktree you're running from), so it must not be
/// reported as `CheckedOutInWorktree`.
pub fn try_other_worktree_for_branch(
    repo_path: &Path,
    branch: &str,
) -> Result<Option<WorktreeInfo>, String> {
    try_list_worktrees(repo_path).map(|worktrees| {
        worktrees.into_iter().find(|worktree| {
            worktree.branch.as_deref() == Some(branch)
                && !is_current_worktree(repo_path, &worktree.path)
        })
    })
}

/// Best-effort path lookup for "is `branch` checked out in some *other*
/// worktree" (excludes the caller's own worktree at `repo_path` — see
/// [`try_other_worktree_for_branch`]). Retained for non-destructive callers
/// that only need a path and can tolerate collapsing lookup failure into
/// `None`. Destructive/recovery paths should call
/// [`try_other_worktree_for_branch`] directly when they need to distinguish
/// "not checked out elsewhere" from "could not inspect worktrees".
pub fn worktree_path_for_branch(repo_path: &Path, branch: &str) -> Option<PathBuf> {
    try_other_worktree_for_branch(repo_path, branch)
        .ok()
        .flatten()
        .map(|worktree| worktree.path)
}
