use super::*;
use std::path::PathBuf;
use tempfile::TempDir;

#[test]
fn count_files_counts_nested_files() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir_all(root.join("a/b")).unwrap();
    std::fs::write(root.join("top.txt"), b"1").unwrap();
    std::fs::write(root.join("a/mid.txt"), b"1").unwrap();
    std::fs::write(root.join("a/b/leaf.txt"), b"1").unwrap();

    assert_eq!(count_files(&root), 3);
}

#[test]
fn count_files_empty_dir_is_zero() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir_all(&root).unwrap();
    assert_eq!(count_files(&root), 0);
}

#[test]
fn count_files_missing_root_is_zero() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("does-not-exist");
    assert_eq!(count_files(&root), 0);
}

#[cfg(unix)]
#[test]
fn count_and_delete_do_not_follow_symlinks() {
    use std::os::unix::fs::symlink;

    let dir = TempDir::new().unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir_all(&root).unwrap();

    // Target lives outside `root` -- if the symlink were ever followed
    // (instead of just having the link itself removed), this file would
    // vanish too.
    let outside_target = dir.path().join("outside.txt");
    std::fs::write(&outside_target, b"keep me").unwrap();
    symlink(&outside_target, root.join("link_to_outside")).unwrap();

    // A dangling symlink (target never existed) must still count and
    // delete cleanly.
    symlink(
        dir.path().join("does-not-exist"),
        root.join("dangling_link"),
    )
    .unwrap();

    assert_eq!(count_files(&root), 2);

    let cancel = AtomicBool::new(false);
    let partial = AtomicBool::new(false);
    let mut removed = Vec::new();
    let outcome = delete_recursive(&root, &cancel, &partial, |p| removed.push(p.to_path_buf()));

    assert!(matches!(outcome, DeleteOutcome::Completed));
    assert!(!root.exists());
    assert!(
        outside_target.exists(),
        "symlink target must survive -- the link was removed, not followed"
    );
    assert_eq!(removed.len(), 2);
    assert!(partial.load(Ordering::Relaxed));
}

#[test]
fn delete_recursive_removes_nested_tree_and_reports_each_file() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir_all(root.join("a/b")).unwrap();
    std::fs::write(root.join("top.txt"), b"1").unwrap();
    std::fs::write(root.join("a/mid.txt"), b"1").unwrap();
    std::fs::write(root.join("a/b/leaf.txt"), b"1").unwrap();

    let cancel = AtomicBool::new(false);
    let partial = AtomicBool::new(false);
    let mut removed = Vec::new();
    let outcome = delete_recursive(&root, &cancel, &partial, |p| removed.push(p.to_path_buf()));

    assert!(matches!(outcome, DeleteOutcome::Completed));
    assert!(!root.exists());
    assert_eq!(removed.len(), 3);
    assert!(partial.load(Ordering::Relaxed));
    assert!(removed.iter().all(|p| p.is_relative()));
    assert!(removed.contains(&PathBuf::from("top.txt")));
    assert!(removed.contains(&PathBuf::from("a/mid.txt")));
    assert!(removed.contains(&PathBuf::from("a/b/leaf.txt")));
}

#[test]
fn delete_recursive_empty_dir_removes_root_itself() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir_all(&root).unwrap();

    let cancel = AtomicBool::new(false);
    let partial = AtomicBool::new(false);
    let outcome = delete_recursive(&root, &cancel, &partial, |_| {});

    assert!(matches!(outcome, DeleteOutcome::Completed));
    assert!(!root.exists());
    assert!(!partial.load(Ordering::Relaxed));
}

#[test]
fn delete_recursive_missing_root_is_a_no_op_completed() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("does-not-exist");

    let cancel = AtomicBool::new(false);
    let partial = AtomicBool::new(false);
    let outcome = delete_recursive(&root, &cancel, &partial, |_| {});

    assert!(matches!(outcome, DeleteOutcome::Completed));
    assert!(!partial.load(Ordering::Relaxed));
}

#[test]
fn delete_recursive_cancellation_leaves_remainder_on_disk() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir_all(&root).unwrap();
    for i in 0..5 {
        std::fs::write(root.join(format!("f{i}.txt")), b"x").unwrap();
    }

    let cancel = AtomicBool::new(false);
    let partial = AtomicBool::new(false);
    let mut removed_count = 0usize;
    let outcome = delete_recursive(&root, &cancel, &partial, |_| {
        removed_count += 1;
        if removed_count == 2 {
            cancel.store(true, Ordering::Relaxed);
        }
    });

    assert!(matches!(outcome, DeleteOutcome::Cancelled));
    assert_eq!(removed_count, 2);
    assert!(partial.load(Ordering::Relaxed));
    assert!(root.exists(), "root must survive a cancelled delete");
    assert_eq!(count_files(&root), 3, "unvisited files remain on disk");
}

#[cfg(unix)]
#[test]
fn delete_recursive_stops_cleanly_on_permission_denied() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new().unwrap();
    let root = dir.path().join("root");
    let blocked = root.join("blocked");
    std::fs::create_dir_all(&blocked).unwrap();
    std::fs::write(blocked.join("secret.txt"), b"x").unwrap();
    // Deny read+execute on the subdirectory so it can't be listed.
    std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o000)).unwrap();

    let cancel = AtomicBool::new(false);
    let partial = AtomicBool::new(false);
    let outcome = delete_recursive(&root, &cancel, &partial, |_| {});

    // Restore permissions so `TempDir`'s own cleanup on drop can succeed.
    std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o700)).unwrap();

    assert!(matches!(outcome, DeleteOutcome::Error(_)));
    assert!(
        blocked.join("secret.txt").exists(),
        "blocked subtree must be left untouched"
    );
}

fn run_git(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap_or_else(|e| panic!("failed to run git {args:?}: {e}"));
    if !out.status.success() {
        panic!(
            "git {:?} failed in {}: {}",
            args,
            dir.display(),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn find_worktree_for_path_and_prune_admin_round_trip() {
    let dir = TempDir::new().unwrap();
    let repo_dir = dir.path().join("repo");
    std::fs::create_dir_all(&repo_dir).unwrap();
    run_git(&repo_dir, &["init", "-b", "main"]);
    run_git(&repo_dir, &["config", "user.email", "a@b.com"]);
    run_git(&repo_dir, &["config", "user.name", "A"]);
    std::fs::write(repo_dir.join("f.txt"), "x").unwrap();
    run_git(&repo_dir, &["add", "."]);
    run_git(&repo_dir, &["commit", "-m", "init"]);
    run_git(&repo_dir, &["branch", "wtb"]);
    let wt_path = dir.path().join("wt");
    run_git(
        &repo_dir,
        &["worktree", "add", wt_path.to_str().unwrap(), "wtb"],
    );

    let repo = Repository::open(&repo_dir).unwrap();
    let wt = find_worktree_for_path(&repo, &wt_path).expect("worktree should be found");

    let cancel = AtomicBool::new(false);
    let partial = AtomicBool::new(false);
    let outcome = delete_recursive(&wt_path, &cancel, &partial, |_| {});
    assert!(matches!(outcome, DeleteOutcome::Completed));

    prune_admin(&wt).expect("prune_admin should succeed after files are deleted");

    let admin_dir = repo_dir.join(".git").join("worktrees").join("wt");
    assert!(!admin_dir.exists(), "admin metadata should be pruned");
}

#[test]
fn find_worktree_for_path_returns_none_for_unrelated_path() {
    let dir = TempDir::new().unwrap();
    let repo_dir = dir.path().join("repo");
    std::fs::create_dir_all(&repo_dir).unwrap();
    run_git(&repo_dir, &["init", "-b", "main"]);
    run_git(&repo_dir, &["config", "user.email", "a@b.com"]);
    run_git(&repo_dir, &["config", "user.name", "A"]);
    std::fs::write(repo_dir.join("f.txt"), "x").unwrap();
    run_git(&repo_dir, &["add", "."]);
    run_git(&repo_dir, &["commit", "-m", "init"]);

    let repo = Repository::open(&repo_dir).unwrap();
    assert!(find_worktree_for_path(&repo, dir.path()).is_none());
}
