
use git_branch_manager::git::{
    cache, cherry_loader, merge_detection, squash_loader,
};
use git_branch_manager::types::{
    MergeStatus, SquashConfidence,
};

use crate::common::*;

#[test]
fn test_likely_squash_merged_merge_tree_confirms_when_bundled_with_unrelated_change() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/bundled"]);
    std::fs::write(dir.join("shared.txt"), "shared content\n").unwrap();
    run_git(dir, &["add", "shared.txt"]);
    run_git(dir, &["commit", "-m", "add shared.txt"]);
    run_git(dir, &["checkout", "main"]);

    run_git(dir, &["merge", "--squash", "feature/bundled"]);
    std::fs::write(dir.join("other.txt"), "unrelated content\n").unwrap();
    run_git(dir, &["add", "."]);
    run_git(
        dir,
        &["commit", "-m", "squash shared.txt + unrelated other.txt"],
    );

    assert!(!merge_detection::is_squash_merged(
        dir,
        "main",
        "feature/bundled",
        None,
        None
    ));
    assert_eq!(
        merge_detection::likely_squash_merged(dir, "main", "feature/bundled", None, None),
        Some(SquashConfidence::MergeTreeConfirmed)
    );
}

#[test]
fn test_likely_squash_merged_rejects_invalid_supplied_merge_base() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/invalid-merge-base"]);
    std::fs::write(dir.join("shared.txt"), "shared content\n").unwrap();
    run_git(dir, &["add", "shared.txt"]);
    run_git(dir, &["commit", "-m", "add shared.txt"]);
    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/invalid-merge-base"]);
    std::fs::write(dir.join("other.txt"), "unrelated content\n").unwrap();
    run_git(dir, &["add", "."]);
    run_git(
        dir,
        &["commit", "-m", "squash shared.txt + unrelated other.txt"],
    );

    assert_eq!(
        merge_detection::likely_squash_merged(
            dir,
            "main",
            "feature/invalid-merge-base",
            None,
            Some("invalid-ref"),
        ),
        None,
        "an invalid supplied merge base must fail closed before merge-tree confirmation"
    );
}

#[test]
fn test_likely_squash_merged_fuzzy_confirms_when_merge_tree_conflicts() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    let original = "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\n";
    std::fs::write(dir.join("f8c.txt"), original).unwrap();
    run_git(dir, &["add", "f8c.txt"]);
    run_git(dir, &["commit", "-m", "f8c baseline"]);

    run_git(dir, &["checkout", "-b", "feature/8c-fuzzy"]);
    let branch_content = "l1-b\nl2-b\nl3-b\nl4-b\nl5-b\nl6-b\nl7\nl8\n";
    std::fs::write(dir.join("f8c.txt"), branch_content).unwrap();
    run_git(dir, &["commit", "-am", "branch changes lines 1-6"]);

    run_git(dir, &["checkout", "main"]);
    let main_parent_content = "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8-main\n";
    std::fs::write(dir.join("f8c.txt"), main_parent_content).unwrap();
    run_git(dir, &["commit", "-am", "main changes line 8 independently"]);

    run_git(dir, &["merge", "--squash", "feature/8c-fuzzy"]);
    let squash_content = "l1-b\nl2-b\nl3-b\nl4-b\nl5-b\nl6-b\nl7-resolved\nl8-main\n";
    std::fs::write(dir.join("f8c.txt"), squash_content).unwrap();
    run_git(dir, &["add", "f8c.txt"]);
    run_git(
        dir,
        &["commit", "-m", "squash landing with extra resolution edit"],
    );

    assert!(!merge_detection::is_squash_merged(
        dir,
        "main",
        "feature/8c-fuzzy",
        None,
        None
    ));
    assert_eq!(
        merge_detection::likely_squash_merged(dir, "main", "feature/8c-fuzzy", None, None),
        Some(SquashConfidence::FuzzyMatch {
            similarity_percent: 75
        })
    );
}

#[test]
fn test_likely_squash_merged_returns_none_when_no_signal() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/unrelated"]);
    std::fs::write(dir.join("unique-file.txt"), "unique\n").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "feature: add unique file"]);

    run_git(dir, &["checkout", "main"]);
    std::fs::write(dir.join("other2.txt"), "other2\n").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "main: add unrelated other2"]);

    assert!(!merge_detection::is_squash_merged(
        dir,
        "main",
        "feature/unrelated",
        None,
        None
    ));
    assert_eq!(
        merge_detection::likely_squash_merged(dir, "main", "feature/unrelated", None, None),
        None
    );
}

#[test]
fn test_spawn_squash_checker_reports_likely_squash_merged_with_confidence() {
    // End-to-end through the real spawn_squash_checker pipeline, using the
    // same "squash bundled with an unrelated change" fixture as
    // test_likely_squash_merged_merge_tree_confirms_when_bundled_with_unrelated_change.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/bundled-e2e"]);
    std::fs::write(dir.join("shared.txt"), "shared content\n").unwrap();
    run_git(dir, &["add", "shared.txt"]);
    run_git(dir, &["commit", "-m", "add shared.txt"]);
    let branch_tip = git_output(dir, &["rev-parse", "feature/bundled-e2e"]);
    run_git(dir, &["checkout", "main"]);

    run_git(dir, &["merge", "--squash", "feature/bundled-e2e"]);
    std::fs::write(dir.join("other.txt"), "unrelated content\n").unwrap();
    run_git(dir, &["add", "."]);
    run_git(
        dir,
        &["commit", "-m", "squash shared.txt + unrelated other.txt"],
    );

    let candidates = vec![("feature/bundled-e2e".to_string(), branch_tip, None)];
    let cache = cache::BranchCache::load_for_base(
        dir,
        "main",
        &cache::CacheRoot::at(tmpdir.cache_root().to_path_buf()),
    );
    let rx = squash_loader::spawn_squash_checker(
        dir.to_path_buf(),
        "main".to_string(),
        candidates,
        cache,
    );

    let results: Vec<_> = rx.into_iter().collect();
    assert_eq!(results.len(), 1);
    let result = &results[0];
    assert_eq!(result.branch_name, "feature/bundled-e2e");
    assert_eq!(result.status, MergeStatus::LikelySquashMerged);
    assert_eq!(
        result.confidence,
        Some(SquashConfidence::MergeTreeConfirmed)
    );
}

#[test]
fn test_squash_checker_rechecks_cached_unmerged_after_base_advances() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();
    let cache_path = dir.join("branch-cache.sqlite3");

    run_git(dir, &["checkout", "-b", "feature/cached-unmerged"]);
    std::fs::write(dir.join("shared.txt"), "shared content\n").unwrap();
    run_git(dir, &["add", "shared.txt"]);
    run_git(dir, &["commit", "-m", "add shared.txt"]);
    let branch_tip = git_output(dir, &["rev-parse", "feature/cached-unmerged"]);
    run_git(dir, &["checkout", "main"]);

    let candidates = vec![(
        "feature/cached-unmerged".to_string(),
        branch_tip.clone(),
        None,
    )];
    let cherry_results: Vec<_> = cherry_loader::spawn_cherry_checker(
        dir.to_path_buf(),
        "main".to_string(),
        candidates.clone(),
        cache::BranchCache::load_from_path(cache_path.clone()),
    )
    .into_iter()
    .collect();
    assert_eq!(cherry_results.len(), 1);
    assert_eq!(cherry_results[0].status, MergeStatus::Unmerged);
    assert_eq!(
        cache::BranchCache::load_from_path(cache_path.clone())
            .lookup("feature/cached-unmerged", &branch_tip),
        Some(MergeStatus::Unmerged),
        "the real cherry checker must seed the same-tip Unmerged cache entry"
    );

    run_git(dir, &["merge", "--squash", "feature/cached-unmerged"]);
    std::fs::write(dir.join("other.txt"), "unrelated content\n").unwrap();
    run_git(dir, &["add", "."]);
    run_git(
        dir,
        &["commit", "-m", "squash shared.txt with unrelated other.txt"],
    );

    let squash_results: Vec<_> = squash_loader::spawn_squash_checker(
        dir.to_path_buf(),
        "main".to_string(),
        candidates,
        cache::BranchCache::load_from_path(cache_path),
    )
    .into_iter()
    .collect();
    assert_eq!(squash_results.len(), 1);
    assert_eq!(
        squash_results[0].status,
        MergeStatus::LikelySquashMerged,
        "a cached Unmerged must not hide fresh squash detection after base advances"
    );
    assert_eq!(
        squash_results[0].confidence,
        Some(SquashConfidence::MergeTreeConfirmed)
    );
}
