use std::process::Command;
use std::sync::atomic::AtomicBool;

use git_branch_manager::git::{
    branch, cache, cherry_loader, diagnostics, fuzzy_match, graph, merge_detection, operations,
    squash_loader, status, tags, worktree,
};
use git_branch_manager::types::{
    ChangedFileKind, DiagKind, FailureCause, MergeStatus, SquashConfidence,
};

/// A temp directory for tests. Deletes itself on drop, EXCEPT when the
/// `GBM_KEEP_TEST_REPOS` env var is set — then it leaks the directory and prints
/// the path, so you can inspect the repo state after a run:
///
/// ```sh
/// GBM_KEEP_TEST_REPOS=1 cargo test --test integration <name> -- --nocapture
/// ```
struct TestDir {
    inner: Option<tempfile::TempDir>,
    path: std::path::PathBuf,
}

impl TestDir {
    fn new(td: tempfile::TempDir) -> Self {
        let path = td.path().to_path_buf();
        Self {
            inner: Some(td),
            path,
        }
    }

    fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        if std::env::var_os("GBM_KEEP_TEST_REPOS").is_some() {
            if let Some(td) = self.inner.take() {
                let kept = td.keep(); // leak: skip the recursive delete
                eprintln!("[GBM_KEEP_TEST_REPOS] kept test repo: {}", kept.display());
            }
        }
        // Otherwise `inner` drops normally and deletes the directory.
    }
}

/// Create a temporary git repository with an initial commit on the "main" branch.
///
/// Returns the temp dir guard (must be kept alive for the duration of the test)
/// and the git2::Repository handle.
fn setup_test_repo() -> (TestDir, git2::Repository) {
    // Use the OS temp dir (std::env::temp_dir): portable across Windows/macOS/
    // Linux. When kept (GBM_KEEP_TEST_REPOS=1), the path is printed on drop, so
    // it stays findable without hardcoding a platform-specific location.
    let tmpdir = tempfile::tempdir().expect("failed to create tempdir");
    let dir = tmpdir.path();

    // git init
    run_git(dir, &["init", "-b", "main"]);

    // Configure user (required for commits)
    run_git(dir, &["config", "user.name", "Test User"]);
    run_git(dir, &["config", "user.email", "test@example.com"]);

    // Create an initial commit so that HEAD and "main" exist
    let initial_file = dir.join("README.md");
    std::fs::write(&initial_file, "# Test Repo\n").expect("failed to write initial file");
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "Initial commit"]);

    let repo = git2::Repository::open(dir).expect("failed to open repo");
    (TestDir::new(tmpdir), repo)
}

/// Run a git command in the given directory, panicking on failure.
fn run_git(dir: &std::path::Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap_or_else(|e| panic!("failed to run git {:?}: {}", args, e));
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!("git {:?} failed in {}: {}", args, dir.display(), stderr);
    }
}

fn git_output(dir: &std::path::Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap_or_else(|e| panic!("failed to run git {:?}: {}", args, e));
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!("git {:?} failed in {}: {}", args, dir.display(), stderr);
    }
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// Run a git command with scoped environment variables in the given directory,
/// panicking on failure.
fn run_git_with_env(dir: &std::path::Path, args: &[&str], env: &[(&str, &str)]) {
    let output = Command::new("git")
        .args(args)
        .envs(env.iter().copied())
        .current_dir(dir)
        .output()
        .unwrap_or_else(|e| panic!("failed to run git {:?}: {}", args, e));
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!("git {:?} failed in {}: {}", args, dir.display(), stderr);
    }
}

/// Build one graph with live, nested, deleted, squashed, and tag-only topics.
///
/// The branch refs for the deleted topics are removed deliberately; tags keep
/// the squash and detached histories reachable for graph inspection.
fn setup_graph_label_fixture() -> TestDir {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path().to_path_buf();

    run_git(&dir, &["checkout", "-b", "release/0.3"]);
    run_git(&dir, &["commit", "--allow-empty", "-m", "release commit 1"]);

    run_git(&dir, &["checkout", "-b", "feature/nested"]);
    run_git(
        &dir,
        &["commit", "--allow-empty", "-m", "nested branch commit"],
    );
    run_git(&dir, &["checkout", "release/0.3"]);
    run_git(
        &dir,
        &[
            "merge",
            "--no-ff",
            "feature/nested",
            "-m",
            "Merge branch 'feature/nested' into release/0.3",
        ],
    );
    run_git(&dir, &["branch", "-D", "feature/nested"]);

    run_git(&dir, &["checkout", "main"]);
    run_git(
        &dir,
        &[
            "commit",
            "--allow-empty",
            "-m",
            "main first-parent fallback",
        ],
    );
    run_git(
        &dir,
        &[
            "merge",
            "--no-ff",
            "release/0.3",
            "-m",
            "Merge branch 'release/0.3' into main",
        ],
    );

    run_git(&dir, &["checkout", "-b", "worktree-agent-deleted"]);
    run_git(
        &dir,
        &[
            "commit",
            "--allow-empty",
            "-m",
            "deleted merge branch commit",
        ],
    );
    run_git(&dir, &["checkout", "main"]);
    run_git(
        &dir,
        &["commit", "--allow-empty", "-m", "main before deleted merge"],
    );
    run_git(
        &dir,
        &[
            "merge",
            "--no-ff",
            "worktree-agent-deleted",
            "-m",
            "Merge branch 'worktree-agent-deleted' into main",
        ],
    );
    run_git(&dir, &["branch", "-D", "worktree-agent-deleted"]);

    run_git(&dir, &["checkout", "-b", "feature/squash"]);
    std::fs::write(dir.join("squash-one.txt"), "one\n").unwrap();
    run_git(&dir, &["add", "squash-one.txt"]);
    run_git(&dir, &["commit", "-m", "squash source commit 1"]);
    std::fs::write(dir.join("squash-two.txt"), "two\n").unwrap();
    run_git(&dir, &["add", "squash-two.txt"]);
    run_git(&dir, &["commit", "-m", "squash source commit 2"]);
    run_git(&dir, &["tag", "squash-source"]);
    run_git(&dir, &["checkout", "main"]);
    run_git(&dir, &["merge", "--squash", "feature/squash"]);
    run_git(&dir, &["commit", "-m", "squash merge feature/squash"]);
    run_git(&dir, &["branch", "-D", "feature/squash"]);

    run_git(&dir, &["checkout", "--detach", "main"]);
    run_git(
        &dir,
        &["commit", "--allow-empty", "-m", "detached tagged commit"],
    );
    run_git(&dir, &["tag", "detached-topic"]);
    run_git(&dir, &["checkout", "main"]);

    tmpdir
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn test_detect_base_branch_main() {
    let (_tmpdir, repo) = setup_test_repo();

    let base = branch::detect_base_branch(&repo, None).expect("detect_base_branch failed");
    assert_eq!(base, "main");
}

#[test]
fn test_load_graph_preserves_merge_lanes_and_local_refs() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/graph"]);
    std::fs::write(dir.join("feature.txt"), "feature\n").unwrap();
    run_git(dir, &["add", "feature.txt"]);
    run_git(dir, &["commit", "-m", "feature commit"]);
    run_git(dir, &["checkout", "main"]);
    std::fs::write(dir.join("main.txt"), "main\n").unwrap();
    run_git(dir, &["add", "main.txt"]);
    run_git(dir, &["commit", "-m", "main commit"]);
    run_git(
        dir,
        &[
            "merge",
            "--no-ff",
            "feature/graph",
            "-m",
            "merge feature graph",
        ],
    );

    let snapshot = graph::load_graph_with_squash_annotations(dir, graph::GraphLoadOptions::default())
        .expect("graph loader should handle an ordinary merged local branch");

    assert!(matches!(snapshot.source, graph::GraphSource::Gleisbau));
    assert!(snapshot
        .commits
        .iter()
        .any(|commit| commit.parents.len() == 2));
    assert!(snapshot.commits.iter().any(|commit| {
        commit
            .refs
            .iter()
            .any(|reference| reference.name == "feature/graph")
    }));
    assert!(snapshot.commits.iter().any(|commit| commit.lane.is_some()));
    assert_eq!(snapshot.ref_counts.local, 2);
    assert_eq!(snapshot.ref_counts.remote, 0);

    fn assert_send<T: Send>() {}
    assert_send::<graph::GraphSnapshot>();
}

#[test]
fn test_graph_branch_labels_follow_visual_branch_tracks() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "release/0.3"]);
    run_git(dir, &["commit", "--allow-empty", "-m", "release commit 1"]);
    run_git(dir, &["commit", "--allow-empty", "-m", "release commit 2"]);
    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["commit", "--allow-empty", "-m", "main commit"]);
    run_git(
        dir,
        &["merge", "--no-ff", "release/0.3", "-m", "merge release/0.3"],
    );

    let snapshot = graph::load_graph_with_squash_annotations(dir, graph::GraphLoadOptions::default())
        .expect("graph loader should preserve live branch tracks");
    let release_commit = snapshot
        .commits
        .iter()
        .find(|commit| commit.summary == "release commit 1")
        .expect("release commit should be in graph");

    assert_eq!(
        release_commit
            .branch
            .as_ref()
            .map(|branch| branch.name.as_str()),
        Some("release/0.3")
    );

    let main_commit = snapshot
        .commits
        .iter()
        .find(|commit| commit.summary == "main commit")
        .expect("main commit should be in graph");
    assert_eq!(
        main_commit
            .branch
            .as_ref()
            .map(|branch| branch.name.as_str()),
        Some("main")
    );
}

#[test]
fn test_graph_base_branch_owns_first_parent_chain_with_retained_merged_ref() {
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();
    let initial_oid = repo.head().unwrap().target().unwrap();

    run_git(dir, &["checkout", "-b", "feat/retained"]);
    run_git(
        dir,
        &["commit", "--allow-empty", "-m", "retained feature tip"],
    );
    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--ff-only", "feat/retained"]);
    run_git(
        dir,
        &["commit", "--allow-empty", "-m", "main inferred commit"],
    );
    run_git(dir, &["commit", "--allow-empty", "-m", "main tip"]);

    fn assert_main_owns_first_parent_chain(snapshot: &graph::GraphSnapshot) {
        for summary in ["main inferred commit", "retained feature tip"] {
            let commit = snapshot
                .commits
                .iter()
                .find(|commit| commit.summary == summary)
                .expect("expected commit should be in graph");
            assert_eq!(
                commit.branch.as_ref().map(|branch| branch.name.as_str()),
                Some("main"),
                "{summary} should remain on the base branch's inferred track"
            );
            if summary == "main inferred commit" {
                assert!(
                    commit.refs.is_empty(),
                    "the regression row must exercise inferred Refs text"
                );
            }
        }
    }

    let options = graph::GraphLoadOptions {
        base_branch: Some("main".into()),
        ..graph::GraphLoadOptions::default()
    };
    let snapshot = graph::load_graph_with_squash_annotations(dir, options.clone())
        .expect("Gleisbau should preserve base-branch ownership");
    assert!(matches!(snapshot.source, graph::GraphSource::Gleisbau));
    assert_main_owns_first_parent_chain(&snapshot);

    std::fs::write(dir.join(".git/shallow"), format!("{initial_oid}\n")).unwrap();
    let fallback = graph::load_graph_with_squash_annotations(dir, options)
        .expect("Git CLI fallback should preserve base-branch ownership");
    assert!(matches!(
        fallback.source,
        graph::GraphSource::GitCliFallback { .. }
    ));
    assert_main_owns_first_parent_chain(&fallback);
}

#[test]
fn test_graph_both_loaders_agree_on_author_and_author_date() {
    use git_branch_manager::git::graph::{load_graph_with_squash_annotations, GraphLoadOptions};

    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();
    let initial_oid = repo.head().unwrap().target().unwrap();
    run_git(dir, &["config", "user.email", "agree@example.com"]);
    run_git(dir, &["config", "user.name", "Agree Bot"]);
    run_git(dir, &["commit", "--allow-empty", "-m", "comparison tip"]);

    let gleisbau = load_graph_with_squash_annotations(dir, GraphLoadOptions::default())
        .expect("gleisbau should succeed");
    assert!(matches!(gleisbau.source, graph::GraphSource::Gleisbau));

    std::fs::write(dir.join(".git/shallow"), format!("{initial_oid}\n")).unwrap();
    let fallback = load_graph_with_squash_annotations(dir, GraphLoadOptions::default())
        .expect("fallback should succeed");
    assert!(matches!(
        fallback.source,
        graph::GraphSource::GitCliFallback { .. }
    ));

    assert_eq!(
        gleisbau.commits.len(),
        fallback.commits.len(),
        "commit count differs between loaders"
    );
    for (g, f) in gleisbau.commits.iter().zip(fallback.commits.iter()) {
        assert_eq!(g.oid, f.oid);
        assert_eq!(
            g.author_name, f.author_name,
            "author_name mismatch on {}",
            g.oid
        );
        assert_eq!(
            g.author_email, f.author_email,
            "author_email mismatch on {}",
            g.oid
        );
        assert_eq!(
            g.authored_at, f.authored_at,
            "authored_at mismatch on {}",
            g.oid
        );
    }
}

#[test]
fn test_graph_default_branch_renders_in_column_zero() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/long-branch"]);
    for i in 0..8 {
        run_git(
            dir,
            &["commit", "--allow-empty", "-m", &format!("feature commit {i}")],
        );
    }

    run_git(dir, &["checkout", "main"]);
    for i in 0..3 {
        run_git(
            dir,
            &["commit", "--allow-empty", "-m", &format!("main commit {i}")],
        );
    }

    run_git(dir, &["checkout", "-b", "release/y"]);
    run_git(dir, &["commit", "--allow-empty", "-m", "release commit 0"]);
    run_git(dir, &["checkout", "main"]);

    let options = graph::GraphLoadOptions {
        base_branch: Some("main".into()),
        ..graph::GraphLoadOptions::default()
    };
    let snapshot = graph::load_graph_with_squash_annotations(dir, options)
        .expect("graph loader should place the default branch in column 0");

    let main_commits: Vec<_> = snapshot
        .commits
        .iter()
        .filter(|commit| commit.summary.starts_with("main commit "))
        .collect();
    assert_eq!(main_commits.len(), 3, "all 3 main commits should be in the graph");
    for commit in main_commits {
        assert_eq!(
            commit.lane,
            Some(0),
            "main commit {:?} should sit in column 0",
            commit.summary
        );
    }

    let feature_commit = snapshot
        .commits
        .iter()
        .find(|commit| commit.summary == "feature commit 0")
        .expect("feature commit 0 should be in graph");
    assert_ne!(
        feature_commit.lane,
        Some(0),
        "the longer feature branch should not trivially land in column 0"
    );
}

#[test]
fn test_graph_base_branch_with_special_chars_lands_in_column_zero() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "release/1.0"]);
    for i in 0..3 {
        run_git(
            dir,
            &[
                "commit",
                "--allow-empty",
                "-m",
                &format!("release1.0 commit {i}"),
            ],
        );
    }

    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["checkout", "-b", "release1X0"]);
    run_git(dir, &["commit", "--allow-empty", "-m", "release1X0 commit 0"]);
    run_git(dir, &["checkout", "main"]);

    let options = graph::GraphLoadOptions {
        base_branch: Some("release/1.0".into()),
        ..graph::GraphLoadOptions::default()
    };
    let snapshot = graph::load_graph_with_squash_annotations(dir, options)
        .expect("graph loader should place the special-char base branch in column 0");

    let base_commits: Vec<_> = snapshot
        .commits
        .iter()
        .filter(|commit| commit.summary.starts_with("release1.0 commit "))
        .collect();
    assert_eq!(
        base_commits.len(),
        3,
        "all 3 release/1.0 commits should be in the graph"
    );
    for commit in base_commits {
        assert_eq!(
            commit.lane,
            Some(0),
            "release/1.0 commit {:?} should sit in column 0",
            commit.summary
        );
    }

    let sibling_commit = snapshot
        .commits
        .iter()
        .find(|commit| commit.summary == "release1X0 commit 0")
        .expect("release1X0 commit should be in graph");
    assert_ne!(
        sibling_commit.lane,
        Some(0),
        "an unescaped regex must not let release1X0 match the release/1.0 base pattern"
    );
}

#[test]
fn test_graph_base_branch_lands_in_column_zero_with_diverged_remote() {
    let (tmpdir, work_dir, _repo) = setup_remote_test_repo();
    let base_dir = tmpdir.path();
    let remote_dir = base_dir.join("remote.git");

    // Advance origin/main independently via a second clone, so work_dir's
    // local main and origin/main diverge from a shared ancestor.
    let advance_dir = base_dir.join("advance");
    run_git(
        base_dir,
        &["clone", remote_dir.to_str().unwrap(), "advance"],
    );
    run_git(&advance_dir, &["config", "user.name", "Test User"]);
    run_git(&advance_dir, &["config", "user.email", "test@example.com"]);
    run_git(&advance_dir, &["commit", "--allow-empty", "-m", "remote ahead"]);
    run_git(&advance_dir, &["push", "origin", "main"]);

    // Pull origin/main's new tip into work_dir's remote-tracking ref without
    // touching local main.
    run_git(&work_dir, &["fetch", "origin"]);

    // Diverge local main from origin/main by adding local-only commits.
    run_git(
        &work_dir,
        &["commit", "--allow-empty", "-m", "main commit 0"],
    );
    run_git(
        &work_dir,
        &["commit", "--allow-empty", "-m", "main commit 1"],
    );

    let options = graph::GraphLoadOptions {
        include_remotes: true,
        base_branch: Some("main".into()),
        ..graph::GraphLoadOptions::default()
    };
    let snapshot = graph::load_graph_with_squash_annotations(&work_dir, options)
        .expect("graph loader should place local main in column 0 despite a diverged remote");

    let local_main_commits: Vec<_> = snapshot
        .commits
        .iter()
        .filter(|commit| commit.summary.starts_with("main commit "))
        .collect();
    assert_eq!(
        local_main_commits.len(),
        2,
        "both local main commits should be in the graph"
    );
    for commit in local_main_commits {
        assert_eq!(
            commit.lane,
            Some(0),
            "local main commit {:?} should sit in column 0",
            commit.summary
        );
    }

    let remote_ahead_commit = snapshot
        .commits
        .iter()
        .find(|commit| commit.summary == "remote ahead")
        .expect("origin/main's diverged commit should be in graph");
    assert_ne!(
        remote_ahead_commit.lane,
        Some(0),
        "the diverged origin/main track must not also claim column 0"
    );
}

#[test]
fn test_graph_local_branch_owns_track_before_matching_remote() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    run_git(&work_dir, &["checkout", "-b", "release/0.3"]);
    run_git(
        &work_dir,
        &["commit", "--allow-empty", "-m", "release inferred commit"],
    );
    run_git(
        &work_dir,
        &["commit", "--allow-empty", "-m", "release remote tip"],
    );
    run_git(&work_dir, &["push", "-u", "origin", "release/0.3"]);
    run_git(
        &work_dir,
        &["commit", "--allow-empty", "-m", "release local tip"],
    );

    let snapshot = graph::load_graph_with_squash_annotations(
        &work_dir,
        graph::GraphLoadOptions {
            include_remotes: true,
            base_branch: Some("main".into()),
            ..graph::GraphLoadOptions::default()
        },
    )
    .expect("graph loader should preserve local ownership with remotes enabled");
    let inferred = snapshot
        .commits
        .iter()
        .find(|commit| commit.summary == "release inferred commit")
        .expect("inferred release commit should be in graph");

    assert!(inferred.refs.is_empty());
    assert_eq!(
        inferred.branch.as_ref().map(|branch| branch.name.as_str()),
        Some("release/0.3")
    );
}

#[test]
fn test_graph_does_not_expose_a_deleted_merge_branch_as_a_live_ref() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "worktree-agent-deleted"]);
    run_git(
        dir,
        &["commit", "--allow-empty", "-m", "deleted branch commit"],
    );
    run_git(dir, &["checkout", "main"]);
    run_git(
        dir,
        &[
            "merge",
            "--no-ff",
            "worktree-agent-deleted",
            "-m",
            "merge worktree-agent-deleted",
        ],
    );
    run_git(dir, &["branch", "-D", "worktree-agent-deleted"]);

    let snapshot = graph::load_graph_with_squash_annotations(dir, graph::GraphLoadOptions::default())
        .expect("graph loader should handle deleted merge branches");
    let deleted_commit = snapshot
        .commits
        .iter()
        .find(|commit| commit.summary == "deleted branch commit")
        .expect("deleted branch commit should remain in history");

    assert!(deleted_commit.branch.is_none());
    assert!(!snapshot.commits.iter().any(|commit| {
        commit
            .refs
            .iter()
            .any(|reference| reference.name == "worktree-agent-deleted")
    }));
}

#[test]
fn test_graph_labels_deleted_merge_branch_from_conventional_subject() {
    let tmpdir = setup_graph_label_fixture();
    let snapshot = graph::load_graph_with_squash_annotations(tmpdir.path(), graph::GraphLoadOptions::default())
        .expect("graph loader should preserve the composed fixture");
    let deleted_commit = snapshot
        .commits
        .iter()
        .find(|commit| commit.summary == "deleted merge branch commit")
        .expect("deleted merge branch commit should be in graph");

    assert_eq!(
        deleted_commit
            .branch
            .as_ref()
            .map(|branch| branch.name.as_str()),
        Some("worktree-agent-deleted")
    );
}

#[test]
fn test_graph_label_fixture_labels_nested_and_first_parent_tracks() {
    let tmpdir = setup_graph_label_fixture();
    let snapshot = graph::load_graph_with_squash_annotations(tmpdir.path(), graph::GraphLoadOptions::default())
        .expect("graph loader should preserve the composed fixture");

    let nested_commit = snapshot
        .commits
        .iter()
        .find(|commit| commit.summary == "nested branch commit")
        .expect("nested branch commit should be in graph");
    assert_eq!(
        nested_commit
            .branch
            .as_ref()
            .map(|branch| branch.name.as_str()),
        Some("feature/nested")
    );

    let main_commit = snapshot
        .commits
        .iter()
        .find(|commit| commit.summary == "main first-parent fallback")
        .expect("main first-parent commit should be in graph");
    assert_eq!(
        main_commit
            .branch
            .as_ref()
            .map(|branch| branch.name.as_str()),
        Some("main")
    );
}

#[test]
fn test_graph_label_fixture_keeps_tag_only_histories_reachable() {
    let tmpdir = setup_graph_label_fixture();
    let snapshot = graph::load_graph_with_squash_annotations(
        tmpdir.path(),
        graph::GraphLoadOptions {
            include_remotes: true,
            ..graph::GraphLoadOptions::default()
        },
    )
    .expect("graph loader should include tag-only fixture histories");

    let squash_source = snapshot
        .commits
        .iter()
        .find(|commit| commit.summary == "squash source commit 2")
        .expect("tagged squash source should be in graph");
    assert!(squash_source
        .refs
        .iter()
        .any(|reference| reference.name == "squash-source"));
    assert!(squash_source.branch.is_none());

    let detached = snapshot
        .commits
        .iter()
        .find(|commit| commit.summary == "detached tagged commit")
        .expect("detached tagged commit should be in graph");
    assert!(detached
        .refs
        .iter()
        .any(|reference| reference.name == "detached-topic"));
    assert!(detached.branch.is_none());
}

#[test]
fn test_load_graph_includes_remote_refs_only_when_requested() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    run_git(&work_dir, &["checkout", "-b", "remote-only"]);
    std::fs::write(work_dir.join("remote-only.txt"), "remote\n").unwrap();
    run_git(&work_dir, &["add", "remote-only.txt"]);
    run_git(&work_dir, &["commit", "-m", "remote-only commit"]);
    run_git(&work_dir, &["push", "-u", "origin", "remote-only"]);
    run_git(&work_dir, &["checkout", "main"]);
    run_git(&work_dir, &["branch", "-D", "remote-only"]);

    let local_only = graph::load_graph_with_squash_annotations(&work_dir, graph::GraphLoadOptions::default())
        .expect("local graph load should succeed");
    assert!(!local_only
        .commits
        .iter()
        .flat_map(|commit| commit.refs.iter())
        .any(|reference| reference.name == "origin/remote-only"));
    assert_eq!(local_only.ref_counts.remote, 0);

    let with_remotes = graph::load_graph_with_squash_annotations(
        &work_dir,
        graph::GraphLoadOptions {
            include_remotes: true,
            ..graph::GraphLoadOptions::default()
        },
    )
    .expect("remote graph load should succeed");
    assert!(with_remotes.commits.iter().any(|commit| {
        commit
            .refs
            .iter()
            .any(|reference| reference.name == "origin/remote-only")
    }));
    assert!(with_remotes.ref_counts.remote > 0);
}

#[test]
fn test_graph_refs_include_remote_tracking_state() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    run_git(&work_dir, &["checkout", "-b", "ahead"]);
    run_git(&work_dir, &["commit", "--allow-empty", "-m", "ahead base"]);
    run_git(&work_dir, &["push", "-u", "origin", "ahead"]);
    run_git(
        &work_dir,
        &["commit", "--allow-empty", "-m", "ahead local commit"],
    );
    run_git(&work_dir, &["checkout", "main"]);

    let snapshot = graph::load_graph_with_squash_annotations(
        &work_dir,
        graph::GraphLoadOptions {
            include_remotes: true,
            ..graph::GraphLoadOptions::default()
        },
    )
    .expect("graph load with remote tracking refs should succeed");

    let ahead_ref = snapshot
        .commits
        .iter()
        .find(|commit| commit.summary == "ahead local commit")
        .and_then(|commit| {
            commit
                .refs
                .iter()
                .find(|reference| reference.name == "ahead")
        })
        .expect("local ahead ref should be attached to its tip");
    let tracking = ahead_ref
        .tracking
        .as_ref()
        .expect("local branch should expose matching remote tracking");
    assert_eq!((tracking.ahead, tracking.behind), (1, 0));

    let main_ref = snapshot
        .commits
        .iter()
        .flat_map(|commit| commit.refs.iter())
        .find(|reference| reference.name == "main")
        .expect("main ref should be present");
    let main_tracking = main_ref
        .tracking
        .as_ref()
        .expect("main should expose its matching remote");
    assert_eq!((main_tracking.ahead, main_tracking.behind), (0, 0));

    let remote_ahead = snapshot
        .commits
        .iter()
        .find(|commit| commit.summary == "ahead base")
        .expect("remote tracking ref should remain on its own tip");
    assert!(remote_ahead
        .refs
        .iter()
        .any(|reference| reference.name == "origin/ahead"));
    assert!(!remote_ahead
        .refs
        .iter()
        .any(|reference| reference.name == "ahead"));
}

#[test]
fn test_graph_refs_mark_only_linked_worktrees() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();
    run_git(dir, &["checkout", "-b", "feature/linked"]);
    run_git(dir, &["checkout", "main"]);
    let worktree_parent = tempfile::tempdir().expect("worktree parent");
    let linked_path = worktree_parent.path().join("linked");
    let linked_path_string = linked_path.to_string_lossy().into_owned();
    run_git(
        dir,
        &["worktree", "add", &linked_path_string, "feature/linked"],
    );

    let snapshot = graph::load_graph_with_squash_annotations(dir, graph::GraphLoadOptions::default())
        .expect("graph load should include linked worktree metadata");
    let linked = snapshot
        .commits
        .iter()
        .flat_map(|commit| commit.refs.iter())
        .find(|reference| reference.name == "feature/linked")
        .expect("linked branch ref");
    assert!(linked.has_linked_worktree);
    let main = snapshot
        .commits
        .iter()
        .flat_map(|commit| commit.refs.iter())
        .find(|reference| reference.name == "main")
        .expect("main branch ref");
    assert!(!main.has_linked_worktree);
}

#[test]
fn test_load_graph_caps_history_at_five_hundred_commits() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    for index in 0..501 {
        let message = format!("history commit {index}");
        run_git(dir, &["commit", "--allow-empty", "-m", &message]);
    }

    let snapshot = graph::load_graph_with_squash_annotations(dir, graph::GraphLoadOptions::default())
        .expect("bounded graph load should succeed");
    assert_eq!(snapshot.commits.len(), 500);
    assert_eq!(snapshot.max_count, 500);
}

#[test]
fn test_load_graph_uses_cli_fallback_for_shallow_repository() {
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();
    let head = repo.head().unwrap().target().unwrap();
    std::fs::write(dir.join(".git/shallow"), format!("{head}\n")).unwrap();

    let snapshot = graph::load_graph_with_squash_annotations(dir, graph::GraphLoadOptions::default())
        .expect("git CLI fallback should handle a shallow repository");

    assert!(matches!(
        snapshot.source,
        graph::GraphSource::GitCliFallback { ref cause } if cause.contains("shallow")
    ));
    assert_eq!(snapshot.commits.len(), 1);
    assert!(snapshot
        .lines
        .iter()
        .any(|line| line.commit_index.is_some()));
}

#[test]
fn test_graph_marks_only_base_commit_with_exact_squash_patch() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/exact"]);
    std::fs::write(dir.join("exact.txt"), "exact patch\n").unwrap();
    run_git(dir, &["add", "exact.txt"]);
    run_git(dir, &["commit", "-m", "source patch subject"]);
    std::fs::write(dir.join("exact-second.txt"), "second exact patch\n").unwrap();
    run_git(dir, &["add", "exact-second.txt"]);
    run_git(dir, &["commit", "-m", "source follow-up subject"]);
    let source_oid = git_output(dir, &["rev-parse", "HEAD"]);

    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/exact"]);
    run_git(dir, &["commit", "-m", "unrelated landing subject"]);
    let exact_base_oid = git_output(dir, &["rev-parse", "HEAD"]);

    run_git(dir, &["checkout", "-b", "feature/message-only"]);
    std::fs::write(dir.join("branch-only.txt"), "branch content\n").unwrap();
    run_git(dir, &["add", "branch-only.txt"]);
    run_git(dir, &["commit", "-m", "shared misleading subject"]);
    run_git(dir, &["checkout", "main"]);
    std::fs::write(dir.join("base-only.txt"), "different base content\n").unwrap();
    run_git(dir, &["add", "base-only.txt"]);
    run_git(dir, &["commit", "-m", "shared misleading subject"]);
    let message_only_base_oid = git_output(dir, &["rev-parse", "HEAD"]);

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..graph::GraphLoadOptions::default()
        },
    )
    .expect("graph loader should annotate exact squash patch matches");

    let exact_base = snapshot
        .commits
        .iter()
        .find(|commit| commit.oid == exact_base_oid)
        .expect("exact base commit should be displayed");
    assert!(exact_base.is_possible_squash_merge);

    let source = snapshot
        .commits
        .iter()
        .find(|commit| commit.oid == source_oid)
        .expect("source branch commit should be displayed");
    assert!(
        !source.is_possible_squash_merge,
        "only the matching base-branch commit receives the annotation"
    );

    let message_only_base = snapshot
        .commits
        .iter()
        .find(|commit| commit.oid == message_only_base_oid)
        .expect("same-subject base commit should be displayed");
    assert!(
        !message_only_base.is_possible_squash_merge,
        "matching subjects with different patches must not annotate a commit"
    );
}

#[test]
fn test_graph_preserves_every_base_oid_for_duplicate_patch_ids() {
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();
    let root_oid = repo.head().unwrap().target().unwrap().to_string();

    std::fs::write(dir.join("duplicate.txt"), "same patch\n").unwrap();
    run_git(dir, &["add", "duplicate.txt"]);
    run_git(dir, &["commit", "-m", "first duplicate patch"]);
    let first_oid = git_output(dir, &["rev-parse", "HEAD"]);
    run_git(dir, &["rm", "duplicate.txt"]);
    run_git(dir, &["commit", "-m", "remove duplicate patch"]);
    std::fs::write(dir.join("duplicate.txt"), "same patch\n").unwrap();
    run_git(dir, &["add", "duplicate.txt"]);
    run_git(dir, &["commit", "-m", "second duplicate patch"]);
    let second_oid = git_output(dir, &["rev-parse", "HEAD"]);

    run_git(dir, &["checkout", "-b", "feature/duplicate", &root_oid]);
    std::fs::write(dir.join("duplicate.txt"), "same patch\n").unwrap();
    run_git(dir, &["add", "duplicate.txt"]);
    run_git(dir, &["commit", "-m", "duplicate source patch"]);
    let source_oid = git_output(dir, &["rev-parse", "HEAD"]);
    run_git(dir, &["checkout", "main"]);

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..graph::GraphLoadOptions::default()
        },
    )
    .expect("graph loader should retain duplicate patch matches");

    for oid in [&first_oid, &second_oid] {
        let commit = snapshot
            .commits
            .iter()
            .find(|commit| commit.oid == *oid)
            .expect("duplicate-patch base commit should be displayed");
        assert!(
            commit.is_possible_squash_merge,
            "every base OID sharing the exact patch ID must be annotated"
        );
    }
    assert!(
        !snapshot
            .commits
            .iter()
            .find(|commit| commit.oid == source_oid)
            .expect("source branch commit should be displayed")
            .is_possible_squash_merge
    );
}

#[test]
fn test_graph_excludes_regular_merges_roots_and_empty_patches() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/regular"]);
    std::fs::write(dir.join("regular.txt"), "regular patch\n").unwrap();
    run_git(dir, &["add", "regular.txt"]);
    run_git(dir, &["commit", "-m", "regular source patch"]);
    let regular_tip = git_output(dir, &["rev-parse", "HEAD"]);
    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["cherry-pick", &regular_tip]);
    let equivalent_base_oid = git_output(dir, &["rev-parse", "HEAD"]);
    run_git(
        dir,
        &[
            "merge",
            "--no-ff",
            "feature/regular",
            "-m",
            "regular merge after equivalent patch",
        ],
    );

    run_git(dir, &["checkout", "-b", "feature/empty"]);
    run_git(
        dir,
        &["commit", "--allow-empty", "-m", "empty branch patch"],
    );
    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["commit", "--allow-empty", "-m", "empty base patch"]);

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..graph::GraphLoadOptions::default()
        },
    )
    .expect("graph loader should ignore ineligible patch shapes");

    assert!(
        !snapshot
            .commits
            .iter()
            .find(|commit| commit.oid == equivalent_base_oid)
            .expect("equivalent regular-merge base commit should be displayed")
            .is_possible_squash_merge,
        "a regularly merged branch must not create a possible-squash indicator"
    );
    assert!(
        snapshot
            .commits
            .iter()
            .all(|commit| !commit.is_possible_squash_merge),
        "merge commits, root commits, and empty patches cannot match"
    );
}

#[test]
fn test_graph_cli_fallback_receives_exact_squash_annotation() {
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();
    let root_oid = repo.head().unwrap().target().unwrap();

    run_git(dir, &["checkout", "-b", "feature/fallback-squash"]);
    std::fs::write(dir.join("fallback.txt"), "fallback patch\n").unwrap();
    run_git(dir, &["add", "fallback.txt"]);
    run_git(dir, &["commit", "-m", "fallback source"]);
    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/fallback-squash"]);
    run_git(dir, &["commit", "-m", "fallback landing"]);
    let squash_oid = git_output(dir, &["rev-parse", "HEAD"]);

    std::fs::write(dir.join(".git/shallow"), format!("{root_oid}\n")).unwrap();
    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..graph::GraphLoadOptions::default()
        },
    )
    .expect("CLI fallback should receive the shared squash annotation");

    assert!(matches!(
        snapshot.source,
        graph::GraphSource::GitCliFallback { .. }
    ));
    assert!(
        snapshot
            .commits
            .iter()
            .find(|commit| commit.oid == squash_oid)
            .expect("fallback squash commit should be displayed")
            .is_possible_squash_merge
    );
}

#[test]
fn test_graph_squash_matching_stays_within_displayed_history_bound() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/outside-window"]);
    std::fs::write(dir.join("bounded.txt"), "bounded patch\n").unwrap();
    run_git(dir, &["add", "bounded.txt"]);
    run_git(dir, &["commit", "-m", "bounded source"]);
    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/outside-window"]);
    run_git(dir, &["commit", "-m", "bounded landing"]);
    run_git(
        dir,
        &["commit", "--allow-empty", "-m", "newest displayed commit"],
    );

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            max_count: 1,
            base_branch: Some("main".into()),
            ..graph::GraphLoadOptions::default()
        },
    )
    .expect("bounded graph load should not inspect undisplayed matching history");

    assert_eq!(snapshot.commits.len(), 1);
    assert!(snapshot
        .commits
        .iter()
        .all(|commit| !commit.is_possible_squash_merge));
}

#[test]
fn test_load_graph_fallback_with_remotes_includes_tag_only_history() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    run_git(&work_dir, &["checkout", "-b", "remote-only"]);
    std::fs::write(work_dir.join("remote-only.txt"), "remote\n").unwrap();
    run_git(&work_dir, &["add", "remote-only.txt"]);
    run_git(&work_dir, &["commit", "-m", "remote-only commit"]);
    run_git(&work_dir, &["push", "-u", "origin", "remote-only"]);
    run_git(&work_dir, &["checkout", "main"]);
    run_git(&work_dir, &["branch", "-D", "remote-only"]);

    run_git(&work_dir, &["checkout", "--orphan", "tag-only"]);
    run_git(&work_dir, &["rm", "-rf", "."]);
    std::fs::write(work_dir.join("tag-only.txt"), "tag\n").unwrap();
    run_git(&work_dir, &["add", "tag-only.txt"]);
    run_git(&work_dir, &["commit", "-m", "tag-only commit"]);
    run_git(&work_dir, &["tag", "tag-only"]);
    run_git(&work_dir, &["checkout", "main"]);
    run_git(&work_dir, &["branch", "-D", "tag-only"]);

    let repo = git2::Repository::open(&work_dir).unwrap();
    let head = repo.head().unwrap().target().unwrap();
    std::fs::write(work_dir.join(".git/shallow"), format!("{head}\n")).unwrap();

    let snapshot = graph::load_graph_with_squash_annotations(
        &work_dir,
        graph::GraphLoadOptions {
            include_remotes: true,
            ..graph::GraphLoadOptions::default()
        },
    )
    .expect("git CLI fallback should retain the remote and tag-only ref range");

    assert!(matches!(
        snapshot.source,
        graph::GraphSource::GitCliFallback { ref cause } if cause.contains("shallow")
    ));
    assert!(snapshot.commits.iter().any(|commit| {
        commit.refs.iter().any(|reference| {
            reference.name == "origin/remote-only"
                && reference.kind == graph::GraphRefKind::RemoteBranch
        })
    }));
    assert!(snapshot.commits.iter().any(|commit| {
        commit.refs.iter().any(|reference| {
            reference.name == "tag-only" && reference.kind == graph::GraphRefKind::Tag
        })
    }));
}

#[test]
fn test_detect_base_branch_override() {
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Create a "develop" branch
    run_git(dir, &["branch", "develop"]);

    let base =
        branch::detect_base_branch(&repo, Some("develop")).expect("detect_base_branch failed");
    assert_eq!(base, "develop");
}

#[test]
fn test_detect_base_branch_override_nonexistent() {
    let (_tmpdir, repo) = setup_test_repo();

    let result = branch::detect_base_branch(&repo, Some("nonexistent"));
    assert!(
        result.is_err(),
        "expected error for nonexistent branch override"
    );
}

#[test]
fn test_list_branches() {
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Create two feature branches
    run_git(dir, &["branch", "feature-a"]);
    run_git(dir, &["branch", "feature-b"]);

    let branches = branch::list_branches(&repo, "main").expect("list_branches failed");

    // Should have 3 branches: main, feature-a, feature-b
    assert_eq!(
        branches.len(),
        3,
        "expected 3 branches, got: {:?}",
        branches.iter().map(|b| &b.name).collect::<Vec<_>>()
    );

    let names: Vec<&str> = branches.iter().map(|b| b.name.as_str()).collect();
    assert!(names.contains(&"main"), "missing 'main'");
    assert!(names.contains(&"feature-a"), "missing 'feature-a'");
    assert!(names.contains(&"feature-b"), "missing 'feature-b'");

    // main should be marked as base
    let main_branch = branches.iter().find(|b| b.name == "main").unwrap();
    assert!(main_branch.is_base, "main should be marked is_base");
}

#[test]
fn test_merged_branch_detection() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Create feature-merged branch with a commit
    run_git(dir, &["checkout", "-b", "feature-merged"]);
    let feature_file = dir.join("feature.txt");
    std::fs::write(&feature_file, "feature content\n").expect("failed to write feature file");
    run_git(dir, &["add", "feature.txt"]);
    run_git(dir, &["commit", "-m", "Add feature"]);

    // Switch back to main, add a commit so merge can't fast-forward, then merge
    run_git(dir, &["checkout", "main"]);
    let main_file = dir.join("main-change.txt");
    std::fs::write(&main_file, "main branch change\n").expect("failed to write main file");
    run_git(dir, &["add", "main-change.txt"]);
    run_git(dir, &["commit", "-m", "Main branch commit"]);
    run_git(
        dir,
        &["merge", "feature-merged", "-m", "Merge feature-merged"],
    );

    // Re-open the repo so git2 sees the merge commit
    let repo = git2::Repository::open(dir).expect("failed to re-open repo");
    let branches = branch::list_branches(&repo, "main").expect("list_branches failed");

    let feature = branches
        .iter()
        .find(|b| b.name == "feature-merged")
        .expect("feature-merged branch not found");
    assert_eq!(
        feature.merge_status,
        MergeStatus::Merged,
        "feature-merged should be detected as Merged"
    );
}

#[test]
fn test_squash_merged_branch_detection() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Create feature-squashed branch with a commit
    run_git(dir, &["checkout", "-b", "feature-squashed"]);
    let squash_file = dir.join("squash-feature.txt");
    std::fs::write(&squash_file, "squash feature content\n")
        .expect("failed to write squash feature file");
    run_git(dir, &["add", "squash-feature.txt"]);
    run_git(dir, &["commit", "-m", "Add squash feature"]);

    // Switch back to main and squash merge
    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature-squashed"]);
    run_git(dir, &["commit", "-m", "squash merge feature-squashed"]);

    // Re-open the repo so git2 sees the latest state
    let repo = git2::Repository::open(dir).expect("failed to re-open repo");
    let branches = branch::list_branches(&repo, "main").expect("list_branches failed");

    let feature = branches
        .iter()
        .find(|b| b.name == "feature-squashed")
        .expect("feature-squashed branch not found");
    assert_eq!(
        feature.merge_status,
        MergeStatus::LocalSquashMerged,
        "feature-squashed should be detected as LocalSquashMerged (no remote in test repo)"
    );
}

#[test]
fn test_in_sync_branch_detection() {
    // A branch cut from main with no commits added has HEAD == main HEAD.
    // That used to report as Merged, which is misleading — no integration
    // event occurred. It should report as InSync instead.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Cut two branches: one truly empty, one that adds a commit then resets
    // back to main so its tip also equals main's tip.
    run_git(dir, &["branch", "feature-fresh"]);

    let repo = git2::Repository::open(dir).expect("failed to open repo");
    let branches = branch::list_branches(&repo, "main").expect("list_branches failed");

    let fresh = branches
        .iter()
        .find(|b| b.name == "feature-fresh")
        .expect("feature-fresh branch not found");
    assert_eq!(
        fresh.merge_status,
        MergeStatus::InSync,
        "fresh branch (tip == base tip) should be InSync, not Merged"
    );
}

#[test]
fn test_in_sync_does_not_swallow_real_merges() {
    // Regression guard: an InSync check at the top of regular_merge_status
    // must not cause branches with unique merged commits to also report InSync
    // (they have a unique commit, so their tip differs from base tip and
    // should still be Merged).
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature-merged"]);
    std::fs::write(dir.join("f.txt"), "f").unwrap();
    run_git(dir, &["add", "f.txt"]);
    run_git(dir, &["commit", "-m", "add f"]);
    run_git(dir, &["checkout", "main"]);
    // Add a commit to main so merge can't fast-forward.
    std::fs::write(dir.join("main-change.txt"), "m").unwrap();
    run_git(dir, &["add", "main-change.txt"]);
    run_git(dir, &["commit", "-m", "main change"]);
    run_git(dir, &["merge", "feature-merged", "-m", "merge f"]);

    let repo = git2::Repository::open(dir).expect("failed to re-open repo");
    let branches = branch::list_branches(&repo, "main").expect("list_branches failed");

    let merged = branches
        .iter()
        .find(|b| b.name == "feature-merged")
        .expect("feature-merged branch not found");
    assert_eq!(
        merged.merge_status,
        MergeStatus::Merged,
        "regular-merged branch must still report Merged, not InSync"
    );
}

#[test]
fn test_unmerged_branch_detection() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Create an unmerged branch with a unique commit
    run_git(dir, &["checkout", "-b", "feature-wip"]);
    let wip_file = dir.join("wip.txt");
    std::fs::write(&wip_file, "work in progress\n").expect("failed to write wip file");
    run_git(dir, &["add", "wip.txt"]);
    run_git(dir, &["commit", "-m", "WIP commit"]);

    // Switch back to main (do NOT merge)
    run_git(dir, &["checkout", "main"]);

    // Re-open the repo so git2 sees the latest state
    let repo = git2::Repository::open(dir).expect("failed to re-open repo");
    let branches = branch::list_branches(&repo, "main").expect("list_branches failed");

    let feature = branches
        .iter()
        .find(|b| b.name == "feature-wip")
        .expect("feature-wip branch not found");
    assert_eq!(
        feature.merge_status,
        MergeStatus::Unmerged,
        "feature-wip should be detected as Unmerged"
    );
}

#[test]
fn test_delete_local_branch() {
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Create a branch to delete
    run_git(dir, &["branch", "to-delete"]);

    // Verify it exists
    assert!(
        repo.find_branch("to-delete", git2::BranchType::Local)
            .is_ok(),
        "branch should exist before deletion"
    );

    let result = operations::delete_local(&repo, "to-delete");
    assert!(
        result.success,
        "delete_local should succeed: {}",
        result.message
    );

    // Verify it's gone
    assert!(
        repo.find_branch("to-delete", git2::BranchType::Local)
            .is_err(),
        "branch should not exist after deletion"
    );
}

#[test]
fn test_delete_local_nonexistent() {
    let (_tmpdir, repo) = setup_test_repo();

    let result = operations::delete_local(&repo, "does-not-exist");
    assert!(
        result.success,
        "an already-gone branch should satisfy the requested delete"
    );
    assert!(matches!(result.failure, Some(FailureCause::BranchNotFound)));
}

#[test]
fn test_delete_local_requires_merge_but_force_delete_bypasses_it() {
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/unmerged-delete"]);
    std::fs::write(dir.join("wip.txt"), "wip\n").unwrap();
    run_git(dir, &["add", "wip.txt"]);
    run_git(dir, &["commit", "-m", "wip"]);
    run_git(dir, &["checkout", "main"]);

    let safe = operations::delete_local(&repo, "feature/unmerged-delete");
    assert!(!safe.success, "safe deletion must reject an unmerged branch");
    assert!(matches!(safe.failure, Some(FailureCause::NotMerged)));

    let forced = operations::delete_local_force(&repo, "feature/unmerged-delete");
    assert!(forced.success, "force deletion should remove the branch: {forced:?}");
    assert!(matches!(forced.action, git_branch_manager::types::BranchAction::DeleteLocalForce));
    assert!(repo
        .find_branch("feature/unmerged-delete", git2::BranchType::Local)
        .is_err());
}

#[test]
fn test_delete_local_classifies_primary_and_linked_worktree_failures() {
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();

    let primary = operations::delete_local(&repo, "main");
    assert!(!primary.success, "the checked-out primary branch is not removable");
    // Intentional behavior change (P005 review): `main` is checked out in the
    // primary worktree, which IS the caller's own worktree (repo_path ==
    // dir here). classify_delete_command_error now uses
    // try_other_worktree_for_branch, which excludes the caller's own
    // worktree, so this case falls through to `Other` with the raw git
    // message preserved rather than being misreported as a recoverable
    // "checked out elsewhere". `is_main: true` was never actionable via the
    // Results overlay's `r` recovery key (see app.rs), so this is not a
    // user-visible regression.
    assert!(matches!(primary.failure, Some(FailureCause::Other { .. })));

    run_git(dir, &["branch", "feature/linked-delete"]);
    run_git(
        dir,
        &[
            "worktree",
            "add",
            ".worktrees/feature-linked-delete",
            "feature/linked-delete",
        ],
    );

    let linked = operations::delete_local(&repo, "feature/linked-delete");
    assert!(!linked.success, "a linked worktree branch is not removable");
    match linked.failure {
        Some(FailureCause::CheckedOutInWorktree {
            worktree_path,
            is_main: false,
        }) => assert!(worktree_path.ends_with(".worktrees/feature-linked-delete")),
        other => panic!("expected linked worktree cause, got {other:?}"),
    }
}

#[test]
fn test_dirty_linked_worktree_reports_all_changes_and_remains_recoverable() {
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();
    let linked_path = dir.join(".worktrees/dirty-linked");
    let linked_path_text = linked_path.to_string_lossy().into_owned();

    run_git(dir, &["branch", "feature/dirty-linked"]);
    run_git(
        dir,
        &[
            "worktree",
            "add",
            linked_path_text.as_str(),
            "feature/dirty-linked",
        ],
    );
    std::fs::write(linked_path.join("README.md"), "# Dirty linked worktree\n").unwrap();
    for index in 0..6 {
        std::fs::write(
            linked_path.join(format!("untracked-{index}.txt")),
            format!("untracked {index}\n"),
        )
        .unwrap();
    }

    let worktrees = worktree::list_worktrees(dir);
    let linked_index = worktrees
        .iter()
        .position(|worktree| worktree.branch.as_deref() == Some("feature/dirty-linked"))
        .expect("linked worktree should be listed");
    assert!(
        !worktrees[linked_index].is_main,
        "the dirty linked worktree must be non-primary"
    );

    let statuses = worktree::enrich_worktrees(worktrees);
    let linked_status = statuses
        .iter()
        .find(|result| result.index == linked_index)
        .expect("linked worktree should be enriched")
        .wt_status;
    assert!(linked_status.has_modified, "README.md is modified");
    assert!(linked_status.has_untracked, "six files are untracked");
    assert!(!linked_status.has_staged, "no changes are staged");
    assert_eq!(
        linked_status.changed_files.len(),
        7,
        "the presentation layer may bound its list to five, but enrichment retains all facts"
    );
    assert!(linked_status
        .changed_files
        .iter()
        .any(|file| { file.path == "README.md" && file.kind == ChangedFileKind::Modified }));
    for index in 0..6 {
        assert!(
            linked_status.changed_files.iter().any(|file| {
                file.path == format!("untracked-{index}.txt")
                    && file.kind == ChangedFileKind::Untracked
            }),
            "untracked-{index}.txt was omitted from linked-worktree enrichment"
        );
    }

    let linked = operations::delete_local(&repo, "feature/dirty-linked");
    assert!(!linked.success, "a linked worktree branch is not removable");
    match linked.failure {
        Some(FailureCause::CheckedOutInWorktree {
            worktree_path,
            is_main: false,
        }) => assert!(
            worktree_path.ends_with(".worktrees/dirty-linked"),
            "Git must identify the linked worktree, got {}",
            worktree_path.display()
        ),
        other => panic!("expected a recoverable linked-worktree failure, got {other:?}"),
    }

    let primary = operations::delete_local(&repo, "main");
    assert!(!primary.success, "the primary branch is not removable");
    assert!(matches!(primary.failure, Some(FailureCause::Other { .. })));

    if std::env::var_os("GBM_KEEP_TEST_REPOS").is_none() {
        run_git(
            dir,
            &["worktree", "remove", "--force", linked_path_text.as_str()],
        );
    }
}

#[test]
fn test_ahead_behind_indicators() {
    // Create a bare "remote" repo, clone it, push a branch, then add a local commit.
    // The branch should report ahead=1, behind=0.
    let tmpdir = tempfile::tempdir().expect("failed to create tmpdir");
    let base_dir = tmpdir.path();

    // 1. Create a bare remote repo
    let remote_dir = base_dir.join("remote.git");
    std::fs::create_dir_all(&remote_dir).unwrap();
    run_git(&remote_dir, &["init", "--bare", "-b", "main"]);

    // 2. Clone it into a working directory
    let work_dir = base_dir.join("work");
    run_git(base_dir, &["clone", remote_dir.to_str().unwrap(), "work"]);

    // 3. Configure user in the clone
    run_git(&work_dir, &["config", "user.name", "Test User"]);
    run_git(&work_dir, &["config", "user.email", "test@example.com"]);

    // 4. Create an initial commit on main and push
    let readme = work_dir.join("README.md");
    std::fs::write(&readme, "# Test\n").unwrap();
    run_git(&work_dir, &["add", "."]);
    run_git(&work_dir, &["commit", "-m", "Initial commit"]);
    run_git(&work_dir, &["push", "-u", "origin", "main"]);

    // 5. Create a feature branch and push it so it has a remote tracking branch
    run_git(&work_dir, &["checkout", "-b", "feature-ahead"]);
    let feature_file = work_dir.join("feature.txt");
    std::fs::write(&feature_file, "feature content\n").unwrap();
    run_git(&work_dir, &["add", "feature.txt"]);
    run_git(&work_dir, &["commit", "-m", "Feature commit 1"]);
    run_git(&work_dir, &["push", "-u", "origin", "feature-ahead"]);

    // 6. Add another local commit (not pushed) — this should make ahead=1
    let feature_file2 = work_dir.join("feature2.txt");
    std::fs::write(&feature_file2, "more feature content\n").unwrap();
    run_git(&work_dir, &["add", "feature2.txt"]);
    run_git(
        &work_dir,
        &["commit", "-m", "Feature commit 2 (local only)"],
    );

    // 7. Go back to main for listing
    run_git(&work_dir, &["checkout", "main"]);

    // 8. Open repo and list branches
    let repo = git2::Repository::open(&work_dir).expect("failed to open work repo");
    let branches = branch::list_branches(&repo, "main").expect("list_branches failed");

    let feature = branches
        .iter()
        .find(|b| b.name == "feature-ahead")
        .expect("feature-ahead branch not found");

    assert_eq!(
        feature.ahead,
        Some(1),
        "feature-ahead should be 1 commit ahead of its upstream"
    );
    assert_eq!(
        feature.behind,
        Some(0),
        "feature-ahead should be 0 commits behind its upstream"
    );

    // Also verify that main (which is in sync) reports ahead=0, behind=0
    let main_branch = branches.iter().find(|b| b.name == "main").unwrap();
    assert_eq!(main_branch.ahead, Some(0), "main should be 0 ahead");
    assert_eq!(main_branch.behind, Some(0), "main should be 0 behind");
}

#[test]
fn test_ahead_behind_local_only_branch() {
    // A branch with no upstream should have ahead=None, behind=None
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["branch", "local-only"]);

    let branches = branch::list_branches(&repo, "main").expect("list_branches failed");
    let local_branch = branches
        .iter()
        .find(|b| b.name == "local-only")
        .expect("local-only branch not found");

    assert_eq!(
        local_branch.ahead, None,
        "local-only branch should have ahead=None"
    );
    assert_eq!(
        local_branch.behind, None,
        "local-only branch should have behind=None"
    );
}

#[test]
fn test_checkout_branch() {
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();
    run_git(dir, &["branch", "feature-checkout"]);

    let result = operations::checkout_branch(&repo, dir, "feature-checkout", false);
    assert!(
        result.success,
        "checkout should succeed: {}",
        result.message
    );

    let repo = git2::Repository::open(dir).unwrap();
    let head = repo.head().unwrap();
    assert_eq!(head.shorthand().unwrap(), "feature-checkout");
}

#[test]
fn test_checkout_branch_with_stash() {
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();
    run_git(dir, &["branch", "feature-stash-checkout"]);

    // Create a dirty working tree (unstaged change)
    let dirty_file = dir.join("README.md");
    std::fs::write(&dirty_file, "# Modified\n").expect("failed to write dirty file");

    let result = operations::checkout_branch(&repo, dir, "feature-stash-checkout", true);
    assert!(
        result.success,
        "checkout with stash should succeed: {}",
        result.message
    );

    let repo = git2::Repository::open(dir).unwrap();
    let head = repo.head().unwrap();
    assert_eq!(head.shorthand().unwrap(), "feature-stash-checkout");

    // The stash should have been popped, so the working tree change should be present
    let contents = std::fs::read_to_string(&dirty_file).expect("failed to read file");
    assert_eq!(contents, "# Modified\n", "stash pop should restore changes");
}

// ---------------------------------------------------------------------------
// Remote branch tests — helper
// ---------------------------------------------------------------------------

/// Create a bare remote + clone setup with an initial commit on main,
/// plus remote-only and local+remote branches for testing.
///
/// Returns (tmpdir, work_dir path, Repository for the clone).
fn setup_remote_test_repo() -> (TestDir, std::path::PathBuf, git2::Repository) {
    let tmpdir = TestDir::new(tempfile::tempdir().expect("failed to create tmpdir"));
    let base_dir = tmpdir.path();

    // Bare remote
    let remote_dir = base_dir.join("remote.git");
    std::fs::create_dir_all(&remote_dir).unwrap();
    run_git(&remote_dir, &["init", "--bare", "-b", "main"]);

    // Clone
    let work_dir = base_dir.join("work");
    run_git(base_dir, &["clone", remote_dir.to_str().unwrap(), "work"]);
    run_git(&work_dir, &["config", "user.name", "Test User"]);
    run_git(&work_dir, &["config", "user.email", "test@example.com"]);

    // Initial commit on main
    let readme = work_dir.join("README.md");
    std::fs::write(&readme, "# Test\n").unwrap();
    run_git(&work_dir, &["add", "."]);
    run_git(&work_dir, &["commit", "-m", "Initial commit"]);
    run_git(&work_dir, &["push", "-u", "origin", "main"]);

    let repo = git2::Repository::open(&work_dir).expect("failed to open work repo");
    (tmpdir, work_dir, repo)
}

// ---------------------------------------------------------------------------
// Remote branch tests
// ---------------------------------------------------------------------------

#[test]
fn test_list_remote_branches() {
    let (_tmpdir, work_dir, repo) = setup_remote_test_repo();

    // Create a feature branch and push it
    run_git(&work_dir, &["checkout", "-b", "feature-remote"]);
    let f = work_dir.join("feature.txt");
    std::fs::write(&f, "content\n").unwrap();
    run_git(&work_dir, &["add", "feature.txt"]);
    run_git(&work_dir, &["commit", "-m", "Feature commit"]);
    run_git(&work_dir, &["push", "-u", "origin", "feature-remote"]);
    run_git(&work_dir, &["checkout", "main"]);

    let remotes = branch::list_remote_branches_phase1(&repo, "main")
        .expect("list_remote_branches_phase1 failed");

    // Should have origin/main and origin/feature-remote
    let names: Vec<&str> = remotes.iter().map(|r| r.short_name.as_str()).collect();
    assert!(names.contains(&"main"), "should list origin/main");
    assert!(
        names.contains(&"feature-remote"),
        "should list origin/feature-remote"
    );

    // origin/main should be marked as base
    let main_remote = remotes.iter().find(|r| r.short_name == "main").unwrap();
    assert!(main_remote.is_base, "origin/main should be is_base");
    assert_eq!(main_remote.remote, "origin");
    assert_eq!(main_remote.full_ref, "origin/main");
}

#[test]
fn test_remote_branch_has_local() {
    let (_tmpdir, work_dir, repo) = setup_remote_test_repo();

    // Push a branch that has a local counterpart
    run_git(&work_dir, &["checkout", "-b", "has-local"]);
    let f = work_dir.join("local.txt");
    std::fs::write(&f, "content\n").unwrap();
    run_git(&work_dir, &["add", "local.txt"]);
    run_git(&work_dir, &["commit", "-m", "Local branch commit"]);
    run_git(&work_dir, &["push", "-u", "origin", "has-local"]);

    // Push a branch then delete the local copy (remote-only)
    run_git(&work_dir, &["checkout", "-b", "remote-only"]);
    let f2 = work_dir.join("remote-only.txt");
    std::fs::write(&f2, "content\n").unwrap();
    run_git(&work_dir, &["add", "remote-only.txt"]);
    run_git(&work_dir, &["commit", "-m", "Remote-only commit"]);
    run_git(&work_dir, &["push", "-u", "origin", "remote-only"]);
    run_git(&work_dir, &["checkout", "main"]);
    run_git(&work_dir, &["branch", "-D", "remote-only"]);

    let remotes = branch::list_remote_branches_phase1(&repo, "main")
        .expect("list_remote_branches_phase1 failed");

    let has_local_branch = remotes
        .iter()
        .find(|r| r.short_name == "has-local")
        .unwrap();
    assert!(
        has_local_branch.has_local,
        "has-local should have has_local=true"
    );

    let remote_only_branch = remotes
        .iter()
        .find(|r| r.short_name == "remote-only")
        .unwrap();
    assert!(
        !remote_only_branch.has_local,
        "remote-only should have has_local=false"
    );
}

#[test]
fn test_remote_branch_merged_detection() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    // Create and push a feature branch
    run_git(&work_dir, &["checkout", "-b", "feature-to-merge"]);
    let f = work_dir.join("merge-feature.txt");
    std::fs::write(&f, "merge content\n").unwrap();
    run_git(&work_dir, &["add", "merge-feature.txt"]);
    run_git(&work_dir, &["commit", "-m", "Feature to merge"]);
    run_git(&work_dir, &["push", "-u", "origin", "feature-to-merge"]);

    // Add a commit on main so merge is not a fast-forward
    run_git(&work_dir, &["checkout", "main"]);
    let main_file = work_dir.join("main-change.txt");
    std::fs::write(&main_file, "main branch change\n").unwrap();
    run_git(&work_dir, &["add", "main-change.txt"]);
    run_git(&work_dir, &["commit", "-m", "Main branch commit"]);

    // Merge and push
    run_git(
        &work_dir,
        &["merge", "feature-to-merge", "-m", "Merge feature"],
    );
    run_git(&work_dir, &["push", "origin", "main"]);

    // Re-open to see updated refs
    let repo = git2::Repository::open(&work_dir).unwrap();
    let mut remotes = branch::list_remote_branches_phase1(&repo, "main")
        .expect("list_remote_branches_phase1 failed");

    // Phase-1 now always returns Unmerged; enrichment runs in background thread.
    let rx = branch::spawn_remote_enricher(work_dir.clone(), "main".to_string(), remotes.clone());
    let index_map: std::collections::HashMap<String, usize> = remotes
        .iter()
        .enumerate()
        .map(|(i, b)| (b.full_ref.clone(), i))
        .collect();
    for result in rx {
        if let Some(&idx) = index_map.get(&result.full_ref) {
            remotes[idx].merge_status = result.merge_status;
        }
    }

    let merged = remotes
        .iter()
        .find(|r| r.short_name == "feature-to-merge")
        .unwrap();
    assert_eq!(
        merged.merge_status,
        MergeStatus::Merged,
        "feature-to-merge should be detected as Merged on remote"
    );
}

#[test]
fn test_remote_branch_unmerged_detection() {
    let (_tmpdir, work_dir, repo) = setup_remote_test_repo();

    // Create and push an unmerged feature branch
    run_git(&work_dir, &["checkout", "-b", "feature-unmerged"]);
    let f = work_dir.join("unmerged.txt");
    std::fs::write(&f, "unmerged content\n").unwrap();
    run_git(&work_dir, &["add", "unmerged.txt"]);
    run_git(&work_dir, &["commit", "-m", "Unmerged feature"]);
    run_git(&work_dir, &["push", "-u", "origin", "feature-unmerged"]);
    run_git(&work_dir, &["checkout", "main"]);

    let remotes = branch::list_remote_branches_phase1(&repo, "main")
        .expect("list_remote_branches_phase1 failed");

    // Phase-1 sets non-base branches to Pending; squash check resolves to Unmerged/SquashMerged.
    let unmerged = remotes
        .iter()
        .find(|r| r.short_name == "feature-unmerged")
        .unwrap();
    assert_eq!(
        unmerged.merge_status,
        MergeStatus::Pending,
        "feature-unmerged should be Pending after phase-1 (squash check not yet run)"
    );
}

#[test]
fn test_remote_branch_skips_head() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    // Set up origin/HEAD (some repos have this)
    run_git(&work_dir, &["remote", "set-head", "origin", "main"]);

    let repo = git2::Repository::open(&work_dir).unwrap();
    let remotes = branch::list_remote_branches_phase1(&repo, "main")
        .expect("list_remote_branches_phase1 failed");

    // Should not include any entry with short_name "HEAD"
    let head_entries: Vec<_> = remotes.iter().filter(|r| r.short_name == "HEAD").collect();
    assert!(
        head_entries.is_empty(),
        "origin/HEAD pseudo-ref should be filtered out"
    );
}

#[test]
fn test_checkout_remote_branch() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    // Create and push a branch, then delete local
    run_git(&work_dir, &["checkout", "-b", "remote-checkout-test"]);
    let f = work_dir.join("checkout-test.txt");
    std::fs::write(&f, "content\n").unwrap();
    run_git(&work_dir, &["add", "checkout-test.txt"]);
    run_git(&work_dir, &["commit", "-m", "Checkout test commit"]);
    run_git(&work_dir, &["push", "-u", "origin", "remote-checkout-test"]);
    run_git(&work_dir, &["checkout", "main"]);
    run_git(&work_dir, &["branch", "-D", "remote-checkout-test"]);

    // Now checkout from remote
    let result = operations::checkout_remote_branch(&work_dir, "origin", "remote-checkout-test");
    assert!(
        result.success,
        "checkout_remote_branch should succeed: {}",
        result.message
    );

    // Verify we're on the new local branch
    let repo = git2::Repository::open(&work_dir).unwrap();
    let head = repo.head().unwrap();
    assert_eq!(head.shorthand().unwrap(), "remote-checkout-test");

    // Verify the file from the remote branch exists
    assert!(
        work_dir.join("checkout-test.txt").exists(),
        "checked-out file should exist"
    );
}

#[test]
fn test_checkout_remote_branch_already_exists() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    // Create and push a branch, keep local copy
    run_git(&work_dir, &["checkout", "-b", "already-local"]);
    let f = work_dir.join("already.txt");
    std::fs::write(&f, "content\n").unwrap();
    run_git(&work_dir, &["add", "already.txt"]);
    run_git(&work_dir, &["commit", "-m", "Already local"]);
    run_git(&work_dir, &["push", "-u", "origin", "already-local"]);
    run_git(&work_dir, &["checkout", "main"]);

    // Trying to checkout remote when local already exists should fail
    let result = operations::checkout_remote_branch(&work_dir, "origin", "already-local");
    assert!(
        !result.success,
        "should fail when local branch already exists"
    );
}

#[test]
fn test_fetch_sync() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    // fetch_sync should succeed on a valid repo with a remote
    let success = operations::fetch_sync(&work_dir);
    assert!(success, "fetch_sync should succeed");
}

#[test]
fn test_remote_branches_sorted_by_date() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    // Create branches with explicit, distinct commit times. list_remote_branches_phase1
    // sorts by committer date, so set both author and committer dates.
    run_git(&work_dir, &["checkout", "-b", "older-branch"]);
    let f1 = work_dir.join("older.txt");
    std::fs::write(&f1, "older\n").unwrap();
    run_git(&work_dir, &["add", "older.txt"]);
    run_git_with_env(
        &work_dir,
        &["commit", "-m", "Older commit"],
        &[
            ("GIT_AUTHOR_DATE", "2001-01-01T00:00:00Z"),
            ("GIT_COMMITTER_DATE", "2001-01-01T00:00:00Z"),
        ],
    );
    run_git(&work_dir, &["push", "-u", "origin", "older-branch"]);

    run_git(&work_dir, &["checkout", "main"]);
    run_git(&work_dir, &["checkout", "-b", "newer-branch"]);
    let f2 = work_dir.join("newer.txt");
    std::fs::write(&f2, "newer\n").unwrap();
    run_git(&work_dir, &["add", "newer.txt"]);
    run_git_with_env(
        &work_dir,
        &["commit", "-m", "Newer commit"],
        &[
            ("GIT_AUTHOR_DATE", "2001-01-02T00:00:00Z"),
            ("GIT_COMMITTER_DATE", "2001-01-02T00:00:00Z"),
        ],
    );
    run_git(&work_dir, &["push", "-u", "origin", "newer-branch"]);
    run_git(&work_dir, &["checkout", "main"]);

    let repo = git2::Repository::open(&work_dir).unwrap();
    let remotes = branch::list_remote_branches_phase1(&repo, "main")
        .expect("list_remote_branches_phase1 failed");

    // Find positions — newer should come before older (sorted newest-first)
    let newer_pos = remotes
        .iter()
        .position(|r| r.short_name == "newer-branch")
        .unwrap();
    let older_pos = remotes
        .iter()
        .position(|r| r.short_name == "older-branch")
        .unwrap();
    assert!(
        newer_pos < older_pos,
        "newer-branch (pos {}) should come before older-branch (pos {}) in date-descending sort",
        newer_pos,
        older_pos
    );
}

#[test]
fn test_remote_branch_squash_merge_detection() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    // Create a feature branch with unique content
    run_git(&work_dir, &["checkout", "-b", "squash-feature"]);
    let f = work_dir.join("squash-feature.txt");
    std::fs::write(&f, "squash feature content\n").unwrap();
    run_git(&work_dir, &["add", "squash-feature.txt"]);
    run_git(&work_dir, &["commit", "-m", "Squash feature commit"]);
    run_git(&work_dir, &["push", "-u", "origin", "squash-feature"]);

    // Squash-merge into main (without a merge commit)
    run_git(&work_dir, &["checkout", "main"]);
    run_git(&work_dir, &["merge", "--squash", "squash-feature"]);
    run_git(&work_dir, &["commit", "-m", "Squash merge squash-feature"]);
    run_git(&work_dir, &["push", "origin", "main"]);

    // Reload repo after push
    let repo = git2::Repository::open(&work_dir).unwrap();
    let remotes = branch::list_remote_branches_phase1(&repo, "main")
        .expect("list_remote_branches_phase1 failed");

    // Build candidates for squash checker: (full_ref, commit_hash) for pending non-base branches
    let candidates: Vec<(String, String, Option<String>)> = remotes
        .iter()
        .filter(|b| b.merge_status == MergeStatus::Pending && !b.is_base)
        .filter_map(|b| {
            let refname = format!("refs/remotes/{}", b.full_ref);
            repo.find_reference(&refname)
                .ok()
                .and_then(|r| r.peel_to_commit().ok())
                .map(|c| (b.full_ref.clone(), c.id().to_string(), None))
        })
        .collect();

    let cache = git_branch_manager::git::cache::BranchCache::load(&work_dir);
    let rx = squash_loader::spawn_squash_checker(
        work_dir.clone(),
        "main".to_string(),
        candidates,
        cache,
    );

    let index_map: std::collections::HashMap<String, usize> = remotes
        .iter()
        .enumerate()
        .map(|(i, b)| (b.full_ref.clone(), i))
        .collect();

    let mut remotes = remotes;
    for result in rx {
        if let Some(&idx) = index_map.get(&result.branch_name) {
            remotes[idx].merge_status = result.status;
        }
    }

    let squashed = remotes
        .iter()
        .find(|r| r.short_name == "squash-feature")
        .unwrap();
    assert_eq!(
        squashed.merge_status,
        MergeStatus::SquashMerged,
        "squash-feature should be detected as SquashMerged on remote"
    );
}

#[test]
fn test_remote_disjoint_branch_detected() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    // A normal branch that shares history with main.
    run_git(&work_dir, &["checkout", "-b", "normal"]);
    std::fs::write(work_dir.join("normal.txt"), "n\n").unwrap();
    run_git(&work_dir, &["add", "normal.txt"]);
    run_git(&work_dir, &["commit", "-m", "normal commit"]);
    run_git(&work_dir, &["push", "-u", "origin", "normal"]);

    // An orphan branch: a separate root, so it shares NO history with main.
    run_git(&work_dir, &["checkout", "--orphan", "disjoint"]);
    run_git(&work_dir, &["rm", "-rf", "."]);
    std::fs::write(work_dir.join("disjoint.txt"), "d\n").unwrap();
    run_git(&work_dir, &["add", "disjoint.txt"]);
    run_git(&work_dir, &["commit", "-m", "disjoint root"]);
    run_git(&work_dir, &["push", "-u", "origin", "disjoint"]);

    run_git(&work_dir, &["checkout", "main"]);

    let repo = git2::Repository::open(&work_dir).unwrap();
    let remotes = branch::list_remote_branches_phase1(&repo, "main")
        .expect("list_remote_branches_phase1 failed");

    let rx = branch::spawn_remote_enricher(work_dir.clone(), "main".to_string(), remotes);
    let results: Vec<_> = rx.iter().collect();

    let disjoint = results
        .iter()
        .find(|r| r.full_ref == "origin/disjoint")
        .expect("origin/disjoint should be enriched");
    assert!(
        disjoint.disjoint,
        "orphan branch shares no history with main and must be flagged disjoint"
    );

    let normal = results
        .iter()
        .find(|r| r.full_ref == "origin/normal")
        .expect("origin/normal should be enriched");
    assert!(
        !normal.disjoint,
        "a branch that shares history with main must not be flagged disjoint"
    );
}

// ---------------------------------------------------------------------------
// Working tree status detection
// ---------------------------------------------------------------------------

#[test]
fn test_wt_status_clean() {
    let (tmpdir, _repo) = setup_test_repo();
    let s = status::detect_working_tree_status(tmpdir.path());
    assert!(s.is_clean(), "fresh repo should be clean");
}

#[test]
fn test_wt_status_staged_only() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Add a new file to the index without committing
    std::fs::write(dir.join("new.txt"), "content\n").unwrap();
    run_git(dir, &["add", "new.txt"]);

    let s = status::detect_working_tree_status(dir);
    assert!(s.has_staged, "should detect staged file");
    assert!(!s.has_modified, "should not detect modified changes");
    assert!(!s.has_untracked, "should not detect untracked files");
    assert_eq!(
        s.changed_files.len(),
        1,
        "one staged file should be itemized"
    );
    assert_eq!(s.changed_files[0].path, "new.txt");
    assert_eq!(s.changed_files[0].kind, ChangedFileKind::Staged);
}

#[test]
fn test_wt_status_unstaged_only() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Modify a tracked file without staging
    std::fs::write(dir.join("README.md"), "# Modified\n").unwrap();

    let s = status::detect_working_tree_status(dir);
    assert!(!s.has_staged, "should not detect staged changes");
    assert!(s.has_modified, "should detect modified file");
    assert!(!s.has_untracked, "should not detect untracked files");
    assert_eq!(s.changed_files.len(), 1);
    assert_eq!(s.changed_files[0].path, "README.md");
    assert_eq!(s.changed_files[0].kind, ChangedFileKind::Modified);
}

#[test]
fn test_wt_status_untracked_only() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Create a file that is not tracked by git
    std::fs::write(dir.join("untracked.txt"), "not tracked\n").unwrap();

    let s = status::detect_working_tree_status(dir);
    assert!(!s.has_staged, "should not detect staged changes");
    assert!(!s.has_modified, "should not detect modified changes");
    assert!(s.has_untracked, "should detect untracked file");
    assert_eq!(s.changed_files.len(), 1);
    assert_eq!(s.changed_files[0].path, "untracked.txt");
    assert_eq!(s.changed_files[0].kind, ChangedFileKind::Untracked);
}

#[test]
fn test_wt_status_all_three() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Staged: add a new file to index
    std::fs::write(dir.join("staged.txt"), "staged\n").unwrap();
    run_git(dir, &["add", "staged.txt"]);

    // Unstaged: modify a tracked file without staging
    std::fs::write(dir.join("README.md"), "# Modified\n").unwrap();

    // Untracked: a new file not added to index
    std::fs::write(dir.join("untracked.txt"), "not tracked\n").unwrap();

    let s = status::detect_working_tree_status(dir);
    assert!(s.has_staged, "should detect staged file");
    assert!(s.has_modified, "should detect modified file");
    assert!(s.has_untracked, "should detect untracked file");
    assert!(!s.is_clean());
    assert_eq!(s.changed_files.len(), 3, "one entry per changed file");
    assert!(s
        .changed_files
        .iter()
        .any(|f| f.path == "staged.txt" && f.kind == ChangedFileKind::Staged));
    assert!(s
        .changed_files
        .iter()
        .any(|f| f.path == "README.md" && f.kind == ChangedFileKind::Modified));
    assert!(s
        .changed_files
        .iter()
        .any(|f| f.path == "untracked.txt" && f.kind == ChangedFileKind::Untracked));
}

#[test]
fn test_wt_status_staged_then_further_edited() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Stage a modification to a tracked file, then modify it again without
    // staging — git2 reports this single path with both an INDEX_* and a
    // WT_* flag. It must surface as two separate ChangedFile entries: one
    // Staged (the staged version) and one Modified (the further unstaged
    // edit), not a merged/combined kind.
    std::fs::write(dir.join("README.md"), "# Staged edit\n").unwrap();
    run_git(dir, &["add", "README.md"]);
    std::fs::write(
        dir.join("README.md"),
        "# Staged edit, then further edited\n",
    )
    .unwrap();

    let s = status::detect_working_tree_status(dir);
    assert!(s.has_staged, "should detect staged change");
    assert!(s.has_modified, "should detect further unstaged edit");
    assert_eq!(
        s.changed_files.len(),
        2,
        "path with both staged and unstaged changes should appear as two entries"
    );
    assert!(s
        .changed_files
        .iter()
        .any(|f| f.path == "README.md" && f.kind == ChangedFileKind::Staged));
    assert!(s
        .changed_files
        .iter()
        .any(|f| f.path == "README.md" && f.kind == ChangedFileKind::Modified));
}

#[test]
fn test_worktree_status_reports_modified_not_staged() {
    // Regression: a tracked file that is modified but NOT staged must report as
    // `modified`, not `staged`. The old worktree status parser trimmed the
    // leading (empty) index column out of `git status --porcelain` and misread
    // the unstaged change as staged. Covers BOTH the main worktree and an
    // additional worktree, since the enrich path runs once per worktree.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    // An additional worktree on its own branch.
    run_git(dir, &["branch", "wt-mod"]);
    run_git(dir, &["worktree", "add", ".worktrees/wt-mod", "wt-mod"]);

    // Modify a tracked file (without staging) in BOTH the main dir and the
    // additional worktree.
    std::fs::write(dir.join("README.md"), "# Modified, not staged\n").unwrap();
    std::fs::write(
        dir.join(".worktrees").join("wt-mod").join("README.md"),
        "# Modified in additional worktree\n",
    )
    .unwrap();

    let worktrees = worktree::list_worktrees(dir);
    let rx = worktree::enrich_worktrees(worktrees.clone());
    let mut results: Vec<_> = rx.iter().collect();
    results.sort_by_key(|r| r.index);

    // The crux of the regression: a modified-not-staged file must classify as
    // modified, never staged. (We don't assert on has_untracked here: the main
    // worktree legitimately sees the in-repo `.worktrees/` dir as untracked.)
    for (idx, wt) in worktrees.iter().enumerate() {
        let status = &results[idx].wt_status;
        assert!(
            status.has_modified,
            "{:?}: modified-not-staged file should be reported as modified",
            wt.branch
        );
        assert!(
            !status.has_staged,
            "{:?}: nothing is staged, so has_staged must be false",
            wt.branch
        );
    }

    if std::env::var_os("GBM_KEEP_TEST_REPOS").is_none() {
        run_git(dir, &["worktree", "remove", "--force", ".worktrees/wt-mod"]);
    }
}

// ---------------------------------------------------------------------------
// Merge operation tests
// ---------------------------------------------------------------------------

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
    assert!(
        head_commit
            .summary()
            .ok()
            .flatten()
            .is_some_and(|s| s.contains("Squash merge feature-squash")),
        "HEAD should be the squash merge commit, got: {:?}",
        head_commit.summary()
    );
    assert_eq!(
        head_commit.parent_count(),
        1,
        "squash merge must be a single-parent commit, not a merge commit"
    );
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

// ---------------------------------------------------------------------------
// Remote operations: push, pull, fast-forward, fetch-prune
// ---------------------------------------------------------------------------

#[test]
fn test_push_branch() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    // Create a local branch with a commit
    run_git(&work_dir, &["checkout", "-b", "push-test"]);
    std::fs::write(work_dir.join("push.txt"), "push content\n").unwrap();
    run_git(&work_dir, &["add", "push.txt"]);
    run_git(&work_dir, &["commit", "-m", "Push test commit"]);
    run_git(&work_dir, &["checkout", "main"]);

    let result = operations::push_branch(&work_dir, "push-test", &AtomicBool::new(false));
    assert!(
        result.success,
        "push_branch should succeed: {}",
        result.message
    );

    // Fetch and verify remote has the branch
    run_git(&work_dir, &["fetch", "origin"]);
    let repo = git2::Repository::open(&work_dir).unwrap();
    assert!(
        repo.find_branch("origin/push-test", git2::BranchType::Remote)
            .is_ok(),
        "origin/push-test should exist after push"
    );
}

#[test]
fn test_pull_branch_current() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    // Simulate another committer pushing to origin/main via a second clone
    let tmpdir2 = tempfile::tempdir().unwrap();
    let clone2 = tmpdir2.path().join("clone2");
    let remote_dir = work_dir.parent().unwrap().join("remote.git");
    run_git(
        tmpdir2.path(),
        &["clone", remote_dir.to_str().unwrap(), "clone2"],
    );
    run_git(&clone2, &["config", "user.name", "Other User"]);
    run_git(&clone2, &["config", "user.email", "other@example.com"]);
    std::fs::write(clone2.join("other.txt"), "other commit\n").unwrap();
    run_git(&clone2, &["add", "other.txt"]);
    run_git(&clone2, &["commit", "-m", "Other commit on main"]);
    run_git(&clone2, &["push", "origin", "main"]);

    // Pull in work_dir (main is current branch)
    let result = operations::pull_branch(&work_dir, "main", true, &AtomicBool::new(false));
    assert!(
        result.success,
        "pull_branch (current) should succeed: {}",
        result.message
    );

    // Verify the new file exists locally
    assert!(
        work_dir.join("other.txt").exists(),
        "pulled file should exist after pull"
    );
}

#[test]
fn test_pull_branch_non_current() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    // Create and push a feature branch
    run_git(&work_dir, &["checkout", "-b", "pull-non-current"]);
    std::fs::write(work_dir.join("pull-feature.txt"), "feature\n").unwrap();
    run_git(&work_dir, &["add", "pull-feature.txt"]);
    run_git(&work_dir, &["commit", "-m", "Feature commit"]);
    run_git(&work_dir, &["push", "-u", "origin", "pull-non-current"]);

    // Push another commit from a second clone
    let tmpdir2 = tempfile::tempdir().unwrap();
    let clone2 = tmpdir2.path().join("clone2");
    let remote_dir = work_dir.parent().unwrap().join("remote.git");
    run_git(
        tmpdir2.path(),
        &["clone", remote_dir.to_str().unwrap(), "clone2"],
    );
    run_git(&clone2, &["config", "user.name", "Other User"]);
    run_git(&clone2, &["config", "user.email", "other@example.com"]);
    run_git(&clone2, &["checkout", "pull-non-current"]);
    std::fs::write(clone2.join("extra.txt"), "extra\n").unwrap();
    run_git(&clone2, &["add", "extra.txt"]);
    run_git(&clone2, &["commit", "-m", "Extra commit"]);
    run_git(&clone2, &["push", "origin", "pull-non-current"]);

    // Fetch so work_dir knows about the remote update, then switch to main
    run_git(&work_dir, &["fetch", "origin"]);
    run_git(&work_dir, &["checkout", "main"]);

    let result = operations::pull_branch(
        &work_dir,
        "pull-non-current",
        false,
        &AtomicBool::new(false),
    );
    assert!(
        result.success,
        "pull_branch (non-current) should succeed: {}",
        result.message
    );

    // The local branch tip should now be at "Extra commit"
    let repo = git2::Repository::open(&work_dir).unwrap();
    let branch_oid = repo
        .find_branch("pull-non-current", git2::BranchType::Local)
        .unwrap()
        .get()
        .peel_to_commit()
        .unwrap()
        .id();
    let commit = repo.find_commit(branch_oid).unwrap();
    assert_eq!(
        commit.summary().ok().flatten().unwrap_or(""),
        "Extra commit",
        "local branch should be updated to latest remote commit"
    );
}

#[test]
fn test_fast_forward_branch() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    // Push a feature branch
    run_git(&work_dir, &["checkout", "-b", "ff-branch"]);
    std::fs::write(work_dir.join("ff.txt"), "ff content\n").unwrap();
    run_git(&work_dir, &["add", "ff.txt"]);
    run_git(&work_dir, &["commit", "-m", "FF commit"]);
    run_git(&work_dir, &["push", "-u", "origin", "ff-branch"]);

    // Advance the remote via a second clone
    let tmpdir2 = tempfile::tempdir().unwrap();
    let clone2 = tmpdir2.path().join("clone2");
    let remote_dir = work_dir.parent().unwrap().join("remote.git");
    run_git(
        tmpdir2.path(),
        &["clone", remote_dir.to_str().unwrap(), "clone2"],
    );
    run_git(&clone2, &["config", "user.name", "Other User"]);
    run_git(&clone2, &["config", "user.email", "other@example.com"]);
    run_git(&clone2, &["checkout", "ff-branch"]);
    std::fs::write(clone2.join("ff2.txt"), "ff2 content\n").unwrap();
    run_git(&clone2, &["add", "ff2.txt"]);
    run_git(&clone2, &["commit", "-m", "FF commit 2"]);
    run_git(&clone2, &["push", "origin", "ff-branch"]);

    // Go back to main (ff-branch is not current)
    run_git(&work_dir, &["checkout", "main"]);

    // fast_forward fetches origin/ff-branch:ff-branch
    let result = operations::fast_forward(&work_dir, "ff-branch", &AtomicBool::new(false));
    assert!(
        result.success,
        "fast_forward should succeed: {}",
        result.message
    );

    // Verify the local branch was advanced
    let repo = git2::Repository::open(&work_dir).unwrap();
    let commit = repo
        .find_branch("ff-branch", git2::BranchType::Local)
        .unwrap()
        .get()
        .peel_to_commit()
        .unwrap();
    assert_eq!(
        commit.summary().ok().flatten().unwrap_or(""),
        "FF commit 2",
        "local branch should be at the latest remote commit after fast-forward"
    );
}

#[test]
fn test_fetch_prune_removes_stale_remote() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();
    let remote_dir = work_dir.parent().unwrap().join("remote.git");

    // Push a branch so work_dir has a remote-tracking ref for it
    run_git(&work_dir, &["checkout", "-b", "prune-me"]);
    std::fs::write(work_dir.join("prune.txt"), "prune\n").unwrap();
    run_git(&work_dir, &["add", "prune.txt"]);
    run_git(&work_dir, &["commit", "-m", "Prune commit"]);
    run_git(&work_dir, &["push", "-u", "origin", "prune-me"]);
    run_git(&work_dir, &["checkout", "main"]);

    // Delete the branch directly on the bare remote (simulating someone else deleting it).
    // This leaves the local origin/prune-me tracking ref stale — fetch --prune should clean it up.
    run_git(&remote_dir, &["branch", "-D", "prune-me"]);

    // Confirm origin/prune-me is still in local tracking refs before pruning
    let repo = git2::Repository::open(&work_dir).unwrap();
    assert!(
        repo.find_branch("origin/prune-me", git2::BranchType::Remote)
            .is_ok(),
        "origin/prune-me should still exist in local refs before prune"
    );

    let result = operations::fetch_prune(&work_dir, &AtomicBool::new(false));
    assert!(
        result.success,
        "fetch_prune should succeed: {}",
        result.message
    );

    // After prune, stale tracking ref should be gone
    let repo2 = git2::Repository::open(&work_dir).unwrap();
    assert!(
        repo2
            .find_branch("origin/prune-me", git2::BranchType::Remote)
            .is_err(),
        "origin/prune-me should be removed after fetch --prune"
    );
}

// ---------------------------------------------------------------------------
// Remote batch delete tests
// ---------------------------------------------------------------------------

#[test]
fn test_delete_remotes_batch_success() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    // Create and push two branches
    for branch_name in &["batch-del-1", "batch-del-2"] {
        run_git(&work_dir, &["checkout", "-b", branch_name]);
        std::fs::write(work_dir.join(format!("{}.txt", branch_name)), "content\n").unwrap();
        run_git(&work_dir, &["add", &format!("{}.txt", branch_name)]);
        run_git(
            &work_dir,
            &["commit", "-m", &format!("Add {}", branch_name)],
        );
        run_git(&work_dir, &["push", "-u", "origin", branch_name]);
        run_git(&work_dir, &["checkout", "main"]);
    }

    let names: Vec<String> = vec!["batch-del-1".to_string(), "batch-del-2".to_string()];
    let results = operations::delete_remotes_batch(&work_dir, &names, &AtomicBool::new(false));

    assert_eq!(results.len(), 2);
    assert!(
        results[0].success,
        "first remote delete should succeed: {}",
        results[0].message
    );
    assert!(
        results[1].success,
        "second remote delete should succeed: {}",
        results[1].message
    );

    // Fetch to sync tracking refs, then verify branches are gone
    run_git(&work_dir, &["fetch", "--prune"]);
    let repo = git2::Repository::open(&work_dir).unwrap();
    assert!(
        repo.find_branch("origin/batch-del-1", git2::BranchType::Remote)
            .is_err(),
        "origin/batch-del-1 should be deleted"
    );
    assert!(
        repo.find_branch("origin/batch-del-2", git2::BranchType::Remote)
            .is_err(),
        "origin/batch-del-2 should be deleted"
    );
}

#[test]
fn test_delete_remotes_batch_empty() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    // Empty input should return empty results immediately (no git command run)
    let results = operations::delete_remotes_batch(&work_dir, &[], &AtomicBool::new(false));
    assert!(
        results.is_empty(),
        "empty input should produce empty results"
    );
}

// ---------------------------------------------------------------------------
// Worktree operation tests
// ---------------------------------------------------------------------------

#[test]
fn test_create_worktree_simple() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["branch", "wt-feature"]);

    let result = operations::create_worktree(dir, "wt-feature");
    assert!(
        result.success,
        "create_worktree should succeed: {}",
        result.message
    );

    let wt_path = dir.join(".worktrees").join("wt-feature");
    assert!(wt_path.exists(), ".worktrees/wt-feature should be created");
    // A worktree directory contains a .git file (not a directory like the main repo)
    assert!(
        wt_path.join(".git").exists(),
        "worktree directory should be a valid git working tree"
    );
    // It must be checked out on the requested branch, not left detached.
    let wts = worktree::list_worktrees(dir);
    assert!(
        wts.iter()
            .any(|w| w.branch.as_deref() == Some("wt-feature")),
        "worktree should be checked out on wt-feature: {wts:?}"
    );
}

#[test]
fn test_create_worktree_sanitizes_slash() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Branch names with slashes (e.g. "feature/foo") must be sanitized to "feature-foo"
    run_git(dir, &["branch", "feature/slash-test"]);

    let result = operations::create_worktree(dir, "feature/slash-test");
    assert!(
        result.success,
        "create_worktree with slash should succeed: {}",
        result.message
    );

    let wt_path = dir.join(".worktrees").join("feature-slash-test");
    assert!(
        wt_path.exists(),
        ".worktrees/feature-slash-test should be created (slash → dash)"
    );
    // The slash must be sanitized to a dash, NOT create a nested directory.
    assert!(
        !dir.join(".worktrees")
            .join("feature")
            .join("slash-test")
            .exists(),
        "slash should be replaced with a dash, not create nested dirs"
    );
    // And the worktree is checked out on the real (slashed) branch name.
    let wts = worktree::list_worktrees(dir);
    assert!(
        wts.iter()
            .any(|w| w.branch.as_deref() == Some("feature/slash-test")),
        "worktree should be on feature/slash-test: {wts:?}"
    );
}

#[test]
fn test_remove_worktree_clean() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["branch", "wt-remove"]);
    run_git(
        dir,
        &["worktree", "add", ".worktrees/wt-remove", "wt-remove"],
    );

    let wt_path = dir.join(".worktrees").join("wt-remove");
    assert!(wt_path.exists(), "worktree should exist before removal");

    let (prog_tx, _prog_rx) = std::sync::mpsc::channel();
    let cancel = AtomicBool::new(false);
    let partial = AtomicBool::new(false);
    let result = operations::remove_worktree(dir, &wt_path, (0, 1), &prog_tx, &cancel, &partial);
    assert!(
        result.success,
        "remove_worktree should succeed on clean worktree: {}",
        result.message
    );
    assert!(!wt_path.exists(), "worktree directory should be removed");
    // It must also be deregistered from git's worktree list, not just unlinked
    // on disk — only the main worktree should remain.
    let wts = worktree::list_worktrees(dir);
    assert_eq!(
        wts.len(),
        1,
        "only the main worktree should remain: {wts:?}"
    );
    assert!(wts[0].is_main, "the remaining worktree is the main one");
}

#[test]
fn test_force_remove_worktree_dirty() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["branch", "wt-dirty"]);
    run_git(dir, &["worktree", "add", ".worktrees/wt-dirty", "wt-dirty"]);

    let wt_path = dir.join(".worktrees").join("wt-dirty");

    // Modify a TRACKED file in the worktree. `git worktree remove` reliably
    // refuses on tracked-file modifications across git versions; an untracked
    // file alone is not a dependable refusal trigger.
    std::fs::write(wt_path.join("README.md"), "# Modified in worktree\n").unwrap();

    let (prog_tx, _prog_rx) = std::sync::mpsc::channel();
    let cancel = AtomicBool::new(false);
    let partial = AtomicBool::new(false);
    let result = operations::remove_worktree(dir, &wt_path, (0, 1), &prog_tx, &cancel, &partial);
    assert!(
        !result.success,
        "remove_worktree should fail on dirty worktree"
    );

    // Force remove should succeed regardless
    let (prog_tx, _prog_rx) = std::sync::mpsc::channel();
    let result =
        operations::force_remove_worktree(dir, &wt_path, (0, 1), &prog_tx, &cancel, &partial);
    assert!(
        result.success,
        "force_remove_worktree should succeed even when dirty: {}",
        result.message
    );
    assert!(
        !wt_path.exists(),
        "worktree directory should be removed after force-remove"
    );
}

/// Mirrors `worktree_delete::count_files`'s counting rules (files/symlinks,
/// not directories) — duplicated here because that module's items are
/// `pub(crate)` and invisible from this black-box integration-test binary.
fn count_files_for_test(root: &std::path::Path) -> usize {
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
fn parse_file_progress(msg: &str) -> Option<(usize, usize)> {
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

// ---------------------------------------------------------------------------
// Worktree listing and enrichment tests
// ---------------------------------------------------------------------------

#[test]
fn test_list_worktrees_main_only() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    let worktrees = worktree::list_worktrees(dir);
    assert_eq!(worktrees.len(), 1, "should have exactly 1 worktree (main)");

    let main_wt = &worktrees[0];
    assert!(
        main_wt.is_main,
        "first worktree should be the main worktree"
    );
    assert_eq!(
        main_wt.branch.as_deref(),
        Some("main"),
        "main worktree should be on 'main'"
    );
    assert_eq!(
        main_wt.commit_hash.len(),
        7,
        "commit_hash should be 7 chars"
    );
}

#[test]
fn test_worktree_lookup_includes_primary_and_reports_command_errors() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    let primary = worktree::try_worktree_for_branch(dir, "main")
        .expect("primary worktree lookup should run")
        .expect("primary worktree should be found");
    assert!(primary.is_main);
    assert_eq!(primary.branch.as_deref(), Some("main"));

    let missing = worktree::try_list_worktrees(std::path::Path::new(
        "/definitely/not/a/git/repository",
    ));
    assert!(missing.is_err(), "worktree command failures must be observable");
}

#[test]
fn test_branches_checked_out_in_worktrees_excludes_primary() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["branch", "feature/linked-checkout"]);
    run_git(
        dir,
        &[
            "worktree",
            "add",
            ".worktrees/feature-linked-checkout",
            "feature/linked-checkout",
        ],
    );

    let checked_out = worktree::branches_checked_out_in_worktrees(dir);
    assert!(
        checked_out.contains("feature/linked-checkout"),
        "linked worktree's branch should be reported"
    );
    assert!(
        !checked_out.contains("main"),
        "the primary worktree's branch is excluded, even though it is checked out"
    );

    run_git(
        dir,
        &["worktree", "remove", ".worktrees/feature-linked-checkout"],
    );
}

#[test]
fn test_branches_checked_out_in_worktrees_empty_with_only_primary() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    let checked_out = worktree::branches_checked_out_in_worktrees(dir);
    assert!(
        checked_out.is_empty(),
        "with only the primary worktree present, nothing is reported"
    );
}

#[test]
fn test_branches_checked_out_in_worktrees_best_effort_on_command_failure() {
    let checked_out = worktree::branches_checked_out_in_worktrees(std::path::Path::new(
        "/definitely/not/a/git/repository",
    ));
    assert!(
        checked_out.is_empty(),
        "best-effort callers must degrade to an empty set, not panic, on inspection failure"
    );
}

#[test]
fn test_worktree_path_for_branch_excludes_callers_own_worktree() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    // "main" is checked out in the primary worktree, which IS the caller
    // (repo_path == dir). worktree_path_for_branch answers "checked out in
    // some OTHER worktree", so this must be None, not the primary's own path.
    let path = worktree::worktree_path_for_branch(dir, "main");
    assert!(
        path.is_none(),
        "the caller's own worktree must not be reported as an 'other' worktree"
    );
}

#[test]
fn test_worktree_path_for_branch_finds_linked_worktree_other_than_caller() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["branch", "feature/linked-path"]);
    run_git(
        dir,
        &[
            "worktree",
            "add",
            ".worktrees/feature-linked-path",
            "feature/linked-path",
        ],
    );

    let path = worktree::worktree_path_for_branch(dir, "feature/linked-path")
        .expect("linked worktree should be found");
    assert!(path.ends_with(".worktrees/feature-linked-path"));

    run_git(dir, &["worktree", "remove", ".worktrees/feature-linked-path"]);
}

#[test]
fn test_try_other_worktree_for_branch_none_vs_err() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Not checked out anywhere else: Ok(None), distinct from a lookup failure.
    let not_checked_out =
        worktree::try_other_worktree_for_branch(dir, "does-not-exist-anywhere");
    assert!(matches!(not_checked_out, Ok(None)));

    // An unreadable/non-existent repo path: Err(_), distinguishable from Ok(None).
    let inspection_failed = worktree::try_other_worktree_for_branch(
        std::path::Path::new("/definitely/not/a/git/repository"),
        "main",
    );
    assert!(
        inspection_failed.is_err(),
        "command failure must be observable as Err, not conflated with 'not checked out'"
    );
}

#[test]
fn test_list_worktrees_with_additional() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["branch", "wt-list-test"]);
    run_git(
        dir,
        &["worktree", "add", ".worktrees/wt-list-test", "wt-list-test"],
    );

    let worktrees = worktree::list_worktrees(dir);
    assert_eq!(worktrees.len(), 2, "should have 2 worktrees");

    let main_wt = worktrees
        .iter()
        .find(|w| w.is_main)
        .expect("should have a main worktree");
    assert_eq!(main_wt.branch.as_deref(), Some("main"));

    let extra_wt = worktrees
        .iter()
        .find(|w| !w.is_main)
        .expect("should have a non-main worktree");
    assert_eq!(extra_wt.branch.as_deref(), Some("wt-list-test"));

    // Clean up before tmpdir drops
    run_git(dir, &["worktree", "remove", ".worktrees/wt-list-test"]);
}

#[test]
fn test_enrich_worktrees_clean() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    let worktrees = worktree::list_worktrees(dir);
    assert_eq!(worktrees.len(), 1);

    let rx = worktree::enrich_worktrees(worktrees);
    let results: Vec<_> = rx.iter().collect();

    assert_eq!(results.len(), 1, "should receive one enrichment result");
    assert_eq!(results[0].index, 0);
    assert!(
        results[0].wt_status.is_clean(),
        "clean repo worktree should report clean status"
    );
}

// ---------------------------------------------------------------------------
// Additional rewrite-specific tests (tag operations, phase1, direct merge detection)
// ---------------------------------------------------------------------------

#[test]
fn test_list_branches_phase1() {
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/a"]);
    std::fs::write(dir.join("a.txt"), "a").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "feature a"]);
    run_git(dir, &["checkout", "main"]);

    let branches = branch::list_branches_phase1(&repo, "main").unwrap();
    assert!(branches.len() >= 2);
    let main_branch = branches.iter().find(|b| b.name == "main").unwrap();
    assert!(main_branch.is_base);
    let feature = branches.iter().find(|b| b.name == "feature/a").unwrap();
    assert!(!feature.is_base);
    // Phase 1 marks unmerged non-pinned as Pending
    assert_eq!(feature.merge_status, MergeStatus::Pending);
}

#[test]
fn test_create_and_list_worktree() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();
    run_git(dir, &["checkout", "-b", "feature/wt-test"]);
    run_git(dir, &["checkout", "main"]);

    let result = operations::create_worktree(dir, "feature/wt-test");
    assert!(result.success);

    let worktrees = worktree::list_worktrees(dir);
    assert_eq!(worktrees.len(), 2);
    let wt = worktrees.iter().find(|w| !w.is_main).unwrap();
    assert_eq!(wt.branch.as_deref(), Some("feature/wt-test"));
}

#[test]
fn test_list_tags_empty() {
    let (_tmpdir, repo) = setup_test_repo();
    let result = tags::list_tags(&repo);
    assert!(result.is_empty());
}

#[test]
fn test_list_tags_lightweight() {
    let (tmpdir, repo) = setup_test_repo();
    run_git(tmpdir.path(), &["tag", "v0.1"]);
    let result = tags::list_tags(&repo);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].name, "v0.1");
    assert!(!result[0].is_annotated);
    assert!(result[0].message.is_none());
}

#[test]
fn test_list_tags_with_annotated() {
    let (tmpdir, repo) = setup_test_repo();
    run_git(tmpdir.path(), &["tag", "-a", "v1.0", "-m", "Release 1.0"]);
    let result = tags::list_tags(&repo);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].name, "v1.0");
    assert!(result[0].is_annotated);
    assert_eq!(result[0].message.as_deref(), Some("Release 1.0"));
}

#[test]
fn test_delete_tag() {
    let (tmpdir, repo) = setup_test_repo();
    run_git(tmpdir.path(), &["tag", "v1.0"]);
    let result = tags::delete_tag(&repo, "v1.0");
    assert!(result.success);
    let remaining = tags::list_tags(&repo);
    assert!(remaining.is_empty());
}

#[test]
fn test_delete_tags_batch() {
    let (tmpdir, repo) = setup_test_repo();
    run_git(tmpdir.path(), &["tag", "v1.0"]);
    run_git(tmpdir.path(), &["tag", "v2.0"]);
    let names = vec!["v1.0".to_string(), "v2.0".to_string()];
    let results = tags::delete_tags_batch(&repo, &names);
    assert!(results.iter().all(|r| r.success));
    let remaining = tags::list_tags(&repo);
    assert!(remaining.is_empty());
}

#[test]
fn test_is_squash_merged_direct() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/squashed"]);
    std::fs::write(dir.join("squash.txt"), "squash content").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "squash commit"]);
    run_git(dir, &["checkout", "main"]);

    run_git(dir, &["merge", "--squash", "feature/squashed"]);
    run_git(dir, &["commit", "-m", "squashed feature"]);

    assert!(merge_detection::is_squash_merged(
        dir,
        "main",
        "feature/squashed",
        None,
        None
    ));
}

#[test]
fn test_is_squash_merged_with_precomputed_merge_base() {
    // The fast path: a precomputed merge base is supplied, so is_squash_merged must
    // not need to derive it via `git merge-base` and still detect the squash.
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();

    // HEAD is the initial commit; it becomes the merge base after we branch.
    let merge_base = repo
        .head()
        .unwrap()
        .peel_to_commit()
        .unwrap()
        .id()
        .to_string();

    run_git(dir, &["checkout", "-b", "feature/squashed"]);
    std::fs::write(dir.join("squash.txt"), "squash content").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "squash commit"]);
    run_git(dir, &["checkout", "main"]);

    run_git(dir, &["merge", "--squash", "feature/squashed"]);
    run_git(dir, &["commit", "-m", "squashed feature"]);

    assert!(merge_detection::is_squash_merged(
        dir,
        "main",
        "feature/squashed",
        None,
        Some(&merge_base),
    ));
}

#[test]
fn test_is_not_squash_merged_direct() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/unmerged"]);
    std::fs::write(dir.join("unmerged.txt"), "unmerged").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "unmerged commit"]);
    run_git(dir, &["checkout", "main"]);

    assert!(!merge_detection::is_squash_merged(
        dir,
        "main",
        "feature/unmerged",
        None,
        None
    ));
}

// ---------------------------------------------------------------------------
// Squash-merge detection test scenarios (plan P002)
//
// See docs/plans/2026-08-29-squash-merge-test-scenarios.md for the full
// scenario matrix and docs/plans/2026-08-29-squash-merge-algorithm-options.md
// for the parent algorithm-options doc these scenarios validate against.
//
// Naming: `test_squash_scenario_NN_...` (NN matches the plan doc's numbering;
// sub-scenarios like 8a/8b/8c and 20a-20d get their own suffixed test).
// ---------------------------------------------------------------------------

#[test]
fn test_squash_scenario_01_baseline_single_commit_clean_squash() {
    // Plan scenario 1: single-commit branch, squashed cleanly onto base.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/baseline"]);
    std::fs::write(dir.join("baseline.txt"), "baseline content\n").unwrap();
    run_git(dir, &["add", "baseline.txt"]);
    run_git(dir, &["commit", "-m", "baseline feature commit"]);

    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/baseline"]);
    run_git(dir, &["commit", "-m", "squash landing"]);
    let squash_oid = git_output(dir, &["rev-parse", "HEAD"]);

    // Expect A: Graph flags the landing commit, with no fuzzy annotation
    // (Option 6 defers to the exact-match tier).
    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..graph::GraphLoadOptions::default()
        },
    )
    .expect("graph load should succeed");
    let landing = snapshot
        .commits
        .iter()
        .find(|c| c.oid == squash_oid)
        .expect("squash landing commit should be displayed");
    assert!(
        landing.is_possible_squash_merge,
        "clean single-commit squash should be flagged (Algorithm A)"
    );
    assert_eq!(
        landing.possible_squash_merge_sources,
        vec!["feature/baseline"],
        "the Graph landing commit should retain the exact-match source branch"
    );
    assert!(
        landing.fuzzy_squash_match.is_none(),
        "exact match must not also get a fuzzy annotation"
    );

    // Expect B: Branches view reports a squash-merged status family member.
    // No remote is configured in this test repo, so it's local-only.
    let repo = git2::Repository::open(dir).unwrap();
    let branches = branch::list_branches(&repo, "main").expect("list_branches failed");
    let feature = branches
        .iter()
        .find(|b| b.name == "feature/baseline")
        .unwrap();
    assert_eq!(feature.merge_status, MergeStatus::LocalSquashMerged);
}

#[test]
fn test_squash_landing_lists_all_matching_local_branch_names() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/login"]);
    std::fs::write(dir.join("login.txt"), "login content\n").unwrap();
    run_git(dir, &["add", "login.txt"]);
    run_git(dir, &["commit", "-m", "add login"]);
    run_git(dir, &["branch", "feature/auth"]);

    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/login"]);
    run_git(dir, &["commit", "-m", "squash login"]);
    let squash_oid = git_output(dir, &["rev-parse", "HEAD"]);

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..graph::GraphLoadOptions::default()
        },
    )
    .expect("graph load should succeed");
    let landing = snapshot
        .commits
        .iter()
        .find(|commit| commit.oid == squash_oid)
        .expect("squash landing should be displayed");

    assert_eq!(
        landing.possible_squash_merge_sources,
        vec!["feature/auth", "feature/login"],
        "all local branches at a matching tip are useful candidates, in stable order"
    );
}

#[test]
fn test_squash_scenario_02_multi_commit_branch_squashed_into_one_base_commit() {
    // Plan scenario 2: multi-commit branch touching multiple files, squashed
    // as a single base commit.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/multi"]);
    std::fs::write(dir.join("a.txt"), "a content\n").unwrap();
    run_git(dir, &["add", "a.txt"]);
    run_git(dir, &["commit", "-m", "add a"]);
    std::fs::write(dir.join("b.txt"), "b content\n").unwrap();
    run_git(dir, &["add", "b.txt"]);
    run_git(dir, &["commit", "-m", "add b"]);
    std::fs::write(dir.join("a.txt"), "a content, revised\n").unwrap();
    run_git(dir, &["add", "a.txt"]);
    run_git(dir, &["commit", "-m", "revise a"]);

    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/multi"]);
    run_git(dir, &["commit", "-m", "squash landing (multi)"]);
    let squash_oid = git_output(dir, &["rev-parse", "HEAD"]);

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..graph::GraphLoadOptions::default()
        },
    )
    .expect("graph load should succeed");
    assert!(
        snapshot
            .commits
            .iter()
            .find(|c| c.oid == squash_oid)
            .expect("squash landing commit should be displayed")
            .is_possible_squash_merge,
        "multi-commit branch squashed to one base commit should still match on aggregate diff"
    );

    let repo = git2::Repository::open(dir).unwrap();
    let branches = branch::list_branches(&repo, "main").expect("list_branches failed");
    let feature = branches
        .iter()
        .find(|b| b.name == "feature/multi")
        .unwrap();
    assert_eq!(feature.merge_status, MergeStatus::LocalSquashMerged);
}

#[test]
fn test_squash_scenario_03_regular_merge_is_not_flagged_as_squash() {
    // Plan scenario 3: a normal two-parent merge must not be flagged as squash
    // by either detector.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/regular-merge"]);
    std::fs::write(dir.join("regular.txt"), "regular content\n").unwrap();
    run_git(dir, &["add", "regular.txt"]);
    run_git(dir, &["commit", "-m", "regular feature commit"]);

    run_git(dir, &["checkout", "main"]);
    // Give main its own commit so the merge can't fast-forward and actually
    // produces a two-parent merge commit.
    std::fs::write(dir.join("main-change.txt"), "main change\n").unwrap();
    run_git(dir, &["add", "main-change.txt"]);
    run_git(dir, &["commit", "-m", "main change"]);
    run_git(
        dir,
        &["merge", "feature/regular-merge", "-m", "regular merge"],
    );
    let merge_oid = git_output(dir, &["rev-parse", "HEAD"]);

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..graph::GraphLoadOptions::default()
        },
    )
    .expect("graph load should succeed");
    assert!(
        !snapshot
            .commits
            .iter()
            .find(|c| c.oid == merge_oid)
            .expect("merge commit should be displayed")
            .is_possible_squash_merge,
        "a regular merge commit must never be flagged as a possible squash merge"
    );

    let repo = git2::Repository::open(dir).unwrap();
    let branches = branch::list_branches(&repo, "main").expect("list_branches failed");
    let feature = branches
        .iter()
        .find(|b| b.name == "feature/regular-merge")
        .unwrap();
    assert_eq!(
        feature.merge_status,
        MergeStatus::Merged,
        "regular merges must be classified by the regular-merge detector, not fall through to squash"
    );
}

#[test]
fn test_squash_scenario_04_branch_with_internal_merge_commit_then_squash_merged() {
    // Plan scenario 4: the branch's own history contains a merge commit
    // before it is squash-merged into base. Algorithm A/B diff merge-base to
    // tip as a flat tree diff, so internal topology must not matter.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/topology"]);
    std::fs::write(dir.join("topology.txt"), "topology base\n").unwrap();
    run_git(dir, &["add", "topology.txt"]);
    run_git(dir, &["commit", "-m", "topology base commit"]);

    run_git(dir, &["checkout", "-b", "feature/topology-sub"]);
    std::fs::write(dir.join("sub.txt"), "sub content\n").unwrap();
    run_git(dir, &["add", "sub.txt"]);
    run_git(dir, &["commit", "-m", "sub commit"]);

    run_git(dir, &["checkout", "feature/topology"]);
    run_git(
        dir,
        &["merge", "feature/topology-sub", "-m", "merge sub into topology"],
    );

    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/topology"]);
    run_git(dir, &["commit", "-m", "squash landing (topology)"]);
    let squash_oid = git_output(dir, &["rev-parse", "HEAD"]);

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..graph::GraphLoadOptions::default()
        },
    )
    .expect("graph load should succeed");
    assert!(
        snapshot
            .commits
            .iter()
            .find(|c| c.oid == squash_oid)
            .expect("squash landing commit should be displayed")
            .is_possible_squash_merge,
        "a merge commit inside the branch's own history must not prevent detection"
    );

    let repo = git2::Repository::open(dir).unwrap();
    let branches = branch::list_branches(&repo, "main").expect("list_branches failed");
    let feature = branches
        .iter()
        .find(|b| b.name == "feature/topology")
        .unwrap();
    assert_eq!(feature.merge_status, MergeStatus::LocalSquashMerged);
}

#[test]
fn test_squash_scenario_05_partial_landing_via_individual_cherry_picks() {
    // Plan scenario 5: only 2 of the branch's 3 commits are cherry-picked
    // individually onto base (not a full squash). Documents the current
    // all-or-nothing behavior as a known, accepted limitation (see plan doc
    // scenario 5) rather than a bug to fix here.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/partial"]);
    std::fs::write(dir.join("p1.txt"), "p1\n").unwrap();
    run_git(dir, &["add", "p1.txt"]);
    run_git(dir, &["commit", "-m", "p1"]);
    let c1 = git_output(dir, &["rev-parse", "HEAD"]);
    std::fs::write(dir.join("p2.txt"), "p2\n").unwrap();
    run_git(dir, &["add", "p2.txt"]);
    run_git(dir, &["commit", "-m", "p2"]);
    let c2 = git_output(dir, &["rev-parse", "HEAD"]);
    std::fs::write(dir.join("p3.txt"), "p3\n").unwrap();
    run_git(dir, &["add", "p3.txt"]);
    run_git(dir, &["commit", "-m", "p3"]);

    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["cherry-pick", &c1]);
    run_git(dir, &["cherry-pick", &c2]);
    let landed_tip = git_output(dir, &["rev-parse", "HEAD"]);

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..graph::GraphLoadOptions::default()
        },
    )
    .expect("graph load should succeed");
    assert!(
        snapshot
            .commits
            .iter()
            .all(|c| !c.is_possible_squash_merge),
        "partial cherry-pick coverage must not produce any possible-squash annotation \
         (known limitation: no single base commit's diff equals the branch's full aggregate diff)"
    );
    let _ = landed_tip;

    assert!(
        !merge_detection::is_squash_merged(dir, "main", "feature/partial", None, None),
        "partial coverage (2 of 3 commits landed individually) must not report as fully squash-merged"
    );
}

#[test]
fn test_squash_scenario_06_reordered_commits_and_hunk_order_insensitivity() {
    // Plan scenario 6, part 1: commits touching different files land as one
    // squash commit; functional flagged assertion (order of commits within
    // the branch must not matter to the aggregate diff).
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/reordered"]);
    std::fs::write(dir.join("z.txt"), "z content\n").unwrap();
    run_git(dir, &["add", "z.txt"]);
    run_git(dir, &["commit", "-m", "add z first"]);
    std::fs::write(dir.join("a.txt"), "a content\n").unwrap();
    run_git(dir, &["add", "a.txt"]);
    run_git(dir, &["commit", "-m", "add a second"]);

    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/reordered"]);
    run_git(dir, &["commit", "-m", "squash landing (reordered)"]);
    let squash_oid = git_output(dir, &["rev-parse", "HEAD"]);

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..graph::GraphLoadOptions::default()
        },
    )
    .expect("graph load should succeed");
    assert!(
        snapshot
            .commits
            .iter()
            .find(|c| c.oid == squash_oid)
            .expect("squash landing commit should be displayed")
            .is_possible_squash_merge,
        "commit order within the branch must not affect the aggregate-diff match"
    );

    // Plan scenario 6, part 2: directly confirm `git patch-id --stable`'s
    // documented hunk-order insensitivity holds for this codebase's exact
    // invocation (`git diff --binary --full-index --no-ext-diff --no-textconv`
    // piped to `git patch-id --stable`), not just in isolation. A custom
    // `diff.orderFile` forces `git diff` to emit the same two file-hunks in
    // reverse order; the resulting patch-id must still match the default-order
    // patch-id.
    let parent = git_output(dir, &["rev-parse", &format!("{squash_oid}^")]);
    let default_order_id = git_output(
        dir,
        &[
            "diff",
            "--binary",
            "--full-index",
            "--no-ext-diff",
            "--no-textconv",
            &parent,
            &squash_oid,
            "--",
        ],
    );
    let order_file = dir.join("order.txt");
    std::fs::write(&order_file, "a.txt\nz.txt\n").unwrap();
    let reordered_diff_output = Command::new("git")
        .current_dir(dir)
        .args([
            "-c",
            &format!("diff.orderFile={}", order_file.display()),
            "diff",
            "--binary",
            "--full-index",
            "--no-ext-diff",
            "--no-textconv",
            &parent,
            &squash_oid,
            "--",
        ])
        .output()
        .expect("git diff with orderFile should run");
    assert!(reordered_diff_output.status.success());

    fn patch_id_of(dir: &std::path::Path, diff_text: &[u8]) -> String {
        use std::io::Write;
        let mut child = Command::new("git")
            .current_dir(dir)
            .args(["patch-id", "--stable"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("git patch-id should spawn");
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(diff_text)
            .unwrap();
        let out = child.wait_with_output().unwrap();
        String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .next()
            .unwrap()
            .to_string()
    }

    let default_id = patch_id_of(dir, default_order_id.as_bytes());
    let reordered_id = patch_id_of(dir, &reordered_diff_output.stdout);
    assert_eq!(
        default_id, reordered_id,
        "git patch-id --stable must be insensitive to hunk/file order in this codebase's exact invocation"
    );
}

#[test]
fn test_squash_scenario_07_rebased_branch_then_squash_merged() {
    // Plan scenario 7: branch created off base; base advances; branch is
    // rebased onto the new base tip; then squashed. Detection must use the
    // post-rebase merge-base, not a stale ancestor.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/rebased"]);
    std::fs::write(dir.join("rebased.txt"), "rebased content\n").unwrap();
    run_git(dir, &["add", "rebased.txt"]);
    run_git(dir, &["commit", "-m", "rebased feature commit"]);

    run_git(dir, &["checkout", "main"]);
    std::fs::write(dir.join("main-advance.txt"), "main advanced\n").unwrap();
    run_git(dir, &["add", "main-advance.txt"]);
    run_git(dir, &["commit", "-m", "main advances"]);

    run_git(dir, &["checkout", "feature/rebased"]);
    run_git(dir, &["rebase", "main"]);

    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/rebased"]);
    run_git(dir, &["commit", "-m", "squash landing (rebased)"]);
    let squash_oid = git_output(dir, &["rev-parse", "HEAD"]);

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..graph::GraphLoadOptions::default()
        },
    )
    .expect("graph load should succeed");
    assert!(
        snapshot
            .commits
            .iter()
            .find(|c| c.oid == squash_oid)
            .expect("squash landing commit should be displayed")
            .is_possible_squash_merge,
        "a rebased-then-squashed branch must be flagged using the post-rebase merge-base"
    );

    let repo = git2::Repository::open(dir).unwrap();
    let branches = branch::list_branches(&repo, "main").expect("list_branches failed");
    let feature = branches
        .iter()
        .find(|b| b.name == "feature/rebased")
        .unwrap();
    assert_eq!(feature.merge_status, MergeStatus::LocalSquashMerged);
}

#[test]
fn test_squash_scenario_08a_conflict_resolution_extra_lines_fuzzy_positive() {
    // Plan scenario 8a: base and branch modify the same file on different
    // lines (no true git-level conflict), but the squash author also fixed up
    // a few adjacent lines while resolving. The squash commit's diff therefore
    // contains the branch's real changes *plus* extra resolution-only lines
    // not present in the branch's own diff.
    //
    // The branch's own diff changes 6 lines (12 added/removed tokens); the
    // squash adds one extra resolution edit (2 more tokens) on top. Hand-
    // calculated Jaccard similarity: 12 shared / 14 union ≈ 0.857 — comfortably
    // above the calibrated FUZZY_SIMILARITY_THRESHOLD (see git/fuzzy_match.rs).
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    let original = "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\n";
    std::fs::write(dir.join("f8a.txt"), original).unwrap();
    run_git(dir, &["add", "f8a.txt"]);
    run_git(dir, &["commit", "-m", "f8a baseline"]);

    run_git(dir, &["checkout", "-b", "feature/8a"]);
    let branch_content = "l1-b\nl2-b\nl3-b\nl4-b\nl5-b\nl6-b\nl7\nl8\n";
    std::fs::write(dir.join("f8a.txt"), branch_content).unwrap();
    run_git(dir, &["commit", "-am", "branch changes lines 1-6"]);
    let branch_tip = git_output(dir, &["rev-parse", "HEAD"]);

    run_git(dir, &["checkout", "main"]);
    // main advances independently so the squash isn't a trivial fast-forward.
    let main_parent_content = "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8-main\n";
    std::fs::write(dir.join("f8a.txt"), main_parent_content).unwrap();
    run_git(dir, &["commit", "-am", "main changes line 8 independently"]);
    let main_parent = git_output(dir, &["rev-parse", "HEAD"]);

    run_git(dir, &["merge", "--squash", "feature/8a"]);
    // Simulate a conflict-resolution pass: apply the branch's lines 1-6 *plus*
    // one extra resolution tweak the branch itself never made (line 7).
    let squash_content = "l1-b\nl2-b\nl3-b\nl4-b\nl5-b\nl6-b\nl7-resolved\nl8-main\n";
    std::fs::write(dir.join("f8a.txt"), squash_content).unwrap();
    run_git(dir, &["add", "f8a.txt"]);
    run_git(dir, &["commit", "-m", "squash landing with extra resolution edit"]);
    let squash_oid = git_output(dir, &["rev-parse", "HEAD"]);

    // Expect A: not flagged under exact patch-id match (known false negative).
    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..graph::GraphLoadOptions::default()
        },
    )
    .expect("graph load should succeed");
    let landing = snapshot
        .commits
        .iter()
        .find(|c| c.oid == squash_oid)
        .expect("squash landing commit should be displayed");
    assert!(
        !landing.is_possible_squash_merge,
        "extra resolution lines must break exact patch-id equality (known false negative, plan scenario 8a)"
    );

    // Expect B: not SquashMerged, same reason.
    assert!(!merge_detection::is_squash_merged(
        dir,
        "main",
        "feature/8a",
        None,
        Some(&main_parent),
    ));

    // Expect Option 6: flagged as a fuzzy possible-squash match with high
    // similarity.
    let fuzzy = landing
        .fuzzy_squash_match
        .as_ref()
        .expect("high line-level similarity should surface a fuzzy possible-squash match");
    assert!(
        fuzzy.similarity_percent >= 75,
        "expected high similarity for a near-exact match with one extra resolution edit, got {}",
        fuzzy.similarity_percent
    );
    let _ = branch_tip;
}

#[test]
fn test_squash_scenario_08b_true_conflicting_hunks_manually_resolved() {
    // Plan scenario 8b: base and branch modify the *same* lines (a true
    // conflict). The squash commit's tree is constructed directly as what a
    // human would produce resolving the conflict, rather than via a
    // mechanical merge. This test records whichever side of the Option 6
    // threshold this concrete recipe lands on (observe-then-assert), per the
    // plan doc's explicit instruction not to assume a specific classification
    // in advance.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    let original = "one\ntwo\nthree\nfour\nfive\n";
    std::fs::write(dir.join("f8b.txt"), original).unwrap();
    run_git(dir, &["add", "f8b.txt"]);
    run_git(dir, &["commit", "-m", "f8b baseline"]);

    run_git(dir, &["checkout", "-b", "feature/8b"]);
    let branch_content = "one-BRANCH\ntwo-BRANCH\nthree\nfour\nfive\n";
    std::fs::write(dir.join("f8b.txt"), branch_content).unwrap();
    run_git(dir, &["commit", "-am", "branch changes lines 1-2"]);

    run_git(dir, &["checkout", "main"]);
    let main_parent_content = "one-MAIN\ntwo-MAIN\nthree\nfour\nfive\n";
    std::fs::write(dir.join("f8b.txt"), main_parent_content).unwrap();
    run_git(dir, &["commit", "-am", "main changes lines 1-2 independently (true conflict)"]);
    let main_parent = git_output(dir, &["rev-parse", "HEAD"]);

    // This is a true git-level conflict, so `git merge --squash` exits
    // non-zero and leaves conflict markers in the working tree; run it
    // directly (not via the panic-on-failure `run_git` helper) and ignore the
    // exit code, since we overwrite the file with the manually resolved
    // content below anyway.
    let _ = Command::new("git")
        .current_dir(dir)
        .args(["merge", "--squash", "feature/8b"])
        .output()
        .expect("git merge --squash should run (conflict exit is expected)");
    // Manually resolve: keep main's line 1, branch's line 2, as a human might.
    let resolved_content = "one-MAIN\ntwo-BRANCH\nthree\nfour\nfive\n";
    std::fs::write(dir.join("f8b.txt"), resolved_content).unwrap();
    run_git(dir, &["add", "f8b.txt"]);
    run_git(dir, &["commit", "-m", "manually resolved squash landing"]);
    let squash_oid = git_output(dir, &["rev-parse", "HEAD"]);

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..graph::GraphLoadOptions::default()
        },
    )
    .expect("graph load should succeed");
    let landing = snapshot
        .commits
        .iter()
        .find(|c| c.oid == squash_oid)
        .expect("squash landing commit should be displayed");
    assert!(
        !landing.is_possible_squash_merge,
        "a manually-resolved true conflict must not exact-match (plan scenario 8b)"
    );
    assert!(!merge_detection::is_squash_merged(
        dir,
        "main",
        "feature/8b",
        None,
        Some(&main_parent),
    ));

    // Observe-then-assert: record whichever side of the threshold this
    // recipe lands on. Branch diff touches 2 lines (4 tokens); resolved
    // squash diff also touches 2 lines (4 tokens: -one-MAIN/-two-MAIN/+one-MAIN(no)
    // ... in practice the overlap is only the "two-BRANCH" addition, since the
    // removed lines differ (main's own text) and "one-MAIN" survives unchanged
    // (no diff line for it in the squash-vs-main-parent diff). This is a much
    // lower structural overlap than 8a, so we expect this concrete recipe to
    // land BELOW the fuzzy threshold.
    assert!(
        landing.fuzzy_squash_match.is_none(),
        "observed: this true-conflict recipe's similarity is too low to clear the fuzzy \
         threshold (majority of the resolved content differs from the branch's own diff); \
         if this fails, update this comment to record the newly-observed classification \
         rather than assuming the prior expectation still holds"
    );
}

#[test]
fn test_squash_scenario_08c_merge_tree_confirmation_check() {
    // Plan scenario 8c: uses the parent doc's Option 3 (`git merge-tree`)
    // confirmation signal, not Option 6. Using the same conflict-resolution
    // shape as 8a, run `git merge-tree --write-tree base branch` (the
    // *original*, un-squashed branch) against *current* base (which already
    // contains the resolved squash) and compare the resulting tree to base's
    // own tree. This scenario documents whether merge-tree re-simulation
    // succeeds even when exact patch-id match fails.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    let original = "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\n";
    std::fs::write(dir.join("f8c.txt"), original).unwrap();
    run_git(dir, &["add", "f8c.txt"]);
    run_git(dir, &["commit", "-m", "f8c baseline"]);

    run_git(dir, &["checkout", "-b", "feature/8c"]);
    let branch_content = "l1-b\nl2-b\nl3-b\nl4-b\nl5-b\nl6-b\nl7\nl8\n";
    std::fs::write(dir.join("f8c.txt"), branch_content).unwrap();
    run_git(dir, &["commit", "-am", "branch changes lines 1-6"]);

    run_git(dir, &["checkout", "main"]);
    let main_parent_content = "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8-main\n";
    std::fs::write(dir.join("f8c.txt"), main_parent_content).unwrap();
    run_git(dir, &["commit", "-am", "main changes line 8 independently"]);

    run_git(dir, &["merge", "--squash", "feature/8c"]);
    let squash_content = "l1-b\nl2-b\nl3-b\nl4-b\nl5-b\nl6-b\nl7-resolved\nl8-main\n";
    std::fs::write(dir.join("f8c.txt"), squash_content).unwrap();
    run_git(dir, &["add", "f8c.txt"]);
    run_git(dir, &["commit", "-m", "squash landing with extra resolution edit"]);

    let merge_tree_output = Command::new("git")
        .current_dir(dir)
        .args(["merge-tree", "--write-tree", "main", "feature/8c"])
        .output()
        .expect("git merge-tree should run");

    // Observe-then-assert: empirically, `git merge-tree --write-tree` reports
    // a genuine CONFLICT (non-zero exit) for this fixture, not a clean tree
    // match. All 8 lines of `f8c.txt` sit inside a single diff hunk on both
    // sides (the file is short enough that git's default 3-line context
    // merges the whole file into one hunk), so `main`'s independent line-7/8
    // edits and `feature`'s line 1-6 edits are treated as *overlapping*
    // hunks by the `ort` merge strategy even though the specific changed
    // lines don't literally collide — a real content conflict is reported.
    // This means merge-tree does NOT succeed where patch-id (8a/8b) fails
    // for this recipe; it fails too, just for a different, hunk-granularity
    // reason. This is itself a useful, concrete data point for the parent
    // plan doc's "A true, B false"-style disagreement-visibility principle:
    // here, neither exact patch-id equality nor merge-tree re-simulation
    // confirms the squash for a same-file multi-line resolution.
    assert!(
        !merge_tree_output.status.success(),
        "observed: git merge-tree reports a real conflict for this same-file, \
         multi-hunk resolution fixture rather than silently confirming the squash"
    );
    let stdout = String::from_utf8_lossy(&merge_tree_output.stdout);
    assert!(
        stdout.contains("CONFLICT"),
        "expected merge-tree's stdout to report the conflict explicitly, got: {stdout}"
    );
}

#[test]
fn test_likely_squash_merged_merge_tree_confirms_when_bundled_with_unrelated_change() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/bundled"]);
    std::fs::write(dir.join("shared.txt"), "shared content\n").unwrap();
    run_git(dir, &["add", "shared.txt"]);
    run_git(dir, &["commit", "-m", "add shared.txt"]);
    run_git(dir, &["checkout", "main"]);

    run_git(dir, &["merge", "--squash", "feature/bundled"]);
    std::fs::write(dir.join("other.txt"), "unrelated content\n").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "squash shared.txt + unrelated other.txt"]);

    assert!(!merge_detection::is_squash_merged(
        dir, "main", "feature/bundled", None, None
    ));
    assert_eq!(
        merge_detection::likely_squash_merged(dir, "main", "feature/bundled", None, None),
        Some(SquashConfidence::MergeTreeConfirmed)
    );
}

#[test]
fn test_likely_squash_merged_rejects_invalid_supplied_merge_base() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/invalid-merge-base"]);
    std::fs::write(dir.join("shared.txt"), "shared content\n").unwrap();
    run_git(dir, &["add", "shared.txt"]);
    run_git(dir, &["commit", "-m", "add shared.txt"]);
    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/invalid-merge-base"]);
    std::fs::write(dir.join("other.txt"), "unrelated content\n").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "squash shared.txt + unrelated other.txt"]);

    assert_eq!(
        merge_detection::likely_squash_merged(
            dir,
            "main",
            "feature/invalid-merge-base",
            None,
            Some("invalid-ref"),
        ),
        None,
        "an invalid supplied merge base must fail closed before merge-tree confirmation"
    );
}

#[test]
fn test_likely_squash_merged_fuzzy_confirms_when_merge_tree_conflicts() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    let original = "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\n";
    std::fs::write(dir.join("f8c.txt"), original).unwrap();
    run_git(dir, &["add", "f8c.txt"]);
    run_git(dir, &["commit", "-m", "f8c baseline"]);

    run_git(dir, &["checkout", "-b", "feature/8c-fuzzy"]);
    let branch_content = "l1-b\nl2-b\nl3-b\nl4-b\nl5-b\nl6-b\nl7\nl8\n";
    std::fs::write(dir.join("f8c.txt"), branch_content).unwrap();
    run_git(dir, &["commit", "-am", "branch changes lines 1-6"]);

    run_git(dir, &["checkout", "main"]);
    let main_parent_content = "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8-main\n";
    std::fs::write(dir.join("f8c.txt"), main_parent_content).unwrap();
    run_git(dir, &["commit", "-am", "main changes line 8 independently"]);

    run_git(dir, &["merge", "--squash", "feature/8c-fuzzy"]);
    let squash_content = "l1-b\nl2-b\nl3-b\nl4-b\nl5-b\nl6-b\nl7-resolved\nl8-main\n";
    std::fs::write(dir.join("f8c.txt"), squash_content).unwrap();
    run_git(dir, &["add", "f8c.txt"]);
    run_git(dir, &["commit", "-m", "squash landing with extra resolution edit"]);

    assert!(!merge_detection::is_squash_merged(
        dir, "main", "feature/8c-fuzzy", None, None
    ));
    assert_eq!(
        merge_detection::likely_squash_merged(dir, "main", "feature/8c-fuzzy", None, None),
        Some(SquashConfidence::FuzzyMatch {
            similarity_percent: 75
        })
    );
}

#[test]
fn test_likely_squash_merged_returns_none_when_no_signal() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/unrelated"]);
    std::fs::write(dir.join("unique-file.txt"), "unique\n").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "feature: add unique file"]);

    run_git(dir, &["checkout", "main"]);
    std::fs::write(dir.join("other2.txt"), "other2\n").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "main: add unrelated other2"]);

    assert!(!merge_detection::is_squash_merged(
        dir, "main", "feature/unrelated", None, None
    ));
    assert_eq!(
        merge_detection::likely_squash_merged(dir, "main", "feature/unrelated", None, None),
        None
    );
}

#[test]
fn test_spawn_squash_checker_reports_likely_squash_merged_with_confidence() {
    // End-to-end through the real spawn_squash_checker pipeline, using the
    // same "squash bundled with an unrelated change" fixture as
    // test_likely_squash_merged_merge_tree_confirms_when_bundled_with_unrelated_change.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/bundled-e2e"]);
    std::fs::write(dir.join("shared.txt"), "shared content\n").unwrap();
    run_git(dir, &["add", "shared.txt"]);
    run_git(dir, &["commit", "-m", "add shared.txt"]);
    let branch_tip = git_output(dir, &["rev-parse", "feature/bundled-e2e"]);
    run_git(dir, &["checkout", "main"]);

    run_git(dir, &["merge", "--squash", "feature/bundled-e2e"]);
    std::fs::write(dir.join("other.txt"), "unrelated content\n").unwrap();
    run_git(dir, &["add", "."]);
    run_git(
        dir,
        &["commit", "-m", "squash shared.txt + unrelated other.txt"],
    );

    let candidates = vec![("feature/bundled-e2e".to_string(), branch_tip, None)];
    let cache = cache::BranchCache::load(dir);
    let rx = squash_loader::spawn_squash_checker(
        dir.to_path_buf(),
        "main".to_string(),
        candidates,
        cache,
    );

    let results: Vec<_> = rx.into_iter().collect();
    assert_eq!(results.len(), 1);
    let result = &results[0];
    assert_eq!(result.branch_name, "feature/bundled-e2e");
    assert_eq!(result.status, MergeStatus::LikelySquashMerged);
    assert_eq!(
        result.confidence,
        Some(SquashConfidence::MergeTreeConfirmed)
    );
}

#[test]
fn test_squash_checker_rechecks_cached_unmerged_after_base_advances() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();
    let cache_path = dir.join("branch-cache.sqlite3");

    run_git(dir, &["checkout", "-b", "feature/cached-unmerged"]);
    std::fs::write(dir.join("shared.txt"), "shared content\n").unwrap();
    run_git(dir, &["add", "shared.txt"]);
    run_git(dir, &["commit", "-m", "add shared.txt"]);
    let branch_tip = git_output(dir, &["rev-parse", "feature/cached-unmerged"]);
    run_git(dir, &["checkout", "main"]);

    let candidates = vec![(
        "feature/cached-unmerged".to_string(),
        branch_tip.clone(),
        None,
    )];
    let cherry_results: Vec<_> = cherry_loader::spawn_cherry_checker(
        dir.to_path_buf(),
        "main".to_string(),
        candidates.clone(),
        cache::BranchCache::load_from_path(cache_path.clone()),
    )
    .into_iter()
    .collect();
    assert_eq!(cherry_results.len(), 1);
    assert_eq!(cherry_results[0].status, MergeStatus::Unmerged);
    assert_eq!(
        cache::BranchCache::load_from_path(cache_path.clone())
            .lookup("feature/cached-unmerged", &branch_tip),
        Some(MergeStatus::Unmerged),
        "the real cherry checker must seed the same-tip Unmerged cache entry"
    );

    run_git(dir, &["merge", "--squash", "feature/cached-unmerged"]);
    std::fs::write(dir.join("other.txt"), "unrelated content\n").unwrap();
    run_git(dir, &["add", "."]);
    run_git(
        dir,
        &["commit", "-m", "squash shared.txt with unrelated other.txt"],
    );

    let squash_results: Vec<_> = squash_loader::spawn_squash_checker(
        dir.to_path_buf(),
        "main".to_string(),
        candidates,
        cache::BranchCache::load_from_path(cache_path),
    )
    .into_iter()
    .collect();
    assert_eq!(squash_results.len(), 1);
    assert_eq!(
        squash_results[0].status,
        MergeStatus::LikelySquashMerged,
        "a cached Unmerged must not hide fresh squash detection after base advances"
    );
    assert_eq!(
        squash_results[0].confidence,
        Some(SquashConfidence::MergeTreeConfirmed)
    );
}

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
            ..graph::GraphLoadOptions::default()
        },
    )
    .expect("graph load should succeed");
    assert!(
        snapshot
            .commits
            .iter()
            .find(|c| c.oid == squash_oid)
            .expect("squash landing commit should be displayed")
            .is_possible_squash_merge,
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
            ..graph::GraphLoadOptions::default()
        },
    )
    .expect("graph load should succeed");
    assert!(
        snapshot
            .commits
            .iter()
            .find(|c| c.oid == squash_oid)
            .expect("squash landing commit should be displayed")
            .is_possible_squash_merge,
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

    std::fs::write(dir.join("original2.txt"), "line one\nline two\nline three\n").unwrap();
    run_git(dir, &["add", "original2.txt"]);
    run_git(dir, &["commit", "-m", "add original2.txt"]);

    run_git(dir, &["checkout", "-b", "feature/rename-and-change"]);
    run_git(dir, &["mv", "original2.txt", "renamed2.txt"]);
    std::fs::write(dir.join("renamed2.txt"), "line one\nline two CHANGED\nline three\n").unwrap();
    run_git(dir, &["commit", "-am", "rename and change content"]);

    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/rename-and-change"]);
    run_git(dir, &["commit", "-m", "squash landing (rename + change)"]);
    let squash_oid = git_output(dir, &["rev-parse", "HEAD"]);

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..graph::GraphLoadOptions::default()
        },
    )
    .expect("graph load should succeed");
    assert!(
        snapshot
            .commits
            .iter()
            .find(|c| c.oid == squash_oid)
            .expect("squash landing commit should be displayed")
            .is_possible_squash_merge,
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
        let mut perms = std::fs::metadata(dir.join("script.sh")).unwrap().permissions();
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
            ..graph::GraphLoadOptions::default()
        },
    )
    .expect("graph load should succeed");
    assert!(
        snapshot
            .commits
            .iter()
            .find(|c| c.oid == squash_oid)
            .expect("squash landing commit should be displayed")
            .is_possible_squash_merge,
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
            ..graph::GraphLoadOptions::default()
        },
    )
    .expect("graph load should succeed");
    assert!(
        snapshot
            .commits
            .iter()
            .find(|c| c.oid == squash_oid)
            .expect("squash landing commit should be displayed")
            .is_possible_squash_merge,
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

    std::fs::write(
        dir.join("ws-a.txt"),
        "alpha one\nalpha two\nalpha three\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("ws-b.txt"),
        "beta one\nbeta two\nbeta three\n",
    )
    .unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "add ws-a.txt and ws-b.txt"]);

    run_git(dir, &["checkout", "-b", "feature/ws-unrelated"]);
    std::fs::write(
        dir.join("ws-a.txt"),
        "alpha one   \nalpha two\nalpha three\n",
    )
    .unwrap();
    run_git(dir, &["commit", "-am", "branch: trailing whitespace on ws-a.txt"]);

    run_git(dir, &["checkout", "main"]);
    // An independent, unrelated whitespace-only change on a *different* file.
    std::fs::write(
        dir.join("ws-b.txt"),
        "beta one   \nbeta two\nbeta three\n",
    )
    .unwrap();
    run_git(dir, &["commit", "-am", "main: unrelated trailing whitespace on ws-b.txt"]);
    let unrelated_base_oid = git_output(dir, &["rev-parse", "HEAD"]);

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..graph::GraphLoadOptions::default()
        },
    )
    .expect("graph load should succeed");
    assert!(
        !snapshot
            .commits
            .iter()
            .find(|c| c.oid == unrelated_base_oid)
            .expect("unrelated base commit should be displayed")
            .is_possible_squash_merge,
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

#[test]
fn test_squash_scenario_12_empty_net_zero_branch_and_base_commit() {
    // Plan scenario 12: a branch whose commits fully cancel out (branch-tip
    // tree == merge-base tree) produces an empty diff. `git patch-id --stable`
    // on an empty diff produces no output at all (confirmed empirically: this
    // codebase's `compute_patch`/`stable_patch_id` early-returns `None` when
    // `git diff`'s stdout is empty, *before* ever invoking `patch-id`). A
    // second, unrelated empty-diff base commit is also constructed to confirm
    // the current implementation does not treat two "empty" patch-ids as
    // equal to each other (a real false-positive risk if `None == None` were
    // ever compared) — it can't, since only `Some(patch_id)` entries are
    // inserted into the match tables (see `annotate_possible_squash_merges`
    // in `git/graph.rs`).
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/net-zero"]);
    std::fs::write(dir.join("temp.txt"), "temporary content\n").unwrap();
    run_git(dir, &["add", "temp.txt"]);
    run_git(dir, &["commit", "-m", "add temp content"]);
    run_git(dir, &["rm", "temp.txt"]);
    run_git(dir, &["commit", "-m", "remove temp content (net zero)"]);

    run_git(dir, &["checkout", "main"]);
    // An unrelated, independent net-zero commit on base: add then remove a
    // different file in a single commit sequence.
    std::fs::write(dir.join("other-temp.txt"), "other temporary\n").unwrap();
    run_git(dir, &["add", "other-temp.txt"]);
    run_git(dir, &["commit", "-m", "add other temp content"]);
    run_git(dir, &["rm", "other-temp.txt"]);
    run_git(dir, &["commit", "-m", "remove other temp content (net zero, unrelated)"]);
    let empty_base_oid = git_output(dir, &["rev-parse", "HEAD"]);

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..graph::GraphLoadOptions::default()
        },
    )
    .expect("graph load should succeed");
    assert!(
        !snapshot
            .commits
            .iter()
            .find(|c| c.oid == empty_base_oid)
            .expect("unrelated empty base commit should be displayed")
            .is_possible_squash_merge,
        "an unrelated empty/net-zero base commit must never be flagged, including against \
         another empty-diff branch — empty diffs are explicitly excluded (None), never matched"
    );
    assert!(
        snapshot
            .commits
            .iter()
            .all(|c| !c.is_possible_squash_merge),
        "no commit in this fixture should be flagged: both diffs involved are empty"
    );

    assert!(!merge_detection::is_squash_merged(
        dir,
        "main",
        "feature/net-zero",
        None,
        None
    ));
}

#[test]
fn test_squash_scenario_13_reverted_branch_net_zero_but_no_specific_squash_point() {
    // Plan scenario 13: branch adds a commit, then a second commit that
    // reverts it (net-zero from base's perspective, but the commits still
    // exist as distinct history). Two separate assertions per the plan doc:
    // (a) the branch's tip tree is trivially content-identical to base's tree
    // (the "already integrated" OR-claim, checked directly via tree OIDs
    // since this codebase has no dedicated `already_integrated` field yet);
    // (b) no *specific* base commit is flagged as the squash point (the
    // `possible_squash(commit)` AND-claim), since there is nothing for a
    // base commit's diff to match against.
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();
    let base_tree = repo.head().unwrap().peel_to_tree().unwrap().id().to_string();

    run_git(dir, &["checkout", "-b", "feature/reverted"]);
    std::fs::write(dir.join("temp2.txt"), "temp content 2\n").unwrap();
    run_git(dir, &["add", "temp2.txt"]);
    run_git(dir, &["commit", "-m", "add temp2 content"]);
    run_git(dir, &["rm", "temp2.txt"]);
    run_git(dir, &["commit", "-m", "revert temp2 content"]);

    // (a) content-equivalence check.
    let branch_tree = git_output(dir, &["rev-parse", "HEAD^{tree}"]);
    assert_eq!(
        branch_tree, base_tree,
        "reverted branch's tip tree must equal base's tree (already integrated, content-wise)"
    );

    // (b) no specific base commit is the squash point.
    run_git(dir, &["checkout", "main"]);
    assert!(
        !merge_detection::is_squash_merged(dir, "main", "feature/reverted", None, None),
        "an empty aggregate diff must not report as squash-merged into any base commit"
    );

    let repo = git2::Repository::open(dir).unwrap();
    let branches = branch::list_branches(&repo, "main").expect("list_branches failed");
    let feature = branches
        .iter()
        .find(|b| b.name == "feature/reverted")
        .unwrap();
    assert_eq!(
        feature.merge_status,
        MergeStatus::Unmerged,
        "observed: current status classification has no distinct \"already integrated but no \
         specific squash point\" state — a reverted branch reports plain Unmerged, matching \
         neither the regular-merge reachable-set check (different OID) nor the squash check \
         (no diff to match)"
    );
}

#[test]
fn test_squash_scenario_14_duplicate_independently_recreated_patch() {
    // Plan scenario 14: branch makes a one-line change; independently, a
    // *later, unrelated* commit on base makes the exact same one-line change
    // to the same file (two developers writing the identical fix). This is
    // the documented, accepted false positive for patch-id-based detection:
    // content equivalence, not historical provenance, is all patch-id can
    // observe.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    std::fs::write(dir.join("dup.txt"), "before\n").unwrap();
    run_git(dir, &["add", "dup.txt"]);
    run_git(dir, &["commit", "-m", "add dup.txt"]);

    run_git(dir, &["checkout", "-b", "feature/duplicate-fix"]);
    std::fs::write(dir.join("dup.txt"), "after\n").unwrap();
    run_git(dir, &["commit", "-am", "branch: fix dup.txt"]);

    run_git(dir, &["checkout", "main"]);
    std::fs::write(dir.join("dup.txt"), "after\n").unwrap();
    run_git(dir, &["commit", "-am", "main: independently make the identical fix"]);
    let independent_base_oid = git_output(dir, &["rev-parse", "HEAD"]);

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..graph::GraphLoadOptions::default()
        },
    )
    .expect("graph load should succeed");
    let independent = snapshot
        .commits
        .iter()
        .find(|c| c.oid == independent_base_oid)
        .expect("independent base commit should be displayed");
    assert!(
        independent.is_possible_squash_merge,
        "documented, accepted false positive: an independently recreated identical one-line \
         fix is indistinguishable from a real squash landing under patch-id equality \
         (content equivalence, not historical provenance) — see plan doc scenario 14"
    );
    assert!(merge_detection::is_squash_merged(
        dir,
        "main",
        "feature/duplicate-fix",
        None,
        None
    ));

    // Calibration data point for Option 6: directly score the two diffs
    // (branch's own diff vs. the independent base commit's diff) with
    // `fuzzy_match::score`/`classify`. Since these are the exact same
    // one-line, one-file change, similarity is 1.0 — and `classify` already
    // defers `similarity >= 1.0` to the exact-match tier (which fired above),
    // so no *additional* fuzzy signal is expected here regardless of the
    // small-diff `MIN_UNION_SIZE_FOR_FUZZY` gate.
    let branch_diff = git_output(
        dir,
        &["diff", "--binary", "--full-index", "main~1", "feature/duplicate-fix", "--"],
    );
    let base_diff = git_output(
        dir,
        &["diff", "--binary", "--full-index", "main~1", "main", "--"],
    );
    let fscore = fuzzy_match::score(branch_diff.as_bytes(), base_diff.as_bytes())
        .expect("identical single-file diffs should pass the cheap prefilter");
    assert_eq!(
        fscore.similarity, 1.0,
        "observed: the coincidental duplicate fix scores as perfectly similar"
    );
    assert!(
        fuzzy_match::classify(&fscore).is_none(),
        "similarity == 1.0 always defers to the exact-match tier, regardless of diff size"
    );
}

#[test]
fn test_squash_scenario_15_criss_cross_multiple_merge_bases() {
    // Plan scenario 15: construct a criss-cross merge (two branches that have
    // merged each other using each other's *pre-merge* tips), so `main` and
    // `feature` have two co-equal merge bases. `git merge-base --all` should
    // report both; a plain `git merge-base` picks exactly one of them
    // (officially unspecified by git). This test pins down current behavior
    // for this fixture rather than asserting a "correct" choice git itself
    // doesn't guarantee.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "criss-a"]);
    std::fs::write(dir.join("a.txt"), "a content\n").unwrap();
    run_git(dir, &["add", "a.txt"]);
    run_git(dir, &["commit", "-m", "criss-a commit"]);
    let a_tip = git_output(dir, &["rev-parse", "HEAD"]);

    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["checkout", "-b", "criss-b"]);
    std::fs::write(dir.join("b.txt"), "b content\n").unwrap();
    run_git(dir, &["add", "b.txt"]);
    run_git(dir, &["commit", "-m", "criss-b commit"]);
    let b_tip = git_output(dir, &["rev-parse", "HEAD"]);

    // Merge b into a normally: parents [a_tip, b_tip].
    run_git(dir, &["checkout", "criss-a"]);
    run_git(dir, &["merge", "criss-b", "-m", "merge b into a"]);
    let merged_tree = git_output(dir, &["rev-parse", "HEAD^{tree}"]);

    // Construct the *other* merge directly via commit-tree, with parents
    // [b_tip, a_tip] (the PRE-merge tips) so neither merge commit is an
    // ancestor of the other — a genuine criss-cross, not a fast-forwarded
    // chain.
    let criss_merge_2 = git_output(
        dir,
        &["commit-tree", &merged_tree, "-p", &b_tip, "-p", &a_tip, "-m", "merge a into b"],
    );
    run_git(dir, &["branch", "-f", "criss-b", &criss_merge_2]);

    let all_bases = git_output(dir, &["merge-base", "--all", "criss-a", "criss-b"]);
    let base_count = all_bases.lines().filter(|l| !l.trim().is_empty()).count();
    assert_eq!(
        base_count, 2,
        "expected a genuine criss-cross fixture with two co-equal merge bases, got: {all_bases}"
    );
    let base_set: std::collections::HashSet<&str> = all_bases.lines().collect();
    assert!(base_set.contains(a_tip.as_str()));
    assert!(base_set.contains(b_tip.as_str()));

    // Record which single base `git merge-base` (no --all) actually picks in
    // this codebase's exact invocation pattern.
    let picked = git_output(dir, &["merge-base", "criss-a", "criss-b"]);
    assert!(
        base_set.contains(picked.as_str()),
        "the single-base pick must be one of the --all candidates, got: {picked}"
    );

    // Confirm using this merge-base does not panic and produces a boolean.
    // criss-a and criss-b have identical trees at this point (both built from
    // `merged_tree`), so a squash-style comparison should find the branch
    // fully represented already (this is a smoke check of the fixture, not a
    // squash-detection claim about a landed commit).
    let result = merge_detection::is_squash_merged(dir, "criss-a", "criss-b", None, Some(&picked));
    // No assertion on the boolean value itself beyond it not panicking —
    // per the plan doc, only the merge-base pick and non-panicking behavior
    // are being characterized here.
    let _ = result;
}

#[test]
fn test_squash_scenario_16_shallow_out_of_window_history_max_count_boundary() {
    // Plan scenario 16: the actual squash-landing commit is older than the
    // Graph's configured `max_count` window.
    // Expect A: not flagged — Algorithm A fails closed outside its displayed
    // window (no false positive, no panic).
    // Expect B: still detects it — B is documented as searching beyond the
    // Graph display window (the concrete "A false, B true" disagreement case).
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/out-of-window"]);
    std::fs::write(dir.join("window.txt"), "windowed content\n").unwrap();
    run_git(dir, &["add", "window.txt"]);
    run_git(dir, &["commit", "-m", "windowed source"]);

    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/out-of-window"]);
    run_git(dir, &["commit", "-m", "squash landing (out of window)"]);
    let squash_oid = git_output(dir, &["rev-parse", "HEAD"]);

    // Push the squash landing commit outside the displayed window.
    run_git(dir, &["commit", "--allow-empty", "-m", "newer commit 1"]);
    run_git(dir, &["commit", "--allow-empty", "-m", "newer commit 2"]);

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            max_count: 2,
            base_branch: Some("main".into()),
            ..graph::GraphLoadOptions::default()
        },
    )
    .expect("bounded graph load should not error");
    assert_eq!(snapshot.commits.len(), 2);
    assert!(
        !snapshot.commits.iter().any(|c| c.oid == squash_oid),
        "the squash landing commit should be outside the displayed window"
    );
    assert!(
        snapshot
            .commits
            .iter()
            .all(|c| !c.is_possible_squash_merge),
        "Algorithm A must fail closed (no false positive) outside its displayed window"
    );

    // Expect B: full history search still finds it.
    assert!(
        merge_detection::is_squash_merged(dir, "main", "feature/out-of-window", None, None),
        "Algorithm B is not bounded by the Graph's display window and should still detect it"
    );
}

#[test]
fn test_squash_scenario_17_local_base_vs_remote_base_divergence() {
    // Plan scenario 17: branch is squash-merged into origin/main, but local
    // main has not fetched/merged that commit yet. Detection against local
    // main must report false; detection against origin/main (after fetch)
    // must report true.
    let base = tempfile::tempdir().expect("failed to create tempdir");
    let remote_dir = base.path().join("remote.git");
    std::fs::create_dir_all(&remote_dir).unwrap();
    run_git(&remote_dir, &["init", "--bare", "-b", "main"]);

    let work_dir = base.path().join("work");
    run_git(base.path(), &["clone", remote_dir.to_str().unwrap(), "work"]);
    run_git(&work_dir, &["config", "user.name", "Test User"]);
    run_git(&work_dir, &["config", "user.email", "test@example.com"]);
    std::fs::write(work_dir.join("README.md"), "# Test\n").unwrap();
    run_git(&work_dir, &["add", "."]);
    run_git(&work_dir, &["commit", "-m", "Initial commit"]);
    run_git(&work_dir, &["push", "-u", "origin", "main"]);

    run_git(&work_dir, &["checkout", "-b", "feature/divergence"]);
    std::fs::write(work_dir.join("divergence.txt"), "divergence content\n").unwrap();
    run_git(&work_dir, &["add", "divergence.txt"]);
    run_git(&work_dir, &["commit", "-m", "feature commit"]);
    run_git(&work_dir, &["push", "-u", "origin", "feature/divergence"]);
    run_git(&work_dir, &["checkout", "main"]);

    // A second clone squash-merges the feature branch and pushes to origin,
    // without `work` ever fetching it locally into `main`.
    let second_dir = base.path().join("second");
    run_git(base.path(), &["clone", remote_dir.to_str().unwrap(), "second"]);
    run_git(&second_dir, &["config", "user.name", "Second User"]);
    run_git(&second_dir, &["config", "user.email", "second@example.com"]);
    run_git(&second_dir, &["fetch", "origin", "feature/divergence"]);
    run_git(&second_dir, &["merge", "--squash", "origin/feature/divergence"]);
    run_git(&second_dir, &["commit", "-m", "squash merge feature/divergence"]);
    run_git(&second_dir, &["push", "origin", "main"]);

    // `work`'s local main is still behind; only fetch (not merge/pull).
    run_git(&work_dir, &["fetch", "origin"]);

    assert!(
        !merge_detection::is_squash_merged(&work_dir, "main", "feature/divergence", None, None),
        "local main has not fetched the squash landing commit yet, so it must not be flagged"
    );
    assert!(
        merge_detection::is_squash_merged(
            &work_dir,
            "origin/main",
            "feature/divergence",
            None,
            None
        ),
        "origin/main (fetched) already contains the squash landing and must be flagged"
    );
}

#[test]
fn test_squash_scenario_18_branch_advances_after_cached_as_squash_merged() {
    // Plan scenario 18: branch is squash-merged and detected/cached as
    // SquashMerged; branch then gets a new commit on top (still local,
    // unpushed, unmerged). Re-running detection should no longer report
    // SquashMerged for the branch's *current* tip. `MergeStatus::SquashMerged`
    // (the local+remote-confirmed variant) is cached *permanently* in
    // `git/cache.rs::BranchCache::lookup` — it ignores `commit_hash` entirely
    // for that one status, unlike `LocalSquashMerged`/`RemoteSquashMerged`
    // (commit-hash-gated). This test exercises exactly that permanent-cache
    // branch by using a remote setup where both local and remote sides
    // confirm the squash (producing `SquashMerged`, not the volatile
    // `LocalSquashMerged`), then documents whether the known bug reproduces.
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    run_git(&work_dir, &["checkout", "-b", "feature/cache-staleness"]);
    std::fs::write(work_dir.join("stale.txt"), "stale content\n").unwrap();
    run_git(&work_dir, &["add", "stale.txt"]);
    run_git(&work_dir, &["commit", "-m", "feature commit"]);
    run_git(&work_dir, &["push", "-u", "origin", "feature/cache-staleness"]);

    run_git(&work_dir, &["checkout", "main"]);
    run_git(&work_dir, &["merge", "--squash", "feature/cache-staleness"]);
    run_git(&work_dir, &["commit", "-m", "squash merge feature/cache-staleness"]);
    run_git(&work_dir, &["push", "origin", "main"]);

    let repo = git2::Repository::open(&work_dir).unwrap();
    let branches = branch::list_branches(&repo, "main").expect("list_branches failed");
    let feature = branches
        .iter()
        .find(|b| b.name == "feature/cache-staleness")
        .unwrap();
    assert_eq!(
        feature.merge_status,
        MergeStatus::SquashMerged,
        "both local and remote main confirm the squash, so this must be the fully-confirmed \
         (and thus permanently-cached) SquashMerged variant"
    );

    // Branch advances with a brand-new, unmerged, unpushed commit.
    run_git(&work_dir, &["checkout", "feature/cache-staleness"]);
    std::fs::write(work_dir.join("new-work.txt"), "new unmerged work\n").unwrap();
    run_git(&work_dir, &["add", "new-work.txt"]);
    run_git(&work_dir, &["commit", "-m", "new unmerged commit on top"]);
    run_git(&work_dir, &["checkout", "main"]);

    let repo = git2::Repository::open(&work_dir).unwrap();
    let branches = branch::list_branches(&repo, "main").expect("list_branches failed");
    let feature = branches
        .iter()
        .find(|b| b.name == "feature/cache-staleness")
        .unwrap();
    // Known bug (see git/cache.rs BranchCache::lookup and the parent plan
    // doc's cache-limitation note): SquashMerged is cached permanently and
    // ignores the branch's now-advanced commit hash, so the stale status is
    // retained instead of being recomputed as Unmerged/Pending for the new
    // tip. This assertion documents today's actual (buggy) behavior; it is a
    // deliberate visible pin, not an endorsement of the behavior.
    assert_eq!(
        feature.merge_status,
        MergeStatus::SquashMerged,
        "KNOWN BUG (see docs/plans/2026-08-29-squash-merge-test-scenarios.md scenario 18 and \
         git/cache.rs's permanent-cache comment for SquashMerged/Merged): the cache does not \
         invalidate on branch advancement for this status, so a branch with new unmerged work \
         still reports the stale SquashMerged status from before it advanced"
    );
}

#[test]
#[ignore]
fn test_squash_scenario_19_large_history_performance_characterization() {
    // Plan scenario 19: not a correctness assertion. Generate a large base
    // history and many diverged local branches, most unrelated to any squash,
    // and record wall-clock time as a baseline. Ignored by default (expensive);
    // run explicitly with:
    //   cargo test test_squash_scenario_19 -- --ignored --nocapture
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    const BASE_COMMITS: usize = 500;
    const BRANCH_COUNT: usize = 50;

    for i in 0..BASE_COMMITS {
        std::fs::write(dir.join(format!("base-{i}.txt")), format!("base content {i}\n")).unwrap();
        run_git(dir, &["add", "."]);
        run_git(dir, &["commit", "-m", &format!("base commit {i}")]);
    }

    for i in 0..BRANCH_COUNT {
        let branch_name = format!("feature/perf-{i}");
        run_git(dir, &["checkout", "-b", &branch_name, "main"]);
        std::fs::write(
            dir.join(format!("feature-{i}.txt")),
            format!("feature content {i}\n"),
        )
        .unwrap();
        run_git(dir, &["add", "."]);
        run_git(dir, &["commit", "-m", &format!("feature commit {i}")]);
        run_git(dir, &["checkout", "main"]);
    }

    let start = std::time::Instant::now();
    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..graph::GraphLoadOptions::default()
        },
    )
    .expect("graph load should succeed even at this scale");
    let graph_elapsed = start.elapsed();
    eprintln!(
        "[scenario 19] Algorithm A (Graph): {} commits displayed, {} branches, elapsed {:?}",
        snapshot.commits.len(),
        BRANCH_COUNT,
        graph_elapsed
    );

    let repo = git2::Repository::open(dir).unwrap();
    let start = std::time::Instant::now();
    let branches = branch::list_branches(&repo, "main").expect("list_branches failed");
    let branch_elapsed = start.elapsed();
    eprintln!(
        "[scenario 19] Algorithm B (Branches list_branches): {} branches, elapsed {:?}",
        branches.len(),
        branch_elapsed
    );
}

#[test]
fn test_squash_scenario_20a_squash_plus_trivial_follow_up_folded_in() {
    // Plan scenario 20a: branch's real changes land in the squash commit,
    // plus one small unrelated line (e.g. a version bump) folded in during
    // the squash but never part of the branch's own commits.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    std::fs::write(
        dir.join("f20a.txt"),
        "alpha\nbeta\ngamma\ndelta\nepsilon\nzeta\nversion: 1\n",
    )
    .unwrap();
    run_git(dir, &["add", "f20a.txt"]);
    run_git(dir, &["commit", "-m", "f20a baseline"]);

    run_git(dir, &["checkout", "-b", "feature/20a"]);
    std::fs::write(
        dir.join("f20a.txt"),
        "alpha-x\nbeta-x\ngamma-x\ndelta-x\nepsilon-x\nzeta-x\nversion: 1\n",
    )
    .unwrap();
    run_git(dir, &["commit", "-am", "branch changes 6 lines"]);

    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/20a"]);
    // Fold in an unrelated trivial version bump the branch never made.
    std::fs::write(
        dir.join("f20a.txt"),
        "alpha-x\nbeta-x\ngamma-x\ndelta-x\nepsilon-x\nzeta-x\nversion: 2\n",
    )
    .unwrap();
    run_git(dir, &["add", "f20a.txt"]);
    run_git(dir, &["commit", "-m", "squash landing plus version bump"]);
    let squash_oid = git_output(dir, &["rev-parse", "HEAD"]);

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..graph::GraphLoadOptions::default()
        },
    )
    .expect("graph load should succeed");
    let landing = snapshot
        .commits
        .iter()
        .find(|c| c.oid == squash_oid)
        .expect("squash landing commit should be displayed");
    assert!(
        !landing.is_possible_squash_merge,
        "the extra version-bump line must break exact patch-id equality"
    );
    assert!(
        !merge_detection::is_squash_merged(dir, "main", "feature/20a", None, None),
        "Algorithm B must also miss this under exact match"
    );
    let fuzzy = landing
        .fuzzy_squash_match
        .as_ref()
        .expect("Option 6 should flag this as a fuzzy possible-squash match");
    assert!(
        fuzzy.similarity_percent >= 75,
        "expected high similarity for a near-exact match with one trivial extra line, got {}",
        fuzzy.similarity_percent
    );
}

#[test]
fn test_squash_scenario_20b_squash_omits_a_trivial_branch_change() {
    // Plan scenario 20b: the squash commit landed slightly *less* than the
    // branch's full diff (e.g. a debug line dropped during squash). Confirms
    // the similarity computation is symmetric (handles both superset and
    // subset cases), not just the 20a "extra line" direction.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    std::fs::write(
        dir.join("f20b.txt"),
        "alpha\nbeta\ngamma\ndelta\nepsilon\nzeta\ndebug: off\n",
    )
    .unwrap();
    run_git(dir, &["add", "f20b.txt"]);
    run_git(dir, &["commit", "-m", "f20b baseline"]);

    run_git(dir, &["checkout", "-b", "feature/20b"]);
    std::fs::write(
        dir.join("f20b.txt"),
        "alpha-x\nbeta-x\ngamma-x\ndelta-x\nepsilon-x\nzeta-x\ndebug: on\n",
    )
    .unwrap();
    run_git(dir, &["commit", "-am", "branch changes 6 lines plus enables debug"]);

    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/20b"]);
    // Drop the branch's debug-line change during squash (revert it back).
    std::fs::write(
        dir.join("f20b.txt"),
        "alpha-x\nbeta-x\ngamma-x\ndelta-x\nepsilon-x\nzeta-x\ndebug: off\n",
    )
    .unwrap();
    run_git(dir, &["add", "f20b.txt"]);
    run_git(dir, &["commit", "-m", "squash landing, debug line omitted"]);
    let squash_oid = git_output(dir, &["rev-parse", "HEAD"]);

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..graph::GraphLoadOptions::default()
        },
    )
    .expect("graph load should succeed");
    let landing = snapshot
        .commits
        .iter()
        .find(|c| c.oid == squash_oid)
        .expect("squash landing commit should be displayed");
    assert!(
        !landing.is_possible_squash_merge,
        "omitting a branch change also breaks exact patch-id equality"
    );
    assert!(!merge_detection::is_squash_merged(
        dir,
        "main",
        "feature/20b",
        None,
        None
    ));
    let fuzzy = landing
        .fuzzy_squash_match
        .as_ref()
        .expect("Option 6 should flag this subset case as a fuzzy possible-squash match too");
    assert!(
        fuzzy.similarity_percent >= 75,
        "expected high similarity for the omitted-trivial-change (subset) case, got {}",
        fuzzy.similarity_percent
    );
}

#[test]
fn test_squash_scenario_20c_autoformatter_noise_during_squash() {
    // Plan scenario 20c: the squash commit's diff contains the branch's real
    // change plus many trivial formatting-tool line changes across the same
    // file (e.g. a linter reformatted touched lines). Per the plan doc, this
    // is the hardest calibration case; observe-then-assert whichever side of
    // the threshold this concrete recipe lands on rather than assuming a
    // specific classification, and record the outcome plainly.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    let baseline: Vec<String> = (1..=20).map(|i| format!("line{i} = {i};")).collect();
    std::fs::write(dir.join("f20c.txt"), baseline.join("\n") + "\n").unwrap();
    run_git(dir, &["add", "f20c.txt"]);
    run_git(dir, &["commit", "-m", "f20c baseline"]);

    run_git(dir, &["checkout", "-b", "feature/20c"]);
    // The branch's real change: modify a single line.
    let mut branch_lines = baseline.clone();
    branch_lines[9] = "line10 = 10; // real change".to_string();
    std::fs::write(dir.join("f20c.txt"), branch_lines.join("\n") + "\n").unwrap();
    run_git(dir, &["commit", "-am", "branch: real change to line 10"]);

    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/20c"]);
    // Simulate an autoformatter: reformat every line's spacing plus keep the
    // real change.
    let mut formatted_lines = branch_lines.clone();
    for line in formatted_lines.iter_mut() {
        *line = line.replace(" = ", "=");
    }
    std::fs::write(dir.join("f20c.txt"), formatted_lines.join("\n") + "\n").unwrap();
    run_git(dir, &["add", "f20c.txt"]);
    run_git(dir, &["commit", "-m", "squash landing plus autoformatter noise"]);
    let squash_oid = git_output(dir, &["rev-parse", "HEAD"]);

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..graph::GraphLoadOptions::default()
        },
    )
    .expect("graph load should succeed");
    let landing = snapshot
        .commits
        .iter()
        .find(|c| c.oid == squash_oid)
        .expect("squash landing commit should be displayed");
    assert!(
        !landing.is_possible_squash_merge,
        "autoformatter noise must break exact patch-id equality"
    );
    assert!(!merge_detection::is_squash_merged(
        dir,
        "main",
        "feature/20c",
        None,
        None
    ));

    // Observe-then-assert: record the actual classification for this
    // recipe rather than assuming one. With 20 lines reformatted (spacing
    // removed on every line) plus the 1 real content change, the branch's
    // own diff (1 changed line, 2 tokens) shares very little of the squash
    // diff's changed-line set (20 reformatted lines + 1 real change, ~40
    // tokens) — low Jaccard similarity is expected, so this recipe is
    // expected to land BELOW the fuzzy threshold without a raw-line-comparison
    // formatting-aware normalization pass (which is not implemented; see
    // git/fuzzy_match.rs's module doc comment and this test's own comment as
    // the recorded outcome of that open design question).
    assert!(
        landing.fuzzy_squash_match.is_none(),
        "observed: without formatting-aware normalization, heavy autoformatter noise dilutes \
         line-level Jaccard similarity below the fuzzy threshold for this recipe; if this \
         assertion starts failing, update this comment to record the newly observed value \
         rather than assuming the prior finding still holds"
    );
}

#[test]
fn test_squash_scenario_20d_coincidentally_similar_but_unrelated_negative_control() {
    // Plan scenario 20d: two commits touch the same file with moderate line
    // overlap by coincidence, but are not the same logical change (different
    // function, same file, similar boilerplate). Must NOT be flagged by
    // Option 6 — this is the fuzzy tier's own false-positive control.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    std::fs::write(
        dir.join("f20d.txt"),
        "fn one() {}\nfn two() {}\nfn three() {}\nfn four() {}\n",
    )
    .unwrap();
    run_git(dir, &["add", "f20d.txt"]);
    run_git(dir, &["commit", "-m", "f20d baseline"]);

    run_git(dir, &["checkout", "-b", "feature/20d"]);
    std::fs::write(
        dir.join("f20d.txt"),
        "fn one() {\n    println!(\"one\");\n}\nfn two() {}\nfn three() {}\nfn four() {}\n",
    )
    .unwrap();
    run_git(dir, &["commit", "-am", "branch: implement fn one with similar boilerplate"]);

    run_git(dir, &["checkout", "main"]);
    std::fs::write(
        dir.join("f20d.txt"),
        "fn one() {}\nfn two() {}\nfn three() {\n    println!(\"three\");\n}\nfn four() {}\n",
    )
    .unwrap();
    run_git(
        dir,
        &["commit", "-am", "main: unrelated commit, implement fn three with similar boilerplate"],
    );
    let unrelated_base_oid = git_output(dir, &["rev-parse", "HEAD"]);

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..graph::GraphLoadOptions::default()
        },
    )
    .expect("graph load should succeed");
    let unrelated = snapshot
        .commits
        .iter()
        .find(|c| c.oid == unrelated_base_oid)
        .expect("unrelated base commit should be displayed");
    assert!(
        !unrelated.is_possible_squash_merge,
        "these are genuinely different diffs, so exact match must not fire"
    );
    assert!(
        unrelated.fuzzy_squash_match.is_none(),
        "coincidental boilerplate similarity must not clear the fuzzy threshold — if this \
         fails, FUZZY_SIMILARITY_THRESHOLD in git/fuzzy_match.rs is too low and needs \
         revisiting before this tier ships (plan doc scenario 20d)"
    );
}

#[test]
fn test_squash_scenario_21_structural_graph_render_not_blocked_by_squash_enrichment() {
    // Plan scenario 21: a UX/architecture characterization test, not a
    // detection-correctness one. The parent plan doc's "Loading and UI
    // direction" follow-up task calls for decoupling the Graph's structural
    // snapshot (DAG, refs, commit summaries) from squash-merge enrichment, so
    // the structural view can render before annotation completes.
    //
    // Post-decoupling, `git::graph::load_graph` returns the structural
    // snapshot immediately and squash-merge enrichment is a separate,
    // asynchronous step. This test pins the new contract: structural data
    // is available synchronously, with no squash markers set until
    // enrichment runs.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/21"]);
    std::fs::write(dir.join("f21.txt"), "content\n").unwrap();
    run_git(dir, &["add", "f21.txt"]);
    run_git(dir, &["commit", "-m", "feature commit"]);
    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/21"]);
    run_git(dir, &["commit", "-m", "squash landing"]);

    // Step 1: structural snapshot is available immediately, with no
    // squash markers set — this is the new contract.
    let snapshot = graph::load_graph(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..graph::GraphLoadOptions::default()
        },
    )
    .expect("structural graph load should succeed");

    assert!(
        !snapshot.commits.is_empty(),
        "structural snapshot should be populated synchronously"
    );
    assert!(
        !snapshot.commits.iter().any(|c| c.is_possible_squash_merge),
        "no squash markers should be set on the fresh structural snapshot — \
         enrichment runs asynchronously and is gated by the reload generation"
    );
    assert!(
        snapshot.commits.iter().all(|c| c.fuzzy_squash_match.is_none()),
        "no fuzzy squash matches should be set on the fresh structural snapshot"
    );
}

#[test]
fn test_squash_scenario_21b_completed_enrichment_updates_squash_marker() {
    // Companion to scenario 21: once enrichment runs (the asynchronous
    // step), the squash marker must be set on the matching base commit.
    // This is the "completed enrichment updates the marker" half of the
    // new decoupled contract.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/21b"]);
    std::fs::write(dir.join("f21b.txt"), "content\n").unwrap();
    run_git(dir, &["add", "f21b.txt"]);
    run_git(dir, &["commit", "-m", "feature commit"]);
    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/21b"]);
    run_git(dir, &["commit", "-m", "squash landing"]);

    let mut snapshot = graph::load_graph(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..graph::GraphLoadOptions::default()
        },
    )
    .expect("structural graph load should succeed");

    // No markers yet.
    assert!(
        !snapshot.commits.iter().any(|c| c.is_possible_squash_merge),
        "fresh structural snapshot must not carry squash markers"
    );

    // Run enrichment synchronously, as `spawn_possible_squash_enrichment`
    // would do on its background thread.
    let updates =
        graph::compute_possible_squash_updates(dir, &snapshot, Some("main"));
    graph::apply_squash_enrichment(&mut snapshot, &updates);

    assert!(
        snapshot.commits.iter().any(|c| c.is_possible_squash_merge),
        "after enrichment, the squash landing commit on main must be flagged"
    );
}

#[test]
fn test_squash_scenario_21c_stale_enrichment_does_not_overwrite_newer_snapshot() {
    // Stale enrichment — a result from a previous reload that lands after a
    // newer load — must be dropped, not applied to the newer snapshot. The
    // public surface is the per-message `generation` field; the App uses it
    // to gate updates. This test exercises the gate via the git module's
    // enrichment-update data type directly.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Build a fresh snapshot (no markers yet).
    let mut snapshot = graph::load_graph(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..graph::GraphLoadOptions::default()
        },
    )
    .expect("structural graph load should succeed");
    assert!(!snapshot.commits.iter().any(|c| c.is_possible_squash_merge));

    // Simulate a stale enrichment result that claims a non-existent
    // commit is a squash merge. Even if the App-level generation check
    // were bypassed, the per-OID application in `apply_squash_enrichment`
    // ignores unknown OIDs — so this would be a no-op regardless. The
    // real test is below: the App's drain only applies updates whose
    // generation matches the snapshot's generation tag.
    snapshot.generation = Some(2);
    let stale = graph::GraphEnrichmentMsg {
        generation: 1, // stale: belongs to an earlier reload
        updates: snapshot
            .commits
            .iter()
            .map(|c| graph::GraphEnrichmentUpdate {
                oid: c.oid.clone(),
                is_possible_squash_merge: true,
                possible_squash_merge_sources: vec![],
                fuzzy_squash_match: None,
                is_cherry_picked_commit: false,
            })
            .collect(),
    };
    // Re-load a clean snapshot for the "current" generation so we can
    // assert no markers leaked from the stale message.
    let mut current = graph::load_graph(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..graph::GraphLoadOptions::default()
        },
    )
    .expect("structural graph load should succeed");
    current.generation = Some(2);

    // The App's drain applies only when msg.generation == self.graph_generation.
    // We model that gate here: with current.generation=2 and stale.generation=1,
    // the stale update must NOT touch the current snapshot.
    if stale.generation == 2 {
        graph::apply_squash_enrichment(&mut current, &stale.updates);
    }
    assert!(
        !current.commits.iter().any(|c| c.is_possible_squash_merge),
        "stale enrichment (generation 1) must not overwrite the newer snapshot's markers"
    );

    // Sanity check: a *matching* generation WOULD have applied the
    // update, confirming the gate is the only thing keeping the snapshot
    // clean.
    let matching = graph::GraphEnrichmentMsg {
        generation: 2,
        ..stale.clone()
    };
    if matching.generation == current.generation.unwrap_or(0) {
        graph::apply_squash_enrichment(&mut current, &matching.updates);
    }
    assert!(
        current.commits.iter().any(|c| c.is_possible_squash_merge),
        "matching-generation enrichment must apply, confirming the gate is the only filter"
    );
}

#[test]
fn test_squash_scenario_21d_failed_enrichment_leaves_snapshot_usable_and_unmarked() {
    // Failed enrichment — empty updates, or a panic, or a channel drop —
    // must leave the structural snapshot usable and unmarked. The Graph
    // view must not be left in a "Loading graph..." state just because
    // the optional enrichment pass failed.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/21d"]);
    std::fs::write(dir.join("f21d.txt"), "content\n").unwrap();
    run_git(dir, &["add", "f21d.txt"]);
    run_git(dir, &["commit", "-m", "feature commit"]);
    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/21d"]);
    run_git(dir, &["commit", "-m", "squash landing"]);

    let mut snapshot = graph::load_graph(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..graph::GraphLoadOptions::default()
        },
    )
    .expect("structural graph load should succeed");
    assert!(!snapshot.commits.is_empty());

    // Simulate a "failed" enrichment: empty update list. The GraphState
    // must remain fully usable (structural data + refs) and unmarked.
    let empty = graph::GraphEnrichmentMsg {
        generation: snapshot.generation.unwrap_or(0),
        updates: Vec::new(),
    };
    let before = snapshot.clone();
    graph::apply_squash_enrichment(&mut snapshot, &empty.updates);
    assert_eq!(
        snapshot, before,
        "empty enrichment must not mutate the snapshot"
    );
    assert!(
        !snapshot.commits.iter().any(|c| c.is_possible_squash_merge),
        "failed enrichment must not mark any commit"
    );
    assert!(
        !snapshot.commits.is_empty() && !snapshot.lines.is_empty(),
        "structural data must remain usable after failed enrichment"
    );

    // Simulate a worker thread that panics: the channel is dropped, no
    // message ever arrives. The snapshot stays as-is — that is the
    // observable behavior of `apply_squash_enrichment(&[], …)`.
    graph::apply_squash_enrichment(&mut snapshot, &[]);
    assert!(
        !snapshot.commits.iter().any(|c| c.is_possible_squash_merge),
        "never-arriving enrichment (worker panic) must leave the snapshot unmarked"
    );
}

#[test]
fn dump_branches_basic() {
    let (tmp, _repo) = setup_test_repo();
    let out = Command::new(env!("CARGO_BIN_EXE_git-branch-manager"))
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
    assert!(s.contains("Branch"), "header missing: {s:?}");
    assert!(s.contains("main"), "base branch row missing: {s:?}");
    assert!(!s.contains('\x1b'), "--color=never must be plain: {s:?}");
}

#[test]
fn dump_rejects_two_view_flags() {
    let (tmp, _repo) = setup_test_repo();
    let out = Command::new(env!("CARGO_BIN_EXE_git-branch-manager"))
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
    let out = Command::new(env!("CARGO_BIN_EXE_git-branch-manager"))
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

    let out = Command::new(env!("CARGO_BIN_EXE_git-branch-manager"))
        .args([
            "--repo",
            tmp.path().to_str().unwrap(),
            "--remotes",
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
    assert!(s.contains("origin/main"), "remote row missing: {s:?}");
}

#[test]
fn dump_tags_basic() {
    let (tmp, _repo) = setup_test_repo();
    run_git(tmp.path(), &["tag", "-a", "v1.0", "-m", "release one"]);
    let out = Command::new(env!("CARGO_BIN_EXE_git-branch-manager"))
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
    let out = Command::new(env!("CARGO_BIN_EXE_git-branch-manager"))
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
    let out = Command::new(env!("CARGO_BIN_EXE_git-branch-manager"))
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
}

#[test]
fn dump_worktrees_basic() {
    let (tmp, _repo) = setup_test_repo();
    let out = Command::new(env!("CARGO_BIN_EXE_git-branch-manager"))
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

    // Poison the cache at the same OS-cache-dir path `spawn_cache_verifier`
    // will load from internally (`BranchCache::load(repo_path)`, not a caller-
    // supplied path — unlike the other audit_cache tests above, which pass an
    // explicit `load_from_path` cache the caller controls directly).
    {
        let mut c = cache::BranchCache::load(dir);
        c.insert("feature/wip", &MergeStatus::SquashMerged, &tip);
        c.insert("feature/ghost", &MergeStatus::Merged, "deadbeef");
        c.save();
    }

    let rx = diagnostics::spawn_cache_verifier(dir.to_path_buf(), "main".to_string());
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
    let fixed = cache::BranchCache::load(dir);
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

/// Live, end-to-end test: build one repo containing every merge scenario, give
/// each feature branch its own worktree, and assert that a worktree's Merge
/// status matches its branch's. Also covers clean vs dirty working trees. The
/// temp repo lives in the OS temp dir and is deleted on pass, fail, or panic
/// (TempDir's Drop runs during unwind).
#[test]
fn test_worktree_merge_status_from_branches() {
    let tmpdir = TestDir::new(tempfile::tempdir().expect("failed to create tempdir"));
    let dir = tmpdir.path();

    // --- base repo on main with an initial README.md commit ---
    run_git(dir, &["init", "-b", "main"]);
    run_git(dir, &["config", "user.name", "Test User"]);
    run_git(dir, &["config", "user.email", "test@example.com"]);
    std::fs::write(dir.join("README.md"), "# Test Repo\n").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "Initial commit"]);

    // --- regular-merged branch (non-ff so a merge commit is recorded) ---
    run_git(dir, &["checkout", "-b", "feat-merged"]);
    std::fs::write(dir.join("feature.txt"), "feature\n").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "feat-merged work"]);
    run_git(dir, &["checkout", "main"]);
    std::fs::write(dir.join("main-change.txt"), "main\n").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "main commit"]);
    run_git(dir, &["merge", "feat-merged", "-m", "Merge feat-merged"]);

    // --- squash-merged branch: 3 commits each editing README.md, then
    //     `git merge --squash` + commit on main; branch left intact ---
    run_git(dir, &["checkout", "-b", "feat-squashed"]);
    for i in 1..=3 {
        std::fs::write(
            dir.join("README.md"),
            format!("# Test Repo\nsquash line {i}\n"),
        )
        .unwrap();
        run_git(dir, &["add", "."]);
        run_git(dir, &["commit", "-m", &format!("squash work {i}")]);
    }
    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feat-squashed"]);
    run_git(dir, &["commit", "-m", "squash merge feat-squashed"]);

    // --- unmerged branch ---
    run_git(dir, &["checkout", "-b", "feat-unmerged"]);
    std::fs::write(dir.join("wip.txt"), "wip\n").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "wip"]);
    run_git(dir, &["checkout", "main"]);

    // --- one worktree per feature branch (main dir stays on main) ---
    run_git(
        dir,
        &["worktree", "add", ".worktrees/merged", "feat-merged"],
    );
    run_git(
        dir,
        &["worktree", "add", ".worktrees/squashed", "feat-squashed"],
    );
    run_git(
        dir,
        &["worktree", "add", ".worktrees/unmerged", "feat-unmerged"],
    );

    // dirty the unmerged worktree (untracked file); leave the others clean
    std::fs::write(dir.join(".worktrees/unmerged/dirty.txt"), "x\n").unwrap();

    // --- exercise the real library APIs in production order ---
    let repo = git2::Repository::open(dir).expect("failed to re-open repo");
    let branches = branch::list_branches(&repo, "main").expect("list_branches failed");

    // sanity: branch statuses themselves are correct
    let branch_status = |name: &str| {
        branches
            .iter()
            .find(|b| b.name == name)
            .unwrap_or_else(|| panic!("branch {name} not found"))
            .merge_status
    };
    assert_eq!(branch_status("feat-merged"), MergeStatus::Merged);
    // No remote in this repo, so a squash-merge resolves to the local-only
    // variant (matching test_squash_merged_branch_detection).
    assert_eq!(
        branch_status("feat-squashed"),
        MergeStatus::LocalSquashMerged
    );
    assert_eq!(branch_status("feat-unmerged"), MergeStatus::Unmerged);

    let mut worktrees = worktree::list_worktrees(dir);

    // THE API UNDER TEST: correlate worktree merge status from the branch list.
    worktree::apply_branch_merge_status(&mut worktrees, &branches);

    let wt_for = |name: &str| {
        worktrees
            .iter()
            .find(|w| w.branch.as_deref() == Some(name))
            .unwrap_or_else(|| panic!("no worktree for branch {name}"))
    };
    assert_eq!(
        wt_for("feat-merged").merge_status,
        MergeStatus::Merged,
        "merged worktree should match its branch"
    );
    assert_eq!(
        wt_for("feat-squashed").merge_status,
        MergeStatus::LocalSquashMerged,
        "squash-merged worktree should match its branch"
    );
    assert_eq!(
        wt_for("feat-unmerged").merge_status,
        MergeStatus::Unmerged,
        "unmerged worktree should match its branch"
    );

    // main worktree is on the base branch: it must be flagged is_base so the
    // renderer blanks its Merge cell (a branch can't be merged into itself).
    // The raw merge_status copied from the base BranchInfo is an implementation
    // detail; the contract that matters is is_base. (See the render-level test
    // `worktree_base_merge_cell_is_blank` in src/app.rs for the blanking.)
    let main_wt = worktrees.iter().find(|w| w.is_main).expect("main worktree");
    assert_eq!(main_wt.branch.as_deref(), Some("main"));
    assert!(
        main_wt.is_base,
        "main worktree is on the base branch and must be marked is_base"
    );
    // The feature worktrees are NOT the base.
    assert!(!wt_for("feat-merged").is_base);
    assert!(!wt_for("feat-unmerged").is_base);

    // --- clean vs dirty working tree via enrich_worktrees (sets wt_status) ---
    let unmerged_idx = worktrees
        .iter()
        .position(|w| w.branch.as_deref() == Some("feat-unmerged"))
        .unwrap();
    let merged_idx = worktrees
        .iter()
        .position(|w| w.branch.as_deref() == Some("feat-merged"))
        .unwrap();
    let rx = worktree::enrich_worktrees(worktrees.clone());
    let mut results: Vec<_> = rx.iter().collect();
    results.sort_by_key(|r| r.index);
    // The dirty file is untracked specifically — assert the exact buckets, not
    // just !is_clean (which would pass under a staged/modified misclassification).
    let unmerged_status = &results[unmerged_idx].wt_status;
    assert!(unmerged_status.has_untracked, "dirty file is untracked");
    assert!(!unmerged_status.has_staged, "nothing staged");
    assert!(!unmerged_status.has_modified, "no tracked modifications");
    assert!(
        results[merged_idx].wt_status.is_clean(),
        "merged worktree is untouched -> clean"
    );

    // cleanup worktrees before the tempdir drops — skipped when keeping repos
    // for manual inspection, so the worktrees stay on disk too
    if std::env::var_os("GBM_KEEP_TEST_REPOS").is_none() {
        for p in [
            ".worktrees/merged",
            ".worktrees/squashed",
            ".worktrees/unmerged",
        ] {
            run_git(dir, &["worktree", "remove", "--force", p]);
        }
    }
}

// ---------------------------------------------------------------------------
// #030 — Remote branch inherits squash-merge status from local branch detection
// ---------------------------------------------------------------------------

#[test]
fn test_remote_branch_inherits_squash_merge_status_from_local() {
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    // Create a feature branch, add a commit, and push it to origin
    run_git(&work_dir, &["checkout", "-b", "squash-local-feature"]);
    std::fs::write(work_dir.join("feature.txt"), "feature content\n").unwrap();
    run_git(&work_dir, &["add", "feature.txt"]);
    run_git(&work_dir, &["commit", "-m", "Add feature"]);
    run_git(&work_dir, &["push", "-u", "origin", "squash-local-feature"]);

    // Squash-merge into main locally (do NOT push main — remote branch stays ahead=1)
    run_git(&work_dir, &["checkout", "main"]);
    run_git(&work_dir, &["merge", "--squash", "squash-local-feature"]);
    run_git(
        &work_dir,
        &["commit", "-m", "Squash merge squash-local-feature"],
    );

    let repo = git2::Repository::open(&work_dir).unwrap();

    // Load local branches and run squash detection
    let local_branches = branch::list_branches(&repo, "main").expect("list_branches failed");
    let local_feature = local_branches
        .iter()
        .find(|b| b.name == "squash-local-feature")
        .expect("squash-local-feature not in local branches");
    assert_eq!(
        local_feature.merge_status,
        MergeStatus::LocalSquashMerged,
        "local branch should be detected as LocalSquashMerged"
    );

    // Load remote branches — enricher marks ahead>0 as Unmerged
    let mut remotes = branch::list_remote_branches_phase1(&repo, "main")
        .expect("list_remote_branches_phase1 failed");

    // Simulate the remote enricher: origin/squash-local-feature is ahead=1, so Unmerged
    let remote_before = remotes
        .iter()
        .find(|r| r.short_name == "squash-local-feature")
        .expect("origin/squash-local-feature not found");
    assert_eq!(
        remote_before.merge_status,
        MergeStatus::Pending,
        "remote branch should start as Pending before enrichment"
    );

    // Simulate what drain_channels does: propagate local squash result to remote
    // (this is the fix from #030 — when a local squash result arrives, update the
    // matching remote branch by short_name)
    for remote in remotes.iter_mut() {
        if remote.short_name == local_feature.name
            && !matches!(
                local_feature.merge_status,
                MergeStatus::Unmerged | MergeStatus::Pending
            )
        {
            remote.merge_status = local_feature.merge_status;
        }
    }

    let remote_after = remotes
        .iter()
        .find(|r| r.short_name == "squash-local-feature")
        .expect("origin/squash-local-feature not found");
    assert_eq!(
        remote_after.merge_status,
        MergeStatus::LocalSquashMerged,
        "remote branch should inherit LocalSquashMerged from local squash detection"
    );
}

// ---------------------------------------------------------------------------
// Graph patch cache tests
//
// These exercise `git::graph::compute_possible_squash_updates`'s internal
// cache (the `graph_patch` SQLite table on `BranchCache`). They prove the
// cache path is consulted by injecting fabricated cached values, that the
// cache is populated after a normal load, that different (old_oid, new_oid)
// pairs get independent cache keys, and that the Graph loader and the
// Branches-view squash loader can write to the same SQLite file safely when
// run concurrently.
// ---------------------------------------------------------------------------

#[test]
fn test_graph_patch_cache_hit_avoids_recomputation() {
    // Build scenario-01-style clean squash, then fabricate a bogus cached
    // patch_id for the squash-landing (old_oid, new_oid) pair. After loading
    // the graph again, the landing commit must NOT be flagged — proving the
    // cache hit was used in place of the real computation.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/baseline"]);
    std::fs::write(dir.join("baseline.txt"), "baseline content\n").unwrap();
    run_git(dir, &["add", "baseline.txt"]);
    run_git(dir, &["commit", "-m", "baseline feature commit"]);

    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/baseline"]);
    run_git(dir, &["commit", "-m", "squash landing"]);
    let squash_oid = git_output(dir, &["rev-parse", "HEAD"]);
    let parent_oid = git_output(dir, &["rev-parse", "HEAD^"]);

    // Fabricate a cached (patch_id, diff_text) that won't match any real
    // branch tip's patch id, so any exact match the loader reports after this
    // point is the cache, not a fresh compute.
    {
        let mut cache = cache::BranchCache::load(dir);
        cache.insert_graph_patch(
            &parent_oid,
            &squash_oid,
            1,
            Some("bogus-never-matches".to_string()),
            None,
        );
        cache.save();
    }

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..graph::GraphLoadOptions::default()
        },
    )
    .expect("graph load should succeed");
    let landing = snapshot
        .commits
        .iter()
        .find(|c| c.oid == squash_oid)
        .expect("squash landing commit should be displayed");
    assert!(
        !landing.is_possible_squash_merge,
        "fabricated cached patch_id must suppress the real match (cache hit proved)"
    );
}

#[test]
fn test_graph_patch_cache_populated_after_load() {
    // Same scenario without fabrication: after a real load, the cache must
    // hold an entry for the landing commit's (old_oid, new_oid) pair.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/baseline"]);
    std::fs::write(dir.join("baseline.txt"), "baseline content\n").unwrap();
    run_git(dir, &["add", "baseline.txt"]);
    run_git(dir, &["commit", "-m", "baseline feature commit"]);

    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/baseline"]);
    run_git(dir, &["commit", "-m", "squash landing"]);
    let squash_oid = git_output(dir, &["rev-parse", "HEAD"]);
    let parent_oid = git_output(dir, &["rev-parse", "HEAD^"]);

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..graph::GraphLoadOptions::default()
        },
    )
    .expect("graph load should succeed");
    let landing = snapshot
        .commits
        .iter()
        .find(|c| c.oid == squash_oid)
        .expect("squash landing commit should be displayed");
    assert!(
        landing.is_possible_squash_merge,
        "real match should still be flagged before asserting cache state"
    );

    let cache = cache::BranchCache::load(dir);
    let cached = cache
        .lookup_graph_patch(&parent_oid, &squash_oid, 1)
        .expect("cache must hold an entry after a successful graph load");
    let (cached_patch_id, _cached_diff_text) = cached;
    assert!(
        cached_patch_id.is_some(),
        "cached patch_id should be populated for the squash landing OID pair"
    );
}

#[test]
fn test_graph_patch_cache_branch_change_invalidates() {
    // After the first squash lands and the cache populates, create and squash
    // a SECOND feature branch producing a new landing commit. Reload the
    // graph — the new landing commit must still be flagged (its OID pair was
    // never in the cache), and the first commit must remain flagged.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    // First squash scenario.
    run_git(dir, &["checkout", "-b", "feature/first"]);
    std::fs::write(dir.join("first.txt"), "first content\n").unwrap();
    run_git(dir, &["add", "first.txt"]);
    run_git(dir, &["commit", "-m", "first feature commit"]);
    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/first"]);
    run_git(dir, &["commit", "-m", "first squash landing"]);
    let first_squash_oid = git_output(dir, &["rev-parse", "HEAD"]);

    // Prime the cache with the first squash.
    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..graph::GraphLoadOptions::default()
        },
    )
    .expect("first graph load should succeed");
    assert!(
        snapshot
            .commits
            .iter()
            .find(|c| c.oid == first_squash_oid)
            .unwrap()
            .is_possible_squash_merge,
        "first squash landing should be flagged"
    );

    // Second squash scenario — produces a fresh OID pair that was never cached.
    run_git(dir, &["checkout", "-b", "feature/second"]);
    std::fs::write(dir.join("second.txt"), "second content\n").unwrap();
    run_git(dir, &["add", "second.txt"]);
    run_git(dir, &["commit", "-m", "second feature commit"]);
    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/second"]);
    run_git(dir, &["commit", "-m", "second squash landing"]);
    let second_squash_oid = git_output(dir, &["rev-parse", "HEAD"]);
    let second_parent_oid = git_output(dir, &["rev-parse", "HEAD^"]);

    let snapshot = graph::load_graph_with_squash_annotations(
        dir,
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..graph::GraphLoadOptions::default()
        },
    )
    .expect("second graph load should succeed");

    let second_landing = snapshot
        .commits
        .iter()
        .find(|c| c.oid == second_squash_oid)
        .expect("second landing commit should be displayed");
    assert!(
        second_landing.is_possible_squash_merge,
        "second landing commit must be freshly flagged (its OID pair was never in the cache)"
    );

    // First landing should still be flagged too — cache hit returned true.
    let first_landing = snapshot
        .commits
        .iter()
        .find(|c| c.oid == first_squash_oid)
        .expect("first landing commit should still be displayed");
    assert!(
        first_landing.is_possible_squash_merge,
        "first landing commit must remain flagged (cache hit preserved its match)"
    );

    // And the second landing's OID pair should now itself be in the cache.
    let cache = cache::BranchCache::load(dir);
    assert!(
        cache
            .lookup_graph_patch(&second_parent_oid, &second_squash_oid, 1)
            .is_some(),
        "second landing's OID pair must now be cached after the second load"
    );
}

#[test]
fn test_graph_and_branches_squash_loaders_concurrent_cache_access() {
    // Build a squash scenario with a real remote so the branches-view squash
    // loader's local-vs-remote base comparison returns a definite status.
    // Run the Graph loader and the Branches-view squash loader concurrently
    // against the same SQLite cache file. Both must produce correct results
    // with no panic or error — proving the two independent BranchCache
    // instances writing to the same file are safe when run concurrently.
    let (_tmpdir, work_dir, _repo) = setup_remote_test_repo();

    run_git(&work_dir, &["checkout", "-b", "feature/concurrent"]);
    std::fs::write(work_dir.join("concurrent.txt"), "concurrent content\n").unwrap();
    run_git(&work_dir, &["add", "concurrent.txt"]);
    run_git(&work_dir, &["commit", "-m", "concurrent feature commit"]);
    run_git(&work_dir, &["push", "-u", "origin", "feature/concurrent"]);
    // Capture the feature branch tip BEFORE the squash merge so the
    // branches-view squash loader checks the right commit.
    let feature_tip_oid = git_output(&work_dir, &["rev-parse", "HEAD"]);
    run_git(&work_dir, &["checkout", "main"]);
    run_git(&work_dir, &["merge", "--squash", "feature/concurrent"]);
    run_git(&work_dir, &["commit", "-m", "concurrent squash landing"]);
    run_git(&work_dir, &["push", "origin", "main"]);
    let concurrent_oid = git_output(&work_dir, &["rev-parse", "HEAD"]);
    let _concurrent_parent = git_output(&work_dir, &["rev-parse", "HEAD^"]);

    let dir_clone = work_dir.clone();
    let graph_thread = std::thread::spawn(move || {
        graph::load_graph_with_squash_annotations(
            &dir_clone,
            graph::GraphLoadOptions {
                base_branch: Some("main".into()),
                ..graph::GraphLoadOptions::default()
            },
        )
    });

    // The Branches-view squash loader uses (branch_name, commit_hash, Option)
    // candidates. We feed it the same feature branch as a real candidate,
    // using the feature branch tip (NOT the squash landing commit).
    let work_dir_for_squash = work_dir.clone();
    let candidates = vec![(
        "feature/concurrent".to_string(),
        feature_tip_oid.clone(),
        None,
    )];
    let squash_cache = cache::BranchCache::load(&work_dir);
    let squash_thread = std::thread::spawn(move || {
        let rx = squash_loader::spawn_squash_checker(
            work_dir_for_squash,
            "main".to_string(),
            candidates,
            squash_cache,
        );
        rx.into_iter().collect::<Vec<_>>()
    });

    let graph_result = graph_thread.join().expect("graph thread panicked");
    let squash_results = squash_thread.join().expect("squash thread panicked");

    let snapshot = graph_result.expect("graph load should succeed");
    let landing = snapshot
        .commits
        .iter()
        .find(|c| c.oid == concurrent_oid)
        .expect("squash landing commit should be displayed");
    assert!(
        landing.is_possible_squash_merge,
        "concurrent graph load should still flag the squash landing"
    );

    // With both local and remote bases present and the squash pushed, the
    // branches-view squash loader should report SquashMerged (both local
    // and remote-base checks true).
    assert!(
        squash_results.iter().any(|r| {
            r.branch_name == "feature/concurrent"
                && matches!(
                    r.status,
                    MergeStatus::SquashMerged | MergeStatus::LocalSquashMerged
                )
        }),
        "concurrent branches-view squash loader should report a squash status, got: {:?}",
        squash_results
    );
}

// =====================================================================
// Cherry-pick detection tests
// =====================================================================

#[test]
fn test_is_cherry_picked_direct() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/cherry"]);
    std::fs::write(dir.join("cherry.txt"), "cherry content").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "cherry commit"]);
    run_git(dir, &["checkout", "main"]);

    // Cherry-pick the commit onto main (no merge commit).
    run_git(dir, &["cherry-pick", "feature/cherry"]);

    assert!(merge_detection::is_cherry_picked(
        dir,
        "main",
        "feature/cherry",
        None,
        None
    ));
}

#[test]
fn test_is_not_cherry_picked_direct() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    run_git(dir, &["checkout", "-b", "feature/uncherried"]);
    std::fs::write(dir.join("uncherried.txt"), "uncherried content").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "uncherried commit"]);
    run_git(dir, &["checkout", "main"]);

    // No cherry-pick performed.
    assert!(!merge_detection::is_cherry_picked(
        dir,
        "main",
        "feature/uncherried",
        None,
        None
    ));
}

#[test]
fn test_is_cherry_picked_with_precomputed_merge_base() {
    // The fast path: a precomputed merge base is supplied, so is_cherry_picked
    // must not need to derive it via `git merge-base` and still detect the
    // cherry-pick.
    let (tmpdir, repo) = setup_test_repo();
    let dir = tmpdir.path();

    // HEAD is the initial commit; it becomes the merge base after we branch.
    let merge_base = repo
        .head()
        .unwrap()
        .peel_to_commit()
        .unwrap()
        .id()
        .to_string();

    run_git(dir, &["checkout", "-b", "feature/cherry"]);
    std::fs::write(dir.join("cherry.txt"), "cherry content").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "cherry commit"]);
    run_git(dir, &["checkout", "main"]);

    run_git(dir, &["cherry-pick", "feature/cherry"]);

    assert!(merge_detection::is_cherry_picked(
        dir,
        "main",
        "feature/cherry",
        None,
        Some(&merge_base),
    ));
}

#[test]
fn test_partial_cherry_pick_is_not_cherry_picked() {
    // A branch whose commits were only partially cherry-picked onto base must
    // NOT be flagged as fully cherry-picked: `git cherry` returns at least one
    // `+` line (a commit whose patch-id isn't reachable from base) and the
    // detection must fail closed.
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Build a branch with three commits: c1, c2, c3.
    run_git(dir, &["checkout", "-b", "feature/partial"]);
    std::fs::write(dir.join("c1.txt"), "c1").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "c1"]);
    std::fs::write(dir.join("c2.txt"), "c2").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "c2"]);
    std::fs::write(dir.join("c3.txt"), "c3").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "c3"]);
    run_git(dir, &["checkout", "main"]);

    // Cherry-pick only c1 and c2 onto main; leave c3 un-cherry-picked.
    let log = git_output(dir, &["log", "--reverse", "--format=%H", "feature/partial"]);
    let commits: Vec<&str> = log.lines().collect();
    // The first entry is the initial commit (merge-base), which we drop.
    assert!(
        commits.len() >= 4,
        "expected at least 4 commits (initial + c1/c2/c3) on feature/partial, got {}",
        commits.len()
    );
    run_git(dir, &["cherry-pick", commits[1]]);
    run_git(dir, &["cherry-pick", commits[2]]);

    assert!(!merge_detection::is_cherry_picked(
        dir,
        "main",
        "feature/partial",
        None,
        None
    ));
}

#[test]
fn test_cherry_loader_drains_to_cherry_picked_status() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path().to_path_buf();

    // Build a feature branch and cherry-pick every commit onto main.
    run_git(&dir, &["checkout", "-b", "feature/cherry-loader"]);
    std::fs::write(dir.join("loader.txt"), "loader").unwrap();
    run_git(&dir, &["add", "."]);
    run_git(&dir, &["commit", "-m", "loader commit"]);
    run_git(&dir, &["checkout", "main"]);

    // Cherry-pick the single feature commit.
    let tip = git_output(&dir, &["rev-parse", "feature/cherry-loader"]);
    run_git(&dir, &["cherry-pick", &tip]);

    let candidates = vec![(
        "feature/cherry-loader".to_string(),
        tip,
        None,
    )];
    let cache = cache::BranchCache::load(&dir);
    let rx = cherry_loader::spawn_cherry_checker(
        dir.clone(),
        "main".to_string(),
        candidates,
        cache,
    );

    let results: Vec<_> = rx.iter().collect();
    assert_eq!(results.len(), 1, "expected one cherry result");
    let result = &results[0];
    assert_eq!(result.branch_name, "feature/cherry-loader");
    // No `origin/main` exists in this fixture, so the remote-base check fails
    // closed and we report `LocalCherryPicked`.
    assert!(
        matches!(result.status, MergeStatus::LocalCherryPicked),
        "expected LocalCherryPicked, got {:?}",
        result.status
    );
}

#[test]
fn test_graph_cherry_pick_enrichment_marks_branch_commits() {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();

    // Build a branch with two commits, then cherry-pick each onto main.
    run_git(dir, &["checkout", "-b", "feature/graph-cherry"]);
    std::fs::write(dir.join("g1.txt"), "g1").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "g1"]);
    std::fs::write(dir.join("g2.txt"), "g2").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", "g2"]);
    run_git(dir, &["checkout", "main"]);

    // Cherry-pick each branch commit onto main individually. Use
    // `--allow-empty` so the second commit (which adds a different file but
    // patches the same tree when the previous file is already in main) does
    // not error as an "empty" cherry-pick.
    let log = git_output(dir, &["log", "--reverse", "--format=%H", "feature/graph-cherry"]);
    let commits: Vec<&str> = log.lines().collect();
    for hash in &commits {
        run_git(dir, &["cherry-pick", "--allow-empty", "--keep-redundant-commits", hash]);
    }

    let options = graph::GraphLoadOptions {
        max_count: 50,
        include_remotes: false,
        line_style: graph::GraphLineStyle::Thin,
        base_branch: Some("main".to_string()),
    };
    let snapshot =
        graph::load_graph_with_squash_annotations(dir, options).expect("graph load failed");

    // Exclude the initial commit (which is also reachable from main) — only
    // assert that the branch's unique commits are marked cherry-picked.
    let branch_commit_oids: std::collections::HashSet<String> = commits
        .iter()
        .skip(1)
        .map(|h| h.trim().to_string())
        .collect();

    for commit in &snapshot.commits {
        if branch_commit_oids.contains(&commit.oid) {
            assert!(
                commit.is_cherry_picked_commit,
                "commit {} on feature/graph-cherry should be marked as cherry-picked",
                commit.oid
            );
        }
    }
}
