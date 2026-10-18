
use git_branch_manager::git::graph;

use crate::common::*;
use crate::cherry::{advance_main, rev};

#[test]
fn cherry_pick_sources_match_git_cherry() {
    for fixture in ["text", "rename", "rename-content", "mode"] {
        let (tmpdir, _repo) = setup_test_repo();
        let dir = tmpdir.path();
        std::fs::write(dir.join("rename-me.txt"), "contents\n".repeat(20)).unwrap();
        std::fs::write(dir.join("script.sh"), "echo hi\n").unwrap();
        run_git(dir, &["add", "."]);
        run_git(dir, &["commit", "-q", "-m", "seed"]);
        run_git(dir, &["checkout", "-q", "-b", "feature/parity"]);
        match fixture {
            "text" => {
                std::fs::write(dir.join("text.txt"), "picked text\n").unwrap();
                run_git(dir, &["add", "."]);
            }
            "rename" | "rename-content" => {
                run_git(dir, &["mv", "rename-me.txt", "renamed.txt"]);
                if fixture == "rename-content" {
                    std::fs::write(
                        dir.join("renamed.txt"),
                        format!("{}changed\n", "contents\n".repeat(20)),
                    )
                    .unwrap();
                    run_git(dir, &["add", "."]);
                }
            }
            "mode" => run_git(dir, &["update-index", "--chmod=+x", "script.sh"]),
            _ => unreachable!(),
        }
        run_git(dir, &["commit", "-q", "-m", fixture]);
        let source = rev(dir, "HEAD");
        std::fs::write(dir.join("unpicked.txt"), "not landed\n").unwrap();
        run_git(dir, &["add", "."]);
        run_git(dir, &["commit", "-q", "-m", "unpicked"]);
        let unpicked = rev(dir, "HEAD");
        run_git(dir, &["checkout", "-q", "main"]);
        advance_main(dir, fixture);
        run_git(dir, &["cherry-pick", &source]);
        assert_ne!(source, rev(dir, "HEAD"));
        let expected = git_output(dir, &["cherry", "main", "feature/parity"])
            .lines()
            .filter_map(|line| line.strip_prefix("- "))
            .map(str::to_owned)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            expected,
            [source.clone()].into_iter().collect(),
            "meaningful {fixture} oracle"
        );
        assert!(!expected.contains(&unpicked));
        let snapshot =
            graph::load_graph_with_squash_annotations(dir, tmpdir.graph_options_for("main"))
                .unwrap();
        assert!(snapshot.commits.iter().any(|c| c.oid == source));
        let actual = snapshot
            .commits
            .iter()
            .filter(|c| c.is_cherry_picked_commit())
            .map(|c| c.oid.clone())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(actual, expected, "source-set parity: {fixture}");
    }
}

#[test]
fn cherry_pick_repicks_keep_all_destinations() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();
    run_git(dir, &["checkout", "-q", "-b", "feature/pick"]);
    std::fs::write(dir.join("picked.txt"), "picked\n").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-q", "-m", "source"]);
    let source = rev(dir, "HEAD");
    run_git(dir, &["branch", "feature/alias"]);
    run_git(dir, &["checkout", "-q", "main"]);
    advance_main(dir, "repick");
    run_git(dir, &["cherry-pick", &source]);
    let first = rev(dir, "HEAD");
    run_git(dir, &["revert", "--no-edit", &first]);
    run_git(dir, &["cherry-pick", &source]);
    let second = rev(dir, "HEAD");
    let snapshot =
        graph::load_graph_with_squash_annotations(dir, tmpdir.graph_options_for("main")).unwrap();
    let commit = snapshot.commits.iter().find(|c| c.oid == source).unwrap();
    assert!(commit.is_cherry_picked_commit());
    let pairs = commit.cherry_pick_destinations();
    assert_eq!(pairs.len(), 2);
    assert_eq!(
        pairs
            .iter()
            .map(|r| r.destination_oid.clone())
            .collect::<std::collections::BTreeSet<_>>(),
        [first.clone(), second.clone()].into_iter().collect()
    );
    for dest in [&first, &second] {
        let landing = snapshot.commits.iter().find(|c| c.oid == *dest).unwrap();
        let incoming = landing.cherry_pick_sources();
        assert_eq!(incoming.len(), 1);
        assert_eq!(incoming[0].source_oid, source);
        assert_eq!(incoming[0].source_refs, ["feature/alias", "feature/pick"]);
        assert_eq!(incoming[0].destination_refs, ["main"]);
        assert!(landing.is_possible_squash_merge());
        assert!(!landing.is_cherry_picked_commit());
    }
}

#[test]
#[ignore = "nonblocking binary diagnostic"]
fn cherry_pick_binary_identity_and_unrelated_binary_diagnostic() {
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();
    run_git(dir, &["checkout", "-q", "-b", "feature/binary"]);
    std::fs::write(dir.join("picked.bin"), b"\0picked binary\xff").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-q", "-m", "binary source"]);
    let source = rev(dir, "HEAD");
    run_git(dir, &["checkout", "-q", "main"]);
    advance_main(dir, "binary");
    std::fs::write(dir.join("unrelated.bin"), b"\0unrelated binary\xfe").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-q", "-m", "unrelated binary"]);
    let unrelated = rev(dir, "HEAD");
    run_git(dir, &["cherry-pick", &source]);
    let destination = rev(dir, "HEAD");
    let patch = |oid: &str| {
        let id = git2::Oid::from_str(oid).unwrap();
        let parent = repo.find_commit(id).unwrap().parent_id(0).unwrap();
        graph::git2_diff_patch(&repo, parent, id).0
    };
    let source_id = patch(&source);
    let destination_id = patch(&destination);
    let unrelated_id = patch(&unrelated);
    let expected = git_output(dir, &["cherry", "main", "feature/binary"])
        .lines()
        .filter_map(|line| line.strip_prefix("- "))
        .map(str::to_owned)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(expected, [source.clone()].into_iter().collect());
    let snapshot =
        graph::load_graph_with_squash_annotations(dir, tmpdir.graph_options_for("main")).unwrap();
    let actual = snapshot
        .commits
        .iter()
        .filter(|c| c.is_cherry_picked_commit())
        .map(|c| c.oid.clone())
        .collect::<std::collections::BTreeSet<_>>();
    eprintln!("binary diagnostic: source={source_id:?}, destination={destination_id:?}, unrelated={unrelated_id:?}, expected={expected:?}, actual={actual:?}");
    assert_eq!(source_id, destination_id, "picked binary patch identity");
    assert_ne!(source_id, unrelated_id, "unrelated binary must not collide");
    assert_eq!(actual, expected, "binary source-set diagnostic");
}

#[test]
fn same_commit_records_squash_and_cherry_pick() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();
    run_git(dir, &["checkout", "-q", "-b", "feature/single"]);
    std::fs::write(dir.join("single.txt"), "single\n").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-q", "-m", "single source"]);
    let source = rev(dir, "HEAD");
    run_git(dir, &["checkout", "-q", "main"]);
    advance_main(dir, "single");
    run_git(dir, &["merge", "-q", "--squash", "feature/single"]);
    run_git(dir, &["commit", "-q", "-m", "single squash"]);
    let destination = rev(dir, "HEAD");
    assert_ne!(source, destination);
    let snapshot =
        graph::load_graph_with_squash_annotations(dir, tmpdir.graph_options_for("main")).unwrap();
    let landing = snapshot
        .commits
        .iter()
        .find(|c| c.oid == destination)
        .unwrap();
    let original = snapshot.commits.iter().find(|c| c.oid == source).unwrap();
    assert!(landing.is_possible_squash_merge());
    assert_eq!(
        landing.relationships.len(),
        2,
        "one fact of each kind for the same OIDs"
    );
    let incoming = landing.cherry_pick_sources();
    assert_eq!(incoming.len(), 1);
    assert_eq!(incoming[0].source_oid, source);
    assert_eq!(incoming[0].source_refs, ["feature/single"]);
    assert_eq!(incoming[0].destination_refs, ["main"]);
    assert_eq!(incoming[0].matching, graph::RelationshipMatch::Exact);
    assert!(original.is_cherry_picked_commit());
    assert_eq!(
        original.cherry_pick_destinations()[0].destination_oid,
        destination
    );
    assert!(!original.is_possible_squash_merge());
}
