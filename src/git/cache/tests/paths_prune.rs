use super::*;
use crate::git::cache::prune::{
    cache_database_name, prune_stale_caches_at, prune_stale_caches_at_with,
};

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
