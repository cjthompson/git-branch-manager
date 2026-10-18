mod entries;
mod graph_patch;
mod paths_prune;

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
