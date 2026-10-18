
use git_branch_manager::git::{
    cache, cherry_loader, graph, merge_detection,
};
use git_branch_manager::types::MergeStatus;

use crate::common::*;
mod sources;

#[test]
fn test_is_cherry_picked_direct() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/cherry"]);
    std::fs::write(dir.join("cherry.txt"), "cherry content").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "cherry commit"]);
    run_git(dir, &["checkout", "main"]);

    // Cherry-pick the commit onto main (no merge commit).
    run_git(dir, &["cherry-pick", "feature/cherry"]);

    assert!(merge_detection::is_cherry_picked(
        dir,
        "main",
        "feature/cherry",
        None,
        None
    ));
}

#[test]
fn test_is_not_cherry_picked_direct() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/uncherried"]);
    std::fs::write(dir.join("uncherried.txt"), "uncherried content").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "uncherried commit"]);
    run_git(dir, &["checkout", "main"]);

    // No cherry-pick performed.
    assert!(!merge_detection::is_cherry_picked(
        dir,
        "main",
        "feature/uncherried",
        None,
        None
    ));
}

#[test]
fn test_is_cherry_picked_with_precomputed_merge_base() {
    // The fast path: a precomputed merge base is supplied, so is_cherry_picked
    // must not need to derive it via `git merge-base` and still detect the
    // cherry-pick.
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();

    // HEAD is the initial commit; it becomes the merge base after we branch.
    let merge_base = repo
        .head()
        .unwrap()
        .peel_to_commit()
        .unwrap()
        .id()
        .to_string();

    run_git(dir, &["checkout", "-b", "feature/cherry"]);
    std::fs::write(dir.join("cherry.txt"), "cherry content").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "cherry commit"]);
    run_git(dir, &["checkout", "main"]);

    run_git(dir, &["cherry-pick", "feature/cherry"]);

    assert!(merge_detection::is_cherry_picked(
        dir,
        "main",
        "feature/cherry",
        None,
        Some(&merge_base),
    ));
}

#[test]
fn test_partial_cherry_pick_is_not_cherry_picked() {
    // A branch whose commits were only partially cherry-picked onto base must
    // NOT be flagged as fully cherry-picked: `git cherry` returns at least one
    // `+` line (a commit whose patch-id isn't reachable from base) and the
    // detection must fail closed.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Build a branch with three commits: c1, c2, c3.
    run_git(dir, &["checkout", "-b", "feature/partial"]);
    std::fs::write(dir.join("c1.txt"), "c1").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "c1"]);
    std::fs::write(dir.join("c2.txt"), "c2").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "c2"]);
    std::fs::write(dir.join("c3.txt"), "c3").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "c3"]);
    run_git(dir, &["checkout", "main"]);

    // Cherry-pick only c1 and c2 onto main; leave c3 un-cherry-picked.
    let log = git_output(dir, &["log", "--reverse", "--format=%H", "feature/partial"]);
    let commits: Vec<&str> = log.lines().collect();
    // The first entry is the initial commit (merge-base), which we drop.
    assert!(
        commits.len() >= 4,
        "expected at least 4 commits (initial + c1/c2/c3) on feature/partial, got {}",
        commits.len()
    );
    run_git(dir, &["cherry-pick", commits[1]]);
    run_git(dir, &["cherry-pick", commits[2]]);

    assert!(!merge_detection::is_cherry_picked(
        dir,
        "main",
        "feature/partial",
        None,
        None
    ));
}

#[test]
fn test_cherry_loader_drains_to_cherry_picked_status() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path().to_path_buf();

    // Build a feature branch and cherry-pick every commit onto main.
    run_git(&dir, &["checkout", "-b", "feature/cherry-loader"]);
    std::fs::write(dir.join("loader.txt"), "loader").unwrap();
    run_git(&dir, &["add", "."]);
    run_git(&dir, &["commit", "-m", "loader commit"]);
    run_git(&dir, &["checkout", "main"]);

    // Cherry-pick the single feature commit.
    let tip = git_output(&dir, &["rev-parse", "feature/cherry-loader"]);
    run_git(&dir, &["cherry-pick", &tip]);

    let candidates = vec![("feature/cherry-loader".to_string(), tip, None)];
    let cache = cache::BranchCache::load_for_base(
        &dir,
        "main",
        &cache::CacheRoot::at(tmpdir.cache_root().to_path_buf()),
    );
    let rx =
        cherry_loader::spawn_cherry_checker(dir.clone(), "main".to_string(), candidates, cache);

    let results: Vec<_> = rx.iter().collect();
    assert_eq!(results.len(), 1, "expected one cherry result");
    let result = &results[0];
    assert_eq!(result.branch_name, "feature/cherry-loader");
    // No `origin/main` exists in this fixture, so the remote-base check fails
    // closed and we report `LocalCherryPicked`.
    assert!(
        matches!(result.status, MergeStatus::LocalCherryPicked),
        "expected LocalCherryPicked, got {:?}",
        result.status
    );
}

#[test]
fn test_graph_cherry_pick_enrichment_marks_branch_commits() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Build a branch with two commits, then cherry-pick each onto main.
    run_git(dir, &["checkout", "-b", "feature/graph-cherry"]);
    std::fs::write(dir.join("g1.txt"), "g1").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "g1"]);
    std::fs::write(dir.join("g2.txt"), "g2").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "g2"]);
    run_git(dir, &["checkout", "main"]);

    let log = git_output(
        dir,
        &["rev-list", "--reverse", "main..feature/graph-cherry"],
    );
    let commits: Vec<&str> = log.lines().collect();
    assert_eq!(commits.len(), 2);
    advance_main(dir, "old-cherry-fixture");
    for hash in &commits {
        run_git(dir, &["cherry-pick", hash]);
    }

    let options = graph::GraphLoadOptions {
        max_count: 50,
        include_remotes: false,
        line_style: graph::GraphLineStyle::Thin,
        base_branch: Some("main".to_string()),
        cache_root: tmpdir.graph_options().cache_root,
    };
    let snapshot =
        graph::load_graph_with_squash_annotations(dir, options).expect("graph load failed");

    let expected = commits
        .into_iter()
        .map(str::to_owned)
        .collect::<std::collections::BTreeSet<_>>();
    let actual = snapshot
        .commits
        .iter()
        .filter(|c| c.is_cherry_picked_commit())
        .map(|c| c.oid.clone())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(actual, expected);
}

pub(crate) fn rev(dir: &std::path::Path, spec: &str) -> String {
    git_output(dir, &["rev-parse", spec]).trim().to_owned()
}

pub(crate) fn advance_main(dir: &std::path::Path, label: &str) {
    std::fs::write(dir.join(format!("{label}.base")), label).unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-q", "-m", &format!("base {label}")]);
}
