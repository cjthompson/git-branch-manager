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

    /// Delete the cache file and clear in-memory entries.
    pub fn clear(&mut self) {
        self.entries.clear();
        let _ = std::fs::remove_file(&self.path);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clear_removes_entries_and_file() {
        // Use a unique fake repo path so this test doesn't collide with real caches
        let fake_repo = Path::new("/tmp/test-clear-cache-repo-unique-12345");
        let mut cache = BranchCache::load(fake_repo);

        // Insert some entries and save to disk
        cache.insert("feature-a", &MergeStatus::Merged, "abc123");
        cache.insert("feature-b", &MergeStatus::Unmerged, "def456");
        cache.save();

        // Verify the cache file exists
        assert!(cache.path.exists(), "cache file should exist after save");

        // Verify lookups work before clear
        assert_eq!(cache.lookup("feature-a", ""), Some(MergeStatus::Merged));
        assert_eq!(
            cache.lookup("feature-b", "def456"),
            Some(MergeStatus::Unmerged)
        );

        // Clear the cache
        cache.clear();

        // In-memory entries should be gone
        assert!(cache.lookup("feature-a", "").is_none());
        assert!(cache.lookup("feature-b", "def456").is_none());

        // Cache file should be removed from disk
        assert!(
            !cache.path.exists(),
            "cache file should be removed after clear"
        );
    }

    #[test]
    fn test_clear_on_empty_cache_is_noop() {
        let fake_repo = Path::new("/tmp/test-clear-empty-cache-repo-unique-67890");
        let mut cache = BranchCache::load(fake_repo);

        // Clear on an empty cache (no file on disk) should not panic
        cache.clear();

        assert!(cache.lookup("anything", "").is_none());
        assert!(!cache.path.exists());
    }
}
