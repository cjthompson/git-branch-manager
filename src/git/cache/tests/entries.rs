use super::*;
use crate::types::MergeStatus;

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
