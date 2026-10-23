use super::super::cherry::compute_cross_branch_cherry_relationships;
use super::*;

fn fixture(
    source_message: &str,
    destination_message: &str,
    destination_text: &str,
) -> (tempfile::TempDir, GraphSnapshot, CacheRoot) {
    let dir = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(dir.path()).unwrap();
    let root = native_commit(&repo, None, &[], "root");
    let source = native_commit(&repo, Some(root), &[("picked", "same\n")], source_message);
    let advance = native_commit(&repo, Some(root), &[("other", "other\n")], "advance");
    let destination = native_commit(
        &repo,
        Some(advance),
        &[("other", "other\n"), ("picked", destination_text)],
        destination_message,
    );
    let commits = [
        (destination, "integration"),
        (source, "feature"),
        (root, "main"),
    ]
    .into_iter()
    .map(|(oid, name)| {
        repo.reference(&format!("refs/heads/{name}"), oid, true, "fixture")
            .unwrap();
        let commit = repo.find_commit(oid).unwrap();
        GraphCommit {
            oid: oid.to_string(),
            summary: commit.summary().unwrap().unwrap().into(),
            parents: commit.parent_ids().map(|id| id.to_string()).collect(),
            refs: vec![GraphRef {
                name: name.into(),
                kind: GraphRefKind::LocalBranch,
                has_linked_worktree: false,
                is_current: false,
                tracking: None,
                ahead_of_base: None,
            }],
            ..Default::default()
        }
    })
    .collect();
    repo.set_head("refs/heads/integration").unwrap();
    let cache = CacheRoot::at(dir.path().join("cache"));
    (dir, relationship_snapshot(commits), cache)
}

#[test]
fn full_graph_loader_detects_pick_between_non_base_branches_by_first_line() {
    let (dir, expected, root) = fixture(
        "fix checkout\noriginal continuation",
        "fix checkout\ndifferent continuation",
        "same\n",
    );
    let snapshot = load_graph_with_squash_annotations(
        dir.path(),
        GraphLoadOptions {
            base_branch: Some("main".into()),
            cache_root: root,
            ..Default::default()
        },
    )
    .unwrap();
    for commit in &expected.commits[..2] {
        let actual = snapshot
            .commits
            .iter()
            .find(|c| c.oid == commit.oid)
            .unwrap();
        assert!(actual.is_cherry_picked_commit());
    }
}

#[test]
fn matching_subjects_detect_cross_branch_picks_with_different_bodies_and_mark_both_endpoints() {
    let (dir, mut snapshot, cache) = fixture(
        "fix checkout\n\nOriginal body",
        "fix checkout\n\nDifferent body",
        "same\n",
    );
    let updates = compute_relationships(
        dir.path(),
        &snapshot,
        Some("main"),
        &cache,
        &EnrichmentCancel::never(),
    );
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].kind, RelationshipKind::CherryPick);
    let endpoints = [
        updates[0].source_oid.clone(),
        updates[0].destination_oid.clone(),
    ]
    .into_iter()
    .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        endpoints,
        snapshot.commits[..2]
            .iter()
            .map(|c| c.oid.clone())
            .collect()
    );
    assert!(updates[0].source_refs.iter().all(|name| name != "main"));
    assert!(updates[0]
        .destination_refs
        .iter()
        .all(|name| name != "main"));
    apply_squash_enrichment(&mut snapshot, &updates);
    assert!(snapshot.commits[0].is_cherry_picked_commit());
    assert!(snapshot.commits[1].is_cherry_picked_commit());
    assert!(!snapshot.commits[2].is_cherry_picked_commit());
}

#[test]
fn subject_filter_skips_unique_subjects_and_patch_ids_reject_different_changes() {
    for (destination_message, destination_text, expected_computations) in [
        ("different subject", "same\n", 0),
        ("fix checkout", "different patch\n", 2),
    ] {
        let (dir, snapshot, root) = fixture("fix checkout", destination_message, destination_text);
        let cache = BranchCache::load_for_base(dir.path(), "main", &root);
        let calls = Arc::new(AtomicU64::new(0));
        let observed = Arc::clone(&calls);
        let observer: EnrichmentObserver = Arc::new(move |stage| {
            if stage == EnrichmentStage::CherryPatchCompute {
                observed.fetch_add(1, Ordering::Relaxed);
            }
        });
        assert!(compute_cross_branch_cherry_relationships(
            dir.path(),
            &snapshot,
            &snapshot.commits[2].oid,
            &cache,
            &EnrichmentCancel::never(),
            &observer,
        )
        .is_empty());
        assert_eq!(calls.load(Ordering::Relaxed), expected_computations);
    }
}

#[test]
fn cross_branch_scan_handles_historical_commits_with_branch_labels_and_reuses_cache() {
    let (dir, mut snapshot, root) = fixture("fix checkout", "fix checkout\n\nPicked", "same\n");
    for commit in &mut snapshot.commits[..2] {
        let reference = commit.refs.remove(0);
        commit.branch = Some(GraphBranchLabel {
            name: reference.name,
            kind: reference.kind,
            target_oid: commit.oid.clone(),
        });
    }
    let cache = BranchCache::load_for_base(dir.path(), "main", &root);
    let calls = Arc::new(AtomicU64::new(0));
    let observed = Arc::clone(&calls);
    let observer: EnrichmentObserver = Arc::new(move |stage| {
        if stage == EnrichmentStage::CherryPatchCompute {
            observed.fetch_add(1, Ordering::Relaxed);
        }
    });
    for expected_calls in [2, 0] {
        calls.store(0, Ordering::Relaxed);
        let updates = compute_cross_branch_cherry_relationships(
            dir.path(),
            &snapshot,
            &snapshot.commits[2].oid,
            &cache,
            &EnrichmentCancel::never(),
            &observer,
        );
        assert_eq!(updates.len(), 1);
        assert_eq!(calls.load(Ordering::Relaxed), expected_calls);
    }
}

#[test]
fn repeated_patch_in_the_same_ancestry_is_not_a_cross_branch_pick() {
    let (dir, mut snapshot, root) = fixture("fix checkout", "fix checkout", "same\n");
    let repo = git2::Repository::open(dir.path()).unwrap();
    let source = git2::Oid::from_str(&snapshot.commits[1].oid).unwrap();
    let reverted = native_commit(&repo, Some(source), &[], "revert");
    let repeated = native_commit(
        &repo,
        Some(reverted),
        &[("picked", "same\n")],
        "fix checkout",
    );
    snapshot.commits[0].oid = repeated.to_string();
    snapshot.commits[0].parents = vec![reverted.to_string()];
    let cache = BranchCache::load_for_base(dir.path(), "main", &root);
    let observer: EnrichmentObserver = Arc::new(|_| {});
    assert!(compute_cross_branch_cherry_relationships(
        dir.path(),
        &snapshot,
        &snapshot.commits[2].oid,
        &cache,
        &EnrichmentCancel::never(),
        &observer,
    )
    .is_empty());
}

#[test]
fn cross_branch_scan_cancels_before_computing_or_caching_patches() {
    let (dir, snapshot, root) = fixture("fix checkout", "fix checkout", "same\n");
    let cache = BranchCache::load_for_base(dir.path(), "main", &root);
    let latest = Arc::new(AtomicU64::new(1));
    let cancel = EnrichmentCancel::new(Arc::clone(&latest), 1);
    let observer: EnrichmentObserver = Arc::new(move |stage| {
        if stage == EnrichmentStage::CherryPatchCompute {
            latest.store(2, Ordering::Release);
        }
    });
    assert!(compute_cross_branch_cherry_relationships(
        dir.path(),
        &snapshot,
        &snapshot.commits[2].oid,
        &cache,
        &cancel,
        &observer,
    )
    .is_empty());
    assert!(cancel.is_cancelled());
    let keys = snapshot.commits[..2]
        .iter()
        .map(|c| format!("{}:v1", c.oid))
        .collect::<Vec<_>>();
    assert!(cache.lookup_commit_patch_ids(&keys).is_empty());
}
