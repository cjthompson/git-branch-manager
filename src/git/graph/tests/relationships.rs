use super::*;

fn commit(oid: &str, parents: &[&str]) -> GraphCommit {
    GraphCommit {
        oid: oid.into(),
        summary: oid.into(),
        parents: parents.iter().map(|parent| (*parent).into()).collect(),
        ..GraphCommit::default()
    }
}

#[test]
fn displayed_history_bounds_branch_relationships() {
    let commits = vec![
        commit("base-tip", &["base-parent"]),
        commit("branch-tip", &["shared"]),
        commit("base-parent", &["shared"]),
        commit("shared", &["root"]),
        commit("root", &[]),
    ];
    assert_eq!(
        displayed_branch_relation(&commits, "base-tip", "branch-tip"),
        DisplayedBranchRelation::Diverged {
            merge_base: "shared".into()
        }
    );

    let regular = vec![
        commit("base-tip", &["branch-tip"]),
        commit("branch-tip", &["root"]),
        commit("root", &[]),
    ];
    assert_eq!(
        displayed_branch_relation(&regular, "base-tip", "branch-tip"),
        DisplayedBranchRelation::RegularlyMerged
    );

    let truncated = vec![
        commit("base-tip", &["missing-base-parent"]),
        commit("branch-tip", &["missing-branch-parent"]),
    ];
    assert_eq!(
        displayed_branch_relation(&truncated, "base-tip", "branch-tip"),
        DisplayedBranchRelation::Ineligible
    );
}

#[test]
fn enrichment_never_erases_and_attaches_every_endpoint() {
    let existing = squash(
        "d",
        "s",
        &["feature/old"],
        RelationshipMatch::Fuzzy {
            similarity_percent: 80,
        },
    );
    let mut snapshot = relationship_snapshot(vec![
        GraphCommit {
            oid: "d".into(),
            relationships: vec![existing.clone()],
            ..Default::default()
        },
        GraphCommit {
            oid: "s".into(),
            ..Default::default()
        },
    ]);
    apply_squash_enrichment(&mut snapshot, &[]);
    assert_eq!(snapshot.commits[0].relationships.as_slice(), std::slice::from_ref(&existing));
    let fresh = GraphRelationship {
        matching: RelationshipMatch::Exact,
        destination_refs: vec!["develop".into(), "develop".into()],
        source_refs: vec!["feature/new".into()],
        ..existing.clone()
    };
    apply_squash_enrichment(&mut snapshot, std::slice::from_ref(&fresh));
    let merged = &snapshot.commits[0].relationships[0];
    assert_eq!(merged.matching, RelationshipMatch::Exact);
    assert_eq!(merged.source_refs, ["feature/new", "feature/old"]);
    assert_eq!(merged.destination_refs, ["develop", "main"]);
    let mut normalized = fresh;
    normalized.destination_refs.dedup();
    assert_eq!(snapshot.commits[1].relationships, [normalized]);
    apply_squash_enrichment(&mut snapshot, &[existing]);
    assert_eq!(
        snapshot.commits[0].relationships[0].matching,
        RelationshipMatch::Exact
    );
    assert_eq!(
        snapshot.commits[0].relationships,
        snapshot.commits[1].relationships
    );
    let before = snapshot.clone();
    apply_squash_enrichment(&mut snapshot, &[]);
    assert_eq!(snapshot, before);
    assert!(!snapshot.commits[1].is_possible_squash_merge());
}

#[test]
fn enrichment_distinguishes_kinds_missing_endpoints_self_pairs_and_idempotence() {
    let mut snapshot = relationship_snapshot(vec![
        GraphCommit {
            oid: "d".into(),
            ..Default::default()
        },
        GraphCommit {
            oid: "d".into(),
            ..Default::default()
        },
    ]);
    let squash = squash("d", "missing", &["feature/gone"], RelationshipMatch::Exact);
    let cherry = GraphRelationship {
        kind: RelationshipKind::CherryPick,
        ..squash.clone()
    };
    let self_pair = GraphRelationship {
        source_oid: "d".into(),
        ..cherry.clone()
    };
    let updates = [squash, cherry, self_pair];
    apply_squash_enrichment(&mut snapshot, &updates);
    for commit in &snapshot.commits {
        assert_eq!(commit.relationships.len(), 3);
        assert_eq!(commit.cherry_pick_sources().len(), 2);
        assert_eq!(commit.cherry_pick_destinations().len(), 1);
    }
    let before = snapshot.clone();
    apply_squash_enrichment(&mut snapshot, &updates);
    assert_eq!(snapshot, before);
}

#[test]
fn graph_state_reports_relationship_changes_without_erasure() {
    let relationship = squash("d", "s", &["feature/source"], RelationshipMatch::Exact);
    let mut state = crate::view::graph::GraphState::new();
    assert!(!state.apply_squash_enrichment(std::slice::from_ref(&relationship)));
    state.apply_result(Ok(relationship_snapshot(vec![GraphCommit {
        oid: "d".into(),
        ..Default::default()
    }])));
    assert!(state.apply_squash_enrichment(std::slice::from_ref(&relationship)));
    assert!(!state.apply_squash_enrichment(std::slice::from_ref(&relationship)));
    assert!(!state.apply_squash_enrichment(&[]));
    assert_eq!(
        state.snapshot().unwrap().commits[0].relationships,
        [relationship]
    );
}
