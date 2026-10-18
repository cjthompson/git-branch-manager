

use crate::common::*;

#[test]
fn test_primary_branch_code_check_canonical_ref_types_and_outcomes() {
    use git_branch_manager::types::PrimaryBranchCodeMatch as Match;
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();

    // A regular merge is an ancestry match.
    run_git(dir, &["checkout", "-b", "regular"]);
    std::fs::write(dir.join("regular.txt"), "regular").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "regular"]);
    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--no-ff", "regular", "-m", "merge regular"]);
    assert_eq!(
        check_primary_code(&repo, "main", "refs/heads/regular").unwrap(),
        Match::Merged
    );

    // A branch whose tree was integrated with squash semantics is equivalent
    // even though its tip is not an ancestor of main.
    run_git(dir, &["checkout", "-b", "squashed"]);
    std::fs::write(dir.join("squashed.txt"), "squashed").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "squashed source"]);
    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "squashed"]);
    run_git(dir, &["commit", "-m", "squashed integration"]);
    let integrated_oid = repo.head().unwrap().target().unwrap().to_string();
    assert_eq!(
        check_primary_code(&repo, "main", "refs/heads/squashed").unwrap(),
        Match::ContentEquivalent {
            commit_oid: integrated_oid
        }
    );

    // Two separate commits cherry-picked into main retain neither ancestry nor
    // the aggregate squash patch-id of the source branch.
    run_git(dir, &["checkout", "-b", "cherry-source", "main"]);
    std::fs::write(dir.join("cherry-a.txt"), "a").unwrap();
    run_git(dir, &["add", "."]);
    run_git_with_env(
        dir,
        &["commit", "-m", "cherry a"],
        &[
            ("GIT_AUTHOR_DATE", "2001-01-01T00:00:00Z"),
            ("GIT_COMMITTER_DATE", "2001-01-01T00:00:00Z"),
        ],
    );
    let cherry_a = repo.head().unwrap().target().unwrap().to_string();
    std::fs::write(dir.join("cherry-b.txt"), "b").unwrap();
    run_git(dir, &["add", "."]);
    run_git_with_env(
        dir,
        &["commit", "-m", "cherry b"],
        &[
            ("GIT_AUTHOR_DATE", "2001-01-02T00:00:00Z"),
            ("GIT_COMMITTER_DATE", "2001-01-02T00:00:00Z"),
        ],
    );
    let cherry_tip = repo.head().unwrap().target().unwrap().to_string();
    run_git(dir, &["checkout", "main"]);
    run_git_with_env(
        dir,
        &["cherry-pick", &cherry_a],
        &[("GIT_COMMITTER_DATE", "2002-01-01T00:00:00Z")],
    );
    run_git_with_env(
        dir,
        &["cherry-pick", &cherry_tip],
        &[("GIT_COMMITTER_DATE", "2002-01-02T00:00:00Z")],
    );
    assert!(!repo
        .graph_descendant_of(repo.head().unwrap().target().unwrap(), git2::Oid::from_str(&cherry_tip).unwrap())
        .unwrap());
    assert_eq!(
        check_primary_code(&repo, "main", "refs/heads/cherry-source").unwrap(),
        Match::ContentEquivalent {
            commit_oid: repo.head().unwrap().target().unwrap().to_string()
        }
    );

    // A tag and synthetic remote ref resolve through their canonical names.
    run_git(dir, &["tag", "-a", "release", "-m", "annotated release"]);
    run_git(
        dir,
        &["update-ref", "refs/remotes/origin/regular", "regular"],
    );
    assert_eq!(
        check_primary_code(&repo, "main", "refs/tags/release").unwrap(),
        Match::Merged
    );
    assert_eq!(
        check_primary_code(&repo, "main", "refs/remotes/origin/regular").unwrap(),
        Match::Merged
    );

    // A detached worktree target is accepted as an abbreviated OID.
    let oid = repo
        .find_branch("regular", git2::BranchType::Local)
        .unwrap()
        .get()
        .target()
        .unwrap()
        .to_string();
    let detached = &oid[..8];
    assert_eq!(
        check_primary_code(&repo, "main", detached).unwrap(),
        Match::Merged
    );

    // A ref with no change represented in the primary branch is absent.
    run_git(dir, &["checkout", "-b", "absent"]);
    std::fs::write(dir.join("absent.txt"), "absent").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "absent"]);
    assert_eq!(
        check_primary_code(&repo, "main", "refs/heads/absent").unwrap(),
        Match::NotFound
    );
}

#[test]
fn test_primary_branch_code_check_finds_integrated_change_after_revert() {
    use git_branch_manager::types::PrimaryBranchCodeMatch as Match;

    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/historical"]);
    std::fs::write(dir.join("historical.txt"), "integrated content\n").unwrap();
    run_git(dir, &["add", "historical.txt"]);
    run_git(dir, &["commit", "-m", "selected change"]);

    run_git(dir, &["checkout", "main"]);
    run_git(
        dir,
        &[
            "merge",
            "--no-ff",
            "feature/historical",
            "-m",
            "integrate selected change",
        ],
    );
    run_git(dir, &["revert", "-m", "1", "--no-edit", "HEAD"]);

    let outcome = check_primary_code(&repo, "main", "refs/heads/feature/historical").unwrap();
    assert_eq!(outcome, Match::Merged);
}

#[test]
fn test_primary_branch_code_check_finds_manual_integration_with_unrelated_changes() {
    use git_branch_manager::types::PrimaryBranchCodeMatch as Match;

    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();
    run_git(dir, &["checkout", "-b", "feature/manual"]);
    std::fs::write(dir.join("feature.txt"), "feature code\n").unwrap();
    run_git(dir, &["add", "feature.txt"]);
    run_git(dir, &["commit", "-m", "feature code"]);

    run_git(dir, &["checkout", "main"]);
    std::fs::write(dir.join("feature.txt"), "feature code\n").unwrap();
    std::fs::write(dir.join("unrelated.txt"), "primary-only change\n").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "manual integration with unrelated edit"]);

    let outcome = check_primary_code(&repo, "main", "refs/heads/feature/manual").unwrap();
    assert!(matches!(outcome, Match::ContentEquivalent { .. }));
}

#[test]
fn test_primary_branch_code_check_finds_split_integration_at_complete_snapshot() {
    use git_branch_manager::types::PrimaryBranchCodeMatch as Match;

    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();
    run_git(dir, &["checkout", "-b", "feature/split"]);
    std::fs::write(dir.join("first.txt"), "first change\n").unwrap();
    run_git(dir, &["add", "first.txt"]);
    run_git(dir, &["commit", "-m", "first source change"]);
    std::fs::write(dir.join("second.txt"), "second change\n").unwrap();
    run_git(dir, &["add", "second.txt"]);
    run_git(dir, &["commit", "-m", "second source change"]);

    run_git(dir, &["checkout", "main"]);
    std::fs::write(dir.join("first.txt"), "first change\n").unwrap();
    run_git(dir, &["add", "first.txt"]);
    run_git(dir, &["commit", "-m", "integrate first source change"]);
    std::fs::write(dir.join("unrelated.txt"), "unrelated\n").unwrap();
    run_git(dir, &["add", "unrelated.txt"]);
    run_git(dir, &["commit", "-m", "unrelated primary change"]);
    std::fs::write(dir.join("second.txt"), "second change\n").unwrap();
    run_git(dir, &["add", "second.txt"]);
    run_git(dir, &["commit", "-m", "integrate second source change"]);
    let final_snapshot = repo.head().unwrap().target().unwrap().to_string();

    assert_eq!(
        check_primary_code(&repo, "main", "refs/heads/feature/split").unwrap(),
        Match::ContentEquivalent {
            commit_oid: final_snapshot
        }
    );
}

#[test]
fn test_primary_branch_code_check_rejects_partial_integration() {
    use git_branch_manager::types::PrimaryBranchCodeMatch as Match;

    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();
    run_git(dir, &["checkout", "-b", "feature/partial"]);
    std::fs::write(dir.join("first.txt"), "first change\n").unwrap();
    run_git(dir, &["add", "first.txt"]);
    run_git(dir, &["commit", "-m", "first source change"]);
    std::fs::write(dir.join("second.txt"), "second change\n").unwrap();
    run_git(dir, &["add", "second.txt"]);
    run_git(dir, &["commit", "-m", "second source change"]);
    run_git(dir, &["checkout", "main"]);
    std::fs::write(dir.join("first.txt"), "first change\n").unwrap();
    run_git(dir, &["add", "first.txt"]);
    run_git(dir, &["commit", "-m", "integrate only first source change"]);

    assert_eq!(
        check_primary_code(&repo, "main", "refs/heads/feature/partial").unwrap(),
        Match::NotFound
    );
}
