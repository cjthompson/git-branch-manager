use crate::types::MergeStatus;
use rusqlite::{params, Connection};
use std::cell::{Cell, RefCell};
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use tracing::{field, instrument, Span};

#[derive(Debug)]
struct CacheEntry {
    merge_status: String,
    commit_hash: String,
}

pub struct BranchCache {
    path: PathBuf,
    entries: HashMap<String, CacheEntry>,
    dirty_entries: RefCell<HashSet<String>>,
    hits: Cell<u32>,
    misses: Cell<u32>,
}

impl BranchCache {
    #[instrument(skip(repo_path), fields(path = ?repo_path, entry_count = field::Empty))]
    pub fn load(repo_path: &Path) -> Self {
        Self::load_from_path(cache_path(repo_path))
    }

    fn load_from_path(path: PathBuf) -> Self {
        let span = Span::current();
        let entries = read_entries(&path);
        span.record("entry_count", entries.len() as u64);
        Self {
            path,
            entries,
            dirty_entries: RefCell::new(HashSet::new()),
            hits: Cell::new(0),
            misses: Cell::new(0),
        }
    }

    #[instrument(skip(self), fields(entry_count = self.entries.len()))]
    pub fn save(&self) {
        let dirty_entries: Vec<String> = self.dirty_entries.borrow().iter().cloned().collect();
        if dirty_entries.is_empty() {
            return;
        }
        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let Ok(mut conn) = Connection::open(&self.path) else {
            return;
        };
        if ensure_schema(&conn).is_err() {
            return;
        }
        let Ok(tx) = conn.transaction() else {
            return;
        };

        for branch_name in &dirty_entries {
            let Some(entry) = self.entries.get(branch_name) else {
                continue;
            };
            if tx
                .execute(
                    "INSERT INTO branch_cache (branch_name, merge_status, commit_hash)
                     VALUES (?1, ?2, ?3)
                     ON CONFLICT(branch_name) DO UPDATE SET
                         merge_status = excluded.merge_status,
                         commit_hash = excluded.commit_hash",
                    params![branch_name, entry.merge_status, entry.commit_hash],
                )
                .is_err()
            {
                return;
            }
        }

        if tx.commit().is_ok() {
            self.dirty_entries.borrow_mut().clear();
        }
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
        self.dirty_entries
            .borrow_mut()
            .insert(branch_name.to_string());
        span.record("inserted", true);
        span.record("result_state", "inserted");
    }

    #[instrument(skip(self), fields(entry_count = self.entries.len()))]
    pub fn clear(&mut self) {
        self.entries.clear();
        self.dirty_entries.borrow_mut().clear();
        let _ = fs::remove_file(&self.path);
    }
}

fn ensure_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS branch_cache (
            branch_name TEXT PRIMARY KEY,
            merge_status TEXT NOT NULL,
            commit_hash TEXT NOT NULL
        )",
        [],
    )?;
    Ok(())
}

fn read_entries(path: &Path) -> HashMap<String, CacheEntry> {
    if !path.exists() {
        return HashMap::new();
    }

    let Ok(conn) = Connection::open(path) else {
        return HashMap::new();
    };
    let Ok(mut stmt) =
        conn.prepare("SELECT branch_name, merge_status, commit_hash FROM branch_cache")
    else {
        return HashMap::new();
    };
    let Ok(rows) = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            CacheEntry {
                merge_status: row.get(1)?,
                commit_hash: row.get(2)?,
            },
        ))
    }) else {
        return HashMap::new();
    };

    let mut entries = HashMap::new();
    for (branch_name, entry) in rows.flatten() {
        entries.insert(branch_name, entry);
    }
    entries
}

fn cache_path(repo_path: &Path) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    repo_path.hash(&mut hasher);
    let hash = hasher.finish();
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("git-branch-manager")
        .join(format!("git-bm-cache-{hash:x}.sqlite3"))
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
        let cache_path = dir.path().join("cache.sqlite3");
        let mut cache = BranchCache::load_from_path(cache_path.clone());
        cache.insert("feature/x", &MergeStatus::SquashMerged, "abc123");
        cache.save();

        let reloaded = BranchCache::load_from_path(cache_path);
        assert_eq!(
            reloaded.lookup("feature/x", "abc123"),
            Some(MergeStatus::SquashMerged)
        );
    }

    #[test]
    fn cache_path_uses_app_cache_directory() {
        let dir = TempDir::new().unwrap();
        let path = cache_path(dir.path());

        assert_eq!(
            path.parent().and_then(Path::file_name),
            Some(std::ffi::OsStr::new("git-branch-manager"))
        );
        assert!(path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("git-bm-cache-") && name.ends_with(".sqlite3")));
    }
}
