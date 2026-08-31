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

/// Merge-base and base-tip cache. Persisted in the `merge_base` and `meta` tables.
/// When the base branch tip hasn't changed, we can skip the full revwalk and
/// restore merge statuses + merge bases entirely from these cached values.
#[derive(Debug, Default)]
pub struct MergeBaseData {
    /// Last-seen base branch tip OID (hex string). When this matches the current
    /// base tip, all cached merge statuses and merge bases are still valid.
    /// Read directly; mutate via [`BranchCache::set_base_tip`] so the change is persisted.
    pub base_tip: Option<String>,
    /// Merge base hash keyed by "{branch_tip_oid}:{base_tip_oid}".
    /// Value is None for disconnected branches, Some(hash) for connected ones.
    pub entries: HashMap<String, Option<String>>,
}

/// Cached result of the `git diff` + `git patch-id --stable` pipeline for a
/// single (old_oid, new_oid) pair, used by Graph's squash-merge patch
/// matching (`git::graph::compute_possible_squash_updates`). Keyed by OID
/// pair plus a diff-option/algorithm version rather than by branch/base
/// tip: a diff between two fixed, immutable Git objects never changes, so
/// the OID pair alone is a sufficient and permanently-valid cache key.
/// Base tip, branch tip, merge base, and the Graph display-bounds window
/// only affect *which* OID pairs get queried (via job construction in
/// `compute_possible_squash_updates`) — never what a given pair's diff
/// value is — so those inputs don't need to appear in this key. Bumping
/// `graph::GRAPH_DIFF_VERSION` invalidates every entry at once if the
/// `git diff`/`patch-id` invocation changes.
#[derive(Debug, Clone)]
struct GraphPatchEntry {
    patch_id: Option<String>,
    diff_text: Option<Vec<u8>>,
}

/// On-disk cache for a single repository. All four caches (merge status,
/// ahead/behind, merge base, Graph patch) live in one SQLite database;
/// writes are incremental (only dirtied keys are upserted), which makes
/// the concurrent saves from the phase-1 threads and the squash loader
/// safe without clobbering each other.
///
/// The `Connection` is opened per save/load rather than held, so `BranchCache`
/// stays `Send` and can be moved across the background-thread channels.
pub struct BranchCache {
    path: PathBuf,
    entries: HashMap<String, CacheEntry>,
    /// Ahead/behind counts keyed by "{branch_oid}:{upstream_oid}".
    /// Same OID pair always yields the same count — valid until either tip changes.
    ab_entries: HashMap<String, [u32; 2]>,
    pub mb_data: MergeBaseData,
    graph_patch_entries: HashMap<String, GraphPatchEntry>,
    dirty_entries: RefCell<HashSet<String>>,
    dirty_ab: RefCell<HashSet<String>>,
    dirty_mb: RefCell<HashSet<String>>,
    dirty_graph_patch: RefCell<HashSet<String>>,
    /// Branch-status rows to remove from disk on the next save (orphan cleanup).
    deleted_entries: RefCell<HashSet<String>>,
    base_tip_dirty: Cell<bool>,
    hits: Cell<u32>,
    misses: Cell<u32>,
}

impl BranchCache {
    #[instrument(skip(repo_path), fields(path = ?repo_path, entry_count = field::Empty))]
    pub fn load(repo_path: &Path) -> Self {
        Self::load_from_path(cache_path(repo_path))
    }

    /// Load a cache from an explicit path. Primarily for tests that need a
    /// controlled location instead of the per-repo OS cache directory.
    pub fn load_from_path(path: PathBuf) -> Self {
        let span = Span::current();
        let (entries, ab_entries, mb_entries, base_tip, graph_patch_entries) = read_all(&path);
        span.record("entry_count", entries.len() as u64);
        Self {
            path,
            entries,
            ab_entries,
            mb_data: MergeBaseData {
                base_tip,
                entries: mb_entries,
            },
            graph_patch_entries,
            dirty_entries: RefCell::new(HashSet::new()),
            dirty_ab: RefCell::new(HashSet::new()),
            dirty_mb: RefCell::new(HashSet::new()),
            dirty_graph_patch: RefCell::new(HashSet::new()),
            deleted_entries: RefCell::new(HashSet::new()),
            base_tip_dirty: Cell::new(false),
            hits: Cell::new(0),
            misses: Cell::new(0),
        }
    }

    #[instrument(skip(self), fields(entry_count = self.entries.len()))]
    pub fn save(&self) {
        let dirty_entries: Vec<String> = self.dirty_entries.borrow().iter().cloned().collect();
        let dirty_ab: Vec<String> = self.dirty_ab.borrow().iter().cloned().collect();
        let dirty_mb: Vec<String> = self.dirty_mb.borrow().iter().cloned().collect();
        let dirty_graph_patch: Vec<String> = self.dirty_graph_patch.borrow().iter().cloned().collect();
        let deleted_entries: Vec<String> = self.deleted_entries.borrow().iter().cloned().collect();
        let write_base_tip = self.base_tip_dirty.get();
        if dirty_entries.is_empty()
            && dirty_ab.is_empty()
            && dirty_mb.is_empty()
            && dirty_graph_patch.is_empty()
            && deleted_entries.is_empty()
            && !write_base_tip
        {
            return;
        }
        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let Ok(mut conn) = open_conn(&self.path) else {
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

        for key in &dirty_ab {
            let Some(&[ahead, behind]) = self.ab_entries.get(key) else {
                continue;
            };
            if tx
                .execute(
                    "INSERT INTO ahead_behind (key, ahead, behind)
                     VALUES (?1, ?2, ?3)
                     ON CONFLICT(key) DO UPDATE SET
                         ahead = excluded.ahead,
                         behind = excluded.behind",
                    params![key, ahead, behind],
                )
                .is_err()
            {
                return;
            }
        }

        for key in &dirty_mb {
            let Some(merge_base) = self.mb_data.entries.get(key) else {
                continue;
            };
            if tx
                .execute(
                    "INSERT INTO merge_base (key, merge_base)
                     VALUES (?1, ?2)
                     ON CONFLICT(key) DO UPDATE SET merge_base = excluded.merge_base",
                    params![key, merge_base.as_deref()],
                )
                .is_err()
            {
                return;
            }
        }

        for key in &dirty_graph_patch {
            let Some(entry) = self.graph_patch_entries.get(key) else {
                continue;
            };
            if tx
                .execute(
                    "INSERT INTO graph_patch (key, patch_id, diff_text)
                     VALUES (?1, ?2, ?3)
                     ON CONFLICT(key) DO UPDATE SET
                         patch_id = excluded.patch_id,
                         diff_text = excluded.diff_text",
                    params![key, entry.patch_id.as_deref(), entry.diff_text.as_deref()],
                )
                .is_err()
            {
                return;
            }
        }

        for branch_name in &deleted_entries {
            if tx
                .execute(
                    "DELETE FROM branch_cache WHERE branch_name = ?1",
                    params![branch_name],
                )
                .is_err()
            {
                return;
            }
        }

        if write_base_tip {
            if let Some(base_tip) = &self.mb_data.base_tip {
                if tx
                    .execute(
                        "INSERT INTO meta (key, value) VALUES ('base_tip', ?1)
                         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                        params![base_tip],
                    )
                    .is_err()
                {
                    return;
                }
            }
        }

        if tx.commit().is_ok() {
            self.dirty_entries.borrow_mut().clear();
            self.dirty_ab.borrow_mut().clear();
            self.dirty_mb.borrow_mut().clear();
            self.dirty_graph_patch.borrow_mut().clear();
            self.deleted_entries.borrow_mut().clear();
            self.base_tip_dirty.set(false);
        }
    }

    /// Set the cached base-tip OID and mark it for persistence. Only the writer
    /// that actually computes the base tip should call this; other cache holders
    /// leave it untouched so they don't clobber a fresher value on save.
    pub fn set_base_tip(&mut self, base_tip: Option<String>) {
        self.mb_data.base_tip = base_tip;
        self.base_tip_dirty.set(true);
    }

    /// Returns the cached merge base for (branch_tip, base_tip):
    /// - `None` → not in cache (miss)
    /// - `Some(None)` → cached as disconnected (no common ancestor within walk limit)
    /// - `Some(Some(hash))` → cached merge base hash
    pub fn lookup_merge_base(
        &self,
        branch_tip: git2::Oid,
        base_tip: git2::Oid,
    ) -> Option<Option<String>> {
        let key = format!("{branch_tip}:{base_tip}");
        self.mb_data.entries.get(&key).cloned()
    }

    pub fn insert_merge_base(
        &mut self,
        branch_tip: git2::Oid,
        base_tip: git2::Oid,
        merge_base: Option<String>,
    ) {
        let key = format!("{branch_tip}:{base_tip}");
        self.mb_data.entries.insert(key.clone(), merge_base);
        self.dirty_mb.borrow_mut().insert(key);
    }

    /// Returns the cached `(patch_id, diff_text)` for the diff between
    /// `old_oid` and `new_oid` under diff-algorithm `version`:
    /// - `None` → not cached (miss)
    /// - `Some((patch_id, diff_text))` → cached result, including the
    ///   legitimate "empty/failed diff" case where both are `None`.
    pub fn lookup_graph_patch(
        &self,
        old_oid: &str,
        new_oid: &str,
        version: u32,
    ) -> Option<(Option<String>, Option<Vec<u8>>)> {
        let key = format!("{old_oid}:{new_oid}:v{version}");
        self.graph_patch_entries
            .get(&key)
            .map(|entry| (entry.patch_id.clone(), entry.diff_text.clone()))
    }

    pub fn insert_graph_patch(
        &mut self,
        old_oid: &str,
        new_oid: &str,
        version: u32,
        patch_id: Option<String>,
        diff_text: Option<Vec<u8>>,
    ) {
        let key = format!("{old_oid}:{new_oid}:v{version}");
        self.graph_patch_entries
            .insert(key.clone(), GraphPatchEntry { patch_id, diff_text });
        self.dirty_graph_patch.borrow_mut().insert(key);
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
        self.ab_entries.insert(key.clone(), [ahead, behind]);
        self.dirty_ab.borrow_mut().insert(key);
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
            "in_sync" => MergeStatus::InSync,
            "squash_merged" => MergeStatus::SquashMerged,
            "local_merged" => MergeStatus::LocalMerged,
            "remote_merged" => MergeStatus::RemoteMerged,
            "local_squash_merged" => MergeStatus::LocalSquashMerged,
            "remote_squash_merged" => MergeStatus::RemoteSquashMerged,
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
            // Unmerged, in-sync, and local/remote variants are only valid if commit hasn't changed.
            // InSync is inherently volatile — base moves, the in-sync state breaks.
            MergeStatus::Unmerged
            | MergeStatus::InSync
            | MergeStatus::LocalMerged
            | MergeStatus::RemoteMerged
            | MergeStatus::LocalSquashMerged
            | MergeStatus::RemoteSquashMerged => {
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
            MergeStatus::InSync => "in_sync",
            MergeStatus::SquashMerged => "squash_merged",
            MergeStatus::LocalMerged => "local_merged",
            MergeStatus::RemoteMerged => "remote_merged",
            MergeStatus::LocalSquashMerged => "local_squash_merged",
            MergeStatus::RemoteSquashMerged => "remote_squash_merged",
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

    /// All branch names that have a cached merge-status row. Used by the cache
    /// audit to detect orphans (rows whose branch no longer exists).
    pub fn cached_branch_names(&self) -> Vec<String> {
        self.entries.keys().cloned().collect()
    }

    /// Remove a branch's merge-status row. The deletion is buffered and applied
    /// to disk on the next [`save`](Self::save). Used to clean up orphan rows.
    pub fn delete_branch_entry(&mut self, branch_name: &str) {
        self.entries.remove(branch_name);
        self.dirty_entries.borrow_mut().remove(branch_name);
        self.deleted_entries
            .borrow_mut()
            .insert(branch_name.to_string());
    }

    #[instrument(skip(self), fields(entry_count = self.entries.len()))]
    pub fn clear(&mut self) {
        self.entries.clear();
        self.ab_entries.clear();
        self.mb_data = MergeBaseData::default();
        self.graph_patch_entries.clear();
        self.dirty_entries.borrow_mut().clear();
        self.dirty_ab.borrow_mut().clear();
        self.dirty_mb.borrow_mut().clear();
        self.dirty_graph_patch.borrow_mut().clear();
        self.deleted_entries.borrow_mut().clear();
        self.base_tip_dirty.set(false);
        let _ = fs::remove_file(&self.path);
    }
}

fn open_conn(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    // Wait out concurrent writers (phase-1 threads + squash loader share this file)
    // instead of failing fast with SQLITE_BUSY.
    let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
    let _ = conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;");
    Ok(conn)
}

fn ensure_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS branch_cache (
            branch_name  TEXT PRIMARY KEY,
            merge_status TEXT NOT NULL,
            commit_hash  TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS ahead_behind (
            key    TEXT PRIMARY KEY,
            ahead  INTEGER NOT NULL,
            behind INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS merge_base (
            key        TEXT PRIMARY KEY,
            merge_base TEXT
        );
        CREATE TABLE IF NOT EXISTS graph_patch (
            key        TEXT PRIMARY KEY,
            patch_id   TEXT,
            diff_text  BLOB
        );
        CREATE TABLE IF NOT EXISTS meta (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );",
    )
}

#[allow(clippy::type_complexity)]
fn read_all(
    path: &Path,
) -> (
    HashMap<String, CacheEntry>,
    HashMap<String, [u32; 2]>,
    HashMap<String, Option<String>>,
    Option<String>,
    HashMap<String, GraphPatchEntry>,
) {
    if !path.exists() {
        return (
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            None,
            HashMap::new(),
        );
    }
    let Ok(conn) = open_conn(path) else {
        return (
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            None,
            HashMap::new(),
        );
    };
    (
        read_entries(&conn),
        read_ahead_behind(&conn),
        read_merge_base(&conn),
        read_base_tip(&conn),
        read_graph_patch(&conn),
    )
}

fn read_entries(conn: &Connection) -> HashMap<String, CacheEntry> {
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
    rows.flatten().collect()
}

fn read_ahead_behind(conn: &Connection) -> HashMap<String, [u32; 2]> {
    let Ok(mut stmt) = conn.prepare("SELECT key, ahead, behind FROM ahead_behind") else {
        return HashMap::new();
    };
    let Ok(rows) = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            [row.get::<_, u32>(1)?, row.get::<_, u32>(2)?],
        ))
    }) else {
        return HashMap::new();
    };
    rows.flatten().collect()
}

fn read_merge_base(conn: &Connection) -> HashMap<String, Option<String>> {
    let Ok(mut stmt) = conn.prepare("SELECT key, merge_base FROM merge_base") else {
        return HashMap::new();
    };
    let Ok(rows) = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
    }) else {
        return HashMap::new();
    };
    rows.flatten().collect()
}

fn read_graph_patch(conn: &Connection) -> HashMap<String, GraphPatchEntry> {
    let Ok(mut stmt) = conn.prepare("SELECT key, patch_id, diff_text FROM graph_patch") else {
        return HashMap::new();
    };
    let Ok(rows) = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            GraphPatchEntry {
                patch_id: row.get::<_, Option<String>>(1)?,
                diff_text: row.get::<_, Option<Vec<u8>>>(2)?,
            },
        ))
    }) else {
        return HashMap::new();
    };
    rows.flatten().collect()
}

fn read_base_tip(conn: &Connection) -> Option<String> {
    conn.query_row("SELECT value FROM meta WHERE key = 'base_tip'", [], |row| {
        row.get::<_, String>(0)
    })
    .ok()
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

    fn temp_cache() -> (TempDir, BranchCache) {
        let dir = TempDir::new().unwrap();
        let cache = BranchCache::load_from_path(dir.path().join("cache.sqlite3"));
        (dir, cache)
    }

    #[test]
    fn cache_insert_and_lookup() {
        let (_dir, mut cache) = temp_cache();
        cache.insert("feature/x", &MergeStatus::SquashMerged, "abc123");
        assert_eq!(
            cache.lookup("feature/x", "abc123"),
            Some(MergeStatus::SquashMerged)
        );
    }

    #[test]
    fn cache_unmerged_invalidated_on_new_commit() {
        let (_dir, mut cache) = temp_cache();
        cache.insert("feature/x", &MergeStatus::Unmerged, "abc123");
        assert_eq!(
            cache.lookup("feature/x", "abc123"),
            Some(MergeStatus::Unmerged)
        );
        assert_eq!(cache.lookup("feature/x", "def456"), None);
    }

    #[test]
    fn cache_merged_permanent() {
        let (_dir, mut cache) = temp_cache();
        cache.insert("feature/x", &MergeStatus::Merged, "abc123");
        // Merged is permanent regardless of commit hash
        assert_eq!(
            cache.lookup("feature/x", "def456"),
            Some(MergeStatus::Merged)
        );
    }

    #[test]
    fn cached_branch_names_lists_all_entries() {
        let (_dir, mut cache) = temp_cache();
        cache.insert("feature/x", &MergeStatus::Merged, "abc123");
        cache.insert("feature/y", &MergeStatus::Unmerged, "def456");
        let mut names = cache.cached_branch_names();
        names.sort();
        assert_eq!(
            names,
            vec!["feature/x".to_string(), "feature/y".to_string()]
        );
    }

    #[test]
    fn delete_branch_entry_removes_from_memory_and_disk() {
        let dir = TempDir::new().unwrap();
        let cache_path = dir.path().join("cache.sqlite3");
        let mut cache = BranchCache::load_from_path(cache_path.clone());
        cache.insert("feature/x", &MergeStatus::Merged, "abc123");
        cache.insert("feature/y", &MergeStatus::Merged, "def456");
        cache.save();

        // Delete one entry and persist.
        cache.delete_branch_entry("feature/x");
        assert_eq!(cache.lookup("feature/x", "abc123"), None);
        cache.save();

        // The deletion survives a reload from disk.
        let reloaded = BranchCache::load_from_path(cache_path);
        assert_eq!(reloaded.lookup("feature/x", "abc123"), None);
        assert_eq!(
            reloaded.lookup("feature/y", "def456"),
            Some(MergeStatus::Merged)
        );
    }

    #[test]
    fn cache_clear_removes_entries() {
        let (_dir, mut cache) = temp_cache();
        cache.insert("feature/x", &MergeStatus::Merged, "abc123");
        cache.clear();
        assert_eq!(cache.lookup("feature/x", "abc123"), None);
    }

    #[test]
    fn cache_counts_hits_and_misses() {
        let (_dir, mut cache) = temp_cache();
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
    fn ahead_behind_cache_hit_and_miss() {
        let (_dir, mut cache) = temp_cache();
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
        let cache_path = dir.path().join("cache.sqlite3");
        let mut cache = BranchCache::load_from_path(cache_path.clone());
        let oid_a = git2::Oid::from_str("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
        let oid_b = git2::Oid::from_str("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap();

        cache.insert_ahead_behind(oid_a, oid_b, 100, 5);
        cache.save();

        let reloaded = BranchCache::load_from_path(cache_path);
        assert_eq!(reloaded.lookup_ahead_behind(oid_a, oid_b), Some((100, 5)));
    }

    #[test]
    fn merge_base_cache_hit_miss_disconnected() {
        let (_dir, mut cache) = temp_cache();
        let branch_tip = git2::Oid::from_str("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
        let base_tip = git2::Oid::from_str("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap();

        // Not in cache yet
        assert_eq!(cache.lookup_merge_base(branch_tip, base_tip), None);

        // Insert disconnected
        cache.insert_merge_base(branch_tip, base_tip, None);
        assert_eq!(cache.lookup_merge_base(branch_tip, base_tip), Some(None));

        // Insert connected
        let c_tip = git2::Oid::from_str("cccccccccccccccccccccccccccccccccccccccc").unwrap();
        cache.insert_merge_base(c_tip, base_tip, Some("deadbeef".to_string()));
        assert_eq!(
            cache.lookup_merge_base(c_tip, base_tip),
            Some(Some("deadbeef".to_string()))
        );
    }

    #[test]
    fn merge_base_cache_save_and_reload() {
        let dir = TempDir::new().unwrap();
        let cache_path = dir.path().join("cache.sqlite3");
        let mut cache = BranchCache::load_from_path(cache_path.clone());
        let branch_tip = git2::Oid::from_str("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
        let base_tip = git2::Oid::from_str("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap();

        cache.insert_merge_base(branch_tip, base_tip, Some("cafebabe".to_string()));
        cache.set_base_tip(Some(base_tip.to_string()));
        cache.save();

        let reloaded = BranchCache::load_from_path(cache_path);
        assert_eq!(
            reloaded.lookup_merge_base(branch_tip, base_tip),
            Some(Some("cafebabe".to_string()))
        );
        assert_eq!(
            reloaded.mb_data.base_tip.as_deref(),
            Some(&*base_tip.to_string())
        );
    }

    #[test]
    fn base_tip_only_persisted_when_set_explicitly() {
        let dir = TempDir::new().unwrap();
        let cache_path = dir.path().join("cache.sqlite3");

        // A holder that inserts a merge base but never calls set_base_tip must
        // not write a base_tip row (it doesn't own that value).
        let mut writer = BranchCache::load_from_path(cache_path.clone());
        let branch_tip = git2::Oid::from_str("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
        let base_tip = git2::Oid::from_str("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap();
        writer.insert_merge_base(branch_tip, base_tip, Some("cafebabe".to_string()));
        writer.save();

        let reloaded = BranchCache::load_from_path(cache_path);
        assert_eq!(reloaded.mb_data.base_tip, None);
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

    #[test]
    fn graph_patch_cache_insert_and_lookup() {
        let (_dir, mut cache) = temp_cache();
        cache.insert_graph_patch(
            "a1",
            "a2",
            1,
            Some("p1".to_string()),
            Some(vec![1, 2, 3]),
        );
        assert_eq!(
            cache.lookup_graph_patch("a1", "a2", 1),
            Some((Some("p1".to_string()), Some(vec![1, 2, 3])))
        );
    }

    #[test]
    fn graph_patch_cache_miss_when_not_present() {
        let (_dir, cache) = temp_cache();
        assert_eq!(cache.lookup_graph_patch("x", "y", 1), None);
    }

    #[test]
    fn graph_patch_cache_version_bump_invalidates() {
        let (_dir, mut cache) = temp_cache();
        cache.insert_graph_patch("a1", "a2", 1, Some("p1".to_string()), None);
        assert_eq!(
            cache.lookup_graph_patch("a1", "a2", 1),
            Some((Some("p1".to_string()), None))
        );
        // Version 2 must miss even though OIDs match.
        assert_eq!(cache.lookup_graph_patch("a1", "a2", 2), None);
    }

    #[test]
    fn graph_patch_cache_empty_and_failed_computation_is_cached_as_hit() {
        let (_dir, mut cache) = temp_cache();
        cache.insert_graph_patch("a1", "a2", 1, None, None);
        // Must return Some((None, None)) — distinct from "not cached" None.
        assert_eq!(cache.lookup_graph_patch("a1", "a2", 1), Some((None, None)));
    }

    #[test]
    fn graph_patch_cache_duplicate_patch_id_different_oid_pairs_do_not_collide() {
        let (_dir, mut cache) = temp_cache();
        cache.insert_graph_patch("a1", "a2", 1, Some("shared".to_string()), None);
        cache.insert_graph_patch(
            "b1",
            "b2",
            1,
            Some("shared".to_string()),
            Some(vec![9, 9, 9]),
        );
        assert_eq!(
            cache.lookup_graph_patch("a1", "a2", 1),
            Some((Some("shared".to_string()), None))
        );
        assert_eq!(
            cache.lookup_graph_patch("b1", "b2", 1),
            Some((Some("shared".to_string()), Some(vec![9, 9, 9])))
        );
    }

    #[test]
    fn graph_patch_cache_save_and_reload() {
        let dir = TempDir::new().unwrap();
        let cache_path = dir.path().join("cache.sqlite3");
        let mut cache = BranchCache::load_from_path(cache_path.clone());
        cache.insert_graph_patch(
            "a1",
            "a2",
            1,
            Some("p1".to_string()),
            Some(vec![0xDE, 0xAD, 0xBE, 0xEF]),
        );
        cache.save();

        let reloaded = BranchCache::load_from_path(cache_path);
        assert_eq!(
            reloaded.lookup_graph_patch("a1", "a2", 1),
            Some((
                Some("p1".to_string()),
                Some(vec![0xDE, 0xAD, 0xBE, 0xEF])
            ))
        );
    }

    #[test]
    fn graph_patch_cache_cross_repository_isolation() {
        let dir_a = TempDir::new().unwrap();
        let dir_b = TempDir::new().unwrap();
        let path_a = dir_a.path().join("cache_a.sqlite3");
        let path_b = dir_b.path().join("cache_b.sqlite3");

        let mut cache_a = BranchCache::load_from_path(path_a);
        cache_a.insert_graph_patch("a1", "a2", 1, Some("only-in-a".to_string()), None);
        cache_a.save();

        let cache_b = BranchCache::load_from_path(path_b);
        // B has its own file — must not see A's entry.
        assert_eq!(cache_b.lookup_graph_patch("a1", "a2", 1), None);
    }

    #[test]
    fn graph_patch_cache_cleared_by_clear() {
        let (_dir, mut cache) = temp_cache();
        cache.insert_graph_patch("a1", "a2", 1, Some("p1".to_string()), None);
        cache.clear();
        assert_eq!(cache.lookup_graph_patch("a1", "a2", 1), None);
    }

    #[test]
    fn graph_patch_cache_concurrent_writers_do_not_clobber() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("cache.sqlite3");
        let n = 8usize;

        let mut handles = Vec::with_capacity(n);
        for thread_index in 0..n {
            let path = path.clone();
            handles.push(std::thread::spawn(move || {
                let mut cache = BranchCache::load_from_path(path);
                cache.insert_graph_patch(
                    &format!("old-{thread_index}"),
                    &format!("new-{thread_index}"),
                    1,
                    Some(format!("patch-{thread_index}")),
                    None,
                );
                cache.save();
            }));
        }
        for handle in handles {
            handle.join().expect("writer thread panicked");
        }

        let reloaded = BranchCache::load_from_path(path);
        for thread_index in 0..n {
            assert_eq!(
                reloaded.lookup_graph_patch(
                    &format!("old-{thread_index}"),
                    &format!("new-{thread_index}"),
                    1,
                ),
                Some((Some(format!("patch-{thread_index}")), None)),
                "thread {thread_index} entry missing after concurrent writes"
            );
        }
    }
}
