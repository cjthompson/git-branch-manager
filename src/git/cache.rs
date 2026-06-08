use crate::types::MergeStatus;
use serde::{Deserialize, Serialize};
use std::cell::Cell;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use tracing::{field, instrument, Span};

#[derive(Debug, Serialize, Deserialize)]
struct CacheEntry {
    merge_status: String,
    commit_hash: String,
}

pub struct BranchCache {
    path: PathBuf,
    entries: HashMap<String, CacheEntry>,
    ab_path: PathBuf,
    /// Ahead/behind counts keyed by "{branch_oid}:{upstream_oid}".
    /// Same OID pair always yields the same count — valid until either tip changes.
    ab_entries: HashMap<String, [u32; 2]>,
    hits: Cell<u32>,
    misses: Cell<u32>,
}

impl BranchCache {
    #[instrument(skip(repo_path), fields(path = ?repo_path, entry_count = field::Empty))]
    pub fn load(repo_path: &Path) -> Self {
        let span = Span::current();
        let path = cache_path(repo_path);
        let entries: HashMap<String, CacheEntry> = fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        span.record("entry_count", entries.len() as u64);
        let ab_path = ab_cache_path(repo_path);
        let ab_entries: HashMap<String, [u32; 2]> = fs::read_to_string(&ab_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self {
            path,
            entries,
            ab_path,
            ab_entries,
            hits: Cell::new(0),
            misses: Cell::new(0),
        }
    }

    #[instrument(skip(self), fields(entry_count = self.entries.len()))]
    pub fn save(&self) {
        if let Ok(json) = serde_json::to_string(&self.entries) {
            let _ = fs::write(&self.path, json);
        }
        if let Ok(json) = serde_json::to_string(&self.ab_entries) {
            let _ = fs::write(&self.ab_path, json);
        }
    }

    pub fn lookup_ahead_behind(
        &self,
        branch_oid: git2::Oid,
        upstream_oid: git2::Oid,
    ) -> Option<(u32, u32)> {
        let key = format!("{branch_oid}:{upstream_oid}");
        self.ab_entries.get(&key).map(|[a, b]| (*a, *b))
    }

    pub fn insert_ahead_behind(
        &mut self,
        branch_oid: git2::Oid,
        upstream_oid: git2::Oid,
        ahead: u32,
        behind: u32,
    ) {
        let key = format!("{branch_oid}:{upstream_oid}");
        self.ab_entries.insert(key, [ahead, behind]);
    }

    #[instrument(
        skip(self),
        fields(
            branch_name,
            current_commit_hash,
            hit = field::Empty,
            cached_status = field::Empty,
            cached_commit_hash = field::Empty,
            result_state = field::Empty,
        )
    )]
    pub fn lookup(&self, branch_name: &str, current_commit_hash: &str) -> Option<MergeStatus> {
        let span = Span::current();
        let entry = match self.entries.get(branch_name) {
            Some(entry) => entry,
            None => {
                self.record_miss();
                span.record("hit", false);
                span.record("result_state", "missing_entry");
                return None;
            }
        };
        span.record("cached_commit_hash", entry.commit_hash.as_str());
        let status = match entry.merge_status.as_str() {
            "merged" => MergeStatus::Merged,
            "squash_merged" => MergeStatus::SquashMerged,
            "unmerged" => MergeStatus::Unmerged,
            _ => {
                self.record_miss();
                span.record("hit", false);
                span.record("result_state", "unknown_status");
                return None;
            }
        };
        span.record("cached_status", entry.merge_status.as_str());
        match status {
            // Merged and SquashMerged are permanent
            MergeStatus::Merged | MergeStatus::SquashMerged => {
                self.record_hit();
                span.record("hit", true);
                span.record("result_state", "hit_permanent");
                Some(status)
            }
            // Unmerged is only valid if commit hasn't changed
            MergeStatus::Unmerged => {
                if entry.commit_hash == current_commit_hash {
                    self.record_hit();
                    span.record("hit", true);
                    span.record("result_state", "hit_current_commit");
                    Some(status)
                } else {
                    self.record_miss();
                    span.record("hit", false);
                    span.record("result_state", "stale_commit");
                    None
                }
            }
            _ => {
                self.record_miss();
                span.record("hit", false);
                span.record("result_state", "uncacheable_status");
                None
            }
        }
    }

    fn record_hit(&self) {
        self.hits.set(self.hits.get() + 1);
    }

    fn record_miss(&self) {
        self.misses.set(self.misses.get() + 1);
    }

    pub fn hits(&self) -> u32 {
        self.hits.get()
    }

    pub fn misses(&self) -> u32 {
        self.misses.get()
    }

    pub fn log_stats(&self, context: &str) {
        tracing::info!(
            target: "git_branch_manager::git::cache",
            context,
            hits = self.hits.get(),
            misses = self.misses.get(),
            "branch cache hit/miss stats"
        );
    }

    #[instrument(
        skip(self),
        fields(
            branch_name,
            commit_hash,
            status = ?status,
            inserted = field::Empty,
            result_state = field::Empty,
        )
    )]
    pub fn insert(&mut self, branch_name: &str, status: &MergeStatus, commit_hash: &str) {
        let span = Span::current();
        let status_str = match status {
            MergeStatus::Merged => "merged",
            MergeStatus::SquashMerged => "squash_merged",
            MergeStatus::Unmerged => "unmerged",
            MergeStatus::Pending => {
                span.record("inserted", false);
                span.record("result_state", "skipped_pending");
                return;
            } // Never cache Pending
        };
        self.entries.insert(
            branch_name.to_string(),
            CacheEntry {
                merge_status: status_str.to_string(),
                commit_hash: commit_hash.to_string(),
            },
        );
        span.record("inserted", true);
        span.record("result_state", "inserted");
    }

    #[instrument(skip(self), fields(entry_count = self.entries.len()))]
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

fn ab_cache_path(repo_path: &Path) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    repo_path.hash(&mut hasher);
    let hash = hasher.finish();
    PathBuf::from(format!("/tmp/git-bm-ab-{hash:x}.json"))
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
    fn cache_counts_hits_and_misses() {
        let dir = TempDir::new().unwrap();
        let mut cache = BranchCache::load(dir.path());
        cache.insert("feature/x", &MergeStatus::Merged, "abc123");
        cache.insert("feature/y", &MergeStatus::Unmerged, "old");

        assert_eq!(cache.lookup("feature/unknown", "zzz"), None);
        assert_eq!(
            cache.lookup("feature/x", "abc123"),
            Some(MergeStatus::Merged)
        );
        assert_eq!(cache.lookup("feature/y", "new"), None);

        assert_eq!(cache.hits(), 1);
        assert_eq!(cache.misses(), 2);
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

    #[test]
    fn ahead_behind_cache_hit_and_miss() {
        let dir = TempDir::new().unwrap();
        let mut cache = BranchCache::load(dir.path());
        let oid_a = git2::Oid::from_str("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
        let oid_b = git2::Oid::from_str("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap();
        let oid_c = git2::Oid::from_str("cccccccccccccccccccccccccccccccccccccccc").unwrap();

        assert_eq!(cache.lookup_ahead_behind(oid_a, oid_b), None);

        cache.insert_ahead_behind(oid_a, oid_b, 42, 3);
        assert_eq!(cache.lookup_ahead_behind(oid_a, oid_b), Some((42, 3)));
        assert_eq!(cache.lookup_ahead_behind(oid_a, oid_c), None);
        assert_eq!(cache.lookup_ahead_behind(oid_b, oid_a), None);
    }

    #[test]
    fn ahead_behind_cache_save_and_reload() {
        let dir = TempDir::new().unwrap();
        let mut cache = BranchCache::load(dir.path());
        let oid_a = git2::Oid::from_str("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
        let oid_b = git2::Oid::from_str("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap();

        cache.insert_ahead_behind(oid_a, oid_b, 100, 5);
        cache.save();

        let reloaded = BranchCache::load(dir.path());
        assert_eq!(reloaded.lookup_ahead_behind(oid_a, oid_b), Some((100, 5)));
    }
}
