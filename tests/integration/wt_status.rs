
use git_branch_manager::git::{
    status, worktree,
};
use git_branch_manager::types::ChangedFileKind;

use crate::common::*;

#[test]
fn test_wt_status_clean() {
    let (tmpdir, _repo) = setup_test_repo();
    let s = status::detect_working_tree_status(tmpdir.path());
    assert!(s.is_clean(), "fresh repo should be clean");
}

#[test]
fn test_wt_status_staged_only() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Add a new file to the index without committing
    std::fs::write(dir.join("new.txt"), "content\n").unwrap();
    run_git(dir, &["add", "new.txt"]);

    let s = status::detect_working_tree_status(dir);
    assert!(s.has_staged, "should detect staged file");
    assert!(!s.has_modified, "should not detect modified changes");
    assert!(!s.has_untracked, "should not detect untracked files");
    assert_eq!(
        s.changed_files.len(),
        1,
        "one staged file should be itemized"
    );
    assert_eq!(s.changed_files[0].path, "new.txt");
    assert_eq!(s.changed_files[0].kind, ChangedFileKind::Staged);
}

#[test]
fn test_wt_status_unstaged_only() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Modify a tracked file without staging
    std::fs::write(dir.join("README.md"), "# Modified\n").unwrap();

    let s = status::detect_working_tree_status(dir);
    assert!(!s.has_staged, "should not detect staged changes");
    assert!(s.has_modified, "should detect modified file");
    assert!(!s.has_untracked, "should not detect untracked files");
    assert_eq!(s.changed_files.len(), 1);
    assert_eq!(s.changed_files[0].path, "README.md");
    assert_eq!(s.changed_files[0].kind, ChangedFileKind::Modified);
}

#[test]
fn test_wt_status_untracked_only() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Create a file that is not tracked by git
    std::fs::write(dir.join("untracked.txt"), "not tracked\n").unwrap();

    let s = status::detect_working_tree_status(dir);
    assert!(!s.has_staged, "should not detect staged changes");
    assert!(!s.has_modified, "should not detect modified changes");
    assert!(s.has_untracked, "should detect untracked file");
    assert_eq!(s.changed_files.len(), 1);
    assert_eq!(s.changed_files[0].path, "untracked.txt");
    assert_eq!(s.changed_files[0].kind, ChangedFileKind::Untracked);
}

#[test]
fn test_wt_status_all_three() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Staged: add a new file to index
    std::fs::write(dir.join("staged.txt"), "staged\n").unwrap();
    run_git(dir, &["add", "staged.txt"]);

    // Unstaged: modify a tracked file without staging
    std::fs::write(dir.join("README.md"), "# Modified\n").unwrap();

    // Untracked: a new file not added to index
    std::fs::write(dir.join("untracked.txt"), "not tracked\n").unwrap();

    let s = status::detect_working_tree_status(dir);
    assert!(s.has_staged, "should detect staged file");
    assert!(s.has_modified, "should detect modified file");
    assert!(s.has_untracked, "should detect untracked file");
    assert!(!s.is_clean());
    assert_eq!(s.changed_files.len(), 3, "one entry per changed file");
    assert!(s
        .changed_files
        .iter()
        .any(|f| f.path == "staged.txt" && f.kind == ChangedFileKind::Staged));
    assert!(s
        .changed_files
        .iter()
        .any(|f| f.path == "README.md" && f.kind == ChangedFileKind::Modified));
    assert!(s
        .changed_files
        .iter()
        .any(|f| f.path == "untracked.txt" && f.kind == ChangedFileKind::Untracked));
}

#[test]
fn test_wt_status_staged_then_further_edited() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Stage a modification to a tracked file, then modify it again without
    // staging — git2 reports this single path with both an INDEX_* and a
    // WT_* flag. It must surface as two separate ChangedFile entries: one
    // Staged (the staged version) and one Modified (the further unstaged
    // edit), not a merged/combined kind.
    std::fs::write(dir.join("README.md"), "# Staged edit\n").unwrap();
    run_git(dir, &["add", "README.md"]);
    std::fs::write(
        dir.join("README.md"),
        "# Staged edit, then further edited\n",
    )
    .unwrap();

    let s = status::detect_working_tree_status(dir);
    assert!(s.has_staged, "should detect staged change");
    assert!(s.has_modified, "should detect further unstaged edit");
    assert_eq!(
        s.changed_files.len(),
        2,
        "path with both staged and unstaged changes should appear as two entries"
    );
    assert!(s
        .changed_files
        .iter()
        .any(|f| f.path == "README.md" && f.kind == ChangedFileKind::Staged));
    assert!(s
        .changed_files
        .iter()
        .any(|f| f.path == "README.md" && f.kind == ChangedFileKind::Modified));
}

#[test]
fn test_worktree_status_reports_modified_not_staged() {
    // Regression: a tracked file that is modified but NOT staged must report as
    // `modified`, not `staged`. The old worktree status parser trimmed the
    // leading (empty) index column out of `git status --porcelain` and misread
    // the unstaged change as staged. Covers BOTH the main worktree and an
    // additional worktree, since the enrich path runs once per worktree.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    // An additional worktree on its own branch.
    run_git(dir, &["branch", "wt-mod"]);
    run_git(dir, &["worktree", "add", ".worktrees/wt-mod", "wt-mod"]);

    // Modify a tracked file (without staging) in BOTH the main dir and the
    // additional worktree.
    std::fs::write(dir.join("README.md"), "# Modified, not staged\n").unwrap();
    std::fs::write(
        dir.join(".worktrees").join("wt-mod").join("README.md"),
        "# Modified in additional worktree\n",
    )
    .unwrap();

    let worktrees = worktree::list_worktrees(dir);
    let rx = worktree::enrich_worktrees(worktrees.clone());
    let mut results: Vec<_> = rx.iter().collect();
    results.sort_by_key(|r| r.path.clone());

    // The crux of the regression: a modified-not-staged file must classify as
    // modified, never staged. (We don't assert on has_untracked here: the main
    // worktree legitimately sees the in-repo `.worktrees/` dir as untracked.)
    for wt in &worktrees {
        let status = &results
            .iter()
            .find(|result| result.path == wt.path)
            .unwrap()
            .wt_status;
        assert!(
            status.has_modified,
            "{:?}: modified-not-staged file should be reported as modified",
            wt.branch
        );
        assert!(
            !status.has_staged,
            "{:?}: nothing is staged, so has_staged must be false",
            wt.branch
        );
    }

    if std::env::var_os("GBM_KEEP_TEST_REPOS").is_none() {
        run_git(dir, &["worktree", "remove", "--force", ".worktrees/wt-mod"]);
    }
}
