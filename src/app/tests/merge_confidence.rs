use super::*;

#[test]
fn squash_result_then_remote_enrich_does_not_regress_status() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    app.branches
        .set_items(vec![branch("feature/x", TrackingStatus::Local)]);
    app.remotes
        .set_items(vec![remote("origin/feature/x", "feature/x")]);

    let (squash_tx, squash_rx) = mpsc::channel();
    squash_tx
        .send(SquashResult {
            branch_name: "feature/x".into(),
            status: MergeStatus::SquashMerged,
            confidence: None,
        })
        .unwrap();
    app.squash_rx = Some(squash_rx);
    app.drain_channels();

    assert_eq!(
        app.remotes.items()[0].merge_status,
        MergeStatus::SquashMerged
    );

    let (enrich_tx, enrich_rx) = mpsc::channel();
    enrich_tx
        .send(RemoteEnrichResult {
            full_ref: "origin/feature/x".into(),
            merge_status: MergeStatus::Unmerged,
            ahead: Some(3),
            behind: Some(1),
            disjoint: false,
        })
        .unwrap();
    app.remote_enrich_rx = Some(enrich_rx);
    app.drain_channels();

    assert_eq!(
        app.remotes.items()[0].merge_status,
        MergeStatus::SquashMerged,
        "stale remote_enrich result must not regress an already-confirmed squash status"
    );
    assert_eq!(app.remotes.items()[0].ahead, Some(3));
    assert_eq!(app.remotes.items()[0].behind, Some(1));
}

#[test]
fn accepted_cherry_result_clears_local_likely_confidence() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    let mut local = branch("feature/x", TrackingStatus::Local);
    local.merge_status = MergeStatus::LikelySquashMerged;
    local.squash_confidence = Some(SquashConfidence::FuzzyMatch {
        similarity_percent: 82,
    });
    app.branches.set_items(vec![local]);

    let mut remote = remote("origin/feature/x", "feature/x");
    remote.merge_status = MergeStatus::LikelySquashMerged;
    remote.squash_confidence = Some(SquashConfidence::MergeTreeConfirmed);
    app.remotes.set_items(vec![remote]);

    let (cherry_tx, cherry_rx) = mpsc::channel();
    cherry_tx
        .send(CherryResult {
            branch_name: "feature/x".into(),
            status: MergeStatus::CherryPicked,
        })
        .unwrap();
    app.cherry_rx = Some(cherry_rx);
    app.drain_channels();

    assert_eq!(
        app.branches.items()[0].merge_status,
        MergeStatus::CherryPicked
    );
    assert_eq!(app.branches.items()[0].squash_confidence, None);
    assert_eq!(
        app.remotes.items()[0].merge_status,
        MergeStatus::CherryPicked
    );
    assert_eq!(
        app.remotes.items()[0].squash_confidence,
        None,
        "accepted cherry propagation must clear stale squash evidence too"
    );
}

#[test]
fn accepted_remote_enrich_result_clears_likely_confidence_and_updates_metadata() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    let mut remote = remote("origin/feature/x", "feature/x");
    remote.merge_status = MergeStatus::LikelySquashMerged;
    remote.squash_confidence = Some(SquashConfidence::MergeTreeConfirmed);
    app.remotes.set_items(vec![remote]);

    let (enrich_tx, enrich_rx) = mpsc::channel();
    enrich_tx
        .send(RemoteEnrichResult {
            full_ref: "origin/feature/x".into(),
            merge_status: MergeStatus::Merged,
            ahead: Some(4),
            behind: Some(2),
            disjoint: true,
        })
        .unwrap();
    app.remote_enrich_rx = Some(enrich_rx);
    app.drain_channels();

    let remote = &app.remotes.items()[0];
    assert_eq!(remote.merge_status, MergeStatus::Merged);
    assert_eq!(
        remote.squash_confidence, None,
        "an accepted non-squash status must clear stale heuristic evidence"
    );
    assert_eq!(remote.ahead, Some(4));
    assert_eq!(remote.behind, Some(2));
    assert!(remote.disjoint);
}

#[test]
fn rejected_unmerged_squash_result_preserves_local_likely_confidence() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    let mut local = branch("feature/x", TrackingStatus::Local);
    local.merge_status = MergeStatus::LikelySquashMerged;
    local.squash_confidence = Some(SquashConfidence::FuzzyMatch {
        similarity_percent: 82,
    });
    app.branches.set_items(vec![local]);

    let (squash_tx, squash_rx) = mpsc::channel();
    squash_tx
        .send(SquashResult {
            branch_name: "feature/x".into(),
            status: MergeStatus::Unmerged,
            confidence: None,
        })
        .unwrap();
    app.squash_rx = Some(squash_rx);
    app.drain_channels();

    assert_eq!(
        app.branches.items()[0].merge_status,
        MergeStatus::LikelySquashMerged
    );
    assert_eq!(
        app.branches.items()[0].squash_confidence,
        Some(SquashConfidence::FuzzyMatch {
            similarity_percent: 82,
        }),
        "rejecting a weaker status must reject its confidence transition too"
    );
}

#[test]
fn rejected_unmerged_remote_squash_result_preserves_likely_confidence() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    let mut remote = remote("origin/feature/x", "feature/x");
    remote.merge_status = MergeStatus::LikelySquashMerged;
    remote.squash_confidence = Some(SquashConfidence::FuzzyMatch {
        similarity_percent: 79,
    });
    app.remotes.set_items(vec![remote]);

    let (squash_tx, squash_rx) = mpsc::channel();
    squash_tx
        .send(SquashResult {
            branch_name: "origin/feature/x".into(),
            status: MergeStatus::Unmerged,
            confidence: None,
        })
        .unwrap();
    app.remote_squash_rx = Some(squash_rx);
    app.drain_channels();

    assert_eq!(
        app.remotes.items()[0].merge_status,
        MergeStatus::LikelySquashMerged
    );
    assert_eq!(
        app.remotes.items()[0].squash_confidence,
        Some(SquashConfidence::FuzzyMatch {
            similarity_percent: 79,
        }),
        "a rejected direct remote result must preserve its existing evidence"
    );
}

#[test]
fn rejected_propagated_likely_result_does_not_attach_confidence_to_exact_status() {
    let tmpdir = tempfile::tempdir().expect("temp repo");
    let mut app = App::new(
        tmpdir.path().to_path_buf(),
        "main".into(),
        Config::default(),
    );
    app.branches
        .set_items(vec![branch("feature/x", TrackingStatus::Local)]);
    let mut remote = remote("origin/feature/x", "feature/x");
    remote.merge_status = MergeStatus::SquashMerged;
    app.remotes.set_items(vec![remote]);

    let (squash_tx, squash_rx) = mpsc::channel();
    squash_tx
        .send(SquashResult {
            branch_name: "feature/x".into(),
            status: MergeStatus::LikelySquashMerged,
            confidence: Some(SquashConfidence::MergeTreeConfirmed),
        })
        .unwrap();
    app.squash_rx = Some(squash_rx);
    app.drain_channels();

    assert_eq!(
        app.branches.items()[0].merge_status,
        MergeStatus::LikelySquashMerged
    );
    assert_eq!(
        app.branches.items()[0].squash_confidence,
        Some(SquashConfidence::MergeTreeConfirmed)
    );
    assert_eq!(
        app.remotes.items()[0].merge_status,
        MergeStatus::SquashMerged
    );
    assert_eq!(
        app.remotes.items()[0].squash_confidence,
        None,
        "rejected heuristic evidence must not be attached to an exact status"
    );
}
