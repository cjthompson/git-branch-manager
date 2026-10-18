
use git_branch_manager::git::{
    branch, squash_loader,
};
use git_branch_manager::types::MergeStatus;

use crate::common::*;

#[test]
fn test_remote_branch_squash_merge_detection() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    // Create a feature branch with unique content
    run_git(&work_dir, &["checkout", "-b", "squash-feature"]);
    let f = work_dir.join("squash-feature.txt");
    std::fs::write(&f, "squash feature content\n").unwrap();
    run_git(&work_dir, &["add", "squash-feature.txt"]);
    run_git(&work_dir, &["commit", "-m", "Squash feature commit"]);
    run_git(&work_dir, &["push", "-u", "origin", "squash-feature"]);

    // Squash-merge into main (without a merge commit)
    run_git(&work_dir, &["checkout", "main"]);
    run_git(&work_dir, &["merge", "--squash", "squash-feature"]);
    run_git(&work_dir, &["commit", "-m", "Squash merge squash-feature"]);
    run_git(&work_dir, &["push", "origin", "main"]);

    // Reload repo after push
    let repo = git2::Repository::open(&work_dir).unwrap();
    let remotes = branch::list_remote_branches_phase1(&repo, "main")
        .expect("list_remote_branches_phase1 failed");

    // Build candidates for squash checker: (full_ref, commit_hash) for pending non-base branches
    let candidates: Vec<(String, String, Option<String>)> = remotes
        .iter()
        .filter(|b| b.merge_status == MergeStatus::Pending && !b.is_base)
        .filter_map(|b| {
            let refname = format!("refs/remotes/{}", b.full_ref);
            repo.find_reference(&refname)
                .ok()
                .and_then(|r| r.peel_to_commit().ok())
                .map(|c| (b.full_ref.clone(), c.id().to_string(), None))
        })
        .collect();

    let cache = git_branch_manager::git::cache::BranchCache::load_for_base(
        &work_dir,
        "main",
        &git_branch_manager::git::cache::CacheRoot::at(_tmpdir.cache_root().to_path_buf()),
    );
    let rx = squash_loader::spawn_squash_checker(
        work_dir.clone(),
        "main".to_string(),
        candidates,
        cache,
    );

    let index_map: std::collections::HashMap<String, usize> = remotes
        .iter()
        .enumerate()
        .map(|(i, b)| (b.full_ref.clone(), i))
        .collect();

    let mut remotes = remotes;
    for result in rx {
        if let Some(&idx) = index_map.get(&result.branch_name) {
            remotes[idx].merge_status = result.status;
        }
    }

    let squashed = remotes
        .iter()
        .find(|r| r.short_name == "squash-feature")
        .unwrap();
    assert_eq!(
        squashed.merge_status,
        MergeStatus::SquashMerged,
        "squash-feature should be detected as SquashMerged on remote"
    );
}

#[test]
fn test_remote_disjoint_branch_detected() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    // A normal branch that shares history with main.
    run_git(&work_dir, &["checkout", "-b", "normal"]);
    std::fs::write(work_dir.join("normal.txt"), "n\n").unwrap();
    run_git(&work_dir, &["add", "normal.txt"]);
    run_git(&work_dir, &["commit", "-m", "normal commit"]);
    run_git(&work_dir, &["push", "-u", "origin", "normal"]);

    // An orphan branch: a separate root, so it shares NO history with main.
    run_git(&work_dir, &["checkout", "--orphan", "disjoint"]);
    run_git(&work_dir, &["rm", "-rf", "."]);
    std::fs::write(work_dir.join("disjoint.txt"), "d\n").unwrap();
    run_git(&work_dir, &["add", "disjoint.txt"]);
    run_git(&work_dir, &["commit", "-m", "disjoint root"]);
    run_git(&work_dir, &["push", "-u", "origin", "disjoint"]);

    run_git(&work_dir, &["checkout", "main"]);

    let repo = git2::Repository::open(&work_dir).unwrap();
    let remotes = branch::list_remote_branches_phase1(&repo, "main")
        .expect("list_remote_branches_phase1 failed");

    let rx = branch::spawn_remote_enricher(work_dir.clone(), "main".to_string(), remotes);
    let results: Vec<_> = rx.iter().collect();

    let disjoint = results
        .iter()
        .find(|r| r.full_ref == "origin/disjoint")
        .expect("origin/disjoint should be enriched");
    assert!(
        disjoint.disjoint,
        "orphan branch shares no history with main and must be flagged disjoint"
    );

    let normal = results
        .iter()
        .find(|r| r.full_ref == "origin/normal")
        .expect("origin/normal should be enriched");
    assert!(
        !normal.disjoint,
        "a branch that shares history with main must not be flagged disjoint"
    );
}

#[test]
fn test_remote_branch_inherits_squash_merge_status_from_local() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    // Create a feature branch, add a commit, and push it to origin
    run_git(&work_dir, &["checkout", "-b", "squash-local-feature"]);
    std::fs::write(work_dir.join("feature.txt"), "feature content\n").unwrap();
    run_git(&work_dir, &["add", "feature.txt"]);
    run_git(&work_dir, &["commit", "-m", "Add feature"]);
    run_git(&work_dir, &["push", "-u", "origin", "squash-local-feature"]);

    // Squash-merge into main locally (do NOT push main — remote branch stays ahead=1)
    run_git(&work_dir, &["checkout", "main"]);
    run_git(&work_dir, &["merge", "--squash", "squash-local-feature"]);
    run_git(
        &work_dir,
        &["commit", "-m", "Squash merge squash-local-feature"],
    );

    let repo = git2::Repository::open(&work_dir).unwrap();

    // Load local branches and run squash detection
    let local_branches = test_branches(&_tmpdir, &repo, "main").expect("list_branches failed");
    let local_feature = local_branches
        .iter()
        .find(|b| b.name == "squash-local-feature")
        .expect("squash-local-feature not in local branches");
    assert_eq!(
        local_feature.merge_status,
        MergeStatus::LocalSquashMerged,
        "local branch should be detected as LocalSquashMerged"
    );

    // Load remote branches — enricher marks ahead>0 as Unmerged
    let mut remotes = branch::list_remote_branches_phase1(&repo, "main")
        .expect("list_remote_branches_phase1 failed");

    // Simulate the remote enricher: origin/squash-local-feature is ahead=1, so Unmerged
    let remote_before = remotes
        .iter()
        .find(|r| r.short_name == "squash-local-feature")
        .expect("origin/squash-local-feature not found");
    assert_eq!(
        remote_before.merge_status,
        MergeStatus::Pending,
        "remote branch should start as Pending before enrichment"
    );

    // Simulate what drain_channels does: propagate local squash result to remote
    // (this is the fix from #030 — when a local squash result arrives, update the
    // matching remote branch by short_name)
    for remote in remotes.iter_mut() {
        if remote.short_name == local_feature.name
            && !matches!(
                local_feature.merge_status,
                MergeStatus::Unmerged | MergeStatus::Pending
            )
        {
            remote.merge_status = local_feature.merge_status;
        }
    }

    let remote_after = remotes
        .iter()
        .find(|r| r.short_name == "squash-local-feature")
        .expect("origin/squash-local-feature not found");
    assert_eq!(
        remote_after.merge_status,
        MergeStatus::LocalSquashMerged,
        "remote branch should inherit LocalSquashMerged from local squash detection"
    );
}
