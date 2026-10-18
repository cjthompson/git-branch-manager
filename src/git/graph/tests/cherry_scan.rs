use super::*;

#[test]
fn cherry_scan_bounds_count_all_yields_and_keep_complete_pairs() {
    let (dir, repo, source, destination, mut snapshot, root) = native_pair_fixture();
    let cache = BranchCache::load_for_base(dir.path(), "main", &root);
    let scan = |snapshot: &GraphSnapshot, source_limit, destination_limit| {
        compute_cherry_pick_relationships_with_bounds(
            dir.path(),
            snapshot,
            "main",
            &snapshot.commits[0].oid,
            &cache,
            &EnrichmentCancel::never(),
            source_limit,
            destination_limit,
        )
    };
    assert_eq!(
        scan(&snapshot, 1, 1).len(),
        1,
        "exact bounds accept the complete pair outside displayed merge base"
    );
    assert!(
        scan(&snapshot, 0, 10).is_empty(),
        "source overflow skips entire tip"
    );
    assert!(scan(&snapshot, 1, 0).is_empty());
    let mut base_tip = destination;
    for index in 0..2_000 {
        base_tip = native_commit(
            &repo,
            Some(base_tip),
            &[("unrelated", "base\n"), ("picked", "picked\n")],
            &format!("base filler {index}"),
        );
    }
    snapshot.commits[0].oid = base_tip.to_string();
    assert!(
        scan(&snapshot, 500, MAX_CHERRY_DESTINATION_SCAN).is_empty(),
        "destination 2001 is outside production bound"
    );
    let pairs = scan(&snapshot, 500, 2_001);
    assert_eq!(pairs.len(), 1);
    assert_eq!(pairs[0].destination_oid, destination.to_string());
    let mut source_tip = source;
    for index in 0..499 {
        source_tip = native_commit(
            &repo,
            Some(source_tip),
            &[("picked", "picked\n")],
            &format!("source filler {index}"),
        );
    }
    snapshot.commits[1].oid = source_tip.to_string();
    assert_eq!(scan(&snapshot, MAX_CHERRY_SOURCE_COMMITS, 2_002).len(), 1);
    source_tip = native_commit(
        &repo,
        Some(source_tip),
        &[("picked", "picked\n")],
        "source 501",
    );
    snapshot.commits[1].oid = source_tip.to_string();
    assert!(scan(&snapshot, MAX_CHERRY_SOURCE_COMMITS, 2_002).is_empty());
    let mut valid = snapshot.commits[1].clone();
    valid.oid = source.to_string();
    valid.refs[0].name = "feature/valid".into();
    snapshot.commits.insert(1, valid);
    let retained = scan(&snapshot, MAX_CHERRY_SOURCE_COMMITS, 2_002);
    assert_eq!(
        retained.len(),
        1,
        "over-bound later tip does not erase earlier complete pairs"
    );
    assert_eq!(retained[0].source_refs, ["feature/valid"]);
    let fresh_root = CacheRoot::at(dir.path().join("fresh-bound-cache"));
    let fresh_cache = BranchCache::load_for_base(dir.path(), "main", &fresh_root);
    assert!(compute_cherry_pick_relationships_with_bounds(
        dir.path(),
        &snapshot,
        "main",
        &snapshot.commits[0].oid,
        &fresh_cache,
        &EnrichmentCancel::never(),
        500,
        2_000
    )
    .is_empty());
    assert!(
        !fresh_cache
            .lookup_commit_patch_ids(&[format!("{destination}:v1")])
            .contains_key(&format!("{destination}:v1")),
        "no patch computation beyond destination budget"
    );
}

#[test]
fn cherry_scan_preserves_equal_patch_source_buckets_and_ref_unions() {
    let (dir, repo, first_source, first_dest, mut snapshot, root) = native_pair_fixture();
    let remove_source = native_commit(&repo, Some(first_source), &[], "source revert");
    let second_source = native_commit(
        &repo,
        Some(remove_source),
        &[("picked", "picked\n")],
        "source repick",
    );
    let remove_dest = native_commit(
        &repo,
        Some(first_dest),
        &[("unrelated", "base\n")],
        "destination revert",
    );
    let second_dest = native_commit(
        &repo,
        Some(remove_dest),
        &[("unrelated", "base\n"), ("picked", "picked\n")],
        "destination repick",
    );
    snapshot.commits[0].oid = second_dest.to_string();
    snapshot.commits[1].oid = second_source.to_string();
    let mut older = snapshot.commits[1].clone();
    older.oid = first_source.to_string();
    older.refs[0].name = "feature/older".into();
    snapshot.commits.push(older);
    let cache = BranchCache::load_for_base(dir.path(), "main", &root);
    let pairs = compute_cherry_pick_relationships_with_bounds(
        dir.path(),
        &snapshot,
        "main",
        &second_dest.to_string(),
        &cache,
        &EnrichmentCancel::never(),
        500,
        2_000,
    );
    let insertions = pairs
        .iter()
        .filter(|r| {
            r.source_oid == first_source.to_string()
                || r.source_oid == second_source.to_string()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        insertions.len(),
        4,
        "two equal source patches times two destinations"
    );
    for source in [first_source, second_source] {
        assert_eq!(
            insertions
                .iter()
                .filter(|r| r.source_oid == source.to_string())
                .map(|r| r.destination_oid.clone())
                .collect::<std::collections::BTreeSet<_>>(),
            [first_dest.to_string(), second_dest.to_string()]
                .into_iter()
                .collect()
        );
    }
    for pair in insertions
        .iter()
        .filter(|r| r.source_oid == first_source.to_string())
    {
        assert_eq!(pair.source_refs, ["feature", "feature/older"]);
    }
    assert!(
        pairs.windows(2).all(|w| w[0].key() < w[1].key()),
        "deduplicated deterministic order"
    );
}

#[test]
fn cherry_scan_reuses_commit_cache_including_none() {
    let (dir, repo, source, destination, mut snapshot, root) = native_pair_fixture();
    snapshot.commits[1].oid = native_commit(
        &repo,
        Some(source),
        &[("picked", "picked\n")],
        "empty source",
    )
    .to_string();
    let cache = BranchCache::load_for_base(dir.path(), "main", &root);
    let calls = Arc::new(AtomicU64::new(0));
    let observer_calls = Arc::clone(&calls);
    let observer: EnrichmentObserver = Arc::new(move |stage| {
        if stage == EnrichmentStage::CherryPatchCompute {
            observer_calls.fetch_add(1, Ordering::Relaxed);
        }
    });
    let first = compute_cherry_pick_relationships_observed(
        dir.path(),
        &snapshot,
        "main",
        &destination.to_string(),
        &cache,
        &EnrichmentCancel::never(),
        500,
        2_000,
        &observer,
    );
    assert_eq!(first.len(), 1);
    assert!(calls.load(Ordering::Relaxed) > 0);
    let key = format!("{}:v1", snapshot.commits[1].oid);
    assert_eq!(
        cache
            .lookup_commit_patch_ids(std::slice::from_ref(&key))
            .get(&key),
        Some(&None)
    );
    calls.store(0, Ordering::Relaxed);
    let second = compute_cherry_pick_relationships_observed(
        dir.path(),
        &snapshot,
        "main",
        &destination.to_string(),
        &cache,
        &EnrichmentCancel::never(),
        500,
        2_000,
        &observer,
    );
    assert_eq!(second, first);
    assert_eq!(
        calls.load(Ordering::Relaxed),
        0,
        "cached None must skip empty diff recomputation"
    );
}

#[test]
fn cherry_scan_contained_tip_emits_nothing() {
    let (dir, _repo, _source, _dest, mut snapshot, root) = native_pair_fixture();
    snapshot.commits.truncate(1);
    let mut alias = snapshot.commits[0].refs[0].clone();
    alias.name = "feature/contained".into();
    snapshot.commits[0].refs.push(alias);
    let cache = BranchCache::load_for_base(dir.path(), "main", &root);
    assert!(compute_cherry_pick_relationships(
        dir.path(),
        &snapshot,
        "main",
        &snapshot.commits[0].oid,
        &cache,
        &EnrichmentCancel::never()
    )
    .is_empty());
}
