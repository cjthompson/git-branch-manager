use std::sync::atomic::AtomicBool;
use std::sync::{mpsc, Arc};

use git_branch_manager::git::{
    capability, worktree,
};
use git_branch_manager::job_queue::{ActionJob, ActionJobQueue};
use git_branch_manager::types::{
    BranchAction,
    TrackingStatus,
};
use git_branch_manager::view::ViewId;

use crate::common::*;
mod more;

/// Reimplements `App::worktree_presence_for` (src/app.rs:3046-3056) against
/// the `pub` `WorktreeInfo`/`WorktreePresence` types, since `App` itself is
/// unreachable from this test binary (`app` is `mod app;` in main.rs, not
/// re-exported by lib.rs). Kept in exact lockstep with the production
/// lookup: first match by `branch == Some(name)`.
pub(crate) fn worktree_presence_for<'a>(
    worktrees: &'a [git_branch_manager::types::WorktreeInfo],
    branch_name: &str,
) -> Option<capability::WorktreePresence<'a>> {
    worktrees
        .iter()
        .find(|wt| wt.branch.as_deref() == Some(branch_name))
        .map(|wt| capability::WorktreePresence {
            path: &wt.path,
            is_main: wt.is_main,
            is_clean: wt.wt_status.is_clean(),
        })
}

/// Mirrors `App::worktree_presence_for_branch_or_base` (src/app.rs:3058-3071):
/// tries the branch's own worktree first, only falling back to the base
/// branch's worktree if the branch itself is checked out nowhere.
pub(crate) fn worktree_presence_for_branch_or_base<'a>(
    worktrees: &'a [git_branch_manager::types::WorktreeInfo],
    branch_name: &str,
    base_branch: &str,
) -> Option<capability::WorktreePresence<'a>> {
    worktree_presence_for(worktrees, branch_name)
        .or_else(|| worktree_presence_for(worktrees, base_branch))
}

/// Real worktree state for `dir`: `list_worktrees` + synchronous
/// `enrich_worktrees` drain.
pub(crate) fn worktrees_with_status(dir: &std::path::Path) -> Vec<git_branch_manager::types::WorktreeInfo> {
    let mut worktrees = worktree::list_worktrees(dir);
    let rx = worktree::enrich_worktrees(worktrees.clone());
    for result in rx.iter() {
        if let Some(wt) = worktrees.iter_mut().find(|wt| wt.path == result.path) {
            wt.wt_status = result.wt_status;
            wt.age_date = result.age_date;
        }
    }
    worktrees
}

/// Repo-wide remote check (src/app.rs:297,3659).
pub(crate) fn has_configured_remote(repo: &git2::Repository) -> bool {
    repo.remotes().map(|r| !r.is_empty()).unwrap_or(false)
}

/// Inject an ActionJob as "running" via `ActionJobQueue::inject_running_for_test`.
/// No background thread spawned, no real git runs.
pub(crate) fn inject_job_for_test(
    queue: &mut ActionJobQueue,
    action: BranchAction,
    targets: Vec<String>,
    dispatch_path: Option<std::path::PathBuf>,
) {
    let job = ActionJob {
        action,
        targets,
        remote: None,
        return_view: ViewId::Branches,
        dispatch_path,
    };
    let (_op_tx, op_rx) = mpsc::channel();
    let (_prog_tx, prog_rx) = mpsc::channel();
    let cancel_flag = Arc::new(AtomicBool::new(false));
    queue.inject_running_for_test(
        job,
        op_rx,
        prog_rx,
        cancel_flag,
        Arc::new(AtomicBool::new(false)),
    );
}

/// Canonicalize a path for comparison. On macOS, `tempfile::tempdir()` returns
/// `/var/folders/...` while `git2::Repository::work_path()` returns
/// `/private/var/folders/...` (the real path behind the `/var` symlink);
/// `std::fs::canonicalize` resolves both to the same canonical form so path
/// equality assertions work portably.
pub(crate) fn canonical(path: &std::path::Path) -> std::path::PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[test]
fn graph_enter_current_clean_branch_enables_rebase_and_push_disables_checkout_and_delete() {
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(
        dir,
        &[
            "remote",
            "add",
            "origin",
            "https://example.invalid/repo.git",
        ],
    );
    run_git(dir, &["checkout", "-b", "feature-current"]);
    std::fs::write(dir.join("current.txt"), "current\n").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "feature-current commit"]);

    let branches = test_branches(&tmpdir, &repo, "main").expect("list_branches failed");
    let current = branches
        .iter()
        .find(|b| b.name == "feature-current")
        .expect("feature-current should be listed");
    assert!(
        current.is_current,
        "feature-current should be the checked-out HEAD of the primary worktree"
    );
    assert!(!current.is_base);
    assert!(matches!(current.tracking, TrackingStatus::Local));

    let worktrees = worktrees_with_status(dir);
    assert!(
        worktree_presence_for(&worktrees, &current.name).is_some(),
        "the current branch is checked out in the primary worktree, which list_worktrees always includes"
    );

    // Checkout / Delete local / Force-delete local / Create worktree: every
    // evaluator checks `is_current` before `worktree.is_some()`
    // (src/git/capability.rs:50-117), so the reason is "current", not
    // "checked out in worktree", even though a WorktreePresence IS resolved
    // here (the primary worktree itself).
    let checkout =
        capability::can_checkout_local(current, worktree_presence_for(&worktrees, &current.name));
    assert!(!checkout.available);
    assert_eq!(checkout.reason, Some("current"));

    let delete_local =
        capability::can_delete_local(current, worktree_presence_for(&worktrees, &current.name));
    assert!(!delete_local.available);
    assert_eq!(delete_local.reason, Some("current"));

    let force_delete = capability::can_force_delete_local(
        current,
        worktree_presence_for(&worktrees, &current.name),
    );
    assert!(!force_delete.available);
    assert_eq!(force_delete.reason, Some("current"));

    let create_wt =
        capability::can_create_worktree(current, worktree_presence_for(&worktrees, &current.name));
    assert!(!create_wt.available);
    assert_eq!(create_wt.reason, Some("current"));

    // Rebase: allowed for the current branch regardless of cleanliness.
    let rebase = capability::can_rebase(
        current,
        worktree_presence_for(&worktrees, &current.name),
        true,
    );
    assert!(
        rebase.available,
        "rebase should be enabled on a clean current branch: {:?}",
        rebase.reason
    );

    // Push: has_configured_remote is repo-wide, not per-branch.
    let push = capability::can_push(current, has_configured_remote(&repo), true);
    assert!(
        push.available,
        "push should be enabled for a clean current branch with no upstream: {:?}",
        push.reason
    );

    // Merge into base: branch-first fallback resolves to current's own worktree.
    let merge = capability::can_merge_into_base(
        current,
        worktree_presence_for_branch_or_base(&worktrees, &current.name, "main"),
    );
    assert!(
        merge.available,
        "merge should be enabled when the resolved worktree is clean: {:?}",
        merge.reason
    );

    // Dispatch-path check for the enabled Rebase.
    let dispatch_path =
        worktree_presence_for(&worktrees, &current.name).map(|wt| wt.path.to_path_buf());
    assert_eq!(
        dispatch_path.as_deref().map(canonical).as_deref(),
        Some(canonical(dir).as_path())
    );

    let mut queue = ActionJobQueue::new(dir.to_path_buf(), "main".to_string());
    inject_job_for_test(
        &mut queue,
        BranchAction::Rebase,
        vec![current.name.clone()],
        dispatch_path,
    );
    assert!(queue.is_running_for_test());
    assert_eq!(queue.current_action(), Some(BranchAction::Rebase));
    assert_eq!(
        queue.current_targets_for_test().map(|t| t.to_vec()),
        Some(vec![current.name.clone()])
    );
}

#[test]
fn graph_enter_p008_style_linked_worktree_exposes_safe_operations_only() {
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();
    let linked_path = dir.join(".worktrees/feature-linked");
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
    run_git(dir, &["branch", "feature-linked"]);
    run_git(
        dir,
        &[
            "worktree",
            "add",
            linked_path_text.as_str(),
            "feature-linked",
        ],
    );
    // A real commit reachable only from the linked worktree's branch --
    // the temp-repo equivalent of "commit 11d7ca1 in the P008 worktree".
    std::fs::write(linked_path.join("linked.txt"), "linked\n").unwrap();
    run_git(&linked_path, &["add", "."]);
    run_git(
        &linked_path,
        &[
            "commit",
            "-m",
            "docs: update changelog for unavailable actions",
        ],
    );

    let branches = test_branches(&tmpdir, &repo, "main").expect("list_branches failed");
    let feature = branches
        .iter()
        .find(|b| b.name == "feature-linked")
        .expect("feature-linked should be listed");
    assert!(
        !feature.is_current,
        "feature-linked is checked out in a linked worktree, not the primary"
    );
    assert!(!feature.is_base);

    let worktrees = worktrees_with_status(dir);
    let presence = worktree_presence_for(&worktrees, &feature.name)
        .expect("feature-linked should resolve to the linked worktree");
    assert!(
        !presence.is_main,
        "the linked worktree is not the primary/main one"
    );

    for (label, cap) in [
        (
            "checkout",
            capability::can_checkout_local(
                feature,
                worktree_presence_for(&worktrees, &feature.name),
            ),
        ),
        (
            "delete_local",
            capability::can_delete_local(feature, worktree_presence_for(&worktrees, &feature.name)),
        ),
        (
            "force_delete_local",
            capability::can_force_delete_local(
                feature,
                worktree_presence_for(&worktrees, &feature.name),
            ),
        ),
        (
            "create_worktree",
            capability::can_create_worktree(
                feature,
                worktree_presence_for(&worktrees, &feature.name),
            ),
        ),
    ] {
        assert!(
            !cap.available,
            "{label} should be disabled for a branch checked out in a linked worktree"
        );
        assert_eq!(
            cap.reason,
            Some("checked out in worktree"),
            "{label} reason mismatch"
        );
    }

    // Rebase: blocked too.
    let rebase = capability::can_rebase(
        feature,
        worktree_presence_for(&worktrees, &feature.name),
        true,
    );
    assert!(!rebase.available);
    assert_eq!(rebase.reason, Some("checked out in other worktree"));

    // Push never consults worktree state.
    let push = capability::can_push(feature, has_configured_remote(&repo), true);
    assert!(
        push.available,
        "push should remain enabled for a branch checked out in a linked worktree: {:?}",
        push.reason
    );

    // Dispatch-path check for the enabled Push: resolves to None.
    let mut queue = ActionJobQueue::new(dir.to_path_buf(), "main".to_string());
    inject_job_for_test(
        &mut queue,
        BranchAction::Push,
        vec![feature.name.clone()],
        None,
    );
    assert!(queue.is_running_for_test());
    assert_eq!(queue.current_action(), Some(BranchAction::Push));
    assert_eq!(
        queue.current_targets_for_test().map(|t| t.to_vec()),
        Some(vec![feature.name.clone()])
    );
}
