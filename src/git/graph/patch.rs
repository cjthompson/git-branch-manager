use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};

use crate::git::cache::BranchCache;

use super::{EnrichmentCancel, EnrichmentObserver, EnrichmentStage};

/// Like the established squash loader, Graph patch matching uses four workers
/// regardless of CPU count. Each worker runs at most one Git subprocess at a
/// time, so both worker and active-subprocess concurrency are capped at four.
const GRAPH_PATCH_WORKER_COUNT: usize = 4;

/// Bump whenever `compute_patch`'s `git diff`/`git patch-id` invocation
/// (its flags, or what gets hashed) changes in a way that could yield a
/// different (patch_id, diff_text) pair for the same (old_oid, new_oid).
/// Embedded in every `graph_patch` cache key so a version bump
/// invalidates all previously cached entries at once, with no schema
/// migration needed.
const GRAPH_DIFF_VERSION: u32 = 1;

pub(super) struct PatchJob {
    pub(super) target: PatchTarget,
    pub(super) old_oid: String,
    pub(super) new_oid: String,
}

#[derive(Clone, Debug)]
pub(super) struct SourceTip {
    pub(super) tip_oid: String,
    pub(super) merge_base: String,
    pub(super) source_names: Vec<String>,
}

pub(super) enum PatchTarget {
    BaseCommit(String),
    BranchTip(SourceTip),
}

pub(super) struct PatchResult {
    pub(super) target: PatchTarget,
    pub(super) patch_id: Option<String>,
    /// Raw `git diff` stdout for this job, retained alongside the patch-id so
    /// `annotate_possible_squash_merges` can run the Option 6 fuzzy-match tier
    /// on pairs whose patch IDs don't exactly match, without a second `git
    /// diff` subprocess per job. `Some` whenever the diff subprocess produced
    /// non-empty output (regardless of whether patch-id computation itself
    /// succeeded); `None` for empty/net-zero diffs or subprocess failures.
    pub(super) diff_text: Option<Vec<u8>>,
}

pub(super) fn load_patch_ids(
    repo_path: &Path,
    jobs: Vec<PatchJob>,
    cache: &mut BranchCache,
    cancel: &EnrichmentCancel,
    observer: &EnrichmentObserver,
) -> Option<Vec<PatchResult>> {
    if cancel.is_cancelled() {
        return None;
    }
    if jobs.is_empty() {
        return Some(Vec::new());
    }

    // Cache-hit fast path: resolve every job whose (old_oid, new_oid)
    // diff is already known, with no subprocess cost, before dispatching
    // the remainder to the worker pool. Mirrors the cache-hit/worker-pool
    // split in `squash_loader::spawn_squash_checker`.
    let mut results = Vec::with_capacity(jobs.len());
    let mut misses = Vec::new();
    for job in jobs {
        if cancel.is_cancelled() {
            return None;
        }
        match cache.lookup_graph_patch(&job.old_oid, &job.new_oid, GRAPH_DIFF_VERSION) {
            Some((patch_id, diff_text)) => results.push(PatchResult {
                target: job.target,
                patch_id,
                diff_text,
            }),
            None => misses.push(job),
        }
    }
    if misses.is_empty() {
        return Some(results);
    }

    // This span measures only uncached patch work: from dispatch through all
    // workers finishing. Cache-only calls return above without emitting it.
    let span = tracing::info_span!(
        "graph_squash_patches",
        jobs = results.len() + misses.len(),
        misses = misses.len()
    );
    let _entered = span.enter();

    let worker_count = misses.len().min(GRAPH_PATCH_WORKER_COUNT);
    let queue = Arc::new(Mutex::new(
        misses.into_iter().collect::<std::collections::VecDeque<_>>(),
    ));
    let (tx, rx) = mpsc::channel();
    let mut handles = Vec::with_capacity(worker_count);
    for _ in 0..worker_count {
        let queue = Arc::clone(&queue);
        let tx = tx.clone();
        let cancel = cancel.clone();
        let observer = Arc::clone(observer);
        let repo_path = repo_path.to_path_buf();
        handles.push(std::thread::spawn(move || {
            while let Some(job) = next_patch_job(&queue, &cancel, &observer) {
                if cancel.is_cancelled() {
                    break;
                }
                let (patch_id, diff_text) = compute_patch(&repo_path, &job.old_oid, &job.new_oid);
                if cancel.is_cancelled() {
                    break;
                }
                let sent = tx.send((
                    job.old_oid,
                    job.new_oid,
                    PatchResult {
                        target: job.target,
                        patch_id,
                        diff_text,
                    },
                ));
                if sent.is_err() {
                    break;
                }
            }
        }));
    }
    drop(tx);

    // Cache owner: only this (calling) thread ever touches `cache`,
    // mirroring squash_loader's single-writer rule.
    for (old_oid, new_oid, result) in rx {
        if cancel.is_cancelled() {
            break;
        }
        cache.insert_graph_patch(
            &old_oid,
            &new_oid,
            GRAPH_DIFF_VERSION,
            result.patch_id.clone(),
            result.diff_text.clone(),
        );
        results.push(result);
    }
    for handle in handles {
        let _ = handle.join();
    }

    if cancel.is_cancelled() {
        None
    } else {
        Some(results)
    }
}

fn next_patch_job(
    queue: &Mutex<std::collections::VecDeque<PatchJob>>,
    cancel: &EnrichmentCancel,
    observer: &EnrichmentObserver,
) -> Option<PatchJob> {
    if cancel.is_cancelled() {
        return None;
    }
    let mut queue = queue.lock().ok()?;
    if cancel.is_cancelled() || queue.is_empty() {
        return None;
    }
    observer(EnrichmentStage::PatchDispatch);
    if cancel.is_cancelled() {
        return None;
    }
    let job = queue.pop_front()?;
    if cancel.is_cancelled() {
        return None;
    }
    Some(job)
}

/// Run a single `git diff` between `old_oid` and `new_oid`, then feed its
/// output through `git patch-id --stable`. Returns `(patch_id, diff_text)`:
/// `diff_text` is the raw diff bytes (for the Option 6 fuzzy-match tier),
/// `patch_id` is the stable patch identity (for the existing exact-match
/// tier). Both share the single `git diff` invocation below rather than
/// re-running it per caller.
fn compute_patch(repo_path: &Path, old_oid: &str, new_oid: &str) -> (Option<String>, Option<Vec<u8>>) {
    let diff = Command::new("git")
        .current_dir(repo_path)
        .args([
            "diff",
            "--binary",
            "--full-index",
            "--no-ext-diff",
            "--no-textconv",
            old_oid,
            new_oid,
            "--",
        ])
        .stdin(Stdio::null())
        .output();
    let Ok(diff) = diff else {
        return (None, None);
    };
    if !diff.status.success() || diff.stdout.is_empty() {
        return (None, None);
    }
    let diff_text = Some(diff.stdout.clone());

    let patch_id = (|| {
        let mut patch_id = Command::new("git")
            .current_dir(repo_path)
            .args(["patch-id", "--stable"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        patch_id.stdin.as_mut()?.write_all(&diff.stdout).ok()?;
        let output = patch_id.wait_with_output().ok()?;
        if !output.status.success() {
            return None;
        }
        String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .next()
            .map(str::to_string)
    })();

    (patch_id, diff_text)
}

/// Compute a stable patch ID and the corresponding diff bytes with libgit2.
///
/// This helper is public only so integration tests can compare its output
/// with Git's CLI implementation.
#[doc(hidden)]
pub fn git2_diff_patch(
    repo: &git2::Repository,
    old: git2::Oid,
    new: git2::Oid,
) -> (Option<String>, Option<Vec<u8>>) {
    let (Ok(old_tree), Ok(new_tree)) = (
        repo.find_commit(old).and_then(|commit| commit.tree()),
        repo.find_commit(new).and_then(|commit| commit.tree()),
    ) else {
        return (None, None);
    };

    let mut options = git2::DiffOptions::new();
    options.show_binary(true).id_abbrev(40);
    let Ok(mut diff) = repo.diff_tree_to_tree(Some(&old_tree), Some(&new_tree), Some(&mut options))
    else {
        return (None, None);
    };
    let mut find_options = git2::DiffFindOptions::new();
    find_options.renames(true);
    if diff.find_similar(Some(&mut find_options)).is_err() || diff.deltas().len() == 0 {
        return (None, None);
    }

    let mut text = Vec::new();
    let printed = diff.print(git2::DiffFormat::Patch, |_, _, line| {
        if matches!(line.origin(), '+' | '-' | ' ') {
            text.push(line.origin() as u8);
        }
        text.extend_from_slice(line.content());
        true
    });
    if printed.is_err() || text.is_empty() {
        return (None, None);
    }

    (diff.patchid(None).ok().map(|id| id.to_string()), Some(text))
}
