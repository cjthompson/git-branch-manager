use super::Result;
use crate::types::*;
use chrono::{TimeZone, Utc};
use git2::{BranchType, Repository};
use std::collections::HashMap;
use std::process::Command;
use std::sync::mpsc;
use tracing::{field, info_span, instrument};

/// List remote branches with basic metadata (phase 1).
#[instrument(skip(repo), fields(base_branch))]
pub fn list_remote_branches_phase1(
    repo: &Repository,
    base_branch: &str,
) -> Result<Vec<RemoteBranchInfo>> {
    let mut remotes = Vec::new();
    let branches = repo.branches(Some(BranchType::Remote))?;

    for branch_result in branches {
        let (branch, _) = branch_result?;
        let name = match branch.name()? {
            Some(n) => n.to_string(),
            None => continue,
        };

        // Skip HEAD pseudo-refs
        if name.ends_with("/HEAD") {
            continue;
        }

        let (remote, short_name) = match name.split_once('/') {
            Some((r, s)) => (r.to_string(), s.to_string()),
            None => continue,
        };

        let commit = branch.get().peel_to_commit()?;
        let time = commit.committer().when();
        let date = Utc
            .timestamp_opt(time.seconds(), 0)
            .single()
            .unwrap_or_else(Utc::now);

        let has_local = repo.find_branch(&short_name, BranchType::Local).is_ok();
        let is_base = short_name == base_branch;

        remotes.push(RemoteBranchInfo {
            full_ref: name,
            remote,
            short_name,
            has_local,
            is_base,
            last_commit_date: date,
            merge_status: if is_base {
                MergeStatus::Merged
            } else {
                MergeStatus::Pending
            },
            ahead: None,
            behind: None,
            disjoint: false,
            pr: None,
            squash_confidence: None,
        });
    }

    remotes.sort_by(|a, b| {
        b.is_pinned()
            .cmp(&a.is_pinned())
            .then(b.last_commit_date.cmp(&a.last_commit_date))
    });

    Ok(remotes)
}

/// Spawn a background thread that enriches remote branches with ahead/behind and merge status.
#[instrument(skip(branches), fields(base_branch, branch_count = branches.len()))]
pub fn spawn_remote_enricher(
    repo_path: std::path::PathBuf,
    base_branch: String,
    branches: Vec<RemoteBranchInfo>,
) -> mpsc::Receiver<RemoteEnrichResult> {
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        let worker_span = info_span!(
            "remote_enricher_worker",
            branch_count = branches.len(),
            result_count = field::Empty,
            ahead_behind_count = field::Empty,
            merged_count = field::Empty,
            skipped_base_count = field::Empty,
            missing_ahead_behind_count = field::Empty,
            result_state = field::Empty,
        );
        let _worker_entered = worker_span.enter();

        let repo = match Repository::open(&repo_path) {
            Ok(r) => r,
            Err(_) => {
                worker_span.record("result_state", "open_repo_error");
                return;
            }
        };

        // Prefer local base ref, fall back to remote tracking.
        let base_oid = repo
            .find_branch(&base_branch, BranchType::Local)
            .or_else(|_| repo.find_branch(&format!("origin/{base_branch}"), BranchType::Remote))
            .and_then(|b| {
                b.get()
                    .target()
                    .ok_or_else(|| git2::Error::from_str("no target"))
            })
            .ok();

        let base_oid = match base_oid {
            Some(oid) => oid,
            None => {
                worker_span.record("result_state", "base_oid_missing");
                return;
            }
        };
        let base_oid = base_oid.to_string();

        let ahead_behind_format = format!("%(refname:short)%09%(ahead-behind:{base_oid})");
        let ahead_behind_output = match remote_enricher_git_stdout(
            &repo_path,
            &[
                "for-each-ref",
                "refs/remotes",
                "--format",
                &ahead_behind_format,
            ],
        ) {
            Some(output) => output,
            None => {
                worker_span.record("result_state", "ahead_behind_command_error");
                return;
            }
        };
        let ahead_behind = parse_remote_ahead_behind(&ahead_behind_output);
        worker_span.record("ahead_behind_count", ahead_behind.len());

        let merged_count = ahead_behind
            .values()
            .filter(|(ahead, _)| *ahead == Some(0))
            .count();
        worker_span.record("merged_count", merged_count);

        // Total commits reachable from base. A remote whose `behind` equals this
        // shares no commit with base (none of base's commits are its ancestors) —
        // i.e. a disjoint history. One cheap command, reused for every ref.
        let base_total: Option<u32> =
            remote_enricher_git_stdout(&repo_path, &["rev-list", "--count", &base_oid])
                .and_then(|s| s.trim().parse::<u32>().ok());

        let mut result_count = 0usize;
        let mut skipped_base_count = 0usize;
        let mut missing_ahead_behind_count = 0usize;
        for branch in branches {
            if branch.is_base {
                skipped_base_count += 1;
                continue;
            }

            let Some((ahead, behind)) = ahead_behind.get(&branch.full_ref).copied() else {
                missing_ahead_behind_count += 1;
                continue;
            };
            let merge_status = if ahead == Some(0) {
                MergeStatus::Merged
            } else {
                MergeStatus::Unmerged
            };
            // Disjoint: base contributes none of its commits as ancestors, so the
            // ref is behind by all of base. ahead/behind are then full history sizes.
            let disjoint = base_total.is_some() && behind == base_total;

            let result = RemoteEnrichResult {
                full_ref: branch.full_ref,
                merge_status,
                ahead,
                behind,
                disjoint,
            };
            if tx.send(result).is_err() {
                worker_span.record("result_state", "receiver_dropped");
                return;
            }
            result_count += 1;
        }

        worker_span.record("result_count", result_count);
        worker_span.record("skipped_base_count", skipped_base_count);
        worker_span.record("missing_ahead_behind_count", missing_ahead_behind_count);
        worker_span.record("result_state", "success");
    });

    rx
}

fn remote_enricher_git_stdout(repo_path: &std::path::Path, args: &[&str]) -> Option<String> {
    let span = info_span!(
        "remote_enricher_git_command",
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
            .current_dir(repo_path)
            .stdin(std::process::Stdio::null())
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
            Some(String::from_utf8_lossy(&output.stdout).to_string())
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

fn parse_remote_ahead_behind(output: &str) -> HashMap<String, (Option<u32>, Option<u32>)> {
    output
        .lines()
        .filter_map(|line| {
            let (name, counts) = line.split_once('\t')?;
            let mut counts = counts.split_whitespace();
            let ahead = counts.next()?.parse::<u32>().ok();
            let behind = counts.next()?.parse::<u32>().ok();
            Some((name.to_string(), (ahead, behind)))
        })
        .collect()
}
