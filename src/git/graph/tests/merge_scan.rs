use super::*;

#[test]
fn cherry_scan_merge_yields_consume_source_and_destination_budgets() {
    let (dir, snapshot, root) = merge_pair_fixture();
    let cache = BranchCache::load_for_base(dir.path(), "main", &root);
    let scan = |sources, destinations| {
        compute_cherry_pick_relationships_with_bounds(
            dir.path(),
            &snapshot,
            "main",
            &snapshot.commits[0].oid,
            &cache,
            &EnrichmentCancel::never(),
            sources,
            destinations,
        )
    };
    assert!(
        scan(1, 10).is_empty(),
        "merge consumes first source yield so whole tip exceeds 1"
    );
    assert!(
        scan(2, 1).is_empty(),
        "merge consumes destination slot before the matching commit"
    );
    assert_eq!(
        scan(2, 2).len(),
        1,
        "complete match in bounded prefix survives destination overflow"
    );
    assert_eq!(
        scan(2, 3).len(),
        1,
        "exactly exhausted source/destination scans preserve pair"
    );
    let latest = Arc::new(AtomicU64::new(2));
    assert!(compute_cherry_pick_relationships_with_bounds(
        dir.path(),
        &snapshot,
        "main",
        &snapshot.commits[0].oid,
        &cache,
        &EnrichmentCancel::new(latest, 1),
        2,
        3
    )
    .is_empty());
}

fn assert_merge_worker_cancelled(stage: EnrichmentStage) {
    let (dir, snapshot, root) = merge_pair_fixture();
    let latest = Arc::new(AtomicU64::new(1));
    let (started_tx, started_rx) = mpsc::sync_channel(0);
    let (resume_tx, resume_rx) = mpsc::sync_channel(0);
    let resume_rx = Mutex::new(resume_rx);
    let observed = Arc::new(AtomicU64::new(0));
    let worker_observed = Arc::clone(&observed);
    let rx = spawn_enrichment_with_observer(
        snapshot,
        dir.path().to_path_buf(),
        Some("main".into()),
        1,
        root,
        EnrichmentCancel::new(Arc::clone(&latest), 1),
        move |current| {
            if current == stage {
                worker_observed.fetch_add(1, Ordering::Relaxed);
                started_tx.send(()).unwrap();
                resume_rx.lock().unwrap().recv().unwrap();
            }
        },
    );
    let timeout = std::time::Duration::from_secs(60);
    started_rx
        .recv_timeout(timeout)
        .expect("actual worker reached real merge revwalk yield");
    latest.store(2, Ordering::Release);
    resume_tx.send(()).unwrap();
    assert_eq!(
        rx.recv_timeout(timeout),
        Err(mpsc::RecvTimeoutError::Disconnected)
    );
    assert_eq!(
        observed.load(Ordering::Relaxed),
        1,
        "cancellation before merge skip stops subsequent work"
    );
}

#[test]
fn started_enrichment_merge_source_revwalk_cancelled_publishes_nothing() {
    assert_merge_worker_cancelled(EnrichmentStage::CherrySourceYield);
}

#[test]
fn started_enrichment_merge_destination_revwalk_cancelled_publishes_nothing() {
    assert_merge_worker_cancelled(EnrichmentStage::CherryDestinationYield);
}

#[test]
fn started_enrichment_cherry_patch_computation_cancelled_publishes_nothing() {
    assert_merge_worker_cancelled(EnrichmentStage::CherryPatchCompute);
}
