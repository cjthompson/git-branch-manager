use crate::types::MergeStatus;
use rusqlite::{params, Connection};
use std::cell::{Cell, RefCell};
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use tracing::{field, instrument, Span};

/// Directory containing the SQLite cache files. Applications can pass an
/// explicit root to keep cache ownership scoped to a process or test fixture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CacheRoot {
    path: PathBuf,
}

impl CacheRoot {
    /// Resolve the startup override, falling back to the platform cache dir.
    /// Tests that spawn the CLI set `GBM_CACHE_DIR` on that child process only.
    pub fn from_env() -> Self {
        let path = std::env::var_os("GBM_CACHE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                dirs::cache_dir()
                    .unwrap_or_else(std::env::temp_dir)
                    .join("git-branch-manager")
            });
        Self { path }
    }

    /// Construct a cache root at an explicit directory.
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn cache_path(&self, repo_path: &Path) -> PathBuf {
        let identity_path = git_common_dir(repo_path).unwrap_or_else(|| {
            fs::canonicalize(repo_path).unwrap_or_else(|_| repo_path.to_path_buf())
        });
        let mut hasher = DefaultHasher::new();
        identity_path.hash(&mut hasher);
        let hash = hasher.finish();
        self.path
            .join(format!("git-bm-repo-cache-{hash:x}.sqlite3"))
    }
}

fn git_common_dir(repo_path: &Path) -> Option<PathBuf> {
    git2::Repository::open(repo_path).ok().map(|repo| {
        fs::canonicalize(repo.commondir()).unwrap_or_else(|_| repo.commondir().to_path_buf())
    })
}

const CACHE_RETENTION: std::time::Duration = std::time::Duration::from_secs(60 * 24 * 60 * 60);

/// Remove app-owned cache databases that have not been used for 60 days.
/// Failures are intentionally ignored because cleanup must never block startup.
pub fn prune_stale_caches(cache_root: &CacheRoot) {
    prune_stale_caches_at(cache_root, std::time::SystemTime::now());
}

fn prune_stale_caches_at(cache_root: &CacheRoot, now: std::time::SystemTime) {
    prune_stale_caches_at_with(cache_root, now, |path| fs::remove_file(path));
}

fn prune_stale_caches_at_with<F>(
    cache_root: &CacheRoot,
    now: std::time::SystemTime,
    mut remove_file: F,
) where
    F: FnMut(&Path) -> std::io::Result<()>,
{
    let Ok(children) = fs::read_dir(cache_root.path()) else {
        return;
    };
    let mut artifacts: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();
    for child in children.flatten() {
        let path = child.path();
        if !child
            .file_type()
            .map(|kind| kind.is_file())
            .unwrap_or(false)
        {
            continue;
        }
        let Some(name) = child.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Some(database_name) = cache_database_name(&name) else {
            continue;
        };
        artifacts
            .entry(cache_root.path().join(database_name))
            .or_default()
            .push(path);
    }

    for (database, files) in artifacts {
        let database_exists = files.iter().any(|path| path == &database);
        if database_exists {
            let database_is_stale = modified_at(&database)
                .map(|modified| is_stale(modified, now))
                .unwrap_or(false);
            let has_recent_sidecar = files.iter().any(|path| {
                path != &database
                    && modified_at(path)
                        .map(|modified| !is_stale(modified, now))
                        .unwrap_or(true)
            });
            if database_is_stale && !has_recent_sidecar {
                if remove_file(&database).is_ok() {
                    // If the database is still present, keep its WAL/SHM files
                    // intact. Failed sidecar removals become orphans and are
                    // retried by a later sweep.
                    for path in files {
                        if path != database {
                            let _ = remove_file(&path);
                        }
                    }
                }
            }
        } else {
            // A crash can leave WAL/SHM files after SQLite removes the database.
            // Age each orphan independently so a recent sidecar is preserved.
            for path in files {
                if modified_at(&path)
                    .map(|modified| is_stale(modified, now))
                    .unwrap_or(false)
                {
                    let _ = remove_file(&path);
                }
            }
        }
    }
}

fn cache_database_name(name: &str) -> Option<String> {
    let database_name = name
        .strip_suffix("-wal")
        .or_else(|| name.strip_suffix("-shm"))
        .unwrap_or(name);
    let hash = database_name
        .strip_prefix("git-bm-repo-cache-")
        .or_else(|| database_name.strip_prefix("git-bm-cache-"))
        .and_then(|rest| rest.strip_suffix(".sqlite3"))?;
    let is_hex_hash = !hash.is_empty()
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    is_hex_hash.then(|| database_name.to_string())
}

fn modified_at(path: &Path) -> Option<std::time::SystemTime> {
    fs::metadata(path).ok()?.modified().ok()
}

fn is_stale(modified: std::time::SystemTime, now: std::time::SystemTime) -> bool {
    now.duration_since(modified)
        .map(|age| age > CACHE_RETENTION)
        .unwrap_or(false)
}

fn refresh_cache_timestamp(path: &Path) {
    if !path.exists() {
        return;
    }
    if let Ok(file) = fs::File::open(path) {
        let _ =
            file.set_times(std::fs::FileTimes::new().set_modified(std::time::SystemTime::now()));
    }
}

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
/// matching (`git::graph::compute_relationships`). Keyed by OID
/// pair plus a diff-option/algorithm version rather than by branch/base
/// tip: a diff between two fixed, immutable Git objects never changes, so
/// the OID pair alone is a sufficient and permanently-valid cache key.
/// Base tip, branch tip, merge base, and the Graph display-bounds window
/// only affect *which* OID pairs get queried (via job construction in
/// `compute_relationships`) — never what a given pair's diff
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
    base_branch: Option<String>,
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
    /// Load the default cache root with no branch-status scope.
    /// Status-aware callers should use [`Self::load_for_base`].
    #[instrument(skip(repo_path), fields(path = ?repo_path, entry_count = field::Empty))]
    pub fn load(repo_path: &Path) -> Self {
        Self::load_with_root(repo_path, &CacheRoot::from_env())
    }

    /// Load the explicit root without a branch-status scope. This is useful
    /// for OID-only work; status-aware callers should use [`Self::load_for_base`].
    #[instrument(skip(repo_path, cache_root), fields(path = ?repo_path, entry_count = field::Empty))]
    pub fn load_with_root(repo_path: &Path, cache_root: &CacheRoot) -> Self {
        Self::load_from_path_for_base(cache_root.cache_path(repo_path), None)
    }

    /// Load the shared repository database with branch-status rows bound to
    /// the selected base branch. OID-keyed tables remain shared across scopes.
    pub fn load_for_base(repo_path: &Path, base_branch: &str, cache_root: &CacheRoot) -> Self {
        Self::load_from_path_for_base(cache_root.cache_path(repo_path), Some(base_branch))
    }

    /// Load a cache from an explicit path. Primarily for tests that need a
    /// controlled location instead of the per-repo OS cache directory.
    pub fn load_from_path(path: PathBuf) -> Self {
        Self::load_from_path_for_base(path, None)
    }

    fn load_from_path_for_base(path: PathBuf, base_branch: Option<&str>) -> Self {
        refresh_cache_timestamp(&path);
        let span = Span::current();
        let base_branch = base_branch.map(str::to_owned);
        let (entries, ab_entries, mb_entries, base_tip, graph_patch_entries) =
            read_all(&path, base_branch.as_deref());
        span.record("entry_count", entries.len() as u64);
        Self {
            path,
            base_branch,
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

    /// Query commit identities on demand. SQL NULL is a cached empty patch.
    pub fn lookup_commit_patch_ids(&self, keys: &[String]) -> HashMap<String, Option<String>> {
        let mut found = HashMap::new();
        if keys.is_empty() || !self.path.exists() {
            return found;
        }
        let Ok(conn) = open_conn(&self.path) else {
            return found;
        };
        for chunk in keys.chunks(400) {
            let sql = format!(
                "SELECT key, patch_id FROM commit_patch_id WHERE key IN ({})",
                vec!["?"; chunk.len()].join(",")
            );
            // An older transient cache has no table and therefore only misses.
            let Ok(mut statement) = conn.prepare(&sql) else {
                return found;
            };
            let Ok(rows) = statement.query_map(rusqlite::params_from_iter(chunk.iter()), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            }) else {
                return found;
            };
            found.extend(rows.flatten());
        }
        found
    }

    /// Store newly computed identities without loading the table into memory.
    pub fn store_commit_patch_ids(&self, entries: &[(String, Option<String>)]) {
        if entries.is_empty() {
            return;
        }
        if let Some(parent) = self.path.parent() {
            if fs::create_dir_all(parent).is_err() {
                return;
            }
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
        {
            let Ok(mut statement) = tx.prepare("INSERT INTO commit_patch_id (key, patch_id) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET patch_id = excluded.patch_id") else { return; };
            for (key, patch_id) in entries {
                if statement.execute(params![key, patch_id]).is_err() {
                    return;
                }
            }
        }
        let _ = tx.commit();
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
                    "INSERT INTO branch_cache (base_branch, branch_name, merge_status, commit_hash)
                     VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(base_branch, branch_name) DO UPDATE SET
                         merge_status = excluded.merge_status,
                         commit_hash = excluded.commit_hash",
                    params![
                        base_scope(self.base_branch.as_deref()),
                        branch_name,
                        entry.merge_status,
                        entry.commit_hash
                    ],
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
                    "DELETE FROM branch_cache WHERE base_branch = ?1 AND branch_name = ?2",
                    params![base_scope(self.base_branch.as_deref()), branch_name],
                )
                .is_err()
            {
                return;
            }
        }

        if write_base_tip {
            let meta_key = base_tip_meta_key(self.base_branch.as_deref());
            if let Some(base_tip) = &self.mb_data.base_tip {
                if tx
                    .execute(
                        "INSERT INTO meta (key, value) VALUES (?1, ?2)
                         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                        params![meta_key, base_tip],
                    )
                    .is_err()
                {
                    return;
                }
            } else if tx
                .execute("DELETE FROM meta WHERE key = ?1", params![meta_key])
                .is_err()
            {
                return;
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
            "cherry_picked" => MergeStatus::CherryPicked,
            "local_cherry_picked" => MergeStatus::LocalCherryPicked,
            "remote_cherry_picked" => MergeStatus::RemoteCherryPicked,
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
            | MergeStatus::RemoteSquashMerged
            | MergeStatus::LocalCherryPicked
            | MergeStatus::RemoteCherryPicked => {
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
            MergeStatus::CherryPicked => "cherry_picked",
            MergeStatus::LocalCherryPicked => "local_cherry_picked",
            MergeStatus::RemoteCherryPicked => "remote_cherry_picked",
            MergeStatus::Unmerged => "unmerged",
            MergeStatus::LikelySquashMerged => {
                span.record("inserted", false);
                span.record("result_state", "skipped_likely_squash_merged");
                return;
            }
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
        let _ = fs::remove_file(sidecar_path(&self.path, "-wal"));
        let _ = fs::remove_file(sidecar_path(&self.path, "-shm"));
    }
}

fn sidecar_path(database: &Path, suffix: &str) -> PathBuf {
    let mut value = database.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
}

fn base_scope(base_branch: Option<&str>) -> &str {
    base_branch.unwrap_or("")
}

fn base_tip_meta_key(base_branch: Option<&str>) -> String {
    format!("base_tip:{}", base_scope(base_branch))
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
            base_branch  TEXT NOT NULL,
            branch_name  TEXT NOT NULL,
            merge_status TEXT NOT NULL,
            commit_hash  TEXT NOT NULL,
            PRIMARY KEY (base_branch, branch_name)
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
        CREATE TABLE IF NOT EXISTS commit_patch_id (
            key TEXT PRIMARY KEY,
            patch_id TEXT
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
    base_branch: Option<&str>,
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
        read_entries(&conn, base_scope(base_branch)),
        read_ahead_behind(&conn),
        read_merge_base(&conn),
        read_base_tip(&conn, &base_tip_meta_key(base_branch)),
        read_graph_patch(&conn),
    )
}

fn read_entries(conn: &Connection, base_branch: &str) -> HashMap<String, CacheEntry> {
    let Ok(mut stmt) = conn.prepare(
        "SELECT branch_name, merge_status, commit_hash FROM branch_cache WHERE base_branch = ?1",
    ) else {
        return HashMap::new();
    };
    let Ok(rows) = stmt.query_map([base_branch], |row| {
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

fn read_base_tip(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| {
        row.get::<_, String>(0)
    })
    .ok()
}

#[cfg(test)]
fn cache_path(repo_path: &Path) -> PathBuf {
    CacheRoot::from_env().cache_path(repo_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    const DAY: u64 = 24 * 60 * 60;

    fn temp_cache() -> (TempDir, BranchCache) {
        let dir = TempDir::new().unwrap();
        let cache = BranchCache::load_from_path(dir.path().join("cache.sqlite3"));
        (dir, cache)
    }

    fn git(directory: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(directory)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn set_mtime(path: &Path, modified: std::time::SystemTime) {
        let file = fs::OpenOptions::new().write(true).open(path).unwrap();
        file.set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();
    }

    #[test]
    fn cache_path_uses_common_git_directory_for_worktrees() {
        let fixture = TempDir::new().unwrap();
        let main = fixture.path().join("main");
        let worktree = fixture.path().join("linked-worktree");
        let clone = fixture.path().join("separate-clone");
        fs::create_dir(&main).unwrap();
        git(&main, &["init", "-b", "main"]);
        git(&main, &["config", "user.name", "Cache Test"]);
        git(&main, &["config", "user.email", "cache@example.com"]);
        fs::write(main.join("readme"), "repo\n").unwrap();
        git(&main, &["add", "readme"]);
        git(&main, &["commit", "-m", "initial"]);
        git(
            &main,
            &[
                "worktree",
                "add",
                "-b",
                "feature",
                worktree.to_str().unwrap(),
            ],
        );
        git(
            fixture.path(),
            &[
                "clone",
                "--quiet",
                main.to_str().unwrap(),
                clone.to_str().unwrap(),
            ],
        );

        let root = CacheRoot::at(fixture.path().join("cache"));
        assert_eq!(
            root.cache_path(&main),
            root.cache_path(&worktree),
            "linked worktrees share a cache database"
        );
        assert_ne!(
            root.cache_path(&main),
            root.cache_path(&clone),
            "separate clones keep independent cache databases"
        );
    }

    #[test]
    fn status_rows_and_base_tips_are_scoped_while_oid_values_are_shared() {
        let dir = TempDir::new().unwrap();
        let cache_path = dir.path().join("cache.sqlite3");
        let branch_tip = git2::Oid::from_str("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
        let base_tip = git2::Oid::from_str("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap();

        let mut main_cache = BranchCache::load_from_path_for_base(cache_path.clone(), Some("main"));
        main_cache.insert("feature/x", &MergeStatus::Merged, "feature-tip");
        main_cache.set_base_tip(Some("main-tip".to_string()));
        main_cache.insert_ahead_behind(branch_tip, base_tip, 2, 3);
        main_cache.save();

        let mut release_cache =
            BranchCache::load_from_path_for_base(cache_path.clone(), Some("release"));
        assert_eq!(release_cache.lookup("feature/x", "feature-tip"), None);
        assert_eq!(release_cache.mb_data.base_tip, None);
        assert_eq!(
            release_cache.lookup_ahead_behind(branch_tip, base_tip),
            Some((2, 3)),
            "OID-keyed values remain shared across base scopes"
        );
        release_cache.insert("feature/x", &MergeStatus::Unmerged, "feature-tip");
        release_cache.set_base_tip(Some("release-tip".to_string()));
        release_cache.save();

        let mut main_cache =
            BranchCache::load_from_path_for_base(cache_path.clone(), Some("main"));
        let release_cache =
            BranchCache::load_from_path_for_base(cache_path.clone(), Some("release"));
        assert_eq!(
            main_cache.lookup("feature/x", "feature-tip"),
            Some(MergeStatus::Merged)
        );
        assert_eq!(main_cache.mb_data.base_tip.as_deref(), Some("main-tip"));
        assert_eq!(
            release_cache.lookup("feature/x", "feature-tip"),
            Some(MergeStatus::Unmerged)
        );
        assert_eq!(
            release_cache.mb_data.base_tip.as_deref(),
            Some("release-tip")
        );
        main_cache.clear();
        assert!(!cache_path.exists(), "clearing removes the shared database");
    }

    #[test]
    fn stale_cache_pruning_expires_databases_and_orphan_sidecars_only() {
        let dir = TempDir::new().unwrap();
        let root = CacheRoot::at(dir.path());
        let now = std::time::SystemTime::now();
        let stale = now - std::time::Duration::from_secs(61 * DAY);
        let recent = now - std::time::Duration::from_secs(59 * DAY);
        let expired_db = dir.path().join("git-bm-repo-cache-e11e.sqlite3");
        let expired_legacy_db = dir.path().join("git-bm-cache-1e9ac7.sqlite3");
        let expired_wal = dir.path().join("git-bm-repo-cache-e11e.sqlite3-wal");
        let expired_shm = dir.path().join("git-bm-repo-cache-e11e.sqlite3-shm");
        let old_orphan_wal = dir.path().join("git-bm-cache-0bad.sqlite3-wal");
        let old_orphan_shm = dir.path().join("git-bm-cache-0bad.sqlite3-shm");
        let recent_db = dir.path().join("git-bm-cache-cafe.sqlite3");
        let unrelated = dir.path().join("notes.sqlite3");
        let unrelated_journal = dir.path().join("git-bm-cache-journal.sqlite3-journal");

        for path in [
            &expired_db,
            &expired_legacy_db,
            &expired_wal,
            &expired_shm,
            &old_orphan_wal,
            &old_orphan_shm,
            &recent_db,
            &unrelated,
            &unrelated_journal,
        ] {
            fs::write(path, "test cache").unwrap();
        }
        for path in [
            &expired_db,
            &expired_legacy_db,
            &expired_wal,
            &expired_shm,
            &old_orphan_wal,
            &old_orphan_shm,
        ] {
            set_mtime(path, stale);
        }
        set_mtime(&recent_db, recent);
        set_mtime(&unrelated, stale);
        set_mtime(&unrelated_journal, stale);

        prune_stale_caches_at(&root, now);

        for path in [
            &expired_db,
            &expired_legacy_db,
            &expired_wal,
            &expired_shm,
            &old_orphan_wal,
            &old_orphan_shm,
        ] {
            assert!(
                !path.exists(),
                "stale cache artifact should be removed: {path:?}"
            );
        }
        assert!(recent_db.exists(), "cache younger than 60 days remains");
        assert!(unrelated.exists(), "unrecognized files are untouched");
        assert!(
            unrelated_journal.exists(),
            "non-WAL SQLite sidecars are untouched"
        );
    }

    #[test]
    fn failed_stale_database_removal_preserves_its_sidecars() {
        let dir = TempDir::new().unwrap();
        let root = CacheRoot::at(dir.path());
        let now = std::time::SystemTime::now();
        let stale = now - std::time::Duration::from_secs(61 * DAY);
        let database = dir.path().join("git-bm-repo-cache-a11ce.sqlite3");
        let wal = dir.path().join("git-bm-repo-cache-a11ce.sqlite3-wal");
        let shm = dir.path().join("git-bm-repo-cache-a11ce.sqlite3-shm");
        for path in [&database, &wal, &shm] {
            fs::write(path, "test cache").unwrap();
            set_mtime(path, stale);
        }

        prune_stale_caches_at_with(&root, now, |path| {
            if path == database {
                Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "injected database removal failure",
                ))
            } else {
                fs::remove_file(path)
            }
        });

        assert!(
            database.exists(),
            "failed database deletion is retried later"
        );
        assert!(
            wal.exists(),
            "WAL stays with a database that could not be removed"
        );
        assert!(
            shm.exists(),
            "SHM stays with a database that could not be removed"
        );
    }

    #[test]
    fn cache_filename_matcher_accepts_only_generated_hex_hashes() {
        assert_eq!(
            cache_database_name("git-bm-cache-deadbeef.sqlite3"),
            Some("git-bm-cache-deadbeef.sqlite3".to_string())
        );
        assert_eq!(
            cache_database_name("git-bm-repo-cache-123abc.sqlite3"),
            Some("git-bm-repo-cache-123abc.sqlite3".to_string())
        );
        for name in [
            "git-bm-cache-expired.sqlite3",
            "git-bm-repo-cache-xyz.sqlite3",
            "git-bm-cache-DEADBEEF.sqlite3",
            "git-bm-cache-deadbeef.sqlite3-journal",
        ] {
            assert_eq!(
                cache_database_name(name),
                None,
                "unexpected match for {name}"
            );
        }
    }

    #[test]
    fn loading_cache_without_writes_refreshes_its_last_used_time() {
        let dir = TempDir::new().unwrap();
        let root = CacheRoot::at(dir.path());
        let repo_dir = TempDir::new().unwrap();
        let db = root.cache_path(repo_dir.path());
        fs::create_dir_all(db.parent().unwrap()).unwrap();
        fs::write(&db, "not a sqlite db").unwrap();
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(61 * DAY);
        set_mtime(&db, old);

        let _cache = BranchCache::load_with_root(repo_dir.path(), &root);
        let now = std::time::SystemTime::now();
        prune_stale_caches_at(&root, now);

        assert!(
            db.exists(),
            "a recently loaded cache must survive pruning even without writes"
        );
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
    fn cache_never_inserts_likely_squash_merged() {
        let (_dir, mut cache) = temp_cache();
        cache.insert("feature/x", &MergeStatus::LikelySquashMerged, "abc123");
        assert_eq!(
            cache.lookup("feature/x", "abc123"),
            None,
            "a heuristic likely-squash-merge result must never be cached"
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
            .is_some_and(|name| {
                name.starts_with("git-bm-repo-cache-") && name.ends_with(".sqlite3")
            }));
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

    #[test]
    fn commit_patch_ids_round_trip_including_none() {
        let (_dir, cache) = temp_cache();
        cache.store_commit_patch_ids(&[("a:v1".into(), Some("p".into())), ("b:v1".into(), None)]);
        let found = cache.lookup_commit_patch_ids(&["a:v1".into(), "b:v1".into(), "c:v1".into()]);
        assert_eq!(found.get("a:v1"), Some(&Some("p".to_owned())));
        assert_eq!(found.get("b:v1"), Some(&None));
        assert!(!found.contains_key("c:v1"));
    }

    #[test]
    fn commit_patch_ids_old_table_absent_root_and_chunking() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/cache.sqlite3");
        let cache = BranchCache::load_from_path(path.clone());
        assert!(cache.lookup_commit_patch_ids(&[]).is_empty());
        assert!(cache
            .lookup_commit_patch_ids(&["missing".into()])
            .is_empty());
        assert!(!path.exists());
        let entries = (0..805)
            .map(|i| (format!("{i}:v1"), (i % 2 == 0).then(|| format!("patch{i}"))))
            .collect::<Vec<_>>();
        cache.store_commit_patch_ids(&entries);
        let keys = entries
            .iter()
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            cache.lookup_commit_patch_ids(&keys),
            entries.into_iter().collect()
        );
        let old = dir.path().join("old.sqlite3");
        let conn = Connection::open(&old).unwrap();
        conn.execute_batch("CREATE TABLE branch_cache (base_branch TEXT, branch_name TEXT, merge_status TEXT, commit_hash TEXT);").unwrap();
        drop(conn);
        let old_cache = BranchCache::load_from_path(old.clone());
        assert!(old_cache
            .lookup_commit_patch_ids(&["a:v1".into()])
            .is_empty());
        let conn = Connection::open(old).unwrap();
        assert_eq!(
            conn.query_row(
                "SELECT count(*) FROM sqlite_master WHERE name='commit_patch_id'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
    }
}
