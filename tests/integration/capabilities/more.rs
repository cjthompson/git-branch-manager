
use git_branch_manager::git::capability;
use git_branch_manager::job_queue::ActionJobQueue;
use git_branch_manager::types::{
    BranchAction,
    TrackingStatus,
};

use crate::common::*;
use crate::capabilities::{canonical, has_configured_remote, inject_job_for_test, worktree_presence_for, worktree_presence_for_branch_or_base, worktrees_with_status};

#[test]
fn graph_enter_no_upstream_branch_enables_push_and_rebase_but_not_pull() {
    let (_tmpdir, work_dir, repo) = setup_remote_test_repo();
    let dir = work_dir.as_path();

    run_git(dir, &["checkout", "-b", "local-only"]);
    std::fs::write(dir.join("local.txt"), "local\n").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "local-only commit"]);
    // Never pushed: local-only has no upstream.

    let branches = test_branches(&_tmpdir, &repo, "main").expect("list_branches failed");
    let local_only = branches
        .iter()
        .find(|b| b.name == "local-only")
        .expect("local-only should be listed");
    assert!(
        matches!(local_only.tracking, TrackingStatus::Local),
        "local-only must have no upstream"
    );
    assert!(local_only.is_current);

    let worktrees = worktrees_with_status(dir);

    // Push: has_configured_remote is repo-wide (origin exists from setup_remote_test_repo).
    let push = capability::can_push(local_only, has_configured_remote(&repo), true);
    assert!(
        push.available,
        "push should be enabled for a local-only branch when any remote is configured: {:?}",
        push.reason
    );

    // Rebase: allowed, current + no worktree exclusion applies.
    let rebase = capability::can_rebase(
        local_only,
        worktree_presence_for(&worktrees, &local_only.name),
        true,
    );
    assert!(
        rebase.available,
        "rebase should be enabled for the clean, no-upstream current branch: {:?}",
        rebase.reason
    );

    // Pull: gated on the per-branch remote-tracking flag, not the repo-wide one.
    let per_branch_has_remote = matches!(
        &local_only.tracking,
        TrackingStatus::Tracked { gone: false, .. }
    );
    assert!(!per_branch_has_remote);
    let pull = capability::can_pull(
        local_only,
        worktree_presence_for(&worktrees, &local_only.name),
        per_branch_has_remote,
    );
    assert!(!pull.available);
    assert_eq!(pull.reason, Some("no remote"));

    // Dispatch-path check for the enabled Rebase.
    let dispatch_path =
        worktree_presence_for(&worktrees, &local_only.name).map(|wt| wt.path.to_path_buf());
    assert_eq!(
        dispatch_path.as_deref().map(canonical).as_deref(),
        Some(canonical(dir).as_path())
    );

    let mut queue = ActionJobQueue::new(dir.to_path_buf(), "main".to_string());
    inject_job_for_test(
        &mut queue,
        BranchAction::Rebase,
        vec![local_only.name.clone()],
        dispatch_path,
    );
    assert!(queue.is_running_for_test());
    assert_eq!(queue.current_action(), Some(BranchAction::Rebase));
}

#[test]
fn graph_enter_dirty_linked_worktree_disables_checkout_delete_and_merge_but_not_push() {
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();
    let linked_path = dir.join(".worktrees/feature-dirty");
    let linked_path_text = linked_path.to_string_lossy().into_owned();

    run_git(
        dir,
        &[
            "remote",
            "add",
            "origin",
            "https://example.invalid/repo.git",
        ],
    );
    run_git(dir, &["branch", "feature-dirty"]);
    run_git(
        dir,
        &[
            "worktree",
            "add",
            linked_path_text.as_str(),
            "feature-dirty",
        ],
    );
    std::fs::write(linked_path.join("dirty.txt"), "dirty\n").unwrap();
    // Untracked, uncommitted: the linked worktree is dirty.

    let branches = test_branches(&tmpdir, &repo, "main").expect("list_branches failed");
    let feature = branches
        .iter()
        .find(|b| b.name == "feature-dirty")
        .expect("feature-dirty should be listed");
    assert!(!feature.is_current);

    let worktrees = worktrees_with_status(dir);
    let presence = worktree_presence_for(&worktrees, &feature.name)
        .expect("feature-dirty has a linked worktree");
    assert!(
        !presence.is_clean,
        "the linked worktree has an untracked file"
    );

    let delete_local =
        capability::can_delete_local(feature, worktree_presence_for(&worktrees, &feature.name));
    assert!(!delete_local.available);
    assert_eq!(delete_local.reason, Some("checked out in worktree"));

    let checkout =
        capability::can_checkout_local(feature, worktree_presence_for(&worktrees, &feature.name));
    assert!(!checkout.available);
    assert_eq!(checkout.reason, Some("checked out in worktree"));

    // Rebase on the branch itself: blocked because different worktree.
    let rebase = capability::can_rebase(
        feature,
        worktree_presence_for(&worktrees, &feature.name),
        false,
    );
    assert!(!rebase.available);
    assert_eq!(rebase.reason, Some("checked out in other worktree"));

    // Push ignores worktree cleanliness.
    let push = capability::can_push(feature, has_configured_remote(&repo), true);
    assert!(
        push.available,
        "push ignores worktree cleanliness entirely: {:?}",
        push.reason
    );

    // Merge into base: branch-first fallback resolves to feature-dirty's OWN worktree.
    let merge = capability::can_merge_into_base(
        feature,
        worktree_presence_for_branch_or_base(&worktrees, &feature.name, "main"),
    );
    assert!(
        !merge.available,
        "merge should be disabled when the resolved worktree is dirty"
    );
    assert_eq!(merge.reason, Some("base dirty"));

    // Dispatch-path check for the enabled Push.
    let mut queue = ActionJobQueue::new(dir.to_path_buf(), "main".to_string());
    inject_job_for_test(
        &mut queue,
        BranchAction::Push,
        vec![feature.name.clone()],
        None,
    );
    assert!(queue.is_running_for_test());
    assert_eq!(
        queue.current_targets_for_test().map(|t| t.to_vec()),
        Some(vec![feature.name.clone()])
    );
}

#[test]
fn graph_enter_base_branch_in_linked_worktree_gates_merge_but_not_squash_on_base_cleanliness() {
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();
    let base_linked_path = dir.join(".worktrees/main-linked");
    let base_linked_path_text = base_linked_path.to_string_lossy().into_owned();

    // Vacate "main" from the primary worktree so it can be checked out in a linked worktree.
    run_git(dir, &["branch", "feature-source"]);
    run_git(dir, &["checkout", "-b", "scratch"]);
    run_git(
        dir,
        &["worktree", "add", base_linked_path_text.as_str(), "main"],
    );

    let branches = test_branches(&tmpdir, &repo, "main").expect("list_branches failed");
    let feature = branches
        .iter()
        .find(|b| b.name == "feature-source")
        .expect("feature-source should be listed");
    assert!(!feature.is_current);
    assert!(!feature.is_base);

    // Clean base worktree.
    let worktrees_clean = worktrees_with_status(dir);
    assert!(
        worktree_presence_for(&worktrees_clean, &feature.name).is_none(),
        "feature-source is not checked out anywhere in this scenario"
    );
    let base_presence_clean = worktree_presence_for(&worktrees_clean, "main")
        .expect("main should resolve to the linked worktree");
    assert!(!base_presence_clean.is_main);
    assert!(base_presence_clean.is_clean);

    let merge_clean = capability::can_merge_into_base(
        feature,
        worktree_presence_for_branch_or_base(&worktrees_clean, &feature.name, "main"),
    );
    assert!(
        merge_clean.available,
        "merge into a clean base worktree should be enabled: {:?}",
        merge_clean.reason
    );
    let squash_clean = capability::can_squash_into_base(feature);
    assert!(
        squash_clean.available,
        "squash into a clean base worktree should be enabled: {:?}",
        squash_clean.reason
    );

    // Dirty base worktree.
    std::fs::write(base_linked_path.join("dirty-base.txt"), "dirty base\n").unwrap();
    let worktrees_dirty = worktrees_with_status(dir);
    let base_presence_dirty = worktree_presence_for(&worktrees_dirty, "main")
        .expect("main should still resolve to the linked worktree");
    assert!(
        !base_presence_dirty.is_clean,
        "the linked base worktree now has an untracked file"
    );

    let merge_dirty = capability::can_merge_into_base(
        feature,
        worktree_presence_for_branch_or_base(&worktrees_dirty, &feature.name, "main"),
    );
    assert!(!merge_dirty.available);
    assert_eq!(merge_dirty.reason, Some("base dirty"));
    let squash_dirty = capability::can_squash_into_base(feature);
    assert!(
        squash_dirty.available,
        "squash stays available with a dirty base (confirm offers a stash): {:?}",
        squash_dirty.reason
    );

    // Dispatch-path check for the enabled Merge (clean-base case).
    let dispatch_path =
        worktree_presence_for_branch_or_base(&worktrees_clean, &feature.name, "main")
            .map(|wt| wt.path.to_path_buf());
    assert_eq!(
        dispatch_path.as_deref().map(canonical).as_deref(),
        Some(canonical(&base_linked_path).as_path()),
        "merge should dispatch into main's own linked worktree"
    );
    assert_ne!(
        dispatch_path.as_deref().map(canonical).as_deref(),
        Some(canonical(dir).as_path()),
        "must not run in the primary worktree instead"
    );

    let mut queue = ActionJobQueue::new(dir.to_path_buf(), "main".to_string());
    inject_job_for_test(
        &mut queue,
        BranchAction::Merge,
        vec![feature.name.clone()],
        dispatch_path,
    );
    assert!(queue.is_running_for_test());
    assert_eq!(queue.current_action(), Some(BranchAction::Merge));
    assert_eq!(
        queue.current_targets_for_test().map(|t| t.to_vec()),
        Some(vec![feature.name.clone()])
    );
}
