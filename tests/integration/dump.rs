

use crate::common::*;

#[test]
fn dump_branches_basic() {
    let (tmp, _repo) = setup_test_repo();
    let out = manager_command(&tmp)
        .args([
            "--repo",
            tmp.path().to_str().unwrap(),
            "--branches",
            "--color=never",
        ])
        .output()
        .expect("failed to run binary");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let s = String::from_utf8(out.stdout).unwrap();
    assert!(s.starts_with("base: main"), "got: {s:?}");
    let lines: Vec<&str> = s.lines().collect();
    let header = lines.get(2).expect("table header");
    assert_eq!(
        header.trim_start().split_whitespace().next(),
        Some("Branch")
    );
    assert!(!header.contains("Up"), "Up must not have a header: {s:?}");
    assert!(s.contains("main"), "base branch row missing: {s:?}");
    let base_row = lines.get(3).expect("base branch row");
    assert!(
        base_row.find('-').unwrap() < base_row.find("main").unwrap(),
        "upstream marker should precede branch name: {base_row:?}"
    );
    assert!(!s.contains('\x1b'), "--color=never must be plain: {s:?}");
}

#[test]
fn dump_rejects_two_view_flags() {
    let (tmp, _repo) = setup_test_repo();
    let out = manager_command(&tmp)
        .args([
            "--repo",
            tmp.path().to_str().unwrap(),
            "--branches",
            "--tags",
        ])
        .output()
        .expect("failed to run binary");
    assert!(
        !out.status.success(),
        "two view flags must be a usage error"
    );
}

#[test]
fn dump_list_is_branches_alias() {
    let (tmp, _repo) = setup_test_repo();
    let out = manager_command(&tmp)
        .args([
            "--repo",
            tmp.path().to_str().unwrap(),
            "--list",
            "--color=never",
        ])
        .output()
        .expect("failed to run binary");
    assert!(out.status.success());
    let s = String::from_utf8(out.stdout).unwrap();
    assert!(s.starts_with("base: main"), "got: {s:?}");
    let err = String::from_utf8(out.stderr).unwrap();
    assert!(
        err.contains("deprecated"),
        "expected --list deprecation note on stderr, got: {err:?}"
    );
}

#[test]
fn dump_remotes_basic() {
    let (tmp, _repo) = setup_test_repo();
    // A bare "remote" with one branch, fetched into the test repo.
    let remote = tempfile::tempdir().unwrap();
    run_git(remote.path(), &["init", "--bare", "-b", "main"]);
    run_git(
        tmp.path(),
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    );
    run_git(tmp.path(), &["push", "origin", "main"]);
    run_git(tmp.path(), &["fetch", "origin"]);

    let out = manager_command(&tmp)
        .args([
            "--repo",
            tmp.path().to_str().unwrap(),
            "--remotes",
            "--color=never",
            "--symbols",
            "ascii",
        ])
        .output()
        .expect("failed to run binary");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let s = String::from_utf8(out.stdout).unwrap();
    assert!(s.contains("Name"), "header missing: {s:?}");
    assert!(!s.contains("Local"), "Local header should be hidden: {s:?}");
    assert!(
        s.lines()
            .find(|line| line.contains("origin/main"))
            .is_some_and(|line| line.trim_start().starts_with('L')),
        "local-branch marker should precede remote name: {s:?}"
    );
    assert!(s.contains("origin/main"), "remote row missing: {s:?}");
}

#[test]
fn dump_tags_basic() {
    let (tmp, _repo) = setup_test_repo();
    run_git(tmp.path(), &["tag", "-a", "v1.0", "-m", "release one"]);
    let out = manager_command(&tmp)
        .args([
            "--repo",
            tmp.path().to_str().unwrap(),
            "--tags",
            "--color=never",
        ])
        .output()
        .expect("failed to run binary");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let s = String::from_utf8(out.stdout).unwrap();
    assert!(s.contains("Name"), "header missing: {s:?}");
    assert!(s.contains("v1.0"), "tag row missing: {s:?}");
}

/// The Tags "Message" column is the last column but is emitted LEFT-aligned.
/// Data-cell alignment must come from the rendered Line, not the positional
/// "last column" heuristic (which would right-align it). With a message
/// shorter than the column width (10), a right-aligned cell would pad the
/// message with leading spaces; a left-aligned cell places it directly after
/// the two-space inter-column gap.
#[test]
fn dump_tags_message_is_left_aligned() {
    let (tmp, _repo) = setup_test_repo();
    // Message "rel one" (7 chars) is shorter than the Message column width (10),
    // so alignment padding is observable (vs. truncated for longer messages).
    run_git(tmp.path(), &["tag", "-a", "v1.0", "-m", "rel one"]);
    let out = manager_command(&tmp)
        .args([
            "--repo",
            tmp.path().to_str().unwrap(),
            "--tags",
            "--color=never",
        ])
        .output()
        .expect("failed to run binary");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let s = String::from_utf8(out.stdout).unwrap();
    assert!(s.contains("rel one"), "tag message missing: {s:?}");
    // Left-aligned: message follows the 2-space inter-column gap directly.
    assert!(
        s.contains("  rel one"),
        "message should sit right after the inter-column gap: {s:?}"
    );
    // Would FAIL under the old positional right-align: that pads the 7-char
    // message to width 10, yielding 3 leading spaces (2-space gap + 3 pad).
    assert!(
        !s.contains("   rel one"),
        "message must not be right-aligned (no leading pad): {s:?}"
    );
}

/// The `--remotes` dump must run the remote squash-merge pass (mirroring the
/// TUI's Remotes tab), so a remote branch whose content was squash-merged into
/// the base shows the SquashMerged indicator rather than Unmerged.
#[test]
fn dump_remotes_detects_squash_merged() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    // Feature branch with unique content, pushed to the remote.
    run_git(&work_dir, &["checkout", "-b", "squash-feature"]);
    std::fs::write(
        work_dir.join("squash-feature.txt"),
        "squash feature content\n",
    )
    .unwrap();
    run_git(&work_dir, &["add", "squash-feature.txt"]);
    run_git(&work_dir, &["commit", "-m", "Squash feature commit"]);
    run_git(&work_dir, &["push", "-u", "origin", "squash-feature"]);

    // Squash-merge into main (no merge commit) and push.
    run_git(&work_dir, &["checkout", "main"]);
    run_git(&work_dir, &["merge", "--squash", "squash-feature"]);
    run_git(&work_dir, &["commit", "-m", "Squash merge squash-feature"]);
    run_git(&work_dir, &["push", "origin", "main"]);

    // ascii symbols make the SquashMerged Status cell deterministic:
    // "squash-merged ~" (status_squash_merged = "~", full text at wide width).
    let out = manager_command(&_tmpdir)
        .args([
            "--repo",
            work_dir.to_str().unwrap(),
            "--remotes",
            "--symbols",
            "ascii",
            "--color=never",
        ])
        .output()
        .expect("failed to run binary");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let s = String::from_utf8(out.stdout).unwrap();
    assert!(
        s.contains("origin/squash-feature"),
        "remote row missing: {s:?}"
    );
    // The squash-feature row must carry the SquashMerged indicator, not Unmerged.
    let row = s
        .lines()
        .find(|l| l.contains("origin/squash-feature"))
        .unwrap_or_else(|| panic!("squash-feature row not found: {s:?}"));
    assert!(
        row.contains("squash-merged ~"),
        "squash-feature should show SquashMerged in --remotes dump, got row: {row:?}\nfull: {s:?}"
    );
    assert!(
        !row.contains("unmerged -"),
        "squash-feature must not show Unmerged: {row:?}"
    );
    assert!(
        std::fs::read_dir(_tmpdir.cache_root())
            .unwrap()
            .flatten()
            .any(|entry| entry.path().extension().is_some_and(|ext| ext == "sqlite3")),
        "the child process should write its cache under the TestDir cache root"
    );
}

#[test]
fn dump_worktrees_basic() {
    let (tmp, _repo) = setup_test_repo();
    let out = manager_command(&tmp)
        .args([
            "--repo",
            tmp.path().to_str().unwrap(),
            "--worktrees",
            "--color=never",
        ])
        .output()
        .expect("failed to run binary");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let s = String::from_utf8(out.stdout).unwrap();
    assert!(s.contains("Path"), "header missing: {s:?}");
    // The main working tree is always listed. The dump auto-sizes the first
    // (Path) column to its content, so the full canonicalized path appears.
    let canonical = tmp.path().canonicalize().unwrap();
    let canonical_str = canonical.to_str().unwrap();
    assert!(
        s.contains(canonical_str),
        "main worktree full path missing: {s:?}\nexpected: {canonical_str}"
    );
    // The main worktree is on the base branch, so its Merge cell must be blank
    // (a branch can't be merged into itself). This guards the render bug where
    // the base worktree wrongly showed "unmerged".
    let main_row = s
        .lines()
        .find(|l| l.contains(canonical_str))
        .expect("main worktree row");
    assert!(
        !main_row.contains("unmerged"),
        "base worktree row must not show a merge status: {main_row:?}"
    );
}
