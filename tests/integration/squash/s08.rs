use std::process::Command;

use git_branch_manager::git::{
    graph, merge_detection,
};
use git_branch_manager::symbols::SymbolSet;
use git_branch_manager::theme::Theme;
use git_branch_manager::ui::graph_render::render_graph_view;
use git_branch_manager::view::graph::GraphState;
use ratatui::backend::TestBackend;
use ratatui::Terminal;

use crate::common::*;

#[test]
fn test_squash_scenario_08a_conflict_resolution_extra_lines_fuzzy_positive() {
    // Plan scenario 8a: base and branch modify the same file on different
    // lines (no true git-level conflict), but the squash author also fixed up
    // a few adjacent lines while resolving. The squash commit's diff therefore
    // contains the branch's real changes *plus* extra resolution-only lines
    // not present in the branch's own diff.
    //
    // The branch's own diff changes 6 lines (12 added/removed tokens); the
    // squash adds one extra resolution edit (2 more tokens) on top. Hand-
    // calculated Jaccard similarity: 12 shared / 14 union ≈ 0.857 — comfortably
    // above the calibrated FUZZY_SIMILARITY_THRESHOLD (see git/fuzzy_match.rs).
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    let original = "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\n";
    std::fs::write(dir.join("f8a.txt"), original).unwrap();
    run_git(dir, &["add", "f8a.txt"]);
    run_git(dir, &["commit", "-m", "f8a baseline"]);

    run_git(dir, &["checkout", "-b", "feature/8a"]);
    let branch_content = "l1-b\nl2-b\nl3-b\nl4-b\nl5-b\nl6-b\nl7\nl8\n";
    std::fs::write(dir.join("f8a.txt"), branch_content).unwrap();
    run_git(dir, &["commit", "-am", "branch changes lines 1-6"]);
    let branch_tip = git_output(dir, &["rev-parse", "HEAD"]);

    run_git(dir, &["checkout", "main"]);
    // main advances independently so the squash isn't a trivial fast-forward.
    let main_parent_content = "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8-main\n";
    std::fs::write(dir.join("f8a.txt"), main_parent_content).unwrap();
    run_git(dir, &["commit", "-am", "main changes line 8 independently"]);
    let main_parent = git_output(dir, &["rev-parse", "HEAD"]);

    run_git(dir, &["merge", "--squash", "feature/8a"]);
    // Simulate a conflict-resolution pass: apply the branch's lines 1-6 *plus*
    // one extra resolution tweak the branch itself never made (line 7).
    let squash_content = "l1-b\nl2-b\nl3-b\nl4-b\nl5-b\nl6-b\nl7-resolved\nl8-main\n";
    std::fs::write(dir.join("f8a.txt"), squash_content).unwrap();
    run_git(dir, &["add", "f8a.txt"]);
    run_git(
        dir,
        &["commit", "-m", "squash landing with extra resolution edit"],
    );
    let squash_oid = git_output(dir, &["rev-parse", "HEAD"]);

    // Expect A: not flagged under exact patch-id match (known false negative).
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
        "extra resolution lines must break exact patch-id equality (known false negative, plan scenario 8a)"
    );

    // Expect B: not SquashMerged, same reason.
    assert!(!merge_detection::is_squash_merged(
        dir,
        "main",
        "feature/8a",
        None,
        Some(&main_parent),
    ));

    // Expect Option 6: flagged as a fuzzy possible-squash match with high
    // similarity.
    let relationship = landing
        .relationships
        .iter()
        .find(|relationship| relationship.source_oid == branch_tip)
        .expect("fuzzy source pairing should be retained");
    assert_eq!(relationship.destination_oid, squash_oid);
    assert_eq!(relationship.destination_refs, ["main"]);
    assert_eq!(relationship.source_refs, ["feature/8a"]);
    assert!(matches!(
        relationship.matching,
        graph::RelationshipMatch::Fuzzy {
            similarity_percent: 0..=99
        }
    ));
    let fuzzy = landing
        .fuzzy_squash_match()
        .expect("high line-level similarity should surface a fuzzy possible-squash match");
    assert!(
        fuzzy.similarity_percent >= 75,
        "expected high similarity for a near-exact match with one extra resolution edit, got {}",
        fuzzy.similarity_percent
    );

    let mut graph_state = GraphState::new();
    graph_state.apply_result(Ok(snapshot.clone()));
    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    terminal
        .draw(|frame| {
            render_graph_view(
                frame,
                frame.area(),
                &mut graph_state,
                &Theme::dark(),
                &SymbolSet::ascii(),
            )
        })
        .unwrap();
    let short_oid = &squash_oid[..7];
    let rendered_lines: Vec<String> = terminal
        .backend()
        .buffer()
        .content()
        .chunks(120)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect())
        .collect();
    let landing_row = rendered_lines
        .iter()
        .find(|line| line.contains(short_oid))
        .expect("fuzzy landing should have a rendered graph row");
    assert!(
        landing_row.contains(&format!("[fuzzy squash {}%]", fuzzy.similarity_percent)),
        "rendered fuzzy landing should show its computed similarity, got: {landing_row}"
    );
    assert!(
        landing_row.contains("│o"),
        "fuzzy classification should keep the ordinary DAG glyph, got: {landing_row}"
    );
    let _ = branch_tip;
}

#[test]
fn test_squash_scenario_08b_true_conflicting_hunks_manually_resolved() {
    // Plan scenario 8b: base and branch modify the *same* lines (a true
    // conflict). The squash commit's tree is constructed directly as what a
    // human would produce resolving the conflict, rather than via a
    // mechanical merge. This test records whichever side of the Option 6
    // threshold this concrete recipe lands on (observe-then-assert), per the
    // plan doc's explicit instruction not to assume a specific classification
    // in advance.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    let original = "one\ntwo\nthree\nfour\nfive\n";
    std::fs::write(dir.join("f8b.txt"), original).unwrap();
    run_git(dir, &["add", "f8b.txt"]);
    run_git(dir, &["commit", "-m", "f8b baseline"]);

    run_git(dir, &["checkout", "-b", "feature/8b"]);
    let branch_content = "one-BRANCH\ntwo-BRANCH\nthree\nfour\nfive\n";
    std::fs::write(dir.join("f8b.txt"), branch_content).unwrap();
    run_git(dir, &["commit", "-am", "branch changes lines 1-2"]);

    run_git(dir, &["checkout", "main"]);
    let main_parent_content = "one-MAIN\ntwo-MAIN\nthree\nfour\nfive\n";
    std::fs::write(dir.join("f8b.txt"), main_parent_content).unwrap();
    run_git(
        dir,
        &[
            "commit",
            "-am",
            "main changes lines 1-2 independently (true conflict)",
        ],
    );
    let main_parent = git_output(dir, &["rev-parse", "HEAD"]);

    // This is a true git-level conflict, so `git merge --squash` exits
    // non-zero and leaves conflict markers in the working tree; run it
    // directly (not via the panic-on-failure `run_git` helper) and ignore the
    // exit code, since we overwrite the file with the manually resolved
    // content below anyway.
    let _ = Command::new("git")
        .current_dir(dir)
        .args(["merge", "--squash", "feature/8b"])
        .output()
        .expect("git merge --squash should run (conflict exit is expected)");
    // Manually resolve: keep main's line 1, branch's line 2, as a human might.
    let resolved_content = "one-MAIN\ntwo-BRANCH\nthree\nfour\nfive\n";
    std::fs::write(dir.join("f8b.txt"), resolved_content).unwrap();
    run_git(dir, &["add", "f8b.txt"]);
    run_git(dir, &["commit", "-m", "manually resolved squash landing"]);
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
        "a manually-resolved true conflict must not exact-match (plan scenario 8b)"
    );
    assert!(!merge_detection::is_squash_merged(
        dir,
        "main",
        "feature/8b",
        None,
        Some(&main_parent),
    ));

    // Observe-then-assert: record whichever side of the threshold this
    // recipe lands on. Branch diff touches 2 lines (4 tokens); resolved
    // squash diff also touches 2 lines (4 tokens: -one-MAIN/-two-MAIN/+one-MAIN(no)
    // ... in practice the overlap is only the "two-BRANCH" addition, since the
    // removed lines differ (main's own text) and "one-MAIN" survives unchanged
    // (no diff line for it in the squash-vs-main-parent diff). This is a much
    // lower structural overlap than 8a, so we expect this concrete recipe to
    // land BELOW the fuzzy threshold.
    assert!(
        landing.fuzzy_squash_match().is_none(),
        "observed: this true-conflict recipe's similarity is too low to clear the fuzzy \
         threshold (majority of the resolved content differs from the branch's own diff); \
         if this fails, update this comment to record the newly-observed classification \
         rather than assuming the prior expectation still holds"
    );
}

#[test]
fn test_squash_scenario_08c_merge_tree_confirmation_check() {
    // Plan scenario 8c: uses the parent doc's Option 3 (`git merge-tree`)
    // confirmation signal, not Option 6. Using the same conflict-resolution
    // shape as 8a, run `git merge-tree --write-tree base branch` (the
    // *original*, un-squashed branch) against *current* base (which already
    // contains the resolved squash) and compare the resulting tree to base's
    // own tree. This scenario documents whether merge-tree re-simulation
    // succeeds even when exact patch-id match fails.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    let original = "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\n";
    std::fs::write(dir.join("f8c.txt"), original).unwrap();
    run_git(dir, &["add", "f8c.txt"]);
    run_git(dir, &["commit", "-m", "f8c baseline"]);

    run_git(dir, &["checkout", "-b", "feature/8c"]);
    let branch_content = "l1-b\nl2-b\nl3-b\nl4-b\nl5-b\nl6-b\nl7\nl8\n";
    std::fs::write(dir.join("f8c.txt"), branch_content).unwrap();
    run_git(dir, &["commit", "-am", "branch changes lines 1-6"]);

    run_git(dir, &["checkout", "main"]);
    let main_parent_content = "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8-main\n";
    std::fs::write(dir.join("f8c.txt"), main_parent_content).unwrap();
    run_git(dir, &["commit", "-am", "main changes line 8 independently"]);

    run_git(dir, &["merge", "--squash", "feature/8c"]);
    let squash_content = "l1-b\nl2-b\nl3-b\nl4-b\nl5-b\nl6-b\nl7-resolved\nl8-main\n";
    std::fs::write(dir.join("f8c.txt"), squash_content).unwrap();
    run_git(dir, &["add", "f8c.txt"]);
    run_git(
        dir,
        &["commit", "-m", "squash landing with extra resolution edit"],
    );

    let merge_tree_output = Command::new("git")
        .current_dir(dir)
        .args(["merge-tree", "--write-tree", "main", "feature/8c"])
        .output()
        .expect("git merge-tree should run");

    // Observe-then-assert: empirically, `git merge-tree --write-tree` reports
    // a genuine CONFLICT (non-zero exit) for this fixture, not a clean tree
    // match. All 8 lines of `f8c.txt` sit inside a single diff hunk on both
    // sides (the file is short enough that git's default 3-line context
    // merges the whole file into one hunk), so `main`'s independent line-7/8
    // edits and `feature`'s line 1-6 edits are treated as *overlapping*
    // hunks by the `ort` merge strategy even though the specific changed
    // lines don't literally collide — a real content conflict is reported.
    // This means merge-tree does NOT succeed where patch-id (8a/8b) fails
    // for this recipe; it fails too, just for a different, hunk-granularity
    // reason. This is itself a useful, concrete data point for the parent
    // plan doc's "A true, B false"-style disagreement-visibility principle:
    // here, neither exact patch-id equality nor merge-tree re-simulation
    // confirms the squash for a same-file multi-line resolution.
    assert!(
        !merge_tree_output.status.success(),
        "observed: git merge-tree reports a real conflict for this same-file, \
         multi-hunk resolution fixture rather than silently confirming the squash"
    );
    let stdout = String::from_utf8_lossy(&merge_tree_output.stdout);
    assert!(
        stdout.contains("CONFLICT"),
        "expected merge-tree's stdout to report the conflict explicitly, got: {stdout}"
    );
}
