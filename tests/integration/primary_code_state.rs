use std::process::Command;

use git_branch_manager::git::merge_detection;

use crate::common::*;

#[test]
fn test_primary_branch_code_check_keeps_repository_state_unchanged() {
    use git_branch_manager::types::PrimaryBranchCodeMatch as Match;

    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();
    std::fs::write(
        dir.join("shared.txt"),
        "alpha\none\ntwo\nthree\nfour\nfive\ngamma\n",
    )
    .unwrap();
    run_git(dir, &["add", "shared.txt"]);
    run_git(dir, &["commit", "-m", "add merge fixture"]);
    run_git(dir, &["checkout", "-b", "feature/source"]);
    std::fs::write(
        dir.join("shared.txt"),
        "source\none\ntwo\nthree\nfour\nfive\ngamma\n",
    )
    .unwrap();
    run_git(dir, &["commit", "-am", "source change"]);
    run_git(dir, &["checkout", "main"]);
    std::fs::write(
        dir.join("shared.txt"),
        "alpha\none\ntwo\nthree\nfour\nfive\nprimary\ngamma\n",
    )
    .unwrap();
    run_git(dir, &["commit", "-am", "diverged primary change"]);
    std::fs::write(dir.join("README.md"), "# Dirty tracked README\n").unwrap();
    std::fs::write(dir.join("staged-dirty.txt"), "dirty staged file\n").unwrap();
    run_git(dir, &["add", "staged-dirty.txt"]);
    std::fs::write(dir.join("untracked-dirty.txt"), "dirty untracked file\n").unwrap();

    let head_before = git_output(dir, &["rev-parse", "HEAD"]);
    let refs_before = git_output(dir, &["for-each-ref", "--format=%(refname) %(objectname)"]);
    let status_before = git_output(dir, &["status", "--porcelain", "--untracked-files=all"]);
    let index_before = std::fs::read(dir.join(".git/index")).unwrap();
    let objects_before = snapshot_files(&repo.path().join("objects"));
    let tracked_before = std::fs::read(dir.join("README.md")).unwrap();
    let untracked_before = std::fs::read(dir.join("untracked-dirty.txt")).unwrap();
    let temporary_merge_blob = git_blob_oid(
        dir,
        b"source\none\ntwo\nthree\nfour\nfive\nprimary\ngamma\n",
    );
    assert!(!Command::new("git")
        .args(["cat-file", "-e", &temporary_merge_blob])
        .current_dir(dir)
        .status()
        .unwrap()
        .success());

    assert_eq!(
        check_primary_code(&repo, "main", "refs/heads/feature/source").unwrap(),
        Match::NotFound
    );

    assert_eq!(git_output(dir, &["rev-parse", "HEAD"]), head_before);
    assert_eq!(
        git_output(dir, &["for-each-ref", "--format=%(refname) %(objectname)"]),
        refs_before
    );
    assert_eq!(
        git_output(dir, &["status", "--porcelain", "--untracked-files=all"]),
        status_before
    );
    assert_eq!(std::fs::read(dir.join(".git/index")).unwrap(), index_before);
    assert_eq!(snapshot_files(&repo.path().join("objects")), objects_before);
    assert_eq!(std::fs::read(dir.join("README.md")).unwrap(), tracked_before);
    assert_eq!(
        std::fs::read(dir.join("untracked-dirty.txt")).unwrap(),
        untracked_before
    );
    assert!(!Command::new("git")
        .args(["cat-file", "-e", &temporary_merge_blob])
        .current_dir(dir)
        .status()
        .unwrap()
        .success());

    std::fs::write(dir.join("post-check.txt"), "commit after comparison\n").unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(std::path::Path::new("post-check.txt")).unwrap();
    index.write().unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let parent = repo.head().unwrap().peel_to_commit().unwrap();
    let signature = repo.signature().unwrap();
    let commit_oid = repo
        .commit(
            Some("HEAD"),
            &signature,
            &signature,
            "commit after read-only comparison",
            &tree,
            &[&parent],
        )
        .unwrap();
    let reopened = git2::Repository::open(dir).unwrap();
    assert_eq!(reopened.find_commit(commit_oid).unwrap().id(), commit_oid);
}

#[test]
fn test_primary_branch_code_check_ignores_union_merge_attributes() {
    use git_branch_manager::types::PrimaryBranchCodeMatch as Match;

    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();
    std::fs::write(dir.join("conflict.txt"), "base content\nkeep\n").unwrap();
    run_git(dir, &["add", "conflict.txt"]);
    run_git(dir, &["commit", "-m", "add union fixture"]);

    run_git(dir, &["checkout", "-b", "feature/delete"]);
    std::fs::write(dir.join("conflict.txt"), "keep\n").unwrap();
    run_git(dir, &["add", "conflict.txt"]);
    run_git(dir, &["commit", "-m", "delete selected file"]);

    run_git(dir, &["checkout", "main"]);
    std::fs::write(dir.join("conflict.txt"), "primary kept this line\nkeep\n").unwrap();
    std::fs::write(dir.join(".git/info/attributes"), "conflict.txt merge=union\n").unwrap();
    run_git(dir, &["commit", "-am", "retain primary-only content"]);

    assert_eq!(
        check_primary_code(&repo, "main", "refs/heads/feature/delete").unwrap(),
        Match::NotFound
    );
}

#[test]
fn test_primary_branch_code_check_does_not_infer_merge_resolution_from_shared_parents() {
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();
    std::fs::write(dir.join("choice.txt"), "base\n").unwrap();
    run_git(dir, &["add", "choice.txt"]);
    run_git(dir, &["commit", "-m", "add merge fixture"]);

    run_git(dir, &["checkout", "-b", "feature/left"]);
    std::fs::write(dir.join("choice.txt"), "left\n").unwrap();
    run_git(dir, &["commit", "-am", "left side"]);
    let left = repo.head().unwrap().target().unwrap().to_string();

    run_git(dir, &["checkout", "-b", "feature/right", "main"]);
    std::fs::write(dir.join("choice.txt"), "right\n").unwrap();
    run_git(dir, &["commit", "-am", "right side"]);
    let right = repo.head().unwrap().target().unwrap().to_string();

    run_git(dir, &["checkout", "feature/left"]);
    let merge = Command::new("git")
        .args(["merge", "--no-ff", &right, "-m", "feature merge"])
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(!merge.status.success(), "fixture merge must conflict");
    std::fs::write(dir.join("choice.txt"), "feature resolution\n").unwrap();
    run_git(dir, &["add", "choice.txt"]);
    run_git(dir, &["commit", "-m", "feature merge resolution"]);

    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--no-ff", &left, "-m", "primary left merge"]);
    let merge = Command::new("git")
        .args(["merge", "--no-ff", &right, "-m", "primary right merge"])
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(!merge.status.success(), "primary fixture merge must conflict");
    std::fs::write(dir.join("choice.txt"), "primary resolution\n").unwrap();
    run_git(dir, &["add", "choice.txt"]);
    run_git(dir, &["commit", "-m", "primary merge resolution"]);

    let primary = merge_detection::resolve_primary_ref_for_code_check(&repo, "main", None).unwrap();
    let error = merge_detection::check_ref_code_in_primary(
        &repo,
        &primary,
        "refs/heads/feature/left",
        None,
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("found 2"));
}
