//! Standalone helpers for deleting a worktree's files on disk and cleaning up
//! git's admin metadata for it afterward.
//!
//! This module is intentionally *not* wired into `git::operations` or
//! `git::mod` yet -- it's groundwork for a later task that will use it to
//! give worktree deletion real progress reporting and cancellation (unlike
//! today's `git worktree remove [--force]`, which is a single opaque git
//! CLI call with no per-file feedback).
//!
//! ## Why not `std::fs::remove_dir_all`?
//!
//! `remove_dir_all` has no per-file callback and no way to check a
//! cancellation flag between files, both of which the eventual UI wiring
//! needs (a progress tick per file, and the ability to stop a large delete
//! partway through). Plain sequential `read_dir` + `remove_file`/`remove_dir`
//! on one thread is at or near the real-world deletion throughput ceiling on
//! local disks on both Linux and macOS, so there's no performance reason to
//! reach for a thread pool or platform-specific fast path here either.
//!
//! ## Why symlinks are never followed
//!
//! Every traversal in this module classifies entries with
//! [`std::fs::DirEntry::file_type`], which (unlike [`std::fs::metadata`])
//! reports the entry itself rather than what it points at. This matters for
//! two reasons: a symlink to a directory must never be recursed into (it can
//! point *outside* the worktree root, so following it could delete files
//! well outside the intended tree), and a symlink -- dangling or not -- must
//! still be removable as the link itself via `remove_file`, without needing
//! its target to exist.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use git2::{Repository, Worktree, WorktreePruneOptions};

/// Outcome of a [`delete_recursive`] call.
#[derive(Debug)]
pub(crate) enum DeleteOutcome {
    /// `root` and everything under it was removed.
    Completed,
    /// `cancel` was observed before the whole tree was removed. Whatever
    /// hadn't been visited yet -- including `root` itself -- is left on disk
    /// exactly as it was.
    Cancelled,
    /// A filesystem operation failed (e.g. permission denied). Traversal
    /// stops at the failure rather than trying to continue past it; anything
    /// already removed stays removed.
    Error(std::io::Error),
}

/// Recursively counts files and symlinks under `root`, without following
/// symlinks and without counting directories themselves. Returns 0 if `root`
/// does not exist (or isn't readable) rather than erroring -- callers use
/// this purely to size a progress bar, so "nothing to count" is a fine
/// answer for "can't be counted".
pub(crate) fn count_files(root: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(root) else {
        return 0;
    };
    let mut count = 0;
    for entry in entries.flatten() {
        match entry.file_type() {
            Ok(ft) if ft.is_dir() => count += count_files(&entry.path()),
            Ok(_) => count += 1,
            Err(_) => {}
        }
    }
    count
}

/// Deletes everything under `root`, then `root` itself, depth-first.
///
/// Before touching each entry, checks `cancel`; if it's set, stops
/// immediately and returns [`DeleteOutcome::Cancelled`] without removing
/// anything further (whatever's left on disk stays exactly as it was).
///
/// `partial_delete_risk` is set to `true` the moment the first file or
/// symlink is actually removed. It's a one-way flag the caller can use to
/// tell "we haven't touched the worktree yet" (safe to just report an error
/// and leave everything alone) apart from "we've started mutating disk"
/// (the worktree is no longer intact either way, so admin cleanup should
/// probably still happen even if this call returns `Cancelled` or `Error`).
///
/// `on_file_removed` is called once per removed file/symlink (not
/// directories) with its path relative to `root`, for progress reporting.
pub(crate) fn delete_recursive<F>(
    root: &Path,
    cancel: &AtomicBool,
    partial_delete_risk: &AtomicBool,
    mut on_file_removed: F,
) -> DeleteOutcome
where
    F: FnMut(&Path),
{
    match delete_dir_contents(
        root,
        root,
        cancel,
        partial_delete_risk,
        &mut on_file_removed,
    ) {
        Ok(()) => match std::fs::remove_dir(root) {
            Ok(()) => DeleteOutcome::Completed,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => DeleteOutcome::Completed,
            Err(e) => DeleteOutcome::Error(e),
        },
        Err(DeleteOutcome::Error(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            DeleteOutcome::Completed
        }
        Err(outcome) => outcome,
    }
}

/// Depth-first worker for [`delete_recursive`]. Empties `dir` (removing
/// nested subdirectories entirely along the way) but does not remove `dir`
/// itself -- the caller removes `root` once after this returns `Ok(())` for
/// the top level; for nested directories, this function removes them itself
/// right after emptying them (see the recursive call site below).
fn delete_dir_contents<F>(
    dir: &Path,
    root: &Path,
    cancel: &AtomicBool,
    partial_delete_risk: &AtomicBool,
    on_file_removed: &mut F,
) -> Result<(), DeleteOutcome>
where
    F: FnMut(&Path),
{
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => return Err(DeleteOutcome::Error(e)),
    };

    for entry in entries {
        if cancel.load(Ordering::Relaxed) {
            return Err(DeleteOutcome::Cancelled);
        }
        let entry = match entry {
            Ok(e) => e,
            Err(e) => return Err(DeleteOutcome::Error(e)),
        };
        let path = entry.path();
        // `file_type()` (like `symlink_metadata`) reports the entry itself,
        // never what a symlink points at -- see the module doc comment.
        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(e) => return Err(DeleteOutcome::Error(e)),
        };

        if file_type.is_dir() {
            delete_dir_contents(&path, root, cancel, partial_delete_risk, on_file_removed)?;
            if let Err(e) = std::fs::remove_dir(&path) {
                return Err(DeleteOutcome::Error(e));
            }
        } else {
            // Regular file, or symlink (dangling or not) -- `remove_file`
            // removes the directory entry itself without following it.
            if let Err(e) = std::fs::remove_file(&path) {
                return Err(DeleteOutcome::Error(e));
            }
            partial_delete_risk.store(true, Ordering::Relaxed);
            let rel = path.strip_prefix(root).unwrap_or(&path);
            on_file_removed(rel);
        }
    }

    Ok(())
}

/// Resolves the [`Worktree`] handle for `target_path`, by canonicalizing
/// both `target_path` and each registered worktree's path and comparing.
///
/// Must be called *before* deleting `target_path`'s contents:
/// `std::fs::canonicalize` requires the path to exist, and `delete_recursive`
/// removes it.
pub(crate) fn find_worktree_for_path(repo: &Repository, target_path: &Path) -> Option<Worktree> {
    let target = std::fs::canonicalize(target_path).ok()?;
    let names = repo.worktrees().ok()?;
    names
        .iter()
        .filter_map(|n| n.ok().flatten())
        .find_map(|name| {
            let wt = repo.find_worktree(name).ok()?;
            let wt_canonical = std::fs::canonicalize(wt.path()).ok()?;
            (wt_canonical == target).then_some(wt)
        })
}

/// Cleans up git's admin metadata (`.git/worktrees/<name>`) for a worktree
/// whose working-tree files have *already* been deleted by
/// [`delete_recursive`].
///
/// All three prune options are forced on, confirmed empirically (see the
/// worktree-delete task notes): by the time this runs, `delete_recursive`
/// has already removed the working-tree files itself, but libgit2's `valid`
/// check only asks "does the worktree directory still exist at all" --
/// since `delete_recursive` removes a directory's contents before removing
/// the directory itself, and callers may not always reach that last step
/// (e.g. a cancelled or failed delete), libgit2 can still see an existing
/// (if empty or partial) directory and consider the worktree "valid",
/// refusing to prune with the default `valid(false)` and failing with "not
/// pruning valid working tree". `locked(true)` likewise forces pruning past
/// a worktree the user had locked, since we've already deleted its files
/// either way. `working_tree(false)` is the one setting that must stay
/// fixed: it tells libgit2 not to *also* try to delete the working tree --
/// we already did that ourselves, with progress reporting and cancellation
/// libgit2's own recursive delete doesn't offer.
pub(crate) fn prune_admin(wt: &Worktree) -> Result<(), git2::Error> {
    let mut opts = WorktreePruneOptions::new();
    opts.valid(true).locked(true).working_tree(false);
    wt.prune(Some(&mut opts))
}

#[cfg(test)]
mod tests {
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
}
