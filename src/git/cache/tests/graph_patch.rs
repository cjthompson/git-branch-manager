use super::*;
use rusqlite::Connection;

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
