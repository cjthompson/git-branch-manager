use super::db::sidecar_path;
use super::{BranchCache, CacheEntry, GraphPatchEntry, MergeBaseData};
use crate::types::MergeStatus;
use std::fs;
use tracing::{field, instrument, Span};

impl BranchCache {
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
