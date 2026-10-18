use std::sync::atomic::AtomicBool;

use git_branch_manager::git::{
    branch, cache, diagnostics, merge_detection,
};
use git_branch_manager::types::{
    DiagKind, MergeStatus,
};

use crate::common::*;

#[test]
fn test_cache_audit_detects_and_fixes_stale_status() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    // An unmerged feature branch with its own commit.
    run_git(dir, &["checkout", "-b", "feature/wip"]);
    std::fs::write(dir.join("wip.txt"), "work in progress\n").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "WIP commit"]);
    run_git(dir, &["checkout", "main"]);

    let repo = git2::Repository::open(dir).expect("re-open repo");
    let tip = repo
        .find_branch("feature/wip", git2::BranchType::Local)
        .unwrap()
        .get()
        .target()
        .unwrap()
        .to_string();

    // Poison the cache: claim the unmerged branch is permanently squash-merged,
    // and add an orphan row for a branch that does not exist.
    let cache_path = dir.join("audit-cache.sqlite3");
    {
        let mut c = cache::BranchCache::load_from_path(cache_path.clone());
        c.insert("feature/wip", &MergeStatus::SquashMerged, &tip);
        c.insert("feature/ghost", &MergeStatus::Merged, "deadbeef");
        c.save();
    }

    let cache = cache::BranchCache::load_from_path(cache_path.clone());
    let cancel = AtomicBool::new(false);
    let audit = diagnostics::audit_cache(&repo, dir, "main", &cache, &cancel, |_, _, _| {});

    // Exactly one merge-status discrepancy for the poisoned branch.
    let status_discrepancies: Vec<_> = audit
        .discrepancies
        .iter()
        .filter(|d| d.kind == DiagKind::MergeStatus)
        .collect();
    assert_eq!(
        status_discrepancies.len(),
        1,
        "expected one stale merge-status entry, got {:?}",
        audit.discrepancies
    );
    let d = status_discrepancies[0];
    assert_eq!(d.branch, "feature/wip");
    assert_eq!(d.cached, "squash-merged");
    assert_eq!(d.actual, "unmerged");
    assert_eq!(audit.merge_status.mismatched, 1);

    // The deleted branch surfaces as an orphan.
    assert!(
        audit.orphans.contains(&"feature/ghost".to_string()),
        "expected feature/ghost orphan, got {:?}",
        audit.orphans
    );

    // Apply the fix, then confirm the cache now reflects reality.
    let mut cache = cache::BranchCache::load_from_path(cache_path.clone());
    diagnostics::apply_fix(&mut cache, &audit);

    let fixed = cache::BranchCache::load_from_path(cache_path.clone());
    assert_eq!(
        fixed.lookup("feature/wip", &tip),
        Some(MergeStatus::Unmerged),
        "stale status should be corrected to unmerged"
    );
    assert_eq!(
        fixed.lookup("feature/ghost", "deadbeef"),
        None,
        "orphan entry should be removed"
    );

    // A re-audit now reports a clean cache.
    let clean = diagnostics::audit_cache(&repo, dir, "main", &fixed, &cancel, |_, _, _| {});
    assert!(
        clean.is_clean(),
        "cache should be clean after fix, got {clean:?}"
    );
}

#[test]
fn test_cache_audit_merge_base_not_false_positive() {
    // Regression: merge-base entries are stored as short hashes via a bounded
    // walk. The audit must recompute truth the same way, or every entry would
    // appear mismatched (cached short hash vs full unbounded merge_base).
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    // An unmerged feature branch...
    run_git(dir, &["checkout", "-b", "feature/x"]);
    std::fs::write(dir.join("x.txt"), "x\n").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "x commit"]);
    run_git(dir, &["checkout", "main"]);
    // ...and advance main so the merge base is a real ancestor, not the tip.
    std::fs::write(dir.join("m.txt"), "m\n").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "main commit"]);

    let repo = git2::Repository::open(dir).unwrap();
    let base_tip = repo
        .find_branch("main", git2::BranchType::Local)
        .unwrap()
        .get()
        .target()
        .unwrap();
    let reachable = merge_detection::build_reachable_set_from_repo(&repo, "main");

    // Fill the merge-base cache exactly as the app does.
    let cache_path = dir.join("mb-cache.sqlite3");
    {
        let mut branches = branch::list_branches_phase1(&repo, "main").unwrap();
        // phase1 already fills merge bases; clear them so the *cached* filler
        // (the path that actually populates the merge-base cache) runs.
        for b in &mut branches {
            b.merge_base_commit = None;
        }
        let mut cache = cache::BranchCache::load_from_path(cache_path.clone());
        branch::fill_merge_base_commits_cached(
            &repo,
            &mut branches,
            &reachable.local,
            base_tip,
            &mut cache,
        );
        cache.save();
    }

    let cache = cache::BranchCache::load_from_path(cache_path);
    let cancel = AtomicBool::new(false);
    let audit = diagnostics::audit_cache(&repo, dir, "main", &cache, &cancel, |_, _, _| {});

    assert!(
        audit.merge_base.verified >= 1,
        "expected at least one merge-base entry to be checked: {audit:?}"
    );
    assert_eq!(
        audit.merge_base.mismatched, 0,
        "merge-base entries must not false-positive: {:?}",
        audit.discrepancies
    );
}

#[test]
fn test_cache_audit_skipped_counts_for_uncached_branches() {
    // Regression test: verify_merge_status must count ALL branches, not just
    // those with a cache row. Base branch, current branch, and regular-merged
    // branches never get a cache row → were silently dropped before this fix.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Create a feature branch that will be regular-merged into main.
    run_git(dir, &["checkout", "-b", "feature/merged-normal"]);
    std::fs::write(dir.join("feat.txt"), "feature\n").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "feature commit"]);
    run_git(dir, &["checkout", "main"]);
    run_git(
        dir,
        &[
            "merge",
            "--no-ff",
            "feature/merged-normal",
            "-m",
            "Merge feature",
        ],
    );

    // Create an unmerged branch with a cache row (squash-checked as unmerged).
    run_git(dir, &["checkout", "-b", "feature/unmerged"]);
    std::fs::write(dir.join("unmerged.txt"), "wip\n").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "unmerged commit"]);
    run_git(dir, &["checkout", "main"]);

    let repo = git2::Repository::open(dir).expect("re-open repo");
    let unmerged_tip = repo
        .find_branch("feature/unmerged", git2::BranchType::Local)
        .unwrap()
        .get()
        .target()
        .unwrap()
        .to_string();

    // Only cache the unmerged branch — base and merged-normal have no cache row.
    let cache_path = dir.join("skipped-test.sqlite3");
    {
        let mut c = cache::BranchCache::load_from_path(cache_path.clone());
        c.insert("feature/unmerged", &MergeStatus::Unmerged, &unmerged_tip);
        c.save();
    }

    let cache = cache::BranchCache::load_from_path(cache_path);
    let cancel = AtomicBool::new(false);
    let audit = diagnostics::audit_cache(&repo, dir, "main", &cache, &cancel, |_, _, _| {});

    // The cached unmerged branch must be verified (truth = Unmerged, cache = Unmerged).
    assert_eq!(
        audit.merge_status.verified, 1,
        "the cached unmerged branch should be verified: {audit:?}"
    );

    // No discrepancies — the cached entry is correct.
    assert_eq!(
        audit.discrepancies.len(),
        0,
        "no discrepancies expected: {:?}",
        audit.discrepancies
    );

    // Two branches have no cache row vs cached: main (base+current) and
    // feature/merged-normal (regular-merged, never cached) are skipped.
    assert_eq!(
        audit.merge_status.skipped, 2,
        "expected 2 skipped (main=base, merged-normal=no cached status): {audit:?}"
    );

    assert!(
        audit.merge_status.skip_reasons.contains(&"base branch"),
        "expected 'base branch' reason: {:?}",
        audit.merge_status.skip_reasons
    );
    assert!(
        audit
            .merge_status
            .skip_reasons
            .contains(&"no cached status"),
        "expected 'no cached status' reason: {:?}",
        audit.merge_status.skip_reasons
    );
}

#[test]
fn test_cache_audit_remote_squash_entry_not_orphaned() {
    // Regression: remote-branch squash-merge results are cached under keys
    // like "origin/some-branch" (the same short form squash_loader uses for
    // remote candidates), but the orphan sweep used to compare only against
    // local branch names — misclassifying every remote-squash cache row as an
    // orphan. audit_cache must also treat live remote branch names as "live".
    let tmpdir = tempfile::tempdir().expect("failed to create tmpdir");
    let base_dir = tmpdir.path();

    let remote_dir = base_dir.join("remote.git");
    std::fs::create_dir_all(&remote_dir).unwrap();
    run_git(&remote_dir, &["init", "--bare", "-b", "main"]);

    let work_dir = base_dir.join("work");
    run_git(base_dir, &["clone", remote_dir.to_str().unwrap(), "work"]);
    run_git(&work_dir, &["config", "user.name", "Test User"]);
    run_git(&work_dir, &["config", "user.email", "test@example.com"]);
    std::fs::write(work_dir.join("README.md"), "# Test\n").unwrap();
    run_git(&work_dir, &["add", "."]);
    run_git(&work_dir, &["commit", "-m", "Initial commit"]);
    run_git(&work_dir, &["push", "-u", "origin", "main"]);

    // A remote-only branch (no local counterpart) so its cache key exercises
    // exactly the collision this test guards against.
    run_git(&work_dir, &["checkout", "-b", "remote-only"]);
    std::fs::write(work_dir.join("r.txt"), "r\n").unwrap();
    run_git(&work_dir, &["add", "."]);
    run_git(&work_dir, &["commit", "-m", "remote-only commit"]);
    run_git(&work_dir, &["push", "-u", "origin", "remote-only"]);
    run_git(&work_dir, &["checkout", "main"]);
    run_git(&work_dir, &["branch", "-D", "remote-only"]);

    let repo = git2::Repository::open(&work_dir).expect("re-open repo");
    let cache_path = work_dir.join("remote-orphan-test.sqlite3");
    {
        let mut c = cache::BranchCache::load_from_path(cache_path.clone());
        c.insert(
            "origin/remote-only",
            &MergeStatus::RemoteSquashMerged,
            "deadbeef",
        );
        c.save();
    }

    let cache = cache::BranchCache::load_from_path(cache_path);
    let cancel = AtomicBool::new(false);
    let audit = diagnostics::audit_cache(&repo, &work_dir, "main", &cache, &cancel, |_, _, _| {});

    assert!(
        !audit.orphans.contains(&"origin/remote-only".to_string()),
        "remote-squash cache row must not be misclassified as orphan: {:?}",
        audit.orphans
    );
}

#[test]
fn test_spawn_cache_verifier_applies_fix_and_persists() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/wip"]);
    std::fs::write(dir.join("wip.txt"), "work in progress\n").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "WIP commit"]);
    run_git(dir, &["checkout", "main"]);

    let repo = git2::Repository::open(dir).expect("re-open repo");
    let tip = repo
        .find_branch("feature/wip", git2::BranchType::Local)
        .unwrap()
        .get()
        .target()
        .unwrap()
        .to_string();

    let cache_root = cache::CacheRoot::at(tmpdir.cache_root().to_path_buf());
    // Seed the same TestDir-owned cache root the verifier receives below.
    {
        let mut c = cache::BranchCache::load_for_base(dir, "main", &cache_root);
        c.insert("feature/wip", &MergeStatus::SquashMerged, &tip);
        c.insert("feature/ghost", &MergeStatus::Merged, "deadbeef");
        c.save();
    }

    let rx = diagnostics::spawn_cache_verifier(
        dir.to_path_buf(),
        "main".to_string(),
        cache_root.clone(),
    );
    let audit = rx.recv().expect("verifier should send a result");

    assert!(
        audit
            .discrepancies
            .iter()
            .any(|d| d.branch == "feature/wip"),
        "expected the stale status to be reported: {:?}",
        audit.discrepancies
    );
    assert!(
        audit.orphans.contains(&"feature/ghost".to_string()),
        "expected feature/ghost orphan: {:?}",
        audit.orphans
    );

    // The verifier applies + persists the fix itself, with no separate
    // apply_fix call from the caller.
    let fixed = cache::BranchCache::load_for_base(dir, "main", &cache_root);
    assert_eq!(
        fixed.lookup("feature/wip", &tip),
        Some(MergeStatus::Unmerged),
        "stale status should already be corrected on disk"
    );
    assert_eq!(
        fixed.lookup("feature/ghost", "deadbeef"),
        None,
        "orphan entry should already be removed on disk"
    );
}
