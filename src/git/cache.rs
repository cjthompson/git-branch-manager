use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::types::MergeStatus;

#[derive(Debug, Serialize, Deserialize)]
struct CacheEntry {
    merge_status: String,
    commit_hash: String,
}

#[derive(Debug)]
pub struct BranchCache {
    path: PathBuf,
    entries: HashMap<String, CacheEntry>,
}

impl BranchCache {
    /// Load cache from disk. Returns empty cache if file is missing or corrupt.
    pub fn load(repo_path: &Path) -> Self {
        let path = cache_path(repo_path);
        let entries = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self { path, entries }
    }

    /// Write cache to disk.
    pub fn save(&self) {
        if let Ok(json) = serde_json::to_string(&self.entries) {
            let _ = std::fs::write(&self.path, json);
        }
    }

    /// Look up a branch's cached status. Returns None if not cached or commit hash changed
    /// (for unmerged branches). Merged/squash-merged entries are permanent.
    pub fn lookup(&self, branch_name: &str, current_commit_hash: &str) -> Option<MergeStatus> {
        let entry = self.entries.get(branch_name)?;
        match entry.merge_status.as_str() {
            "merged" => Some(MergeStatus::Merged),
            "squash_merged" => Some(MergeStatus::SquashMerged),
            "unmerged" if entry.commit_hash == current_commit_hash => Some(MergeStatus::Unmerged),
            _ => None,
        }
    }

    /// Insert or update a branch's cached status.
    pub fn insert(&mut self, branch_name: &str, status: &MergeStatus, commit_hash: &str) {
        let status_str = match status {
            MergeStatus::Merged => "merged",
            MergeStatus::SquashMerged => "squash_merged",
            MergeStatus::Unmerged => "unmerged",
        };
        self.entries.insert(
            branch_name.to_string(),
            CacheEntry {
                merge_status: status_str.to_string(),
                commit_hash: commit_hash.to_string(),
            },
        );
    }
}

fn cache_path(repo_path: &Path) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    repo_path.hash(&mut hasher);
    let hash = hasher.finish();
    PathBuf::from(format!("/tmp/git-bm-cache-{:x}.json", hash))
}
