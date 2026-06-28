use crate::types::{PrInfo, PrMap, PrStatus};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;
use std::process::{Command, Stdio};
use tracing::{field, info_span, instrument, Span};

#[derive(Deserialize)]
struct PrEntry {
    number: u32,
    #[serde(rename = "headRefName")]
    head_ref_name: String,
    #[serde(rename = "isDraft")]
    is_draft: bool,
    state: String,
}

const GH_PR_LIST_ARGS: [&str; 8] = [
    "pr",
    "list",
    "--json",
    "number,headRefName,isDraft,state",
    "--state",
    "all",
    "--limit",
    "200",
];

fn parse_pr_entries(bytes: &[u8]) -> Result<Vec<PrEntry>> {
    serde_json::from_slice(bytes).context("failed to parse gh pr list JSON")
}

fn entries_to_pr_map(entries: Vec<PrEntry>) -> PrMap {
    entries
        .into_iter()
        .map(|e| {
            let status = if e.is_draft {
                PrStatus::Draft
            } else if e.state == "MERGED" {
                PrStatus::Merged
            } else if e.state == "CLOSED" {
                PrStatus::Closed
            } else {
                PrStatus::Open
            };
            (
                e.head_ref_name,
                PrInfo {
                    number: e.number,
                    status,
                },
            )
        })
        .collect()
}

/// Parse the JSON bytes returned by `gh pr list --json ...` into a `PrMap`.
/// Returns `Err` on malformed or unexpected JSON.
#[cfg(test)]
fn parse_pr_list(bytes: &[u8]) -> Result<PrMap> {
    parse_pr_entries(bytes).map(entries_to_pr_map)
}

/// Fetch open PRs from GitHub using the `gh` CLI.
/// Returns `Ok(map)` on success (an empty map genuinely means "no PRs").
/// Returns `Err` when `gh` fails to spawn, exits non-zero, or returns
/// unparseable JSON.
#[instrument(
    skip(repo_path),
    fields(
        repo_path = ?repo_path,
        command = "gh",
        args = ?GH_PR_LIST_ARGS,
        exit_code = field::Empty,
        stdout_bytes = field::Empty,
        stderr_bytes = field::Empty,
        parsed_pr_count = field::Empty,
        draft_count = field::Empty,
        open_count = field::Empty,
        merged_count = field::Empty,
        closed_count = field::Empty,
        parse_result = field::Empty,
        result_state = field::Empty,
    )
)]
pub fn fetch_open_prs_checked(repo_path: &Path) -> Result<PrMap> {
    let span = Span::current();
    let command_span = info_span!(
        "fetch_open_prs_command",
        repo_path = ?repo_path,
        command = "gh",
        args = ?GH_PR_LIST_ARGS,
        exit_code = field::Empty,
        stdout_bytes = field::Empty,
        stderr_bytes = field::Empty,
        success = field::Empty,
        result_state = field::Empty,
    );
    let out = {
        let _entered = command_span.enter();
        Command::new("gh")
            .args(GH_PR_LIST_ARGS)
            .current_dir(repo_path)
            .stdin(Stdio::null())
            .output()
    };

    let output = match out {
        Ok(output) if output.status.success() => {
            let exit_code = output.status.code().map(i64::from).unwrap_or(-1);
            command_span.record("exit_code", exit_code);
            command_span.record("stdout_bytes", output.stdout.len() as u64);
            command_span.record("stderr_bytes", output.stderr.len() as u64);
            command_span.record("success", true);
            command_span.record("result_state", "success");
            span.record("exit_code", exit_code);
            span.record("stdout_bytes", output.stdout.len() as u64);
            span.record("stderr_bytes", output.stderr.len() as u64);
            output.stdout
        }
        Ok(output) => {
            let exit_code = output.status.code().map(i64::from).unwrap_or(-1);
            command_span.record("exit_code", exit_code);
            command_span.record("stdout_bytes", output.stdout.len() as u64);
            command_span.record("stderr_bytes", output.stderr.len() as u64);
            command_span.record("success", false);
            command_span.record("result_state", "nonzero_exit");
            span.record("exit_code", exit_code);
            span.record("stdout_bytes", output.stdout.len() as u64);
            span.record("stderr_bytes", output.stderr.len() as u64);
            span.record("parsed_pr_count", 0);
            span.record("draft_count", 0);
            span.record("open_count", 0);
            span.record("merged_count", 0);
            span.record("closed_count", 0);
            span.record("parse_result", "skipped");
            span.record("result_state", "nonzero_exit");

            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("gh exited with status {}: {}", output.status, stderr.trim());
        }
        Err(err) => {
            command_span.record("success", false);
            command_span.record("result_state", "spawn_error");
            span.record("parsed_pr_count", 0);
            span.record("draft_count", 0);
            span.record("open_count", 0);
            span.record("merged_count", 0);
            span.record("closed_count", 0);
            span.record("parse_result", "skipped");
            span.record("result_state", "spawn_error");
            return Err(err).context("failed to spawn gh");
        }
    };

    let parse_span = info_span!(
        "fetch_open_prs_parse_json",
        stdout_bytes = output.len() as u64,
        parsed_pr_count = field::Empty,
        result_state = field::Empty,
    );
    let entries = match parse_span.in_scope(|| parse_pr_entries(&output)) {
        Ok(entries) => {
            parse_span.record("parsed_pr_count", entries.len() as u64);
            parse_span.record("result_state", "success");
            span.record("parsed_pr_count", entries.len() as u64);
            span.record("parse_result", "success");
            entries
        }
        Err(err) => {
            parse_span.record("parsed_pr_count", 0);
            parse_span.record("result_state", "parse_error");
            span.record("parsed_pr_count", 0);
            span.record("draft_count", 0);
            span.record("open_count", 0);
            span.record("merged_count", 0);
            span.record("closed_count", 0);
            span.record("parse_result", "parse_error");
            span.record("result_state", "parse_error");
            return Err(err);
        }
    };

    let mut draft_count = 0usize;
    let mut open_count = 0usize;
    let mut merged_count = 0usize;
    let mut closed_count = 0usize;
    for entry in &entries {
        if entry.is_draft {
            draft_count += 1;
        }
        match entry.state.as_str() {
            "MERGED" => merged_count += 1,
            "CLOSED" => closed_count += 1,
            _ => open_count += 1,
        }
    }
    span.record("draft_count", draft_count as u64);
    span.record("open_count", open_count as u64);
    span.record("merged_count", merged_count as u64);
    span.record("closed_count", closed_count as u64);
    span.record("result_state", "success");

    Ok(entries_to_pr_map(entries))
}

/// Fetch open PRs from GitHub using the `gh` CLI.
/// Returns a map from branch name to PR info.
/// Silently returns empty map if `gh` is not available or not authenticated.
#[instrument(skip(repo_path))]
pub fn fetch_open_prs(repo_path: &Path) -> PrMap {
    fetch_open_prs_checked(repo_path).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pr_list_invalid_json_returns_err() {
        let err = parse_pr_list(b"not valid json");
        assert!(err.is_err(), "expected Err for invalid JSON");
    }

    #[test]
    fn parse_pr_list_wrong_shape_returns_err() {
        // Valid JSON but not an array of PrEntry objects
        let err = parse_pr_list(b"{\"foo\": 1}");
        assert!(err.is_err(), "expected Err for wrong-shape JSON");
    }

    #[test]
    fn parse_pr_list_empty_array_returns_empty_map() {
        let map = parse_pr_list(b"[]").expect("empty array should parse");
        assert!(map.is_empty());
    }

    #[test]
    fn parse_pr_list_valid_entries() {
        let json = br#"[
            {"number": 42, "headRefName": "feat/foo", "isDraft": false, "state": "OPEN"},
            {"number": 7,  "headRefName": "feat/bar", "isDraft": true,  "state": "OPEN"},
            {"number": 99, "headRefName": "feat/baz", "isDraft": false, "state": "MERGED"}
        ]"#;
        let map = parse_pr_list(json).expect("valid JSON should parse");
        assert_eq!(map.len(), 3);

        let foo = map.get("feat/foo").expect("feat/foo missing");
        assert_eq!(foo.number, 42);
        assert!(matches!(foo.status, PrStatus::Open));

        let bar = map.get("feat/bar").expect("feat/bar missing");
        assert!(matches!(bar.status, PrStatus::Draft));

        let baz = map.get("feat/baz").expect("feat/baz missing");
        assert!(matches!(baz.status, PrStatus::Merged));
    }
}
