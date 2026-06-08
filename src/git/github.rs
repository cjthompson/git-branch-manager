use crate::types::{PrInfo, PrMap, PrStatus};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;
use std::process::{Command, Stdio};
use tracing::instrument;

#[derive(Deserialize)]
struct PrEntry {
    number: u32,
    #[serde(rename = "headRefName")]
    head_ref_name: String,
    #[serde(rename = "isDraft")]
    is_draft: bool,
    state: String,
}

/// Parse the JSON bytes returned by `gh pr list --json ...` into a `PrMap`.
/// Returns `Err` on malformed or unexpected JSON.
fn parse_pr_list(bytes: &[u8]) -> Result<PrMap> {
    let entries: Vec<PrEntry> =
        serde_json::from_slice(bytes).context("failed to parse gh pr list JSON")?;
    Ok(entries
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
        .collect())
}

/// Fetch open PRs from GitHub using the `gh` CLI.
/// Returns `Ok(map)` on success (an empty map genuinely means "no PRs").
/// Returns `Err` when `gh` fails to spawn, exits non-zero, or returns
/// unparseable JSON.
#[instrument(skip(repo_path))]
pub fn fetch_open_prs_checked(repo_path: &Path) -> Result<PrMap> {
    let out = Command::new("gh")
        .args([
            "pr",
            "list",
            "--json",
            "number,headRefName,isDraft,state",
            "--state",
            "all",
            "--limit",
            "200",
        ])
        .current_dir(repo_path)
        .stdin(Stdio::null())
        .output()
        .context("failed to spawn gh")?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!(
            "gh exited with status {}: {}",
            out.status,
            stderr.trim()
        );
    }

    parse_pr_list(&out.stdout)
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
