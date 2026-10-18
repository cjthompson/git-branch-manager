
use git_branch_manager::git::graph;

use crate::common::*;

#[test]
fn exact_destination_keeps_distinct_fuzzy_source() {
    let (tmpdir, destination, exact, high, low) = paired_squash_fixture();
    let snapshot = graph::load_graph_with_squash_annotations(
        tmpdir.path(),
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..tmpdir.graph_options()
        },
    )
    .unwrap();
    let landing = snapshot
        .commits
        .iter()
        .find(|c| c.oid == destination)
        .unwrap();
    assert_eq!(
        landing
            .relationships
            .iter()
            .filter(|r| r.kind == graph::RelationshipKind::SquashMerge)
            .count(),
        3,
        "every distinct paired source must survive"
    );
    for (source, names, matching) in [
        (
            &exact,
            vec!["feature/exact", "feature/exact-alias"],
            graph::RelationshipMatch::Exact,
        ),
        (
            &high,
            vec!["feature/high"],
            graph::RelationshipMatch::Fuzzy {
                similarity_percent: 90,
            },
        ),
        (
            &low,
            vec!["feature/low"],
            graph::RelationshipMatch::Fuzzy {
                similarity_percent: 81,
            },
        ),
    ] {
        let pairs: Vec<_> = landing
            .relationships
            .iter()
            .filter(|r| r.kind == graph::RelationshipKind::SquashMerge && r.source_oid == *source)
            .collect();
        assert_eq!(pairs.len(), 1, "one record per exact or fuzzy pair");
        let relationship = pairs[0];
        assert_eq!(relationship.kind, graph::RelationshipKind::SquashMerge);
        assert_eq!(relationship.destination_oid, destination);
        assert_eq!(relationship.destination_refs, ["main"]);
        assert_eq!(relationship.source_refs, names);
        assert_eq!(relationship.matching, matching);
        assert_eq!(relationship.source_oid.len(), 40);
    }
    assert_eq!(
        landing.squash_match_confidence(),
        Some(graph::SquashMatchConfidence {
            sources: vec!["feature/exact".into(), "feature/exact-alias".into()],
            similarity_percent: 100,
        })
    );
    assert_eq!(landing.fuzzy_squash_match(), None);
}

#[test]
fn fuzzy_destination_keeps_all_sources_and_selects_strongest_for_display() {
    let (tmpdir, destination, _exact, high, low) = paired_squash_fixture();
    run_git(
        tmpdir.path(),
        &["branch", "-D", "feature/exact", "feature/exact-alias"],
    );
    let snapshot = graph::load_graph_with_squash_annotations(
        tmpdir.path(),
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..tmpdir.graph_options()
        },
    )
    .unwrap();
    let landing = snapshot
        .commits
        .iter()
        .find(|c| c.oid == destination)
        .unwrap();
    assert_eq!(
        landing
            .relationships
            .iter()
            .filter(|r| r.kind == graph::RelationshipKind::SquashMerge)
            .count(),
        2
    );
    for (source, percent, name) in [(&high, 90, "feature/high"), (&low, 81, "feature/low")] {
        let relation = landing
            .relationships
            .iter()
            .find(|r| r.source_oid == *source)
            .unwrap();
        assert_eq!(relation.destination_oid, destination);
        assert_eq!(relation.source_refs, [name]);
        assert_eq!(
            relation.matching,
            graph::RelationshipMatch::Fuzzy {
                similarity_percent: percent
            }
        );
    }
    assert_eq!(
        landing.squash_match_confidence(),
        Some(graph::SquashMatchConfidence {
            sources: vec!["feature/high".into()],
            similarity_percent: 90,
        })
    );
    assert!(!landing.is_possible_squash_merge());
}

#[test]
fn test_graph_marks_only_base_commit_with_exact_squash_patch() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/exact"]);
    std::fs::write(dir.join("exact.txt"), "exact patch\n").unwrap();
    run_git(dir, &["add", "exact.txt"]);
    run_git(dir, &["commit", "-m", "source patch subject"]);
    std::fs::write(dir.join("exact-second.txt"), "second exact patch\n").unwrap();
    run_git(dir, &["add", "exact-second.txt"]);
    run_git(dir, &["commit", "-m", "source follow-up subject"]);
    let source_oid = git_output(dir, &["rev-parse", "HEAD"]);

    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/exact"]);
    run_git(dir, &["commit", "-m", "unrelated landing subject"]);
    let exact_base_oid = git_output(dir, &["rev-parse", "HEAD"]);

    run_git(dir, &["checkout", "-b", "feature/message-only"]);
    std::fs::write(dir.join("branch-only.txt"), "branch content\n").unwrap();
    run_git(dir, &["add", "branch-only.txt"]);
    run_git(dir, &["commit", "-m", "shared misleading subject"]);
    run_git(dir, &["checkout", "main"]);
    std::fs::write(dir.join("base-only.txt"), "different base content\n").unwrap();
    run_git(dir, &["add", "base-only.txt"]);
    run_git(dir, &["commit", "-m", "shared misleading subject"]);
    let message_only_base_oid = git_output(dir, &["rev-parse", "HEAD"]);

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..tmpdir.graph_options()
        },
    )
    .expect("graph loader should annotate exact squash patch matches");

    let exact_base = snapshot
        .commits
        .iter()
        .find(|commit| commit.oid == exact_base_oid)
        .expect("exact base commit should be displayed");
    assert!(exact_base.is_possible_squash_merge());
    assert_eq!(
        exact_base
            .relationships
            .iter()
            .filter(|r| r.kind == graph::RelationshipKind::SquashMerge)
            .count(),
        1
    );
    let relationship = &exact_base.relationships[0];
    assert_eq!(relationship.destination_oid, exact_base_oid);
    assert_eq!(relationship.destination_refs, ["main"]);
    assert_eq!(relationship.source_oid, source_oid);
    assert_eq!(relationship.source_refs, ["feature/exact"]);
    assert_eq!(relationship.matching, graph::RelationshipMatch::Exact);

    let source = snapshot
        .commits
        .iter()
        .find(|commit| commit.oid == source_oid)
        .expect("source branch commit should be displayed");
    assert!(
        !source.is_possible_squash_merge(),
        "only the matching base-branch commit receives the annotation"
    );

    let message_only_base = snapshot
        .commits
        .iter()
        .find(|commit| commit.oid == message_only_base_oid)
        .expect("same-subject base commit should be displayed");
    assert!(
        !message_only_base.is_possible_squash_merge(),
        "matching subjects with different patches must not annotate a commit"
    );
}

#[test]
fn test_graph_preserves_every_base_oid_for_duplicate_patch_ids() {
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();
    let root_oid = repo.head().unwrap().target().unwrap().to_string();

    std::fs::write(dir.join("duplicate.txt"), "same patch\n").unwrap();
    run_git(dir, &["add", "duplicate.txt"]);
    run_git(dir, &["commit", "-m", "first duplicate patch"]);
    let first_oid = git_output(dir, &["rev-parse", "HEAD"]);
    run_git(dir, &["rm", "duplicate.txt"]);
    run_git(dir, &["commit", "-m", "remove duplicate patch"]);
    std::fs::write(dir.join("duplicate.txt"), "same patch\n").unwrap();
    run_git(dir, &["add", "duplicate.txt"]);
    run_git(dir, &["commit", "-m", "second duplicate patch"]);
    let second_oid = git_output(dir, &["rev-parse", "HEAD"]);

    run_git(dir, &["checkout", "-b", "feature/duplicate", &root_oid]);
    std::fs::write(dir.join("duplicate.txt"), "same patch\n").unwrap();
    run_git(dir, &["add", "duplicate.txt"]);
    run_git(dir, &["commit", "-m", "duplicate source patch"]);
    let source_oid = git_output(dir, &["rev-parse", "HEAD"]);
    run_git(dir, &["checkout", "main"]);

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..tmpdir.graph_options()
        },
    )
    .expect("graph loader should retain duplicate patch matches");

    for oid in [&first_oid, &second_oid] {
        let commit = snapshot
            .commits
            .iter()
            .find(|commit| commit.oid == *oid)
            .expect("duplicate-patch base commit should be displayed");
        assert!(
            commit.is_possible_squash_merge(),
            "every base OID sharing the exact patch ID must be annotated"
        );
        assert_eq!(
            commit
                .relationships
                .iter()
                .filter(|r| r.kind == graph::RelationshipKind::SquashMerge)
                .count(),
            1
        );
        assert_eq!(commit.relationships[0].destination_oid, *oid);
        assert_eq!(commit.relationships[0].source_oid, source_oid);
        assert_eq!(commit.relationships[0].destination_refs, ["main"]);
    }
    assert!(!snapshot
        .commits
        .iter()
        .find(|commit| commit.oid == source_oid)
        .expect("source branch commit should be displayed")
        .is_possible_squash_merge());
}

#[test]
fn test_graph_excludes_regular_merges_roots_and_empty_patches() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/regular"]);
    std::fs::write(dir.join("regular.txt"), "regular patch\n").unwrap();
    run_git(dir, &["add", "regular.txt"]);
    run_git(dir, &["commit", "-m", "regular source patch"]);
    let regular_tip = git_output(dir, &["rev-parse", "HEAD"]);
    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["cherry-pick", &regular_tip]);
    let equivalent_base_oid = git_output(dir, &["rev-parse", "HEAD"]);
    run_git(
        dir,
        &[
            "merge",
            "--no-ff",
            "feature/regular",
            "-m",
            "regular merge after equivalent patch",
        ],
    );

    run_git(dir, &["checkout", "-b", "feature/empty"]);
    run_git(
        dir,
        &["commit", "--allow-empty", "-m", "empty branch patch"],
    );
    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["commit", "--allow-empty", "-m", "empty base patch"]);

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..tmpdir.graph_options()
        },
    )
    .expect("graph loader should ignore ineligible patch shapes");

    assert!(
        !snapshot
            .commits
            .iter()
            .find(|commit| commit.oid == equivalent_base_oid)
            .expect("equivalent regular-merge base commit should be displayed")
            .is_possible_squash_merge(),
        "a regularly merged branch must not create a possible-squash indicator"
    );
    assert!(
        snapshot
            .commits
            .iter()
            .all(|commit| !commit.is_possible_squash_merge()),
        "merge commits, root commits, and empty patches cannot match"
    );
}
