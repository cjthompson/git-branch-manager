use crate::types::MergeStatus;
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use tracing::instrument;

#[derive(Debug, Serialize, Deserialize)]
struct CacheEntry {
    merge_status: String,
    commit_hash: String,
}

pub struct BranchCache {
    path: PathBuf,
    entries: HashMap<String, CacheEntry>,
}

impl BranchCache {
    #[instrument(skip(repo_path), fields(path = ?repo_path))]
    pub fn load(repo_path: &Path) -> Self {
        let path = cache_path(repo_path);
        let entries = fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self { path, entries }
    }

    pub fn save(&self) {
        if let Ok(json) = serde_json::to_string(&self.entries) {
            let _ = fs::write(&self.path, json);
        }
    }

    pub fn lookup(&self, branch_name: &str, current_commit_hash: &str) -> Option<MergeStatus> {
        let entry = self.entries.get(branch_name)?;
        let status = match entry.merge_status.as_str() {
            "merged" => MergeStatus::Merged,
            "squash_merged" => MergeStatus::SquashMerged,
            "unmerged" => MergeStatus::Unmerged,
            _ => return None,
        };
        match status {
            // Merged and SquashMerged are permanent
            MergeStatus::Merged | MergeStatus::SquashMerged => Some(status),
            // Unmerged is only valid if commit hasn't changed
            MergeStatus::Unmerged => {
                if entry.commit_hash == current_commit_hash {
                    Some(status)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    pub fn insert(&mut self, branch_name: &str, status: &MergeStatus, commit_hash: &str) {
        let status_str = match status {
            MergeStatus::Merged => "merged",
            MergeStatus::SquashMerged => "squash_merged",
            MergeStatus::Unmerged => "unmerged",
            MergeStatus::Pending => return, // Never cache Pending
        };
        self.entries.insert(
            branch_name.to_string(),
            CacheEntry {
                merge_status: status_str.to_string(),
                commit_hash: commit_hash.to_string(),
            },
        );
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        let _ = fs::remove_file(&self.path);
    }
}

fn cache_path(repo_path: &Path) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    repo_path.hash(&mut hasher);
    let hash = hasher.finish();
    PathBuf::from(format!("/tmp/git-bm-cache-{hash:x}.json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn cache_insert_and_lookup() {
        let dir = TempDir::new().unwrap();
        let mut cache = BranchCache::load(dir.path());
        cache.insert("feature/x", &MergeStatus::SquashMerged, "abc123");
        assert_eq!(
            cache.lookup("feature/x", "abc123"),
            Some(MergeStatus::SquashMerged)
        );
    }

    #[test]
    fn cache_unmerged_invalidated_on_new_commit() {
        let dir = TempDir::new().unwrap();
        let mut cache = BranchCache::load(dir.path());
        cache.insert("feature/x", &MergeStatus::Unmerged, "abc123");
        assert_eq!(
            cache.lookup("feature/x", "abc123"),
            Some(MergeStatus::Unmerged)
        );
        assert_eq!(cache.lookup("feature/x", "def456"), None);
    }

    #[test]
    fn cache_merged_permanent() {
        let dir = TempDir::new().unwrap();
        let mut cache = BranchCache::load(dir.path());
        cache.insert("feature/x", &MergeStatus::Merged, "abc123");
        // Merged is permanent regardless of commit hash
        assert_eq!(
            cache.lookup("feature/x", "def456"),
            Some(MergeStatus::Merged)
        );
    }

    #[test]
    fn cache_clear_removes_entries() {
        let dir = TempDir::new().unwrap();
        let mut cache = BranchCache::load(dir.path());
        cache.insert("feature/x", &MergeStatus::Merged, "abc123");
        cache.clear();
        assert_eq!(cache.lookup("feature/x", "abc123"), None);
    }

    #[test]
    fn cache_save_and_reload() {
        let dir = TempDir::new().unwrap();
        let mut cache = BranchCache::load(dir.path());
        cache.insert("feature/x", &MergeStatus::SquashMerged, "abc123");
        cache.save();

        let reloaded = BranchCache::load(dir.path());
        assert_eq!(
            reloaded.lookup("feature/x", "abc123"),
            Some(MergeStatus::SquashMerged)
        );
    }
}
