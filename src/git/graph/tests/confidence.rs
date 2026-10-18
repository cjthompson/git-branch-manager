use super::*;

#[test]
fn squash_confidence_prefers_exact_and_unions_its_sources() {
    let commit = GraphCommit {
        oid: "d".into(),
        relationships: vec![
            squash("d", "t1", &["feature/b"], RelationshipMatch::Exact),
            squash("d", "t2", &["feature/a"], RelationshipMatch::Exact),
            squash(
                "d",
                "t3",
                &["feature/c"],
                RelationshipMatch::Fuzzy {
                    similarity_percent: 97,
                },
            ),
        ],
        ..GraphCommit::default()
    };
    let confidence = commit.squash_match_confidence().unwrap();
    assert_eq!(confidence.similarity_percent, 100);
    assert_eq!(confidence.sources, ["feature/a", "feature/b"]);
    assert!(commit.is_possible_squash_merge());
    assert_eq!(commit.fuzzy_squash_match(), None);
}

#[test]
fn squash_confidence_reports_best_fuzzy_source() {
    let commit = GraphCommit {
        oid: "d".into(),
        relationships: vec![
            squash(
                "d",
                "t1",
                &["feature/low"],
                RelationshipMatch::Fuzzy {
                    similarity_percent: 80,
                },
            ),
            squash(
                "d",
                "t2",
                &["feature/high"],
                RelationshipMatch::Fuzzy {
                    similarity_percent: 97,
                },
            ),
        ],
        ..GraphCommit::default()
    };
    assert_eq!(
        commit.squash_match_confidence(),
        Some(SquashMatchConfidence {
            sources: vec!["feature/high".into()],
            similarity_percent: 97
        })
    );
    assert!(!commit.is_possible_squash_merge());
    assert_eq!(
        commit.fuzzy_squash_match(),
        Some(FuzzySquashMatch {
            similarity_percent: 97
        })
    );
}

#[test]
fn squash_confidence_ignores_relationships_where_commit_is_the_source() {
    let commit = GraphCommit {
        oid: "t1".into(),
        relationships: vec![squash("d", "t1", &["feature/x"], RelationshipMatch::Exact)],
        ..GraphCommit::default()
    };
    assert_eq!(commit.squash_match_confidence(), None);
}

#[test]
fn stronger_match_prefers_exact_then_higher_percent() {
    use RelationshipMatch::*;
    assert_eq!(
        Exact.stronger(Fuzzy {
            similarity_percent: 99
        }),
        Exact
    );
    assert_eq!(
        Fuzzy {
            similarity_percent: 80
        }
        .stronger(Fuzzy {
            similarity_percent: 90
        }),
        Fuzzy {
            similarity_percent: 90
        }
    );
}
