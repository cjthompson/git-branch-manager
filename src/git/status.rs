use crate::types::{ChangedFile, ChangedFileKind, WorkingTreeStatus};
use std::path::Path;
use std::process::{Command, Stdio};
use tracing::{field, instrument, Span};

/// Detect working-tree status by shelling out to `git status --porcelain=v2`
/// rather than git2's `Repository::statuses()`. git2's status scan has no
/// fsmonitor/untracked-cache fast path and was measured taking 90+ seconds on
/// a single large monorepo worktree; the `git` CLI can use those repo-level
/// speedups. Porcelain v2 (not v1) is used because v1's XY columns are
/// fixed-width, position-significant text — trimming or re-splitting it can
/// silently shift an unstaged change into the staged column. v2's fields are
/// token-prefixed and space-delimited instead, so splitting is unambiguous.
#[instrument(skip(dir), fields(path = ?dir, result_state = field::Empty))]
pub fn detect_working_tree_status(dir: &Path) -> WorkingTreeStatus {
    let span = Span::current();
    let output = Command::new("git")
        .args(["status", "--porcelain=v2", "--untracked-files=normal"])
        .current_dir(dir)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .output();

    let Ok(output) = output else {
        span.record("result_state", "spawn_error");
        return WorkingTreeStatus::clean();
    };
    if !output.status.success() {
        span.record("result_state", "nonzero_exit");
        return WorkingTreeStatus::clean();
    }
    span.record("result_state", "success");

    let stdout = String::from_utf8_lossy(&output.stdout);

    let mut has_staged = false;
    let mut has_modified = false;
    let mut has_untracked = false;
    let mut changed_files = Vec::new();

    for line in stdout.lines() {
        match line.split_once(' ') {
            Some(("1", rest)) | Some(("2", rest)) => {
                // "1 XY sub mH mI mW hH hI <path>"
                // "2 XY sub mH mI mW hH hI Xscore <path>\t<origPath>"
                let Some((xy, rest)) = rest.split_once(' ') else {
                    continue;
                };
                let mut xy_chars = xy.chars();
                let x = xy_chars.next().unwrap_or('.');
                let y = xy_chars.next().unwrap_or('.');

                // Skip the fixed fields (sub, mH, mI, mW, hH, hI — and, for
                // rename/copy entries, the trailing score field) to reach the
                // path, which may itself contain spaces.
                let fields_before_path = if line.starts_with("2 ") { 7 } else { 6 };
                let Some(path_field) = rest.splitn(fields_before_path + 1, ' ').last() else {
                    continue;
                };
                // Rename/copy entries append "\t<origPath>"; keep only the
                // current path.
                let path = path_field.split('\t').next().unwrap_or(path_field);

                if x != '.' {
                    has_staged = true;
                    changed_files.push(ChangedFile {
                        path: path.to_string(),
                        kind: ChangedFileKind::Staged,
                    });
                }
                if y != '.' {
                    has_modified = true;
                    changed_files.push(ChangedFile {
                        path: path.to_string(),
                        kind: ChangedFileKind::Modified,
                    });
                }
            }
            Some(("u", rest)) => {
                // Unmerged/conflicted: touches both the index and the working
                // tree, so count as both staged and modified.
                let Some((_xy, rest)) = rest.split_once(' ') else {
                    continue;
                };
                let Some(path) = rest.splitn(9, ' ').last() else {
                    continue;
                };
                has_staged = true;
                has_modified = true;
                changed_files.push(ChangedFile {
                    path: path.to_string(),
                    kind: ChangedFileKind::Staged,
                });
                changed_files.push(ChangedFile {
                    path: path.to_string(),
                    kind: ChangedFileKind::Modified,
                });
            }
            Some(("?", path)) => {
                has_untracked = true;
                changed_files.push(ChangedFile {
                    path: path.to_string(),
                    kind: ChangedFileKind::Untracked,
                });
            }
            _ => {}
        }
    }

    WorkingTreeStatus {
        has_staged,
        has_modified,
        has_untracked,
        changed_files,
    }
}
