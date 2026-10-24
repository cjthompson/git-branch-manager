use git_branch_manager::git::operations;

use crate::common::*;

#[test]
fn test_merge_branch_success() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Create a feature branch with a unique file
    run_git(dir, &["checkout", "-b", "feature-to-merge"]);
    std::fs::write(dir.join("feature.txt"), "feature content\n").unwrap();
    run_git(dir, &["add", "feature.txt"]);
    run_git(dir, &["commit", "-m", "Add feature"]);
    run_git(dir, &["checkout", "main"]);

    let results = operations::merge_branch(dir, "feature-to-merge", "main", false, false);
    assert_eq!(results.len(), 1);
    assert!(
        results[0].success,
        "merge should succeed: {}",
        results[0].message
    );

    // Verify the feature file is present on main
    assert!(
        dir.join("feature.txt").exists(),
        "feature.txt should be on main after merge"
    );

    // The operation's contract is to leave HEAD on the base branch.
    let repo = git2::Repository::open(dir).expect("re-open repo");
    assert_eq!(
        repo.head().unwrap().shorthand().ok(),
        Some("main"),
        "HEAD should remain on the base branch after merge"
    );
}

#[test]
fn test_merge_branch_squash_success() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature-squash"]);
    std::fs::write(dir.join("squash.txt"), "squash content\n").unwrap();
    run_git(dir, &["add", "squash.txt"]);
    run_git(dir, &["commit", "-m", "Squash candidate"]);
    run_git(dir, &["checkout", "main"]);

    let results = operations::merge_branch(dir, "feature-squash", "main", true, false);
    assert_eq!(results.len(), 1);
    assert!(
        results[0].success,
        "squash merge should succeed: {}",
        results[0].message
    );

    // The squash content should exist on main
    assert!(
        dir.join("squash.txt").exists(),
        "squash.txt should be on main after squash merge"
    );

    // And main should have a single new commit (NOT a merge commit): the squash
    // commit must have exactly one parent.
    let repo = git2::Repository::open(dir).expect("re-open repo");
    let head_commit = repo.head().unwrap().peel_to_commit().unwrap();
    assert_eq!(head_commit.message().unwrap(), "Squash candidate");
    assert_eq!(
        head_commit.parent_count(),
        1,
        "squash merge must be a single-parent commit, not a merge commit"
    );
}

#[test]
fn test_squash_commit_uses_full_source_messages_in_ancestry_order() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();
    run_git(dir, &["checkout", "-b", "feature-messages"]);

    write_and_commit_message(
        dir,
        "first.txt",
        "Oldest subject\n\n  indented body\n# literal comment\nCo-Authored-By:\n cO-aUtHoReD-bY: First <first@example.com>\nCo-Authored-By: Duplicate <duplicate@example.com>\n",
        "2030-01-01T00:00:00+0000",
    );
    write_and_commit_message(
        dir,
        "second.txt",
        "Newest subject\n\nbody with $HOME and `ticks`\nCo-Authored-By: Second <second@example.com>\n",
        "2001-01-01T00:00:00+0000",
    );
    run_git(dir, &["checkout", "main"]);

    let results = operations::merge_branch(dir, "feature-messages", "main", true, false);
    assert!(results[0].success, "{}", results[0].message);
    let repo = git2::Repository::open(dir).unwrap();
    let message = repo
        .head()
        .unwrap()
        .peel_to_commit()
        .unwrap()
        .message()
        .unwrap()
        .to_string();
    assert_eq!(
        message,
        "Oldest subject\n\n  indented body\n# literal comment\n\nNewest subject\n\nbody with $HOME and `ticks`\n\nCo-Authored-By: First <first@example.com>"
    );
}

#[test]
fn test_squash_commit_without_coauthor_does_not_add_trailer() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();
    run_git(dir, &["checkout", "-b", "feature-no-coauthor"]);
    write_and_commit_message(
        dir,
        "change.txt",
        "Subject\n\nbody\n",
        "2020-01-01T00:00:00+0000",
    );
    run_git(dir, &["checkout", "main"]);

    let results = operations::merge_branch(dir, "feature-no-coauthor", "main", true, false);
    assert!(results[0].success, "{}", results[0].message);
    let repo = git2::Repository::open(dir).unwrap();
    assert_eq!(
        repo.head()
            .unwrap()
            .peel_to_commit()
            .unwrap()
            .message()
            .unwrap(),
        "Subject\n\nbody"
    );
}

#[cfg(unix)]
#[test]
fn test_squash_commit_failure_is_reported() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();
    run_git(dir, &["checkout", "-b", "feature-commit-failure"]);
    write_and_commit_message(dir, "change.txt", "Subject", "2020-01-01T00:00:00+0000");
    run_git(dir, &["checkout", "main"]);
    let hook = dir.join(".git/hooks/pre-commit");
    std::fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
    let mut permissions = std::fs::metadata(&hook).unwrap().permissions();
    use std::os::unix::fs::PermissionsExt;
    permissions.set_mode(0o755);
    std::fs::set_permissions(&hook, permissions).unwrap();

    let results = operations::merge_branch(dir, "feature-commit-failure", "main", true, false);
    assert!(
        !results[0].success,
        "failed git commit must not report success"
    );
    assert!(
        results[0]
            .message
            .contains("Failed to create squash commit")
    );
}

fn write_and_commit_message(dir: &std::path::Path, file: &str, message: &str, date: &str) {
    std::fs::write(dir.join(file), "content\n").unwrap();
    std::fs::write(dir.join("commit-message.txt"), message).unwrap();
    run_git(dir, &["add", file]);
    run_git_with_env(
        dir,
        &["commit", "--cleanup=verbatim", "--file=commit-message.txt"],
        &[("GIT_AUTHOR_DATE", date), ("GIT_COMMITTER_DATE", date)],
    );
    std::fs::remove_file(dir.join("commit-message.txt")).unwrap();
}

#[test]
fn test_merge_branch_conflict_aborts() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Both branches create the same file with different content → conflict
    run_git(dir, &["checkout", "-b", "conflict-feature"]);
    std::fs::write(dir.join("conflict.txt"), "feature version\n").unwrap();
    run_git(dir, &["add", "conflict.txt"]);
    run_git(dir, &["commit", "-m", "Feature adds conflict.txt"]);

    run_git(dir, &["checkout", "main"]);
    std::fs::write(dir.join("conflict.txt"), "main version\n").unwrap();
    run_git(dir, &["add", "conflict.txt"]);
    run_git(dir, &["commit", "-m", "Main adds conflict.txt"]);

    let results = operations::merge_branch(dir, "conflict-feature", "main", false, false);
    assert_eq!(results.len(), 1);
    assert!(!results[0].success, "conflicting merge should fail");

    // Verify merge was aborted cleanly: no conflicts, no half-staged leftovers,
    // and HEAD restored to the base branch.
    let repo = git2::Repository::open(dir).expect("re-open repo");
    let statuses = repo.statuses(None).unwrap();
    let dirty = statuses.iter().any(|e| {
        e.status().intersects(
            git2::Status::CONFLICTED
                | git2::Status::INDEX_NEW
                | git2::Status::INDEX_MODIFIED
                | git2::Status::INDEX_DELETED,
        )
    });
    assert!(
        !dirty,
        "merge abort should leave no conflicts or staged leftovers"
    );
    assert_eq!(
        repo.head().unwrap().shorthand().ok(),
        Some("main"),
        "HEAD should be back on the base branch after abort"
    );
}

// ---------------------------------------------------------------------------
// Squash merge with auto-stash
// ---------------------------------------------------------------------------

fn commit_file_on_new_branch(dir: &std::path::Path, branch: &str, file: &str, content: &str) {
    run_git(dir, &["checkout", "-b", branch]);
    std::fs::write(dir.join(file), content).unwrap();
    run_git(dir, &["add", file]);
    run_git(dir, &["commit", "-m", &format!("Add {file}")]);
    run_git(dir, &["checkout", "main"]);
}

#[test]
fn squash_merge_stashes_and_restores_tracked_and_untracked_files() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();
    commit_file_on_new_branch(dir, "feature-squash", "squash.txt", "squash content\n");

    std::fs::write(dir.join("README.md"), "# Test Repo\nlocal edit\n").unwrap();
    std::fs::write(dir.join("local.txt"), "untracked\n").unwrap();

    let results = operations::merge_branch(dir, "feature-squash", "main", true, true);
    assert!(results[0].success, "{}", results[0].message);
    assert!(dir.join("squash.txt").exists(), "squash content on main");
    assert_eq!(
        std::fs::read_to_string(dir.join("README.md")).unwrap(),
        "# Test Repo\nlocal edit\n",
        "tracked edit restored"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("local.txt")).unwrap(),
        "untracked\n",
        "untracked file restored"
    );
    assert_eq!(git_output(dir, &["stash", "list"]).trim(), "", "stash popped");
    assert!(
        !results[0].message.contains("stash restore conflicted"),
        "{}",
        results[0].message
    );
}

#[test]
fn squash_merge_conflict_restores_stash() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();
    commit_file_on_new_branch(dir, "conflict-squash", "conflict.txt", "feature version\n");
    std::fs::write(dir.join("conflict.txt"), "main version\n").unwrap();
    run_git(dir, &["add", "conflict.txt"]);
    run_git(dir, &["commit", "-m", "Main adds conflict.txt"]);

    std::fs::write(dir.join("README.md"), "# Test Repo\nlocal edit\n").unwrap();
    std::fs::write(dir.join("local.txt"), "untracked\n").unwrap();

    let results = operations::merge_branch(dir, "conflict-squash", "main", true, true);
    assert!(!results[0].success, "conflicting squash should fail");
    assert_eq!(
        std::fs::read_to_string(dir.join("conflict.txt")).unwrap(),
        "main version\n",
        "conflicted merge reset"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("README.md")).unwrap(),
        "# Test Repo\nlocal edit\n",
        "tracked edit restored"
    );
    assert!(dir.join("local.txt").exists(), "untracked file restored");
    assert_eq!(git_output(dir, &["stash", "list"]).trim(), "", "stash popped");
    let status = git_output(dir, &["status", "--porcelain"]);
    assert!(
        !status.lines().any(|l| l.starts_with("UU") || l.starts_with("AA")),
        "no unmerged entries left: {status}"
    );
}

#[test]
fn squash_stash_pop_conflict_is_reported_and_stash_kept() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();
    commit_file_on_new_branch(dir, "readme-squash", "README.md", "# Feature title\n");
    std::fs::write(dir.join("README.md"), "# Local title\n").unwrap();

    let results = operations::merge_branch(dir, "readme-squash", "main", true, true);
    assert!(
        results[0].success,
        "merge outcome stays success: {}",
        results[0].message
    );
    assert!(
        results[0]
            .message
            .contains("stash restore conflicted; your changes remain in stash@{0} (gbm-auto-stash)"),
        "{}",
        results[0].message
    );
    let stashes = git_output(dir, &["stash", "list"]);
    assert_eq!(stashes.lines().count(), 1, "stash kept: {stashes}");
    assert!(stashes.contains("gbm-auto-stash"), "{stashes}");
}

#[test]
fn squash_does_not_pop_preexisting_stash_when_nothing_was_stashed() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();
    commit_file_on_new_branch(dir, "feature-squash", "squash.txt", "squash content\n");

    std::fs::write(dir.join("README.md"), "# Test Repo\npreexisting\n").unwrap();
    run_git(dir, &["stash", "push", "-m", "user-stash"]);

    // Clean tree: the push stashes nothing, so the user's stash must survive.
    let results = operations::merge_branch(dir, "feature-squash", "main", true, true);
    assert!(results[0].success, "{}", results[0].message);
    let stashes = git_output(dir, &["stash", "list"]);
    assert_eq!(stashes.lines().count(), 1, "{stashes}");
    assert!(stashes.contains("user-stash"), "{stashes}");
    assert_eq!(
        std::fs::read_to_string(dir.join("README.md")).unwrap(),
        "# Test Repo\n",
        "preexisting stash was not applied"
    );
}

#[test]
fn squash_merge_failed_stash_push_does_not_reset_staged_changes() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();
    commit_file_on_new_branch(dir, "conflict-squash", "conflict.txt", "feature version\n");
    std::fs::write(dir.join("conflict.txt"), "main version\n").unwrap();
    run_git(dir, &["add", "conflict.txt"]);
    run_git(dir, &["commit", "-m", "Main adds conflict.txt"]);

    std::fs::write(dir.join("README.md"), "# Test Repo\nstaged edit\n").unwrap();
    run_git(dir, &["add", "README.md"]);

    // A held ref lock makes `stash push` fail before it touches the tree.
    std::fs::write(dir.join(".git/refs/stash.lock"), "").unwrap();

    let results = operations::merge_branch(dir, "conflict-squash", "main", true, true);
    assert!(!results[0].success, "conflicting squash should fail");
    assert_eq!(git_output(dir, &["stash", "list"]).trim(), "", "no stash created");
    assert_eq!(
        git_output(dir, &["diff", "--cached", "--name-only"]).trim(),
        "README.md",
        "staged edit must survive the failed merge"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("README.md")).unwrap(),
        "# Test Repo\nstaged edit\n"
    );
}

#[test]
fn plain_merge_with_stash_flag_and_nothing_stashed_has_no_stash_message() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();
    commit_file_on_new_branch(dir, "feature-plain", "plain.txt", "plain content\n");
    std::fs::write(dir.join("local.txt"), "untracked\n").unwrap();

    // Plain stash push ignores untracked files, so nothing is stashed.
    let results = operations::merge_branch(dir, "feature-plain", "main", false, true);
    assert!(results[0].success, "{}", results[0].message);
    assert!(
        !results[0].message.contains("stash restore conflicted"),
        "{}",
        results[0].message
    );
}

// ---------------------------------------------------------------------------
// Rebase operation tests
// ---------------------------------------------------------------------------

#[test]
fn test_rebase_branch_success() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Feature branch: add a unique file
    run_git(dir, &["checkout", "-b", "feature-rebase"]);
    std::fs::write(dir.join("rebase-feature.txt"), "feature content\n").unwrap();
    run_git(dir, &["add", "rebase-feature.txt"]);
    run_git(dir, &["commit", "-m", "Feature commit"]);

    // Main gets a new commit (so rebase is non-trivial)
    run_git(dir, &["checkout", "main"]);
    std::fs::write(dir.join("main-update.txt"), "main update\n").unwrap();
    run_git(dir, &["add", "main-update.txt"]);
    run_git(dir, &["commit", "-m", "Main update"]);

    // Rebase feature onto main (rebase checks out the feature branch internally)
    let results = operations::rebase_branch(dir, "feature-rebase", "main", false);
    assert_eq!(results.len(), 1);
    assert!(
        results[0].success,
        "rebase should succeed: {}",
        results[0].message
    );

    // After rebase, feature should be 1 commit ahead of main
    let repo = git2::Repository::open(dir).unwrap();
    let feature_oid = repo
        .find_branch("feature-rebase", git2::BranchType::Local)
        .unwrap()
        .get()
        .peel_to_commit()
        .unwrap()
        .id();
    let main_oid = repo
        .find_branch("main", git2::BranchType::Local)
        .unwrap()
        .get()
        .peel_to_commit()
        .unwrap()
        .id();
    let (ahead, behind) = repo.graph_ahead_behind(feature_oid, main_oid).unwrap();
    assert_eq!(
        ahead, 1,
        "feature should be 1 commit ahead of main after rebase"
    );
    assert_eq!(
        behind, 0,
        "feature should be 0 commits behind main after rebase onto it"
    );
}

#[test]
fn test_rebase_branch_conflict_aborts() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Create a shared file on main
    std::fs::write(dir.join("shared.txt"), "original\n").unwrap();
    run_git(dir, &["add", "shared.txt"]);
    run_git(dir, &["commit", "-m", "Add shared file"]);

    // Feature branch modifies shared.txt
    run_git(dir, &["checkout", "-b", "rebase-conflict"]);
    std::fs::write(dir.join("shared.txt"), "feature version\n").unwrap();
    run_git(dir, &["add", "shared.txt"]);
    run_git(dir, &["commit", "-m", "Feature modifies shared"]);

    // Main also modifies shared.txt (divergent history)
    run_git(dir, &["checkout", "main"]);
    std::fs::write(dir.join("shared.txt"), "main version\n").unwrap();
    run_git(dir, &["add", "shared.txt"]);
    run_git(dir, &["commit", "-m", "Main modifies shared"]);

    let results = operations::rebase_branch(dir, "rebase-conflict", "main", false);
    assert_eq!(results.len(), 1);
    assert!(!results[0].success, "conflicting rebase should fail");
    assert!(
        results[0].message.contains("Rebase conflict") && results[0].message.contains("aborted"),
        "message should report the conflict was aborted: {}",
        results[0].message
    );

    // Rebase should be aborted: no ongoing rebase state
    let rebase_head = dir.join(".git").join("REBASE_HEAD");
    assert!(
        !rebase_head.exists(),
        "REBASE_HEAD should not exist after abort"
    );
    // And the working tree must be clean — no conflict markers left behind, HEAD
    // restored to the branch that was being rebased.
    let repo = git2::Repository::open(dir).unwrap();
    let statuses = repo.statuses(None).unwrap();
    assert!(
        !statuses
            .iter()
            .any(|e| e.status().contains(git2::Status::CONFLICTED)),
        "rebase abort should leave no conflicted files"
    );
    assert_eq!(
        repo.head().unwrap().shorthand().ok(),
        Some("rebase-conflict"),
        "HEAD should be restored to the rebased branch after abort"
    );
}
