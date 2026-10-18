
use git_branch_manager::git::{
    graph, merge_detection,
};

use crate::common::*;

#[test]
fn test_squash_scenario_20a_squash_plus_trivial_follow_up_folded_in() {
    // Plan scenario 20a: branch's real changes land in the squash commit,
    // plus one small unrelated line (e.g. a version bump) folded in during
    // the squash but never part of the branch's own commits.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    std::fs::write(
        dir.join("f20a.txt"),
        "alpha\nbeta\ngamma\ndelta\nepsilon\nzeta\nversion: 1\n",
    )
    .unwrap();
    run_git(dir, &["add", "f20a.txt"]);
    run_git(dir, &["commit", "-m", "f20a baseline"]);

    run_git(dir, &["checkout", "-b", "feature/20a"]);
    std::fs::write(
        dir.join("f20a.txt"),
        "alpha-x\nbeta-x\ngamma-x\ndelta-x\nepsilon-x\nzeta-x\nversion: 1\n",
    )
    .unwrap();
    run_git(dir, &["commit", "-am", "branch changes 6 lines"]);

    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/20a"]);
    // Fold in an unrelated trivial version bump the branch never made.
    std::fs::write(
        dir.join("f20a.txt"),
        "alpha-x\nbeta-x\ngamma-x\ndelta-x\nepsilon-x\nzeta-x\nversion: 2\n",
    )
    .unwrap();
    run_git(dir, &["add", "f20a.txt"]);
    run_git(dir, &["commit", "-m", "squash landing plus version bump"]);
    let squash_oid = git_output(dir, &["rev-parse", "HEAD"]);

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..tmpdir.graph_options()
        },
    )
    .expect("graph load should succeed");
    let landing = snapshot
        .commits
        .iter()
        .find(|c| c.oid == squash_oid)
        .expect("squash landing commit should be displayed");
    assert!(
        !landing.is_possible_squash_merge(),
        "the extra version-bump line must break exact patch-id equality"
    );
    assert!(
        !merge_detection::is_squash_merged(dir, "main", "feature/20a", None, None),
        "Algorithm B must also miss this under exact match"
    );
    let relationship = landing
        .relationships
        .iter()
        .find(|relationship| {
            relationship.source_oid == git_output(dir, &["rev-parse", "feature/20a"])
        })
        .expect("fuzzy source pairing should be retained");
    assert_eq!(relationship.destination_oid, squash_oid);
    assert_eq!(relationship.destination_refs, ["main"]);
    assert_eq!(relationship.source_refs, ["feature/20a"]);
    assert!(matches!(
        relationship.matching,
        graph::RelationshipMatch::Fuzzy {
            similarity_percent: 0..=99
        }
    ));
    let fuzzy = landing
        .fuzzy_squash_match()
        .expect("Option 6 should flag this as a fuzzy possible-squash match");
    assert!(
        fuzzy.similarity_percent >= 75,
        "expected high similarity for a near-exact match with one trivial extra line, got {}",
        fuzzy.similarity_percent
    );
}

#[test]
fn test_squash_scenario_20b_squash_omits_a_trivial_branch_change() {
    // Plan scenario 20b: the squash commit landed slightly *less* than the
    // branch's full diff (e.g. a debug line dropped during squash). Confirms
    // the similarity computation is symmetric (handles both superset and
    // subset cases), not just the 20a "extra line" direction.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    std::fs::write(
        dir.join("f20b.txt"),
        "alpha\nbeta\ngamma\ndelta\nepsilon\nzeta\ndebug: off\n",
    )
    .unwrap();
    run_git(dir, &["add", "f20b.txt"]);
    run_git(dir, &["commit", "-m", "f20b baseline"]);

    run_git(dir, &["checkout", "-b", "feature/20b"]);
    std::fs::write(
        dir.join("f20b.txt"),
        "alpha-x\nbeta-x\ngamma-x\ndelta-x\nepsilon-x\nzeta-x\ndebug: on\n",
    )
    .unwrap();
    run_git(
        dir,
        &["commit", "-am", "branch changes 6 lines plus enables debug"],
    );

    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/20b"]);
    // Drop the branch's debug-line change during squash (revert it back).
    std::fs::write(
        dir.join("f20b.txt"),
        "alpha-x\nbeta-x\ngamma-x\ndelta-x\nepsilon-x\nzeta-x\ndebug: off\n",
    )
    .unwrap();
    run_git(dir, &["add", "f20b.txt"]);
    run_git(dir, &["commit", "-m", "squash landing, debug line omitted"]);
    let squash_oid = git_output(dir, &["rev-parse", "HEAD"]);

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..tmpdir.graph_options()
        },
    )
    .expect("graph load should succeed");
    let landing = snapshot
        .commits
        .iter()
        .find(|c| c.oid == squash_oid)
        .expect("squash landing commit should be displayed");
    assert!(
        !landing.is_possible_squash_merge(),
        "omitting a branch change also breaks exact patch-id equality"
    );
    assert!(!merge_detection::is_squash_merged(
        dir,
        "main",
        "feature/20b",
        None,
        None
    ));
    let relationship = landing
        .relationships
        .iter()
        .find(|relationship| {
            relationship.source_oid == git_output(dir, &["rev-parse", "feature/20b"])
        })
        .expect("fuzzy source pairing should be retained");
    assert_eq!(relationship.destination_oid, squash_oid);
    assert_eq!(relationship.destination_refs, ["main"]);
    assert_eq!(relationship.source_refs, ["feature/20b"]);
    assert!(matches!(
        relationship.matching,
        graph::RelationshipMatch::Fuzzy {
            similarity_percent: 0..=99
        }
    ));
    let fuzzy = landing
        .fuzzy_squash_match()
        .expect("Option 6 should flag this subset case as a fuzzy possible-squash match too");
    assert!(
        fuzzy.similarity_percent >= 75,
        "expected high similarity for the omitted-trivial-change (subset) case, got {}",
        fuzzy.similarity_percent
    );
}

#[test]
fn test_squash_scenario_20c_autoformatter_noise_during_squash() {
    // Plan scenario 20c: the squash commit's diff contains the branch's real
    // change plus many trivial formatting-tool line changes across the same
    // file (e.g. a linter reformatted touched lines). Per the plan doc, this
    // is the hardest calibration case; observe-then-assert whichever side of
    // the threshold this concrete recipe lands on rather than assuming a
    // specific classification, and record the outcome plainly.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    let baseline: Vec<String> = (1..=20).map(|i| format!("line{i} = {i};")).collect();
    std::fs::write(dir.join("f20c.txt"), baseline.join("\n") + "\n").unwrap();
    run_git(dir, &["add", "f20c.txt"]);
    run_git(dir, &["commit", "-m", "f20c baseline"]);

    run_git(dir, &["checkout", "-b", "feature/20c"]);
    // The branch's real change: modify a single line.
    let mut branch_lines = baseline.clone();
    branch_lines[9] = "line10 = 10; // real change".to_string();
    std::fs::write(dir.join("f20c.txt"), branch_lines.join("\n") + "\n").unwrap();
    run_git(dir, &["commit", "-am", "branch: real change to line 10"]);

    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/20c"]);
    // Simulate an autoformatter: reformat every line's spacing plus keep the
    // real change.
    let mut formatted_lines = branch_lines.clone();
    for line in formatted_lines.iter_mut() {
        *line = line.replace(" = ", "=");
    }
    std::fs::write(dir.join("f20c.txt"), formatted_lines.join("\n") + "\n").unwrap();
    run_git(dir, &["add", "f20c.txt"]);
    run_git(
        dir,
        &["commit", "-m", "squash landing plus autoformatter noise"],
    );
    let squash_oid = git_output(dir, &["rev-parse", "HEAD"]);

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..tmpdir.graph_options()
        },
    )
    .expect("graph load should succeed");
    let landing = snapshot
        .commits
        .iter()
        .find(|c| c.oid == squash_oid)
        .expect("squash landing commit should be displayed");
    assert!(
        !landing.is_possible_squash_merge(),
        "autoformatter noise must break exact patch-id equality"
    );
    assert!(!merge_detection::is_squash_merged(
        dir,
        "main",
        "feature/20c",
        None,
        None
    ));

    // Observe-then-assert: record the actual classification for this
    // recipe rather than assuming one. With 20 lines reformatted (spacing
    // removed on every line) plus the 1 real content change, the branch's
    // own diff (1 changed line, 2 tokens) shares very little of the squash
    // diff's changed-line set (20 reformatted lines + 1 real change, ~40
    // tokens) — low Jaccard similarity is expected, so this recipe is
    // expected to land BELOW the fuzzy threshold without a raw-line-comparison
    // formatting-aware normalization pass (which is not implemented; see
    // git/fuzzy_match.rs's module doc comment and this test's own comment as
    // the recorded outcome of that open design question).
    assert!(
        landing.fuzzy_squash_match().is_none(),
        "observed: without formatting-aware normalization, heavy autoformatter noise dilutes \
         line-level Jaccard similarity below the fuzzy threshold for this recipe; if this \
         assertion starts failing, update this comment to record the newly observed value \
         rather than assuming the prior finding still holds"
    );
}

#[test]
fn test_squash_scenario_20d_coincidentally_similar_but_unrelated_negative_control() {
    // Plan scenario 20d: two commits touch the same file with moderate line
    // overlap by coincidence, but are not the same logical change (different
    // function, same file, similar boilerplate). Must NOT be flagged by
    // Option 6 — this is the fuzzy tier's own false-positive control.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    std::fs::write(
        dir.join("f20d.txt"),
        "fn one() {}\nfn two() {}\nfn three() {}\nfn four() {}\n",
    )
    .unwrap();
    run_git(dir, &["add", "f20d.txt"]);
    run_git(dir, &["commit", "-m", "f20d baseline"]);

    run_git(dir, &["checkout", "-b", "feature/20d"]);
    std::fs::write(
        dir.join("f20d.txt"),
        "fn one() {\n    println!(\"one\");\n}\nfn two() {}\nfn three() {}\nfn four() {}\n",
    )
    .unwrap();
    run_git(
        dir,
        &[
            "commit",
            "-am",
            "branch: implement fn one with similar boilerplate",
        ],
    );

    run_git(dir, &["checkout", "main"]);
    std::fs::write(
        dir.join("f20d.txt"),
        "fn one() {}\nfn two() {}\nfn three() {\n    println!(\"three\");\n}\nfn four() {}\n",
    )
    .unwrap();
    run_git(
        dir,
        &[
            "commit",
            "-am",
            "main: unrelated commit, implement fn three with similar boilerplate",
        ],
    );
    let unrelated_base_oid = git_output(dir, &["rev-parse", "HEAD"]);

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..tmpdir.graph_options()
        },
    )
    .expect("graph load should succeed");
    let unrelated = snapshot
        .commits
        .iter()
        .find(|c| c.oid == unrelated_base_oid)
        .expect("unrelated base commit should be displayed");
    assert!(
        !unrelated.is_possible_squash_merge(),
        "these are genuinely different diffs, so exact match must not fire"
    );
    assert!(
        unrelated.fuzzy_squash_match().is_none(),
        "coincidental boilerplate similarity must not clear the fuzzy threshold — if this \
         fails, FUZZY_SIMILARITY_THRESHOLD in git/fuzzy_match.rs is too low and needs \
         revisiting before this tier ships (plan doc scenario 20d)"
    );
}
