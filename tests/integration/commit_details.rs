
use git_branch_manager::git::commit_details::{
    load_commit_details, CommitDetailMode, CommitFileKind,
};
use git_branch_manager::git::graph::{GraphCommit, GraphRef, GraphRefKind};

use crate::common::*;

#[test]
fn commit_details_loads_full_message_and_selected_commit_files() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();
    std::fs::write(dir.join("single.txt"), "one\n").unwrap();
    run_git(dir, &["add", "single.txt"]);
    run_git(
        dir,
        &["commit", "-m", "subject\n\nbody\nline four\nline five"],
    );
    let oid = git_output(dir, &["rev-parse", "HEAD"]);
    let commit = GraphCommit {
        oid,
        summary: "subject".to_string(),
        ..GraphCommit::default()
    };

    let details = load_commit_details(dir, &commit, "main").expect("details should load");
    assert!(matches!(details.mode, CommitDetailMode::Commit { .. }));
    assert_eq!(details.message_lines.len(), 4);
    assert_eq!(details.message_lines[0], "subject");
    assert_eq!(details.message_lines[2], "body");
    assert_eq!(details.files.len(), 1);
    assert_eq!(details.files[0].path, "single.txt");
    assert_eq!(details.files[0].kind, CommitFileKind::Added);
}

#[test]
fn commit_details_uses_first_parent_for_merge_and_root_file_patches() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();
    run_git(dir, &["checkout", "-b", "feature/merge-details"]);
    std::fs::write(dir.join("side.txt"), "side\n").unwrap();
    run_git(dir, &["add", "side.txt"]);
    run_git(dir, &["commit", "-m", "side"]);
    run_git(dir, &["checkout", "main"]);
    std::fs::write(dir.join("main.txt"), "main\n").unwrap();
    run_git(dir, &["add", "main.txt"]);
    run_git(dir, &["commit", "-m", "main change"]);
    run_git(
        dir,
        &[
            "merge",
            "--no-ff",
            "feature/merge-details",
            "-m",
            "merge commit",
        ],
    );
    let oid = git_output(dir, &["rev-parse", "HEAD"]);
    let commit = GraphCommit {
        oid: oid.clone(),
        summary: "merge commit".into(),
        ..GraphCommit::default()
    };
    let details = load_commit_details(dir, &commit, "main").expect("merge details");
    assert_eq!(
        details
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        vec!["side.txt"]
    );
    let patch = git_branch_manager::git::commit_details::load_commit_file_diff(
        dir,
        &details,
        &details.files[0],
    )
    .expect("merge patch");
    assert!(patch.patch.contains("+side"));

    let (root_dir, _repo) = setup_test_repo();
    let root_path = root_dir.path();
    let initial = git_output(root_path, &["rev-list", "--max-parents=0", "HEAD"]);
    let root_commit = GraphCommit {
        oid: initial,
        summary: "initial".into(),
        ..GraphCommit::default()
    };
    let root_details = load_commit_details(root_path, &root_commit, "main").expect("root details");
    assert!(!root_details.files.is_empty());
    let root_patch = git_branch_manager::git::commit_details::load_commit_file_diff(
        root_path,
        &root_details,
        &root_details.files[0],
    )
    .expect("root patch");
    assert!(!root_patch.patch.is_empty());
}

#[test]
fn branch_tip_details_uses_merge_base_for_aggregate_files() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();
    run_git(dir, &["checkout", "-b", "feature/details"]);
    std::fs::write(dir.join("earlier.txt"), "earlier\n").unwrap();
    run_git(dir, &["add", "earlier.txt"]);
    run_git(dir, &["commit", "-m", "earlier feature commit"]);
    std::fs::write(dir.join("tip.txt"), "tip\n").unwrap();
    run_git(dir, &["add", "tip.txt"]);
    run_git(dir, &["commit", "-m", "tip feature commit"]);
    run_git(dir, &["checkout", "main"]);
    std::fs::write(dir.join("base-only.txt"), "base side\n").unwrap();
    run_git(dir, &["add", "base-only.txt"]);
    run_git(dir, &["commit", "-m", "base diverges"]);
    run_git(dir, &["checkout", "feature/details"]);
    let oid = git_output(dir, &["rev-parse", "HEAD"]);
    let commit = GraphCommit {
        oid,
        summary: "tip feature commit".to_string(),
        refs: vec![GraphRef {
            name: "feature/details".to_string(),
            kind: GraphRefKind::LocalBranch,
            has_linked_worktree: false,
            is_current: true,
            tracking: None,
            ahead_of_base: None,
        }],
        ..GraphCommit::default()
    };

    let details = load_commit_details(dir, &commit, "main").expect("tip details should load");
    let CommitDetailMode::BranchTip {
        merge_base, tip, ..
    } = &details.mode
    else {
        panic!("branch tip mode");
    };
    assert_eq!(
        merge_base,
        &git_output(dir, &["merge-base", "main", "feature/details"])
    );
    assert_eq!(tip, &git_output(dir, &["rev-parse", "feature/details"]));
    assert!(details
        .branch_log
        .iter()
        .any(|line| line.contains("earlier feature commit")));
    assert!(details.files.iter().any(|file| file.path == "earlier.txt"));
    assert!(details.files.iter().any(|file| file.path == "tip.txt"));
    assert!(!details
        .files
        .iter()
        .any(|file| file.path == "base-only.txt"));
    let patch = git_branch_manager::git::commit_details::load_commit_file_diff(
        dir,
        &details,
        &details.files[0],
    )
    .expect("aggregate patch");
    assert!(!patch.patch.is_empty());
}
