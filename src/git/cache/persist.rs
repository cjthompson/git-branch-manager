use super::db::{base_scope, base_tip_meta_key, ensure_schema, open_conn, read_all};
use super::prune::refresh_cache_timestamp;
use super::{BranchCache, CacheRoot, MergeBaseData};
use rusqlite::params;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{field, instrument, Span};

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

    pub(super) fn load_from_path_for_base(path: PathBuf, base_branch: Option<&str>) -> Self {
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
}
