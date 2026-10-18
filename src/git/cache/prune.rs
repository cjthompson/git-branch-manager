use super::CacheRoot;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

const CACHE_RETENTION: std::time::Duration = std::time::Duration::from_secs(60 * 24 * 60 * 60);

/// Remove app-owned cache databases that have not been used for 60 days.
/// Failures are intentionally ignored because cleanup must never block startup.
pub fn prune_stale_caches(cache_root: &CacheRoot) {
    prune_stale_caches_at(cache_root, std::time::SystemTime::now());
}

pub(super) fn prune_stale_caches_at(cache_root: &CacheRoot, now: std::time::SystemTime) {
    prune_stale_caches_at_with(cache_root, now, |path| fs::remove_file(path));
}

pub(super) fn prune_stale_caches_at_with<F>(
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

pub(super) fn cache_database_name(name: &str) -> Option<String> {
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

pub(super) fn refresh_cache_timestamp(path: &Path) {
    if !path.exists() {
        return;
    }
    if let Ok(file) = fs::File::open(path) {
        let _ =
            file.set_times(std::fs::FileTimes::new().set_modified(std::time::SystemTime::now()));
    }
}
