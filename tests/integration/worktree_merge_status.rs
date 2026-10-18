
use git_branch_manager::git::worktree;
use git_branch_manager::types::MergeStatus;

use crate::common::*;

/// Live, end-to-end test: build one repo containing every merge scenario, give
/// each feature branch its own worktree, and assert that a worktree's Merge
/// status matches its branch's. Also covers clean vs dirty working trees. The
/// temp repo lives in the OS temp dir and is deleted on pass, fail, or panic
/// (TempDir's Drop runs during unwind).
#[test]
fn test_worktree_merge_status_from_branches() {
    let tmpdir = TestDir::new(tempfile::tempdir().expect("failed to create tempdir"));
    let dir = tmpdir.path();

    // --- base repo on main with an initial README.md commit ---
    run_git(dir, &["init", "-b", "main"]);
    run_git(dir, &["config", "user.name", "Test User"]);
    run_git(dir, &["config", "user.email", "test@example.com"]);
    std::fs::write(dir.join("README.md"), "# Test Repo\n").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "Initial commit"]);

    // --- regular-merged branch (non-ff so a merge commit is recorded) ---
    run_git(dir, &["checkout", "-b", "feat-merged"]);
    std::fs::write(dir.join("feature.txt"), "feature\n").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "feat-merged work"]);
    run_git(dir, &["checkout", "main"]);
    std::fs::write(dir.join("main-change.txt"), "main\n").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "main commit"]);
    run_git(dir, &["merge", "feat-merged", "-m", "Merge feat-merged"]);

    // --- squash-merged branch: 3 commits each editing README.md, then
    //     `git merge --squash` + commit on main; branch left intact ---
    run_git(dir, &["checkout", "-b", "feat-squashed"]);
    for i in 1..=3 {
        std::fs::write(
            dir.join("README.md"),
            format!("# Test Repo\nsquash line {i}\n"),
        )
        .unwrap();
        run_git(dir, &["add", "."]);
        run_git(dir, &["commit", "-m", &format!("squash work {i}")]);
    }
    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feat-squashed"]);
    run_git(dir, &["commit", "-m", "squash merge feat-squashed"]);

    // --- unmerged branch ---
    run_git(dir, &["checkout", "-b", "feat-unmerged"]);
    std::fs::write(dir.join("wip.txt"), "wip\n").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "wip"]);
    run_git(dir, &["checkout", "main"]);

    // --- one worktree per feature branch (main dir stays on main) ---
    run_git(
        dir,
        &["worktree", "add", ".worktrees/merged", "feat-merged"],
    );
    run_git(
        dir,
        &["worktree", "add", ".worktrees/squashed", "feat-squashed"],
    );
    run_git(
        dir,
        &["worktree", "add", ".worktrees/unmerged", "feat-unmerged"],
    );

    // dirty the unmerged worktree (untracked file); leave the others clean
    std::fs::write(dir.join(".worktrees/unmerged/dirty.txt"), "x\n").unwrap();

    // --- exercise the real library APIs in production order ---
    let repo = git2::Repository::open(dir).expect("failed to re-open repo");
    let branches = test_branches(&tmpdir, &repo, "main").expect("list_branches failed");

    // sanity: branch statuses themselves are correct
    let branch_status = |name: &str| {
        branches
            .iter()
            .find(|b| b.name == name)
            .unwrap_or_else(|| panic!("branch {name} not found"))
            .merge_status
    };
    assert_eq!(branch_status("feat-merged"), MergeStatus::Merged);
    // No remote in this repo, so a squash-merge resolves to the local-only
    // variant (matching test_squash_merged_branch_detection).
    assert_eq!(
        branch_status("feat-squashed"),
        MergeStatus::LocalSquashMerged
    );
    assert_eq!(branch_status("feat-unmerged"), MergeStatus::Unmerged);

    let mut worktrees = worktree::list_worktrees(dir);

    // THE API UNDER TEST: correlate worktree merge status from the branch list.
    worktree::apply_branch_merge_status(&mut worktrees, &branches);

    let wt_for = |name: &str| {
        worktrees
            .iter()
            .find(|w| w.branch.as_deref() == Some(name))
            .unwrap_or_else(|| panic!("no worktree for branch {name}"))
    };
    assert_eq!(
        wt_for("feat-merged").merge_status,
        MergeStatus::Merged,
        "merged worktree should match its branch"
    );
    assert_eq!(
        wt_for("feat-squashed").merge_status,
        MergeStatus::LocalSquashMerged,
        "squash-merged worktree should match its branch"
    );
    assert_eq!(
        wt_for("feat-unmerged").merge_status,
        MergeStatus::Unmerged,
        "unmerged worktree should match its branch"
    );

    // main worktree is on the base branch: it must be flagged is_base so the
    // renderer blanks its Merge cell (a branch can't be merged into itself).
    // The raw merge_status copied from the base BranchInfo is an implementation
    // detail; the contract that matters is is_base. (See the render-level test
    // `worktree_base_merge_cell_is_blank` in src/app.rs for the blanking.)
    let main_wt = worktrees.iter().find(|w| w.is_main).expect("main worktree");
    assert_eq!(main_wt.branch.as_deref(), Some("main"));
    assert!(
        main_wt.is_base,
        "main worktree is on the base branch and must be marked is_base"
    );
    // The feature worktrees are NOT the base.
    assert!(!wt_for("feat-merged").is_base);
    assert!(!wt_for("feat-unmerged").is_base);

    // --- clean vs dirty working tree via enrich_worktrees (sets wt_status) ---
    let unmerged_idx = worktrees
        .iter()
        .position(|w| w.branch.as_deref() == Some("feat-unmerged"))
        .unwrap();
    let merged_idx = worktrees
        .iter()
        .position(|w| w.branch.as_deref() == Some("feat-merged"))
        .unwrap();
    let rx = worktree::enrich_worktrees(worktrees.clone());
    let mut results: Vec<_> = rx.iter().collect();
    results.sort_by_key(|r| r.path.clone());
    // The dirty file is untracked specifically — assert the exact buckets, not
    // just !is_clean (which would pass under a staged/modified misclassification).
    let unmerged_status = &results
        .iter()
        .find(|r| r.path == worktrees[unmerged_idx].path)
        .unwrap()
        .wt_status;
    assert!(unmerged_status.has_untracked, "dirty file is untracked");
    assert!(!unmerged_status.has_staged, "nothing staged");
    assert!(!unmerged_status.has_modified, "no tracked modifications");
    assert!(
        results
            .iter()
            .find(|r| r.path == worktrees[merged_idx].path)
            .unwrap()
            .wt_status
            .is_clean(),
        "merged worktree is untouched -> clean"
    );

    // cleanup worktrees before the tempdir drops — skipped when keeping repos
    // for manual inspection, so the worktrees stay on disk too
    if std::env::var_os("GBM_KEEP_TEST_REPOS").is_none() {
        for p in [
            ".worktrees/merged",
            ".worktrees/squashed",
            ".worktrees/unmerged",
        ] {
            run_git(dir, &["worktree", "remove", "--force", p]);
        }
    }
}
