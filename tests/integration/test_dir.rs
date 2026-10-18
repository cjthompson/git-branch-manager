

use crate::common::*;

#[test]
fn testdir_removes_cache_root_and_sqlite_sidecars_on_drop() {
    let cache_root = {
        let (test_dir, _repo) = setup_test_repo();
        let cache_root = test_dir.cache_root().to_path_buf();
        for suffix in ["", "-wal", "-shm"] {
            std::fs::write(
                cache_root.join(format!("git-bm-repo-cache-test.sqlite3{suffix}")),
                "cache data",
            )
            .unwrap();
        }
        cache_root
    };

    assert!(
        !cache_root.exists(),
        "TestDir drop must remove its temporary cache root"
    );
}

#[test]
fn testdir_preserves_repo_and_cache_roots_when_requested() {
    let (mut test_dir, repo) = setup_test_repo();
    let cache_marker = test_dir.cache_root().join("kept.sqlite3");
    std::fs::write(&cache_marker, "cache data").unwrap();
    let repo_path = test_dir.path().to_path_buf();
    let cache_path = test_dir.cache_root().to_path_buf();

    let (kept_repo, kept_cache) = test_dir.preserve_tempdirs();
    drop(test_dir);
    drop(repo);

    assert_eq!(kept_repo.as_deref(), Some(repo_path.as_path()));
    assert_eq!(kept_cache.as_deref(), Some(cache_path.as_path()));
    assert!(repo_path.exists(), "preserving keeps the test repository");
    assert!(cache_marker.exists(), "preserving keeps the test cache");
    std::fs::remove_dir_all(repo_path).unwrap();
    std::fs::remove_dir_all(cache_path).unwrap();
}
