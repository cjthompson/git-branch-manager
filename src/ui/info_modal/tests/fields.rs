use super::*;
use crate::ui::info_modal::fields::graph_commit_fields;
use crate::ui::info_modal::ref_fields::{branch_fields, remote_fields};
use chrono::{Duration, TimeZone};

#[test]
fn graph_commit_fields_includes_author_and_date_rows() {
    let commit = GraphCommit {
        oid: "abcdef1234567".into(),
        summary: "test".into(),
        parents: vec![],
        lane: None,
        branch: None,
        refs: vec![],
        relationships: vec![],

        author_name: "Jane Doe".into(),
        author_email: "jane@example.com".into(),
        authored_at: Some(Utc::now() - Duration::hours(3)),
    };
    let fields = graph_commit_fields(&commit);
    let labels: Vec<&str> = fields.iter().map(|f| f.label).collect();
    assert!(
        labels.contains(&"Author"),
        "expected Author field, got {labels:?}"
    );
    assert!(
        labels.contains(&"Date"),
        "expected Date field, got {labels:?}"
    );
    let author = fields.iter().find(|f| f.label == "Author").unwrap();
    assert_eq!(author.value, "Jane Doe <jane@example.com>");
    let date = fields.iter().find(|f| f.label == "Date").unwrap();
    assert!(
        date.value.contains("ago"),
        "expected relative age, got: {}",
        date.value
    );
}

#[test]
fn graph_commit_fields_lists_all_possible_squash_sources() {
    let commit = GraphCommit {
        oid: "abcdef1234567".into(),
        summary: "squash landing".into(),
        parents: vec![],
        lane: None,
        branch: None,
        refs: vec![],
        relationships: vec![crate::git::graph::GraphRelationship {
            kind: crate::git::graph::RelationshipKind::SquashMerge,
            matching: crate::git::graph::RelationshipMatch::Exact,
            destination_oid: "abcdef1234567".into(),
            destination_refs: vec!["main".into()],
            source_oid: "1111111111111111111111111111111111111111".into(),
            source_refs: vec!["feature/auth".into(), "feature/login".into()],
        }],

        author_name: String::new(),
        author_email: String::new(),
        authored_at: None,
    };

    let fields = graph_commit_fields(&commit);
    let sources = fields
        .iter()
        .find(|field| field.label == "Possible Squash Merge From")
        .expect("source field should be present for an exact squash match");
    assert_eq!(sources.value, "feature/auth, feature/login");
}

#[test]
fn graph_commit_fields_shows_fuzzy_similarity_and_source_branch() {
    let commit = GraphCommit {
        relationships: vec![crate::git::graph::GraphRelationship {
            kind: crate::git::graph::RelationshipKind::SquashMerge,
            matching: crate::git::graph::RelationshipMatch::Fuzzy {
                similarity_percent: 97,
            },
            destination_oid: "abcdef1234567".into(),
            destination_refs: vec!["main".into()],
            source_oid: "3333333333333333333333333333333333333333".into(),
            source_refs: vec!["feature/login".into()],
        }],
        oid: "abcdef1234567".into(),
        summary: "near squash landing".into(),
        ..GraphCommit::default()
    };

    let fields = graph_commit_fields(&commit);
    let fuzzy = fields
        .iter()
        .find(|field| field.label == "Possible Squash Merge (fuzzy)")
        .expect("fuzzy similarity should be visible in commit info");
    assert_eq!(fuzzy.value, "97% similarity");
    assert_eq!(
        fields
            .iter()
            .find(|field| field.label == "Possible Squash Merge From")
            .unwrap()
            .value,
        "feature/login"
    );
}

#[test]
fn graph_commit_fields_keeps_exact_squash_and_cherry_classifications_ahead_of_fuzzy() {
    for (is_possible_squash_merge, is_cherry_picked_commit) in
        [(true, false), (false, true), (true, true)]
    {
        let commit = GraphCommit {
            relationships: {
                let mut pairs = vec![
                    crate::git::graph::GraphRelationship {
                        kind: crate::git::graph::RelationshipKind::SquashMerge,
                        matching: crate::git::graph::RelationshipMatch::Exact,
                        destination_oid: String::new(),
                        destination_refs: vec!["main".into()],
                        source_oid: "1111111111111111111111111111111111111111".into(),
                        source_refs: Vec::new(),
                    },
                    crate::git::graph::GraphRelationship {
                        kind: crate::git::graph::RelationshipKind::SquashMerge,
                        matching: crate::git::graph::RelationshipMatch::Fuzzy {
                            similarity_percent: 97,
                        },
                        destination_oid: String::new(),
                        destination_refs: vec!["main".into()],
                        source_oid: "3333333333333333333333333333333333333333".into(),
                        source_refs: Vec::new(),
                    },
                ];
                if !is_possible_squash_merge {
                    pairs.retain(|r| r.matching != crate::git::graph::RelationshipMatch::Exact);
                }
                if is_cherry_picked_commit {
                    pairs.push(crate::git::graph::GraphRelationship {
                        kind: crate::git::graph::RelationshipKind::CherryPick,
                        matching: crate::git::graph::RelationshipMatch::Exact,
                        source_oid: String::new(),
                        source_refs: vec!["feature/picked".into()],
                        destination_oid: "4444444444444444444444444444444444444444".into(),
                        destination_refs: vec!["main".into()],
                    });
                }
                pairs
            },
            ..GraphCommit::default()
        };
        let fields = graph_commit_fields(&commit);
        assert!(!fields
            .iter()
            .any(|field| field.label == "Possible Squash Merge (fuzzy)"));
        if is_possible_squash_merge {
            assert!(fields
                .iter()
                .any(|field| field.label == "Possible Squash Merge"));
        }
    }
}

#[test]
fn graph_commit_fields_omits_author_row_when_both_name_and_email_empty() {
    let commit = GraphCommit {
        oid: "x".into(),
        summary: "s".into(),
        parents: vec![],
        lane: None,
        branch: None,
        refs: vec![],
        relationships: vec![],

        author_name: "".into(),
        author_email: "".into(),
        authored_at: Some(Utc::now()),
    };
    let labels: Vec<&str> = graph_commit_fields(&commit)
        .iter()
        .map(|f| f.label)
        .collect();
    assert!(!labels.contains(&"Author"));
}

#[test]
fn graph_commit_fields_shows_date_row_for_epoch_authored_at() {
    let commit = GraphCommit {
        oid: "x".into(),
        summary: "s".into(),
        parents: vec![],
        lane: None,
        branch: None,
        refs: vec![],
        relationships: vec![],

        author_name: "X".into(),
        author_email: "x@y".into(),
        authored_at: Some(Utc.timestamp_opt(0, 0).unwrap()),
    };
    let labels: Vec<&str> = graph_commit_fields(&commit)
        .iter()
        .map(|f| f.label)
        .collect();
    assert!(labels.contains(&"Date"));
}

#[test]
fn graph_commit_fields_omits_date_row_when_authored_at_is_unknown() {
    let commit = GraphCommit {
        oid: "x".into(),
        summary: "s".into(),
        parents: vec![],
        lane: None,
        branch: None,
        refs: vec![],
        relationships: vec![],

        author_name: "X".into(),
        author_email: "x@y".into(),
        authored_at: None,
    };
    let labels: Vec<&str> = graph_commit_fields(&commit)
        .iter()
        .map(|f| f.label)
        .collect();
    assert!(!labels.contains(&"Date"));
}

#[test]
fn branch_fields_includes_confidence_when_likely_squash_merged() {
    let b = BranchInfo {
        name: "feature/x".into(),
        is_current: false,
        is_base: false,
        tracking: TrackingStatus::Local,
        ahead: None,
        behind: None,
        last_commit_date: Utc::now(),
        merge_status: MergeStatus::LikelySquashMerged,
        base_branch: "main".into(),
        merge_base_commit: None,
        pr: None,
        squash_confidence: Some(SquashConfidence::FuzzyMatch {
            similarity_percent: 82,
        }),
    };
    let fields = branch_fields(&b);
    let confidence_field = fields
        .iter()
        .find(|f| f.label == "Confidence")
        .expect("Confidence field should be present");
    assert_eq!(confidence_field.value, "Fuzzy match (82%)");
}

#[test]
fn branch_fields_omits_confidence_when_none() {
    let b = BranchInfo {
        name: "feature/y".into(),
        is_current: false,
        is_base: false,
        tracking: TrackingStatus::Local,
        ahead: None,
        behind: None,
        last_commit_date: Utc::now(),
        merge_status: MergeStatus::Unmerged,
        base_branch: "main".into(),
        merge_base_commit: None,
        pr: None,
        squash_confidence: None,
    };
    let fields = branch_fields(&b);
    assert!(!fields.iter().any(|f| f.label == "Confidence"));
}

#[test]
fn remote_fields_includes_confidence_when_present() {
    let r = RemoteBranchInfo {
        full_ref: "origin/feature/x".into(),
        remote: "origin".into(),
        short_name: "feature/x".into(),
        has_local: false,
        is_base: false,
        last_commit_date: Utc::now(),
        merge_status: MergeStatus::LikelySquashMerged,
        ahead: None,
        behind: None,
        disjoint: false,
        pr: None,
        squash_confidence: Some(SquashConfidence::MergeTreeConfirmed),
    };
    let fields = remote_fields(&r);
    let confidence_field = fields
        .iter()
        .find(|f| f.label == "Confidence")
        .expect("Confidence field should be present");
    assert_eq!(confidence_field.value, "Merge-tree confirmed");
}
#[test]
fn cherry_pick_provenance_uses_captured_missing_source() {
    let commit = GraphCommit {
        oid: "ddddddd0000000000000000000000000000000000".into(),
        relationships: vec![crate::git::graph::GraphRelationship {
            kind: crate::git::graph::RelationshipKind::CherryPick,
            matching: crate::git::graph::RelationshipMatch::Exact,
            destination_oid: "ddddddd0000000000000000000000000000000000".into(),
            destination_refs: vec!["main".into()],
            source_oid: "abcdef1234567890000000000000000000000000".into(),
            source_refs: vec!["feature/gone".into()],
        }],
        ..Default::default()
    };
    let fields = graph_commit_fields(&commit);
    assert!(fields
        .iter()
        .any(|f| f.label == "Cherry-picked From" && f.value == "abcdef1 (feature/gone)"));
    assert!(!commit.is_cherry_picked_commit());
}
