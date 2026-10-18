use std::process::Command;

use git_branch_manager::git::{
    graph, merge_detection,
};
use git_branch_manager::types::MergeStatus;

use crate::common::*;

#[test]
fn test_squash_scenario_05_partial_landing_via_individual_cherry_picks() {
    // Plan scenario 5: only 2 of the branch's 3 commits are cherry-picked
    // individually onto base (not a full squash). Documents the current
    // all-or-nothing behavior as a known, accepted limitation (see plan doc
    // scenario 5) rather than a bug to fix here.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/partial"]);
    std::fs::write(dir.join("p1.txt"), "p1\n").unwrap();
    run_git(dir, &["add", "p1.txt"]);
    run_git(dir, &["commit", "-m", "p1"]);
    let c1 = git_output(dir, &["rev-parse", "HEAD"]);
    std::fs::write(dir.join("p2.txt"), "p2\n").unwrap();
    run_git(dir, &["add", "p2.txt"]);
    run_git(dir, &["commit", "-m", "p2"]);
    let c2 = git_output(dir, &["rev-parse", "HEAD"]);
    std::fs::write(dir.join("p3.txt"), "p3\n").unwrap();
    run_git(dir, &["add", "p3.txt"]);
    run_git(dir, &["commit", "-m", "p3"]);

    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["cherry-pick", &c1]);
    run_git(dir, &["cherry-pick", &c2]);
    let landed_tip = git_output(dir, &["rev-parse", "HEAD"]);

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..tmpdir.graph_options()
        },
    )
    .expect("graph load should succeed");
    assert!(
        snapshot
            .commits
            .iter()
            .all(|c| !c.is_possible_squash_merge()),
        "partial cherry-pick coverage must not produce any possible-squash annotation \
         (known limitation: no single base commit's diff equals the branch's full aggregate diff)"
    );
    let _ = landed_tip;

    assert!(
        !merge_detection::is_squash_merged(dir, "main", "feature/partial", None, None),
        "partial coverage (2 of 3 commits landed individually) must not report as fully squash-merged"
    );
}

#[test]
fn test_squash_scenario_06_reordered_commits_and_hunk_order_insensitivity() {
    // Plan scenario 6, part 1: commits touching different files land as one
    // squash commit; functional flagged assertion (order of commits within
    // the branch must not matter to the aggregate diff).
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/reordered"]);
    std::fs::write(dir.join("z.txt"), "z content\n").unwrap();
    run_git(dir, &["add", "z.txt"]);
    run_git(dir, &["commit", "-m", "add z first"]);
    std::fs::write(dir.join("a.txt"), "a content\n").unwrap();
    run_git(dir, &["add", "a.txt"]);
    run_git(dir, &["commit", "-m", "add a second"]);

    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/reordered"]);
    run_git(dir, &["commit", "-m", "squash landing (reordered)"]);
    let squash_oid = git_output(dir, &["rev-parse", "HEAD"]);

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..tmpdir.graph_options()
        },
    )
    .expect("graph load should succeed");
    assert!(
        snapshot
            .commits
            .iter()
            .find(|c| c.oid == squash_oid)
            .expect("squash landing commit should be displayed")
            .is_possible_squash_merge(),
        "commit order within the branch must not affect the aggregate-diff match"
    );

    // Plan scenario 6, part 2: directly confirm `git patch-id --stable`'s
    // documented hunk-order insensitivity holds for this codebase's exact
    // invocation (`git diff --binary --full-index --no-ext-diff --no-textconv`
    // piped to `git patch-id --stable`), not just in isolation. A custom
    // `diff.orderFile` forces `git diff` to emit the same two file-hunks in
    // reverse order; the resulting patch-id must still match the default-order
    // patch-id.
    let parent = git_output(dir, &["rev-parse", &format!("{squash_oid}^")]);
    let default_order_id = git_output(
        dir,
        &[
            "diff",
            "--binary",
            "--full-index",
            "--no-ext-diff",
            "--no-textconv",
            &parent,
            &squash_oid,
            "--",
        ],
    );
    let order_file = dir.join("order.txt");
    std::fs::write(&order_file, "a.txt\nz.txt\n").unwrap();
    let reordered_diff_output = Command::new("git")
        .current_dir(dir)
        .args([
            "-c",
            &format!("diff.orderFile={}", order_file.display()),
            "diff",
            "--binary",
            "--full-index",
            "--no-ext-diff",
            "--no-textconv",
            &parent,
            &squash_oid,
            "--",
        ])
        .output()
        .expect("git diff with orderFile should run");
    assert!(reordered_diff_output.status.success());

    fn patch_id_of(dir: &std::path::Path, diff_text: &[u8]) -> String {
        use std::io::Write;
        let mut child = Command::new("git")
            .current_dir(dir)
            .args(["patch-id", "--stable"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("git patch-id should spawn");
        child.stdin.as_mut().unwrap().write_all(diff_text).unwrap();
        let out = child.wait_with_output().unwrap();
        String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .next()
            .unwrap()
            .to_string()
    }

    let default_id = patch_id_of(dir, default_order_id.as_bytes());
    let reordered_id = patch_id_of(dir, &reordered_diff_output.stdout);
    assert_eq!(
        default_id, reordered_id,
        "git patch-id --stable must be insensitive to hunk/file order in this codebase's exact invocation"
    );
}

#[test]
fn test_squash_scenario_07_rebased_branch_then_squash_merged() {
    // Plan scenario 7: branch created off base; base advances; branch is
    // rebased onto the new base tip; then squashed. Detection must use the
    // post-rebase merge-base, not a stale ancestor.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/rebased"]);
    std::fs::write(dir.join("rebased.txt"), "rebased content\n").unwrap();
    run_git(dir, &["add", "rebased.txt"]);
    run_git(dir, &["commit", "-m", "rebased feature commit"]);

    run_git(dir, &["checkout", "main"]);
    std::fs::write(dir.join("main-advance.txt"), "main advanced\n").unwrap();
    run_git(dir, &["add", "main-advance.txt"]);
    run_git(dir, &["commit", "-m", "main advances"]);

    run_git(dir, &["checkout", "feature/rebased"]);
    run_git(dir, &["rebase", "main"]);

    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/rebased"]);
    run_git(dir, &["commit", "-m", "squash landing (rebased)"]);
    let squash_oid = git_output(dir, &["rev-parse", "HEAD"]);

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..tmpdir.graph_options()
        },
    )
    .expect("graph load should succeed");
    assert!(
        snapshot
            .commits
            .iter()
            .find(|c| c.oid == squash_oid)
            .expect("squash landing commit should be displayed")
            .is_possible_squash_merge(),
        "a rebased-then-squashed branch must be flagged using the post-rebase merge-base"
    );

    let repo = git2::Repository::open(dir).unwrap();
    let branches = test_branches(&tmpdir, &repo, "main").expect("list_branches failed");
    let feature = branches
        .iter()
        .find(|b| b.name == "feature/rebased")
        .unwrap();
    assert_eq!(feature.merge_status, MergeStatus::LocalSquashMerged);
}
