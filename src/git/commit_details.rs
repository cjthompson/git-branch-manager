//! Background loading for the graph commit details and per-file patch views.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver};

use chrono::{DateTime, FixedOffset};

use crate::git::graph::{GraphCommit, GraphRefKind};

/// The source represented by a commit details modal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitDetailMode {
    /// One commit compared with its first parent (or the empty tree for a root).
    Commit { oid: String },
    /// A local branch tip compared with the merge base used by squash detection.
    BranchTip {
        branch: String,
        base: String,
        merge_base: String,
        tip: String,
    },
}

/// A commit file status. This is intentionally separate from worktree
/// [`crate::types::ChangedFileKind`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitFileKind {
    Added,
    Deleted,
    Modified,
    Renamed,
    Copied,
    TypeChanged,
    Unmerged,
    Unknown(String),
}

impl CommitFileKind {
    fn from_status(status: &str) -> Self {
        match status.chars().next().unwrap_or('?') {
            'A' => Self::Added,
            'D' => Self::Deleted,
            'M' => Self::Modified,
            'R' => Self::Renamed,
            'C' => Self::Copied,
            'T' => Self::TypeChanged,
            'U' => Self::Unmerged,
            _ => Self::Unknown(status.to_owned()),
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Deleted => "deleted",
            Self::Modified => "modified",
            Self::Renamed => "renamed",
            Self::Copied => "copied",
            Self::TypeChanged => "type-changed",
            Self::Unmerged => "unmerged",
            Self::Unknown(_) => "unknown",
        }
    }
}

/// One path changed by a commit or branch aggregate diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedCommitFile {
    pub path: String,
    pub old_path: Option<String>,
    pub kind: CommitFileKind,
}

/// The selected commit's message and changed-file summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitDetails {
    pub oid: String,
    pub summary: String,
    pub author_name: String,
    pub author_email: String,
    pub authored_at: Option<DateTime<FixedOffset>>,
    pub message_lines: Vec<String>,
    pub mode: CommitDetailMode,
    pub files: Vec<ChangedCommitFile>,
    /// Log entries are populated for branch-tip details, in newest-first order.
    pub branch_log: Vec<String>,
}

/// A patch for the selected changed file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitFileDiff {
    pub path: String,
    pub patch: String,
}

/// Spawn a loader for one selected graph commit.
pub fn spawn_commit_details_loader(
    repo_path: PathBuf,
    commit: GraphCommit,
    base_branch: String,
) -> Receiver<Result<CommitDetails, String>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = load_commit_details(&repo_path, &commit, &base_branch);
        let _ = tx.send(result);
    });
    rx
}

/// Spawn a loader for the patch of one file from an already loaded details payload.
pub fn spawn_commit_file_diff_loader(
    repo_path: PathBuf,
    details: CommitDetails,
    file: ChangedCommitFile,
) -> Receiver<Result<CommitFileDiff, String>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = load_commit_file_diff(&repo_path, &details, &file);
        let _ = tx.send(result);
    });
    rx
}

/// Load details for a graph commit. Branch-tip mode is selected only from a
/// live local branch ref on the selected commit, and never from `branch`'s
/// historical display label.
pub fn load_commit_details(
    repo_path: &Path,
    commit: &GraphCommit,
    base_branch: &str,
) -> Result<CommitDetails, String> {
    let branch = commit
        .refs
        .iter()
        .find(|reference| {
            reference.kind == GraphRefKind::LocalBranch && reference.name != base_branch
        })
        .map(|reference| reference.name.clone());

    let (mode, files, branch_log) = if let Some(branch) = branch {
        let merge_base = run_git(repo_path, &["merge-base", base_branch, &branch])?
            .trim()
            .to_owned();
        let tip = rev_parse(repo_path, &branch)?;
        let files = parse_name_status(&run_git(
            repo_path,
            &["diff", "--name-status", "-z", &merge_base, &tip, "--"],
        )?)?;
        let branch_log = run_git(
            repo_path,
            &["log", "--format=%h %s", &format!("{base_branch}..{branch}")],
        )?
        .lines()
        .map(ToOwned::to_owned)
        .collect();
        (
            CommitDetailMode::BranchTip {
                branch,
                base: base_branch.to_owned(),
                merge_base,
                tip,
            },
            files,
            branch_log,
        )
    } else {
        let parent = first_parent(repo_path, &commit.oid)?;
        let files = parse_name_status(&run_git(
            repo_path,
            &["diff", "--name-status", "-z", &parent, &commit.oid, "--"],
        )?)?;
        (
            CommitDetailMode::Commit {
                oid: commit.oid.clone(),
            },
            files,
            Vec::new(),
        )
    };

    let (author_name, author_email, authored_at) = commit_metadata(repo_path, &commit.oid)?;
    let message = run_git(repo_path, &["show", "-s", "--format=%B", &commit.oid])?;
    let message_lines = message.lines().take(4).map(ToOwned::to_owned).collect();

    Ok(CommitDetails {
        oid: commit.oid.clone(),
        summary: commit.summary.clone(),
        author_name,
        author_email,
        authored_at,
        message_lines,
        mode,
        files,
        branch_log,
    })
}

/// Load one file patch for the selected details payload.
pub fn load_commit_file_diff(
    repo_path: &Path,
    details: &CommitDetails,
    file: &ChangedCommitFile,
) -> Result<CommitFileDiff, String> {
    let (old, new) = match &details.mode {
        CommitDetailMode::Commit { oid } => (first_parent(repo_path, oid)?, oid.clone()),
        CommitDetailMode::BranchTip {
            merge_base, tip, ..
        } => (merge_base.clone(), tip.clone()),
    };
    let patch = run_git(repo_path, &["diff", &old, &new, "--", &file.path])?;
    Ok(CommitFileDiff {
        path: file.path.clone(),
        patch,
    })
}

fn rev_parse(repo_path: &Path, revision: &str) -> Result<String, String> {
    Ok(run_git(repo_path, &["rev-parse", revision])?
        .trim()
        .to_owned())
}

fn first_parent(repo_path: &Path, oid: &str) -> Result<String, String> {
    let parents = run_git(repo_path, &["show", "-s", "--format=%P", oid])?;
    parents
        .split_whitespace()
        .next()
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| "4b825dc642cb6eb9a060e54bf8d69288fbee4904".to_owned())
        .pipe(Ok)
}

fn commit_metadata(
    repo_path: &Path,
    oid: &str,
) -> Result<(String, String, Option<DateTime<FixedOffset>>), String> {
    let metadata = run_git(
        repo_path,
        &["show", "-s", "--format=%an%x00%ae%x00%aI", oid],
    )?;
    let mut fields = metadata.trim_end_matches('\n').split('\0');
    let name = fields.next().unwrap_or_default().to_owned();
    let email = fields.next().unwrap_or_default().to_owned();
    let date = fields.next().and_then(|value| value.parse().ok());
    Ok((name, email, date))
}

fn parse_name_status(raw: &str) -> Result<Vec<ChangedCommitFile>, String> {
    let fields: Vec<&[u8]> = raw.as_bytes().split(|byte| *byte == 0).collect();
    let mut files = Vec::new();
    let mut index = 0;
    while index < fields.len() {
        if fields[index].is_empty() {
            index += 1;
            continue;
        }
        let status = String::from_utf8_lossy(fields[index]).to_string();
        let kind = CommitFileKind::from_status(&status);
        let old_path = if matches!(kind, CommitFileKind::Renamed | CommitFileKind::Copied) {
            index += 1;
            Some(String::from_utf8_lossy(fields.get(index).ok_or("missing old path")?).into_owned())
        } else {
            None
        };
        index += 1;
        let path =
            String::from_utf8_lossy(fields.get(index).ok_or("missing changed path")?).into_owned();
        files.push(ChangedCommitFile {
            path,
            old_path,
            kind,
        });
        index += 1;
    }
    Ok(files)
}

fn run_git(repo_path: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_path)
        .output()
        .map_err(|error| format!("failed to run git: {error}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

trait Pipe: Sized {
    fn pipe<T>(self, function: impl FnOnce(Self) -> T) -> T;
}

impl<T> Pipe for T {
    fn pipe<U>(self, function: impl FnOnce(Self) -> U) -> U {
        function(self)
    }
}
