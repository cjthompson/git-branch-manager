mod db;
mod entries;
mod persist;
mod prune;
#[cfg(test)]
mod tests;

pub use prune::prune_stale_caches;

use std::cell::{Cell, RefCell};
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

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

#[cfg(test)]
fn cache_path(repo_path: &Path) -> PathBuf {
    CacheRoot::from_env().cache_path(repo_path)
}
