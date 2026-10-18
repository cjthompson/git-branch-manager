
use git_branch_manager::git::{
    graph, merge_detection,
};

use crate::common::*;

#[test]
fn test_squash_scenario_09_binary_file_changes() {
    // Plan scenario 9: branch adds/modifies a binary file; squash lands the
    // same binary change. Treated as an empirical determination (binary-diff
    // patch-id behavior is undocumented at the git-scm level per the plan
    // doc), not an assumed-safe path.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/binary"]);
    // A small, fixed-content "binary" blob (embedded NUL byte forces git to
    // treat it as binary).
    std::fs::write(dir.join("blob.bin"), [0u8, 1, 2, 3, 0, 255, 254, 253]).unwrap();
    run_git(dir, &["add", "blob.bin"]);
    run_git(dir, &["commit", "-m", "add binary blob"]);

    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/binary"]);
    run_git(dir, &["commit", "-m", "squash landing (binary)"]);
    let squash_oid = git_output(dir, &["rev-parse", "HEAD"]);

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..tmpdir.graph_options()
        },
    )
    .expect("graph load should succeed");
    assert!(
        snapshot
            .commits
            .iter()
            .find(|c| c.oid == squash_oid)
            .expect("squash landing commit should be displayed")
            .is_possible_squash_merge(),
        "observed: this codebase's `git diff --binary --full-index` + `git patch-id --stable` \
         pipeline produces a matching patch-id for an identical binary-file change"
    );

    assert!(merge_detection::is_squash_merged(
        dir,
        "main",
        "feature/binary",
        None,
        None
    ));
}

#[test]
fn test_squash_scenario_10a_rename_only_no_content_change() {
    // Plan scenario 10a: branch renames a file with no content change; squash
    // lands the rename.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    std::fs::write(dir.join("original-name.txt"), "unchanged content\n").unwrap();
    run_git(dir, &["add", "original-name.txt"]);
    run_git(dir, &["commit", "-m", "add original-name.txt"]);

    run_git(dir, &["checkout", "-b", "feature/rename-only"]);
    run_git(dir, &["mv", "original-name.txt", "renamed.txt"]);
    run_git(dir, &["commit", "-m", "rename only"]);

    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/rename-only"]);
    run_git(dir, &["commit", "-m", "squash landing (rename only)"]);
    let squash_oid = git_output(dir, &["rev-parse", "HEAD"]);

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..tmpdir.graph_options()
        },
    )
    .expect("graph load should succeed");
    assert!(
        snapshot
            .commits
            .iter()
            .find(|c| c.oid == squash_oid)
            .expect("squash landing commit should be displayed")
            .is_possible_squash_merge(),
        "observed: a pure rename (default rename detection in `git diff`, on since Git 2.9) \
         still produces a matching stable patch-id"
    );
    assert!(merge_detection::is_squash_merged(
        dir,
        "main",
        "feature/rename-only",
        None,
        None
    ));
}

#[test]
fn test_squash_scenario_10b_rename_and_content_change() {
    // Plan scenario 10b: branch renames a file *and* changes its content;
    // squash lands both.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    std::fs::write(
        dir.join("original2.txt"),
        "line one\nline two\nline three\n",
    )
    .unwrap();
    run_git(dir, &["add", "original2.txt"]);
    run_git(dir, &["commit", "-m", "add original2.txt"]);

    run_git(dir, &["checkout", "-b", "feature/rename-and-change"]);
    run_git(dir, &["mv", "original2.txt", "renamed2.txt"]);
    std::fs::write(
        dir.join("renamed2.txt"),
        "line one\nline two CHANGED\nline three\n",
    )
    .unwrap();
    run_git(dir, &["commit", "-am", "rename and change content"]);

    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/rename-and-change"]);
    run_git(dir, &["commit", "-m", "squash landing (rename + change)"]);
    let squash_oid = git_output(dir, &["rev-parse", "HEAD"]);

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..tmpdir.graph_options()
        },
    )
    .expect("graph load should succeed");
    assert!(
        snapshot
            .commits
            .iter()
            .find(|c| c.oid == squash_oid)
            .expect("squash landing commit should be displayed")
            .is_possible_squash_merge(),
        "observed: rename-plus-content-change still produces a matching stable patch-id"
    );
    assert!(merge_detection::is_squash_merged(
        dir,
        "main",
        "feature/rename-and-change",
        None,
        None
    ));
}

#[test]
fn test_squash_scenario_10c_executable_bit_only_change() {
    // Plan scenario 10c: branch changes a file's executable bit only.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    std::fs::write(dir.join("script.sh"), "#!/bin/sh\necho hi\n").unwrap();
    run_git(dir, &["add", "script.sh"]);
    run_git(dir, &["commit", "-m", "add script.sh"]);

    run_git(dir, &["checkout", "-b", "feature/exec-bit"]);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(dir.join("script.sh"))
            .unwrap()
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(dir.join("script.sh"), perms).unwrap();
    }
    run_git(dir, &["update-index", "--chmod=+x", "script.sh"]);
    run_git(dir, &["commit", "-m", "make script.sh executable"]);

    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/exec-bit"]);
    run_git(dir, &["commit", "-m", "squash landing (exec bit)"]);
    let squash_oid = git_output(dir, &["rev-parse", "HEAD"]);

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..tmpdir.graph_options()
        },
    )
    .expect("graph load should succeed");
    assert!(
        snapshot
            .commits
            .iter()
            .find(|c| c.oid == squash_oid)
            .expect("squash landing commit should be displayed")
            .is_possible_squash_merge(),
        "observed: a mode-only (executable bit) change still produces a matching stable patch-id"
    );
    assert!(merge_detection::is_squash_merged(
        dir,
        "main",
        "feature/exec-bit",
        None,
        None
    ));
}

#[test]
fn test_squash_scenario_11_whitespace_only_change() {
    // Plan scenario 11 (positive): branch changes only trailing whitespace on
    // otherwise-identical content; squash lands the identical whitespace
    // change. `git patch-id --stable` is documented (and empirically
    // confirmed here) to ignore whitespace differences entirely.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    std::fs::write(dir.join("ws.txt"), "line one\nline two\nline three\n").unwrap();
    run_git(dir, &["add", "ws.txt"]);
    run_git(dir, &["commit", "-m", "add ws.txt"]);

    run_git(dir, &["checkout", "-b", "feature/whitespace"]);
    std::fs::write(dir.join("ws.txt"), "line one   \nline two\nline three\n").unwrap();
    run_git(dir, &["commit", "-am", "trailing whitespace on line one"]);

    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/whitespace"]);
    run_git(dir, &["commit", "-m", "squash landing (whitespace)"]);
    let squash_oid = git_output(dir, &["rev-parse", "HEAD"]);

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..tmpdir.graph_options()
        },
    )
    .expect("graph load should succeed");
    assert!(
        snapshot
            .commits
            .iter()
            .find(|c| c.oid == squash_oid)
            .expect("squash landing commit should be displayed")
            .is_possible_squash_merge(),
        "git patch-id --stable ignores whitespace-only differences"
    );
    assert!(merge_detection::is_squash_merged(
        dir,
        "main",
        "feature/whitespace",
        None,
        None
    ));
}

#[test]
fn test_squash_scenario_11_whitespace_negative_unrelated() {
    // Plan scenario 11 (negative control): base's independent commit makes
    // only an *unrelated* whitespace change (not the branch's change) — must
    // NOT be flagged, since both would otherwise produce a degenerate
    // near-empty diff that could coincidentally collide.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    std::fs::write(dir.join("ws-a.txt"), "alpha one\nalpha two\nalpha three\n").unwrap();
    std::fs::write(dir.join("ws-b.txt"), "beta one\nbeta two\nbeta three\n").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "add ws-a.txt and ws-b.txt"]);

    run_git(dir, &["checkout", "-b", "feature/ws-unrelated"]);
    std::fs::write(
        dir.join("ws-a.txt"),
        "alpha one   \nalpha two\nalpha three\n",
    )
    .unwrap();
    run_git(
        dir,
        &["commit", "-am", "branch: trailing whitespace on ws-a.txt"],
    );

    run_git(dir, &["checkout", "main"]);
    // An independent, unrelated whitespace-only change on a *different* file.
    std::fs::write(dir.join("ws-b.txt"), "beta one   \nbeta two\nbeta three\n").unwrap();
    run_git(
        dir,
        &[
            "commit",
            "-am",
            "main: unrelated trailing whitespace on ws-b.txt",
        ],
    );
    let unrelated_base_oid = git_output(dir, &["rev-parse", "HEAD"]);

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..tmpdir.graph_options()
        },
    )
    .expect("graph load should succeed");
    assert!(
        !snapshot
            .commits
            .iter()
            .find(|c| c.oid == unrelated_base_oid)
            .expect("unrelated base commit should be displayed")
            .is_possible_squash_merge(),
        "an unrelated whitespace-only change to a different file must not be flagged"
    );
    assert!(!merge_detection::is_squash_merged(
        dir,
        "main",
        "feature/ws-unrelated",
        None,
        None
    ));
}
