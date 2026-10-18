use std::sync::atomic::AtomicBool;

use git_branch_manager::git::operations;

use crate::common::*;

/// Mirrors `worktree_delete::count_files`'s counting rules (files/symlinks,
/// not directories) — duplicated here because that module's items are
/// `pub(crate)` and invisible from this black-box integration-test binary.
pub(crate) fn count_files_for_test(root: &std::path::Path) -> usize {
    let Ok(entries) = std::fs::read_dir(root) else {
        return 0;
    };
    let mut count = 0;
    for entry in entries.flatten() {
        match entry.file_type() {
            Ok(ft) if ft.is_dir() => count += count_files_for_test(&entry.path()),
            Ok(_) => count += 1,
            Err(_) => {}
        }
    }
    count
}

/// Extracts `(done, file_total)` from a per-file progress message of the
/// form "<path> — <done>/<file_total>: <file>". Returns `None` for the
/// initial "scanning" tick or any other non-matching message.
pub(crate) fn parse_file_progress(msg: &str) -> Option<(usize, usize)> {
    let (_, rest) = msg.rsplit_once(" — ")?;
    let (counts, _file) = rest.split_once(": ")?;
    let (done, total) = counts.split_once('/')?;
    Some((done.parse().ok()?, total.parse().ok()?))
}

#[test]
fn test_remove_worktree_reports_exact_file_count_parity() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["branch", "wt-parity"]);
    run_git(
        dir,
        &["worktree", "add", ".worktrees/wt-parity", "wt-parity"],
    );
    let wt_path = dir.join(".worktrees").join("wt-parity");

    for i in 0..5 {
        std::fs::write(wt_path.join(format!("extra{i}.txt")), b"x").unwrap();
    }

    let expected_file_total = count_files_for_test(&wt_path);
    assert!(expected_file_total > 0);

    let (prog_tx, prog_rx) = std::sync::mpsc::channel();
    let cancel = AtomicBool::new(false);
    let partial = AtomicBool::new(false);

    let result =
        operations::force_remove_worktree(dir, &wt_path, (0, 1), &prog_tx, &cancel, &partial);
    assert!(
        result.success,
        "force_remove_worktree should succeed: {}",
        result.message
    );

    let per_file_ticks: Vec<(usize, usize)> = prog_rx
        .try_iter()
        .filter_map(|u| parse_file_progress(&u.current_item))
        .collect();

    assert!(
        !per_file_ticks.is_empty(),
        "expected at least one per-file progress tick"
    );
    assert!(
        per_file_ticks
            .iter()
            .all(|(_, total)| *total == expected_file_total),
        "every per-file tick must report the same file_total: {per_file_ticks:?}"
    );
    let max_done = per_file_ticks.iter().map(|(done, _)| *done).max().unwrap();
    assert_eq!(
        max_done, expected_file_total,
        "the final per-file tick must report done == file_total exactly (no off-by-one)"
    );
}

/// `force_remove_worktree`'s dirty-tree path skips the clean-check (which
/// returns before any `prog_tx.send`) — this is the one branch where a
/// regression could silently drop every per-file progress tick.
#[test]
fn test_force_remove_worktree_dirty_reports_exact_file_count_parity() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["branch", "wt-dirty-parity"]);
    run_git(
        dir,
        &[
            "worktree",
            "add",
            ".worktrees/wt-dirty-parity",
            "wt-dirty-parity",
        ],
    );
    let wt_path = dir.join(".worktrees").join("wt-dirty-parity");

    // Modify a TRACKED file so the tree is dirty — force_remove_worktree must
    // still stream a per-file tick for every file, including this one.
    std::fs::write(wt_path.join("README.md"), "# Modified in worktree\n").unwrap();
    for i in 0..5 {
        std::fs::write(wt_path.join(format!("extra{i}.txt")), b"x").unwrap();
    }

    let expected_file_total = count_files_for_test(&wt_path);
    assert!(expected_file_total > 0);

    let (prog_tx, prog_rx) = std::sync::mpsc::channel();
    let cancel = AtomicBool::new(false);
    let partial = AtomicBool::new(false);

    let result =
        operations::force_remove_worktree(dir, &wt_path, (0, 1), &prog_tx, &cancel, &partial);
    assert!(
        result.success,
        "force_remove_worktree should succeed on dirty worktree: {}",
        result.message
    );

    let per_file_ticks: Vec<(usize, usize)> = prog_rx
        .try_iter()
        .filter_map(|u| parse_file_progress(&u.current_item))
        .collect();

    assert!(
        !per_file_ticks.is_empty(),
        "expected at least one per-file progress tick on the dirty force-remove path"
    );
    assert!(
        per_file_ticks
            .iter()
            .all(|(_, total)| *total == expected_file_total),
        "every per-file tick must report the same file_total: {per_file_ticks:?}"
    );
    let max_done = per_file_ticks.iter().map(|(done, _)| *done).max().unwrap();
    assert_eq!(
        max_done, expected_file_total,
        "the final per-file tick must report done == file_total exactly (no off-by-one)"
    );
}

#[test]
fn test_remove_worktree_mid_delete_cancellation_leaves_partial_state() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["branch", "wt-cancel"]);
    run_git(
        dir,
        &["worktree", "add", ".worktrees/wt-cancel", "wt-cancel"],
    );
    let wt_path = dir.join(".worktrees").join("wt-cancel");

    // Enough files that cancellation partway through is reliably observable
    // before the whole delete can race ahead of the watcher thread below.
    for i in 0..300 {
        std::fs::write(wt_path.join(format!("extra{i}.txt")), b"x").unwrap();
    }
    let total_before = count_files_for_test(&wt_path);

    let (prog_tx, prog_rx) =
        std::sync::mpsc::channel::<git_branch_manager::types::ProgressUpdate>();
    let cancel = std::sync::Arc::new(AtomicBool::new(false));
    let partial = AtomicBool::new(false);

    let cancel_for_watcher = std::sync::Arc::clone(&cancel);
    let watcher = std::thread::spawn(move || {
        let mut file_ticks = 0;
        for update in prog_rx.iter() {
            if parse_file_progress(&update.current_item).is_some() {
                file_ticks += 1;
                if file_ticks == 3 {
                    cancel_for_watcher.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
        }
    });

    let result =
        operations::force_remove_worktree(dir, &wt_path, (0, 1), &prog_tx, &cancel, &partial);
    drop(prog_tx);
    watcher.join().unwrap();

    assert!(
        !result.success,
        "cancelled force_remove should not report success"
    );
    assert_eq!(result.message, "Cancelled");
    assert!(
        partial.load(std::sync::atomic::Ordering::Relaxed),
        "partial_delete_risk must be set once at least one file was removed"
    );
    assert!(
        wt_path.exists(),
        "cancelled delete must leave the worktree directory on disk"
    );
    let remaining = count_files_for_test(&wt_path);
    assert!(
        remaining > 0 && remaining < total_before,
        "some files must be removed, some must remain: {remaining}/{total_before}"
    );
}

#[test]
fn test_force_remove_worktree_reports_every_progress_increment_no_gaps() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["branch", "wt-progress"]);
    run_git(
        dir,
        &["worktree", "add", ".worktrees/wt-progress", "wt-progress"],
    );
    let wt_path = dir.join(".worktrees").join("wt-progress");

    // 100 extra files keeps file_total comfortably under 200, so
    // batch_size = (file_total / 200).max(1) == 1 (remove_worktree_impl,
    // src/git/operations.rs) and every single file removal emits its own tick.
    for i in 0..100 {
        std::fs::write(wt_path.join(format!("extra{i}.txt")), b"x").unwrap();
    }

    let expected_file_total = count_files_for_test(&wt_path);
    assert!(expected_file_total > 0);

    let (prog_tx, prog_rx) = std::sync::mpsc::channel();
    let cancel = AtomicBool::new(false);
    let partial = AtomicBool::new(false);

    let result =
        operations::force_remove_worktree(dir, &wt_path, (0, 1), &prog_tx, &cancel, &partial);
    assert!(
        result.success,
        "force_remove_worktree should succeed: {}",
        result.message
    );

    let mut dones: Vec<usize> = prog_rx
        .try_iter()
        .filter_map(|u| parse_file_progress(&u.current_item))
        .map(|(done, _)| done)
        .collect();
    dones.sort_unstable();

    let expected: Vec<usize> = (1..=expected_file_total).collect();
    assert_eq!(
        dones, expected,
        "per-file progress ticks must cover every increment 1..=file_total with no gaps"
    );
}
