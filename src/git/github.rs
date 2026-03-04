use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

/// Mapping of branch name -> PR number.
pub type PrMap = HashMap<String, u32>;

/// Fetch open PRs for the repository using the `gh` CLI.
/// Returns a map of head branch name -> PR number.
/// Returns an empty map if `gh` is not available or not authenticated.
pub fn fetch_open_prs(repo_path: &Path) -> PrMap {
    // Use `gh pr list` to get open PRs with JSON output.
    // This requires `gh` to be installed and authenticated.
    let output = Command::new("gh")
        .args(["pr", "list", "--json", "number,headRefName", "--limit", "200"])
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

    // Parse JSON array of { "number": N, "headRefName": "branch-name" }
    let entries: Vec<PrEntry> = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(_) => return HashMap::new(),
    };

    entries
        .into_iter()
        .map(|e| (e.head_ref_name, e.number))
        .collect()
}

#[derive(serde::Deserialize)]
struct PrEntry {
    number: u32,
    #[serde(rename = "headRefName")]
    head_ref_name: String,
}
