use std::process::Command;
use std::io::Write;

use git_branch_manager::git::{
    branch, cache, graph, merge_detection,
};

/// A temp directory for tests. Deletes itself on drop, EXCEPT when the
/// `GBM_KEEP_TEST_REPOS` env var is set — then it leaks the directory and prints
/// the path, so you can inspect the repo state after a run:
///
/// ```sh
/// GBM_KEEP_TEST_REPOS=1 cargo test --test integration <name> -- --nocapture
/// ```
pub(crate) struct TestDir {
    inner: Option<tempfile::TempDir>,
    cache_root: Option<tempfile::TempDir>,
    path: std::path::PathBuf,
}

impl TestDir {
    pub(crate) fn new(td: tempfile::TempDir) -> Self {
        let path = td.path().to_path_buf();
        Self {
            inner: Some(td),
            cache_root: Some(tempfile::tempdir().expect("failed to create test cache root")),
            path,
        }
    }

    pub(crate) fn path(&self) -> &std::path::Path {
        &self.path
    }

    pub(crate) fn cache_root(&self) -> &std::path::Path {
        self.cache_root
            .as_ref()
            .expect("test cache root already dropped")
            .path()
    }

    pub(crate) fn graph_options(&self) -> graph::GraphLoadOptions {
        let mut options = graph::GraphLoadOptions::default();
        options.cache_root = cache::CacheRoot::at(self.cache_root().to_path_buf());
        options
    }

    pub(crate) fn graph_options_for(&self, base: &str) -> graph::GraphLoadOptions {
        graph::GraphLoadOptions {
            base_branch: Some(base.to_owned()),
            max_count: 50,
            ..self.graph_options()
        }
    }

    pub(crate) fn preserve_tempdirs(&mut self) -> (Option<std::path::PathBuf>, Option<std::path::PathBuf>) {
        let repo = self.inner.take().map(|dir| dir.keep());
        let cache = self.cache_root.take().map(|dir| dir.keep());
        (repo, cache)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        if std::env::var_os("GBM_KEEP_TEST_REPOS").is_some() {
            let (repo, cache) = self.preserve_tempdirs();
            if let Some(kept) = repo {
                eprintln!("[GBM_KEEP_TEST_REPOS] kept test repo: {}", kept.display());
            }
            if let Some(kept) = cache {
                eprintln!("[GBM_KEEP_TEST_REPOS] kept test cache: {}", kept.display());
            }
        }
        // Otherwise both TempDirs drop normally and remove their contents.
    }
}

pub(crate) fn manager_command(test_dir: &TestDir) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_git-branch-manager"));
    command.env("GBM_CACHE_DIR", test_dir.cache_root());
    command
}

pub(crate) fn test_branches(
    test_dir: &TestDir,
    repo: &git2::Repository,
    base_branch: &str,
) -> anyhow::Result<Vec<git_branch_manager::types::BranchInfo>> {
    Ok(branch::list_branches_with_cache_root(
        repo,
        base_branch,
        &cache::CacheRoot::at(test_dir.cache_root().to_path_buf()),
    )?)
}

/// Create a temporary git repository with an initial commit on the "main" branch.
///
/// Returns the temp dir guard (must be kept alive for the duration of the test)
/// and the git2::Repository handle.
pub(crate) fn setup_test_repo() -> (TestDir, git2::Repository) {
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
pub(crate) fn run_git(dir: &std::path::Path, args: &[&str]) {
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

pub(crate) fn git_output(dir: &std::path::Path, args: &[&str]) -> String {
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

pub(crate) fn snapshot_files(root: &std::path::Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
    fn collect(
        root: &std::path::Path,
        current: &std::path::Path,
        files: &mut Vec<(std::path::PathBuf, Vec<u8>)>,
    ) {
        for entry in std::fs::read_dir(current).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                collect(root, &path, files);
            } else {
                files.push((path.strip_prefix(root).unwrap().to_path_buf(), std::fs::read(path).unwrap()));
            }
        }
    }

    let mut files = Vec::new();
    collect(root, root, &mut files);
    files.sort_by(|left, right| left.0.cmp(&right.0));
    files
}

pub(crate) fn check_primary_code(
    repo: &git2::Repository,
    base: &str,
    selected_ref: &str,
) -> anyhow::Result<git_branch_manager::types::PrimaryBranchCodeMatch> {
    let primary = merge_detection::resolve_primary_ref_for_code_check(repo, base, None)?;
    merge_detection::check_ref_code_in_primary(repo, &primary, selected_ref, None)
}

/// Run a git command with scoped environment variables in the given directory,
/// panicking on failure.
pub(crate) fn run_git_with_env(dir: &std::path::Path, args: &[&str], env: &[(&str, &str)]) {
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

pub(crate) fn git_blob_oid(dir: &std::path::Path, contents: &[u8]) -> String {
    let mut child = Command::new("git")
        .args(["hash-object", "--stdin"])
        .current_dir(dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(contents).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

pub(crate) fn paired_squash_fixture() -> (TestDir, String, String, String, String) {
    let (tmpdir, _repo) = setup_test_repo();
    let dir = tmpdir.path();
    let root = git_output(dir, &["rev-parse", "HEAD"]);
    let content: Vec<String> = (0..20).map(|n| format!("paired line {n}\n")).collect();
    let mut sources = Vec::new();
    for (name, changed_lines) in [
        ("feature/exact", 0),
        ("feature/high", 1),
        ("feature/low", 2),
    ] {
        run_git(dir, &["checkout", "-b", name, &root]);
        let mut lines = content.clone();
        for (n, line) in lines.iter_mut().take(changed_lines).enumerate() {
            *line = format!("different line {n}\n");
        }
        std::fs::write(dir.join("paired.txt"), lines.concat()).unwrap();
        run_git(dir, &["add", "paired.txt"]);
        run_git(dir, &["commit", "-m", name]);
        sources.push(git_output(dir, &["rev-parse", "HEAD"]));
    }
    run_git(dir, &["branch", "feature/exact-alias", &sources[0]]);
    run_git(dir, &["checkout", "main"]);
    run_git(dir, &["merge", "--squash", "feature/exact"]);
    run_git(dir, &["commit", "-m", "paired landing"]);
    let destination = git_output(dir, &["rev-parse", "HEAD"]);
    (
        tmpdir,
        destination,
        sources[0].clone(),
        sources[1].clone(),
        sources[2].clone(),
    )
}

/// Create a bare remote + clone setup with an initial commit on main,
/// plus remote-only and local+remote branches for testing.
///
/// Returns (tmpdir, work_dir path, Repository for the clone).
pub(crate) fn setup_remote_test_repo() -> (TestDir, std::path::PathBuf, git2::Repository) {
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
