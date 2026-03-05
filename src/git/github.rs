use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrStatus {
    Draft,
    Open,
    Merged,
    Closed,
}

#[derive(Debug, Clone)]
pub struct PrInfo {
    pub number: u32,
    pub status: PrStatus,
}

/// Mapping of branch name -> PrInfo.
pub type PrMap = HashMap<String, PrInfo>;

/// Fetch PRs for the repository using the `gh` CLI.
/// Returns a map of head branch name -> PrInfo.
/// Returns an empty map if `gh` is not available or not authenticated.
pub fn fetch_open_prs(repo_path: &Path) -> PrMap {
    // Use `gh pr list` to get PRs with JSON output.
    // This requires `gh` to be installed and authenticated.
    let output = Command::new("gh")
        .args(["pr", "list", "--json", "number,headRefName,isDraft,state", "--state", "all", "--limit", "200"])
        .current_dir(repo_path)
        .output();

    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => return HashMap::new(),
    };

    let json_str = match std::str::from_utf8(&output.stdout) {
        Ok(s) => s,
        Err(_) => return HashMap::new(),
    };

    // Parse JSON array of { "number": N, "headRefName": "branch-name", "isDraft": bool, "state": "..." }
    let entries: Vec<PrEntry> = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(_) => return HashMap::new(),
    };

    entries
        .into_iter()
        .map(|e| {
            let status = if e.is_draft {
                PrStatus::Draft
            } else {
                match e.state.as_str() {
                    "MERGED" => PrStatus::Merged,
                    "CLOSED" => PrStatus::Closed,
                    _ => PrStatus::Open,
                }
            };
            (e.head_ref_name, PrInfo { number: e.number, status })
        })
        .collect()
}

#[derive(serde::Deserialize)]
struct PrEntry {
    number: u32,
    #[serde(rename = "headRefName")]
    head_ref_name: String,
    #[serde(rename = "isDraft")]
    is_draft: bool,
    state: String,
}
