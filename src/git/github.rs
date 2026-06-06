use crate::types::{PrInfo, PrMap, PrStatus};
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

/// Fetch open PRs from GitHub using the `gh` CLI.
/// Returns a map from branch name to PR info.
/// Silently returns empty map if `gh` is not available or not authenticated.
#[instrument(skip(repo_path))]
pub fn fetch_open_prs(repo_path: &Path) -> PrMap {
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
        .output();

    let output = match out {
        Ok(o) if o.status.success() => o.stdout,
        _ => return PrMap::new(),
    };

    let entries: Vec<PrEntry> = match serde_json::from_slice(&output) {
        Ok(e) => e,
        Err(_) => return PrMap::new(),
    };

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
