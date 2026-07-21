//! Sequential background execution queue for confirmed `BranchAction`s.
//!
//! Replaces the old single-slot "one action in flight, modal overlay" model:
//! confirming an action while another is already running queues it instead of
//! blocking the UI, and everything runs non-modally. Jobs are strictly
//! sequential -- a new job's thread is never spawned until the previous one
//! has actually finished (see the `draining` handling in [`ActionJobQueue::poll`]),
//! since several actions (`Merge`, `Rebase`, `Checkout`, ...) mutate the single
//! shared working tree and would race if run concurrently.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Arc;

use chrono::{DateTime, TimeDelta, Utc};

use crate::git::{operations, tags, worktree};
use crate::types::{BranchAction, FailureCause, OperationResult, ProgressUpdate};
use crate::view::ViewId;

/// How long a completed job's summary lingers in the status area.
const SUMMARY_LINGER_SECS: i64 = 4;

/// A confirmed action, queued or executing: the action, its targets, optional
/// remote context, and the view to refresh once it completes.
#[derive(Debug, Clone)]
pub struct ActionJob {
    pub action: BranchAction,
    pub targets: Vec<String>,
    pub remote: Option<String>,
    pub return_view: ViewId,
}

struct RunningJob {
    job: ActionJob,
    op_rx: Receiver<Vec<OperationResult>>,
    progress_rx: Receiver<ProgressUpdate>,
    cancel_flag: Arc<AtomicBool>,
    latest_progress: Option<ProgressUpdate>,
}

/// A just-cancelled job whose thread hasn't exited yet. Kept separate from
/// `RunningJob` so the queue can report "nothing running" immediately on
/// cancel while still refusing to start the next job until this one actually
/// resolves -- that's what keeps execution strictly sequential. Keeps the
/// full `ActionJob` (not just a target count) because a cancelled job's
/// thread may still complete real, partial work before it exits -- when it
/// does, `poll` needs the action/targets/return_view to still emit a
/// `JobEvent` for it, rather than silently dropping results for changes that
/// already happened on disk.
struct DrainingJob {
    job: ActionJob,
    op_rx: Receiver<Vec<OperationResult>>,
}

/// Outcome of the most recently completed job, shown briefly in the status area.
#[derive(Debug, Clone)]
pub struct CompletionSummary {
    pub label: String,
    pub success_count: usize,
    /// Count of successful results plus `BranchNotFound` results, which are
    /// treated as "already gone" success — no overlay, no summary failure.
    /// See plan P005 §9.
    pub fail_count: usize,
    /// Failed results excluding `BranchNotFound`. The UI uses this to
    /// decide whether to auto-open the Results overlay (§5) and the
    /// footer renders recovery keys only when at least one row is
    /// recoverable.
    pub failures: Vec<OperationResult>,
    expires_at: DateTime<Utc>,
}

impl CompletionSummary {
    fn new(action: BranchAction, results: &[OperationResult]) -> Self {
        let success_count = results
            .iter()
            .filter(|r| {
                r.success
                    || matches!(&r.failure, Some(crate::types::FailureCause::BranchNotFound))
            })
            .count();
        let failures: Vec<OperationResult> = results
            .iter()
            .filter(|r| {
                !r.success && !matches!(&r.failure, Some(crate::types::FailureCause::BranchNotFound))
            })
            .cloned()
            .collect();
        let fail_count = failures.len();
        Self {
            label: action.label().to_string(),
            success_count,
            fail_count,
            failures,
            expires_at: Utc::now() + TimeDelta::seconds(SUMMARY_LINGER_SECS),
        }
    }

    fn is_expired(&self) -> bool {
        Utc::now() > self.expires_at
    }
}

/// Emitted by [`ActionJobQueue::poll`] when a job's result channel resolves,
/// carrying everything the caller needs to do its own view-specific
/// post-processing (refreshing the right view, the `DeleteLocalAndRemote`
/// remotes filter, etc).
pub struct JobEvent {
    pub action: BranchAction,
    pub targets: Vec<String>,
    pub remote: Option<String>,
    pub return_view: ViewId,
    pub results: Vec<OperationResult>,
    /// Failed results excluding `BranchNotFound`. Mirrors
    /// [`CompletionSummary::failures`] so the caller doesn't need a
    /// separate accessor in the same tick. Non-empty triggers the
    /// Results overlay auto-open (plan P005 §5).
    pub failures: Vec<OperationResult>,
}

/// Result of one [`ActionJobQueue::poll`] call.
pub struct JobPoll {
    pub event: Option<JobEvent>,
    /// True if anything changed that should trigger a redraw (progress
    /// advanced, a job completed, draining resolved, or the lingering
    /// summary just expired).
    pub dirty: bool,
}

/// Read-only snapshot for rendering the job-status area.
pub struct JobStatusView<'a> {
    pub visible: bool,
    pub current_label: Option<&'a str>,
    pub current_progress: Option<&'a ProgressUpdate>,
    pub queued_count: usize,
    pub targets_done: usize,
    pub targets_total: usize,
    pub summary: Option<&'a CompletionSummary>,
}

/// Sequential queue of confirmed actions, running at most one at a time on a
/// background thread.
pub struct ActionJobQueue {
    repo_path: PathBuf,
    base_branch: String,
    current: Option<RunningJob>,
    draining: Option<DrainingJob>,
    queued: VecDeque<ActionJob>,
    targets_done_before_current: usize,
    last_summary: Option<CompletionSummary>,
}

impl ActionJobQueue {
    pub fn new(repo_path: PathBuf, base_branch: String) -> Self {
        Self {
            repo_path,
            base_branch,
            current: None,
            draining: None,
            queued: VecDeque::new(),
            targets_done_before_current: 0,
            last_summary: None,
        }
    }

    fn is_busy(&self) -> bool {
        self.current.is_some() || self.draining.is_some()
    }

    /// Start `action` immediately if idle, otherwise queue it to run once
    /// everything ahead of it finishes. No-op for empty `targets`.
    pub fn enqueue_or_start(
        &mut self,
        action: BranchAction,
        targets: Vec<String>,
        return_view: ViewId,
    ) {
        self.enqueue_or_start_with_remote(action, targets, None, return_view);
    }

    /// Like [`Self::enqueue_or_start`], while retaining the authoritative
    /// remote name for a single remote-branch action.
    pub fn enqueue_or_start_with_remote(
        &mut self,
        action: BranchAction,
        targets: Vec<String>,
        remote: Option<String>,
        return_view: ViewId,
    ) {
        if targets.is_empty() {
            return;
        }
        let job = ActionJob {
            action,
            targets,
            remote,
            return_view,
        };
        if self.is_busy() {
            self.queued.push_back(job);
        } else {
            self.start(job);
        }
    }

    fn start(&mut self, job: ActionJob) {
        let repo_path = self.repo_path.clone();
        let base_branch = self.base_branch.clone();
        let item_names = job.targets.clone();
        let action = job.action;
        let remote = job.remote.clone();

        let (op_tx, op_rx) = mpsc::channel();
        let (prog_tx, prog_rx) = mpsc::channel();
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let cancel_clone = Arc::clone(&cancel_flag);

        std::thread::spawn(move || {
            let needs_stash =
                !crate::git::status::detect_working_tree_status(&repo_path).is_clean();
            let results = execute_action_with_remote(
                action,
                &item_names,
                &repo_path,
                &base_branch,
                needs_stash,
                remote.as_deref(),
                &prog_tx,
                &cancel_clone,
            );
            let _ = op_tx.send(results);
        });

        self.current = Some(RunningJob {
            job,
            op_rx,
            progress_rx: prog_rx,
            cancel_flag,
            latest_progress: None,
        });
    }

    /// Try to start the next queued job. Only allowed when nothing is
    /// currently running and no cancelled job's thread is still draining --
    /// this is what guarantees strict sequencing (see module docs). Resets
    /// the aggregate target counter once the whole queue has drained, so the
    /// next "session" starts clean.
    fn try_advance(&mut self) {
        if self.is_busy() {
            return;
        }
        if let Some(job) = self.queued.pop_front() {
            self.start(job);
        } else {
            self.targets_done_before_current = 0;
        }
    }

    /// Drain progress/result channels once per tick.
    pub fn poll(&mut self) -> JobPoll {
        let mut dirty = false;
        let mut event = None;

        // A cancelled job's thread may still complete real work before it
        // exits -- surface that outcome (view refresh + summary) exactly
        // like a normal completion, rather than discarding it, since the
        // repo may already have changed on disk.
        match self.draining.as_ref().map(|d| d.op_rx.try_recv()) {
            Some(Ok(results)) => {
                let DrainingJob { job, .. } = self.draining.take().unwrap();
                self.targets_done_before_current += job.targets.len();
                let summary = CompletionSummary::new(job.action, &results);
                let failures = summary.failures.clone();
                self.last_summary = Some(summary);
                self.try_advance();
                dirty = true;
                event = Some(JobEvent {
                    action: job.action,
                    targets: job.targets,
                    remote: job.remote,
                    return_view: job.return_view,
                    results,
                    failures,
                });
            }
            Some(Err(TryRecvError::Disconnected)) => {
                // Thread exited without sending (panic) -- nothing to
                // report, but still safe to advance the queue.
                let DrainingJob { job, .. } = self.draining.take().unwrap();
                self.targets_done_before_current += job.targets.len();
                self.try_advance();
                dirty = true;
            }
            _ => {}
        }

        if let Some(running) = &mut self.current {
            for _ in 0..32 {
                match running.progress_rx.try_recv() {
                    Ok(update) => {
                        running.latest_progress = Some(update);
                        dirty = true;
                    }
                    Err(_) => break,
                }
            }
        }

        if event.is_none() {
            event = match self.current.as_ref().map(|r| r.op_rx.try_recv()) {
                Some(Ok(results)) => {
                    let RunningJob { job, .. } = self.current.take().unwrap();
                    self.targets_done_before_current += job.targets.len();
                    let summary = CompletionSummary::new(job.action, &results);
                    let failures = summary.failures.clone();
                    self.last_summary = Some(summary);
                    self.try_advance();
                    dirty = true;
                    Some(JobEvent {
                        action: job.action,
                        targets: job.targets,
                        remote: job.remote,
                        return_view: job.return_view,
                        results,
                        failures,
                    })
                }
                _ => None,
            };
        }

        if self
            .last_summary
            .as_ref()
            .is_some_and(CompletionSummary::is_expired)
        {
            self.last_summary = None;
            dirty = true;
        }

        JobPoll { event, dirty }
    }

    /// Cancel the currently-running job. Its thread keeps running until its
    /// next cooperative cancel check (a single git subprocess call is never
    /// force-killed); the next queued job won't start until that thread
    /// actually resolves (see `poll`). No-op if nothing is running.
    pub fn cancel_current(&mut self) {
        let Some(running) = self.current.take() else {
            return;
        };
        running.cancel_flag.store(true, Ordering::Relaxed);
        self.draining = Some(DrainingJob {
            job: running.job,
            op_rx: running.op_rx,
        });
    }

    /// Drop everything not yet started. Never touches the running or
    /// draining job.
    pub fn clear_queued(&mut self) {
        self.queued.clear();
    }

    /// Read-only snapshot for the UI layer.
    pub fn render_data(&self) -> JobStatusView<'_> {
        let summary = self.last_summary.as_ref().filter(|s| !s.is_expired());
        let visible = self.current.is_some()
            || self.draining.is_some()
            || !self.queued.is_empty()
            || summary.is_some();

        let current_target_count = self.current.as_ref().map_or(0, |r| r.job.targets.len());
        let draining_target_count = self.draining.as_ref().map_or(0, |d| d.job.targets.len());
        let queued_target_count: usize = self.queued.iter().map(|j| j.targets.len()).sum();
        let targets_total = self.targets_done_before_current
            + current_target_count
            + draining_target_count
            + queued_target_count;
        let current_completed = self
            .current
            .as_ref()
            .and_then(|r| r.latest_progress.as_ref())
            .map_or(0, |p| p.completed);
        let targets_done = self.targets_done_before_current + current_completed;

        JobStatusView {
            visible,
            current_label: self.current.as_ref().map(|r| r.job.action.label()),
            current_progress: self
                .current
                .as_ref()
                .and_then(|r| r.latest_progress.as_ref()),
            queued_count: self.queued.len(),
            targets_done,
            targets_total,
            summary,
        }
    }

    // ---- Test-only seams ----
    //
    // These are plain `pub` (not `#[cfg(test)]`) because `App`'s own tests
    // live in the binary crate (`src/app.rs`), which depends on this library
    // crate as a regular dependency -- `cfg(test)` on this side is not set
    // when the binary's test target is compiled, so a cfg-gated method here
    // would be invisible to `app.rs`'s tests.

    /// Inject a fake "currently running" job with test-controlled channels,
    /// bypassing `start`'s real thread spawn.
    pub fn inject_running_for_test(
        &mut self,
        job: ActionJob,
        op_rx: Receiver<Vec<OperationResult>>,
        progress_rx: Receiver<ProgressUpdate>,
        cancel_flag: Arc<AtomicBool>,
    ) {
        self.current = Some(RunningJob {
            job,
            op_rx,
            progress_rx,
            cancel_flag,
            latest_progress: None,
        });
    }

    pub fn is_running_for_test(&self) -> bool {
        self.current.is_some()
    }

    pub fn is_draining_for_test(&self) -> bool {
        self.draining.is_some()
    }

    pub fn queued_len_for_test(&self) -> usize {
        self.queued.len()
    }

    pub fn current_action_for_test(&self) -> Option<BranchAction> {
        self.current.as_ref().map(|r| r.job.action)
    }

    pub fn current_targets_for_test(&self) -> Option<&[String]> {
        self.current
            .as_ref()
            .map(|running| running.job.targets.as_slice())
    }
}

// ---- Action Execution (runs on background thread) ----

#[allow(clippy::too_many_arguments)]
fn execute_branch_name_cascade(
    action: BranchAction,
    item_names: &[String],
    repo_path: &Path,
    remote: Option<&str>,
    prog_tx: &Sender<ProgressUpdate>,
    cancel_flag: &Arc<AtomicBool>,
) -> Vec<OperationResult> {
    let repo = match git2::Repository::open(repo_path) {
        Ok(repo) => repo,
        Err(error) => {
            return vec![OperationResult {
                branch_name: String::new(),
                action,
                success: false,
                message: format!("Failed to open repo: {error}"),
                failure: None,
            }];
        }
    };

    let all_worktrees = match worktree::try_list_worktrees(repo_path) {
        Ok(worktrees) => worktrees,
        Err(error) => {
            return item_names
                .iter()
                .map(|name| {
                    OperationResult::failure(
                        name,
                        action,
                        FailureCause::Other {
                            raw_message: error.clone(),
                        },
                        format!("Cannot inspect worktrees for {name}: {error}"),
                    )
                })
                .collect();
        }
    };

    let force = action == BranchAction::DeleteBranchAndRemoveWorktreeForce;
    let remove_remote = action == BranchAction::DeleteBranchAndRemoveWorktreeRemote;
    let mut results = Vec::new();
    let mut locally_deleted = Vec::new();

    for (index, name) in item_names.iter().enumerate() {
        if cancel_flag.load(Ordering::Relaxed) {
            results.push(OperationResult {
                branch_name: String::new(),
                action,
                success: false,
                message: "Cancelled by user".into(),
                failure: None,
            });
            break;
        }
        let _ = prog_tx.send(ProgressUpdate {
            completed: index,
            total: item_names.len(),
            current_item: name.clone(),
        });

        let Some(worktree) = all_worktrees
            .iter()
            .find(|worktree| worktree.branch.as_deref() == Some(name.as_str()))
        else {
            // The worktree may have disappeared after the pre-flight. Still
            // attempt the branch deletion; its own safe/force checks remain
            // authoritative and classify any race for the Results overlay.
            let delete_result = if force {
                operations::delete_local_force(&repo, name)
            } else {
                operations::delete_local(&repo, name)
            };
            if delete_result.success {
                locally_deleted.push(name.clone());
            }
            results.push(delete_result);
            continue;
        };

        if worktree.is_main {
            results.push(OperationResult::failure(
                name,
                action,
                FailureCause::CheckedOutInWorktree {
                    worktree_path: worktree.path.clone(),
                    is_main: true,
                },
                format!(
                    "Cannot remove the primary worktree at {}",
                    worktree.path.display()
                ),
            ));
            continue;
        }

        let remove_result = if force {
            operations::force_remove_worktree(repo_path, &worktree.path)
        } else {
            operations::remove_worktree(repo_path, &worktree.path)
        };
        let removed = remove_result.success;
        results.push(remove_result);

        if removed {
            let delete_result = if force {
                operations::delete_local_force(&repo, name)
            } else {
                operations::delete_local(&repo, name)
            };
            if delete_result.success {
                locally_deleted.push(name.clone());
            }
            results.push(delete_result);
        }
    }

    if remove_remote && !locally_deleted.is_empty() && !cancel_flag.load(Ordering::Relaxed) {
        let remote_name = remote.unwrap_or("origin");
        let _ = prog_tx.send(ProgressUpdate {
            completed: locally_deleted.len(),
            total: item_names.len(),
            current_item: "Deleting remote branches...".into(),
        });
        results.extend(operations::delete_remotes_batch_for_remote(
            repo_path,
            remote_name,
            &locally_deleted,
            cancel_flag,
        ));
    }

    results
}

#[allow(clippy::too_many_arguments)]
fn execute_action_with_remote(
    action: BranchAction,
    item_names: &[String],
    repo_path: &Path,
    base_branch: &str,
    needs_stash: bool,
    remote: Option<&str>,
    prog_tx: &Sender<ProgressUpdate>,
    cancel_flag: &Arc<AtomicBool>,
) -> Vec<OperationResult> {
    let total = item_names.len();
    let mut results = Vec::new();

    match action {
        BranchAction::DeleteLocal
        | BranchAction::DeleteLocalForce
        | BranchAction::DeleteLocalAndRemote => {
            let repo = match git2::Repository::open(repo_path) {
                Ok(r) => r,
                Err(e) => {
                    return vec![OperationResult {
                        branch_name: String::new(),
                        action,
                        success: false,
                        message: format!("Failed to open repo: {e}"),
                        failure: None,
                    }];
                }
            };
            let mut locally_deleted = Vec::new();
            for (i, name) in item_names.iter().enumerate() {
                if cancel_flag.load(Ordering::Relaxed) {
                    results.push(OperationResult {
                        branch_name: String::new(),
                        action,
                        success: false,
                        message: "Cancelled by user".into(),
                        failure: None,
                    });
                    break;
                }
                let _ = prog_tx.send(ProgressUpdate {
                    completed: i,
                    total,
                    current_item: name.clone(),
                });
                let result = if action == BranchAction::DeleteLocalForce {
                    operations::delete_local_force(&repo, name)
                } else {
                    operations::delete_local(&repo, name)
                };
                if result.success {
                    locally_deleted.push(name.clone());
                }
                results.push(result);
            }
            if action == BranchAction::DeleteLocalAndRemote && !locally_deleted.is_empty() {
                let _ = prog_tx.send(ProgressUpdate {
                    completed: locally_deleted.len(),
                    total,
                    current_item: "Deleting remote branches...".into(),
                });
                let remote_name = remote.unwrap_or("origin");
                results.extend(operations::delete_remotes_batch_for_remote(
                    repo_path,
                    remote_name,
                    &locally_deleted,
                    cancel_flag,
                ));
            }
        }
        BranchAction::Checkout => {
            if let Some(name) = item_names.first() {
                let repo = match git2::Repository::open(repo_path) {
                    Ok(r) => r,
                    Err(e) => {
                        return vec![OperationResult {
                            branch_name: name.clone(),
                            action,
                            success: false,
                            message: format!("Failed to open repo: {e}"),
                            failure: None,
                        }];
                    }
                };
                results.push(operations::checkout_branch(
                    &repo,
                    repo_path,
                    name,
                    needs_stash,
                ));
            }
        }
        BranchAction::FastForward => {
            if let Some(name) = item_names.first() {
                results.push(operations::fast_forward(repo_path, name, cancel_flag));
            }
        }
        BranchAction::Push => {
            for (i, name) in item_names.iter().enumerate() {
                if cancel_flag.load(Ordering::Relaxed) {
                    break;
                }
                let _ = prog_tx.send(ProgressUpdate {
                    completed: i,
                    total,
                    current_item: name.clone(),
                });
                results.push(operations::push_branch(repo_path, name, cancel_flag));
            }
        }
        BranchAction::ForcePush => {
            if let Some(name) = item_names.first() {
                results.push(operations::force_push_branch(repo_path, name, cancel_flag));
            }
        }
        BranchAction::Pull => {
            if let Some(name) = item_names.first() {
                // Assume not current for context menu; pull_branch handles it
                results.push(operations::pull_branch(repo_path, name, false, cancel_flag));
            }
        }
        BranchAction::Merge | BranchAction::SquashMerge => {
            if let Some(name) = item_names.first() {
                let squash = action == BranchAction::SquashMerge;
                results.extend(operations::merge_branch(
                    repo_path,
                    name,
                    base_branch,
                    squash,
                    needs_stash,
                ));
            }
        }
        BranchAction::Rebase => {
            if let Some(name) = item_names.first() {
                results.extend(operations::rebase_branch(
                    repo_path,
                    name,
                    base_branch,
                    needs_stash,
                ));
            }
        }
        BranchAction::Worktree => {
            if let Some(name) = item_names.first() {
                results.push(operations::create_worktree(repo_path, name));
            }
        }
        BranchAction::DeleteTag | BranchAction::DeleteTagAndRemote => {
            let repo = match git2::Repository::open(repo_path) {
                Ok(r) => r,
                Err(e) => {
                    return vec![OperationResult {
                        branch_name: String::new(),
                        action,
                        success: false,
                        message: format!("Failed to open repo: {e}"),
                        failure: None,
                    }];
                }
            };
            let tag_names: Vec<String> = item_names.to_vec();
            results.extend(tags::delete_tags_batch(&repo, &tag_names));
            if action == BranchAction::DeleteTagAndRemote {
                let successfully_deleted: Vec<String> = results
                    .iter()
                    .filter(|r| r.success)
                    .map(|r| r.branch_name.clone())
                    .collect();
                if !successfully_deleted.is_empty() {
                    results.extend(tags::delete_remote_tags_batch(
                        repo_path,
                        &successfully_deleted,
                    ));
                }
            }
        }
        BranchAction::PushTag => {
            for name in item_names {
                results.push(tags::push_tag(repo_path, name));
            }
        }
        BranchAction::DeleteRemoteBranch => {
            let remote_name = remote.unwrap_or("origin");
            results.extend(operations::delete_remotes_with_progress_for_remote(
                repo_path,
                remote_name,
                item_names,
                prog_tx,
                cancel_flag,
            ));
        }
        BranchAction::DeleteRemoteAndLocal => {
            if let Some(name) = item_names.first() {
                let remote_name = remote.unwrap_or("origin");
                let remote_results = operations::delete_remotes_batch_for_remote(
                    repo_path,
                    remote_name,
                    std::slice::from_ref(name),
                    cancel_flag,
                );
                results.extend(remote_results);
                if let Ok(repo) = git2::Repository::open(repo_path) {
                    let local_result = operations::delete_local(&repo, name);
                    results.push(OperationResult {
                        action: BranchAction::DeleteRemoteAndLocal,
                        ..local_result
                    });
                }
            }
        }
        BranchAction::CheckoutRemote => {
            if let Some(name) = item_names.first() {
                let remote_name = remote.unwrap_or("origin");
                results.push(operations::checkout_remote_branch(
                    repo_path,
                    remote_name,
                    name,
                ));
            }
        }
        BranchAction::FetchRemote => {
            if !item_names.is_empty() {
                let remote_name = remote.unwrap_or("origin");
                results.extend(operations::fetch_remote(
                    repo_path,
                    remote_name,
                    cancel_flag,
                ));
            }
        }
        BranchAction::PullRemote => {
            if let Some(name) = item_names.first() {
                let remote_name = remote.unwrap_or("origin");
                results.extend(operations::pull_remote(
                    repo_path,
                    remote_name,
                    name,
                    cancel_flag,
                ));
            }
        }
        BranchAction::MergeRemoteIntoCurrent => {
            if let Some(name) = item_names.first() {
                let remote_name = remote.unwrap_or("origin");
                let full_ref = format!("{remote_name}/{name}");
                results.extend(operations::merge_remote_into_current(
                    repo_path, &full_ref, name,
                ));
            }
        }
        BranchAction::CherryPickRemote => {
            if let Some(name) = item_names.first() {
                let remote_name = remote.unwrap_or("origin");
                let full_ref = format!("{remote_name}/{name}");
                results.extend(operations::cherry_pick_remote(repo_path, &full_ref, name));
            }
        }
        BranchAction::WorktreeRemove | BranchAction::WorktreeForceRemove => {
            let force = action == BranchAction::WorktreeForceRemove;
            for (i, path_str) in item_names.iter().enumerate() {
                if cancel_flag.load(Ordering::Relaxed) {
                    break;
                }
                let wt_path = PathBuf::from(path_str);
                let partial_delete_risk = AtomicBool::new(false);
                let result = if force {
                    operations::force_remove_worktree(
                        repo_path,
                        &wt_path,
                        (i, total),
                        prog_tx,
                        cancel_flag,
                        &partial_delete_risk,
                    )
                } else {
                    operations::remove_worktree(
                        repo_path,
                        &wt_path,
                        (i, total),
                        prog_tx,
                        cancel_flag,
                        &partial_delete_risk,
                    )
                };
                results.push(result);
            }
        }
        BranchAction::WorktreeRemoveAndDeleteBranch
        | BranchAction::WorktreeRemoveAndDeleteBranchRemote => {
            let repo = match git2::Repository::open(repo_path) {
                Ok(r) => r,
                Err(e) => {
                    return vec![OperationResult {
                        branch_name: String::new(),
                        action,
                        success: false,
                        message: format!("Failed to open repo: {e}"),
                        failure: None,
                    }];
                }
            };
            let mut locally_deleted = Vec::new();
            for (i, path_str) in item_names.iter().enumerate() {
                if cancel_flag.load(Ordering::Relaxed) {
                    results.push(OperationResult {
                        branch_name: String::new(),
                        action,
                        success: false,
                        message: "Cancelled by user".into(),
                        failure: None,
                    });
                    break;
                }

                let wt_path = PathBuf::from(path_str);
                let canonical_wt_path = std::fs::canonicalize(&wt_path).ok();

                // Look up the branch checked out in this worktree BEFORE removing
                // it -- removal deregisters the worktree, so `git worktree list`
                // can no longer tell us what branch it held.
                let all_wts = worktree::list_worktrees(repo_path);
                let branch_name = all_wts
                    .into_iter()
                    .find(|w| {
                        if let Ok(wt_canonical) = std::fs::canonicalize(&w.path) {
                            if let Some(ref cwp) = canonical_wt_path {
                                return wt_canonical == *cwp;
                            }
                        }
                        w.path == wt_path
                    })
                    .and_then(|w| w.branch);

                let partial_delete_risk = AtomicBool::new(false);
                let remove_result = operations::remove_worktree(
                    repo_path,
                    &wt_path,
                    (i, total),
                    prog_tx,
                    cancel_flag,
                    &partial_delete_risk,
                );
                let removed = remove_result.success;
                results.push(remove_result);

                if removed {
                    if let Some(name) = branch_name {
                        let delete_result = operations::delete_local(&repo, &name);
                        if delete_result.success {
                            locally_deleted.push(name);
                        }
                        results.push(delete_result);
                    }
                }
            }
            if action == BranchAction::WorktreeRemoveAndDeleteBranchRemote
                && !locally_deleted.is_empty()
            {
                let _ = prog_tx.send(ProgressUpdate {
                    completed: locally_deleted.len(),
                    total,
                    current_item: "Deleting remote branches...".into(),
                });
                results.extend(operations::delete_remotes_batch(
                    repo_path,
                    &locally_deleted,
                    cancel_flag,
                ));
            }
        }
        BranchAction::DeleteBranchAndRemoveWorktree
        | BranchAction::DeleteBranchAndRemoveWorktreeForce
        | BranchAction::DeleteBranchAndRemoveWorktreeRemote => {
            results.extend(match action {
                BranchAction::DeleteBranchAndRemoveWorktree
                | BranchAction::DeleteBranchAndRemoveWorktreeForce
                | BranchAction::DeleteBranchAndRemoveWorktreeRemote => {
                    execute_branch_name_cascade(
                        action,
                        item_names,
                        repo_path,
                        remote,
                        prog_tx,
                        cancel_flag,
                    )
                }
                _ => unreachable!("matched new delete action"),
            });
        }
        BranchAction::Fetch | BranchAction::FetchPrune => {
            let result = if action == BranchAction::FetchPrune {
                operations::fetch_prune(repo_path, cancel_flag)
            } else {
                operations::fetch(repo_path, cancel_flag)
            };
            results.push(result);
        }
        BranchAction::ViewRemotePR => {
            // Handled in App::execute_menu_action, shouldn't reach here
        }
    }

    // Send final progress
    let _ = prog_tx.send(ProgressUpdate {
        completed: results.iter().filter(|r| r.success).count().min(total),
        total,
        current_item: "Done".to_string(),
    });

    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn queue() -> ActionJobQueue {
        let dir = std::env::temp_dir().join("git-branch-manager-job-queue-test");
        ActionJobQueue::new(dir, "main".to_string())
    }

    fn job(action: BranchAction, targets: &[&str]) -> ActionJob {
        ActionJob {
            action,
            targets: targets.iter().map(|s| s.to_string()).collect(),
            remote: None,
            return_view: ViewId::Branches,
        }
    }

    /// Inject a running job with channels the test keeps the sending halves of.
    fn inject(
        q: &mut ActionJobQueue,
        j: ActionJob,
    ) -> (
        Sender<Vec<OperationResult>>,
        Sender<ProgressUpdate>,
        Arc<AtomicBool>,
    ) {
        let (op_tx, op_rx) = mpsc::channel();
        let (prog_tx, prog_rx) = mpsc::channel();
        let cancel_flag = Arc::new(AtomicBool::new(false));
        q.inject_running_for_test(j, op_rx, prog_rx, Arc::clone(&cancel_flag));
        (op_tx, prog_tx, cancel_flag)
    }

    fn wait_until(mut cond: impl FnMut() -> bool) -> bool {
        for _ in 0..200 {
            if cond() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }

    #[test]
    fn enqueue_or_start_runs_immediately_when_idle() {
        let mut q = queue();
        q.enqueue_or_start(
            BranchAction::DeleteLocal,
            vec!["a".into()],
            ViewId::Branches,
        );
        assert!(q.is_running_for_test());
        assert_eq!(q.queued_len_for_test(), 0);
    }

    #[test]
    fn enqueue_or_start_ignores_empty_targets() {
        let mut q = queue();
        q.enqueue_or_start(BranchAction::DeleteLocal, vec![], ViewId::Branches);
        assert!(!q.is_running_for_test());
    }

    #[test]
    fn enqueue_or_start_queues_when_busy() {
        let mut q = queue();
        let _channels = inject(&mut q, job(BranchAction::DeleteLocal, &["a"]));
        q.enqueue_or_start(BranchAction::Push, vec!["b".into()], ViewId::Branches);
        assert_eq!(q.queued_len_for_test(), 1);
        assert_eq!(q.current_action_for_test(), Some(BranchAction::DeleteLocal));
    }

    #[test]
    fn poll_completion_advances_queue() {
        let mut q = queue();
        let (op_tx, _prog_tx, _cancel) = inject(&mut q, job(BranchAction::DeleteLocal, &["a"]));
        q.enqueue_or_start(BranchAction::Push, vec!["b".into()], ViewId::Branches);
        assert_eq!(q.queued_len_for_test(), 1);

        op_tx
            .send(vec![OperationResult::success(
                "a",
                BranchAction::DeleteLocal,
                "ok",
            )])
            .unwrap();

        let poll = q.poll();
        assert!(poll.event.is_some());
        assert_eq!(poll.event.unwrap().action, BranchAction::DeleteLocal);
        assert_eq!(q.queued_len_for_test(), 0);
        assert_eq!(q.current_action_for_test(), Some(BranchAction::Push));
    }

    #[test]
    fn cancel_current_blocks_advance_until_drained() {
        let mut q = queue();
        let (op_tx, _prog_tx, cancel_flag) = inject(&mut q, job(BranchAction::Merge, &["a"]));

        q.cancel_current();
        assert!(cancel_flag.load(Ordering::Relaxed));
        assert!(!q.is_running_for_test());
        assert!(q.is_draining_for_test());

        // A second confirmed action queues instead of starting -- the
        // cancelled job's thread hasn't resolved yet.
        q.enqueue_or_start(BranchAction::Push, vec!["b".into()], ViewId::Branches);
        assert_eq!(q.queued_len_for_test(), 1);
        let poll = q.poll();
        assert!(poll.event.is_none());
        assert_eq!(
            q.queued_len_for_test(),
            1,
            "must not advance while draining"
        );

        // Once the cancelled thread's result arrives, the next job may start.
        op_tx
            .send(vec![OperationResult {
                branch_name: "a".into(),
                action: BranchAction::Merge,
                success: false,
                message: "Cancelled by user".into(),
                failure: None,
            }])
            .unwrap();
        assert!(wait_until(|| {
            q.poll();
            !q.is_draining_for_test()
        }));
        assert_eq!(q.queued_len_for_test(), 0);
        assert_eq!(q.current_action_for_test(), Some(BranchAction::Push));
    }

    #[test]
    fn cancelled_job_that_completes_anyway_still_emits_event() {
        // A cancel doesn't kill the subprocess -- the thread may still do
        // real, partial work (e.g. delete 2 of 3 branches) before it checks
        // the cancel flag and exits. That outcome must not be discarded.
        let mut q = queue();
        let (op_tx, _prog_tx, _cancel) =
            inject(&mut q, job(BranchAction::DeleteLocal, &["a", "b", "c"]));

        q.cancel_current();
        assert!(q.is_draining_for_test());

        // Queue a follow-up job so the aggregate counter has something to
        // carry forward into, rather than resetting to zero on full idle.
        q.enqueue_or_start(BranchAction::Push, vec!["d".into()], ViewId::Branches);

        op_tx
            .send(vec![
                OperationResult::success("a", BranchAction::DeleteLocal, "ok"),
                OperationResult::success("b", BranchAction::DeleteLocal, "ok"),
                OperationResult {
                    branch_name: String::new(),
                    action: BranchAction::DeleteLocal,
                    success: false,
                    message: "Cancelled by user".into(),
                    failure: None,
                },
            ])
            .unwrap();

        let poll = q.poll();
        let event = poll
            .event
            .expect("real partial results must not be dropped");
        assert_eq!(event.action, BranchAction::DeleteLocal);
        assert_eq!(event.results.iter().filter(|r| r.success).count(), 2);
        assert!(!q.is_draining_for_test());
        assert_eq!(q.current_action_for_test(), Some(BranchAction::Push));

        // The cancelled job's full target count (not just the successful
        // ones) folds into the aggregate counter, same as a normal
        // completion -- carried forward since the queued job kept it busy.
        assert_eq!(q.render_data().targets_done, 3);
        assert_eq!(q.render_data().targets_total, 4);
    }

    #[test]
    fn clear_queued_does_not_touch_running_job() {
        let mut q = queue();
        inject(&mut q, job(BranchAction::DeleteLocal, &["a"]));
        q.enqueue_or_start(BranchAction::Push, vec!["b".into()], ViewId::Branches);
        q.enqueue_or_start(BranchAction::Push, vec!["c".into()], ViewId::Branches);
        assert_eq!(q.queued_len_for_test(), 2);

        q.clear_queued();

        assert_eq!(q.queued_len_for_test(), 0);
        assert!(q.is_running_for_test());
        assert_eq!(q.current_action_for_test(), Some(BranchAction::DeleteLocal));
    }

    #[test]
    fn target_count_math_across_queue() {
        let mut q = queue();
        let (_op_tx, prog_tx, _cancel) =
            inject(&mut q, job(BranchAction::DeleteLocal, &["a", "b", "c"]));
        q.enqueue_or_start(
            BranchAction::Push,
            vec!["d".into(), "e".into()],
            ViewId::Branches,
        );

        let view = q.render_data();
        assert_eq!(view.targets_total, 5);
        assert_eq!(view.targets_done, 0);

        prog_tx
            .send(ProgressUpdate {
                completed: 2,
                total: 3,
                current_item: "b".into(),
            })
            .unwrap();
        q.poll();

        let view = q.render_data();
        assert_eq!(view.targets_done, 2);
        assert_eq!(view.targets_total, 5);
    }

    #[test]
    fn render_data_hidden_when_idle() {
        let q = queue();
        assert!(!q.render_data().visible);
    }

    #[test]
    fn completion_summary_counts_success_and_failure() {
        let results = vec![
            OperationResult::success("a", BranchAction::DeleteLocal, "ok"),
            OperationResult {
                branch_name: "b".into(),
                action: BranchAction::DeleteLocal,
                success: false,
                message: "failed".into(),
                failure: None,
            },
            OperationResult::failure(
                "c",
                BranchAction::DeleteLocal,
                FailureCause::BranchNotFound,
                "already gone",
            ),
        ];
        let summary = CompletionSummary::new(BranchAction::DeleteLocal, &results);
        assert_eq!(summary.success_count, 2);
        assert_eq!(summary.fail_count, 1);
        assert!(!summary.is_expired());
    }

    fn run_git(dir: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap_or_else(|e| panic!("failed to run git {:?}: {}", args, e));
        if !output.status.success() {
            panic!(
                "git {:?} failed in {}: {}",
                args,
                dir.display(),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    #[test]
    fn execute_action_worktree_remove_and_delete_branch_local_only() {
        let tmpdir = tempfile::tempdir().expect("temp repo");
        let dir = tmpdir.path();
        run_git(dir, &["init", "-b", "main"]);
        run_git(dir, &["config", "user.email", "test@example.com"]);
        run_git(dir, &["config", "user.name", "Test User"]);
        std::fs::write(dir.join("README.md"), "base\n").unwrap();
        run_git(dir, &["add", "README.md"]);
        run_git(dir, &["commit", "-m", "initial"]);

        run_git(dir, &["branch", "wt-branch-delete"]);
        run_git(
            dir,
            &[
                "worktree",
                "add",
                ".worktrees/wt-branch-delete",
                "wt-branch-delete",
            ],
        );
        let wt_path = dir.join(".worktrees").join("wt-branch-delete");
        assert!(wt_path.exists());

        let (prog_tx, _prog_rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let results = execute_action_with_remote(
            BranchAction::WorktreeRemoveAndDeleteBranch,
            &[wt_path.to_string_lossy().to_string()],
            dir,
            "main",
            false,
            None,
            &prog_tx,
            &cancel,
        );

        assert!(results
            .iter()
            .any(|r| r.action == BranchAction::WorktreeRemove && r.success));
        assert!(results
            .iter()
            .any(|r| r.action == BranchAction::DeleteLocal && r.success));
        assert!(!wt_path.exists());

        let repo = git2::Repository::open(dir).unwrap();
        assert!(repo
            .find_branch("wt-branch-delete", git2::BranchType::Local)
            .is_err());
    }

    #[test]
    fn execute_action_delete_local_force_removes_unmerged_branch() {
        let tmpdir = tempfile::tempdir().expect("temp repo");
        let dir = tmpdir.path();
        run_git(dir, &["init", "-b", "main"]);
        run_git(dir, &["config", "user.email", "test@example.com"]);
        run_git(dir, &["config", "user.name", "Test User"]);
        std::fs::write(dir.join("README.md"), "base\n").unwrap();
        run_git(dir, &["add", "README.md"]);
        run_git(dir, &["commit", "-m", "initial"]);
        run_git(dir, &["checkout", "-b", "force-delete"]);
        std::fs::write(dir.join("wip.txt"), "wip\n").unwrap();
        run_git(dir, &["add", "wip.txt"]);
        run_git(dir, &["commit", "-m", "wip"]);
        run_git(dir, &["checkout", "main"]);

        let (prog_tx, _prog_rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let results = execute_action_with_remote(
            BranchAction::DeleteLocalForce,
            &["force-delete".into()],
            dir,
            "main",
            false,
            None,
            &prog_tx,
            &cancel,
        );

        assert_eq!(results.len(), 1, "force delete should produce one result");
        assert!(results[0].success, "{results:?}");
        assert_eq!(results[0].action, BranchAction::DeleteLocalForce);
        assert!(git2::Repository::open(dir)
            .unwrap()
            .find_branch("force-delete", git2::BranchType::Local)
            .is_err());
    }

    #[test]
    fn execute_action_branch_cascade_resolves_worktree_by_branch_name() {
        let tmpdir = tempfile::tempdir().expect("temp repo");
        let dir = tmpdir.path();
        run_git(dir, &["init", "-b", "main"]);
        run_git(dir, &["config", "user.email", "test@example.com"]);
        run_git(dir, &["config", "user.name", "Test User"]);
        std::fs::write(dir.join("README.md"), "base\n").unwrap();
        run_git(dir, &["add", "README.md"]);
        run_git(dir, &["commit", "-m", "initial"]);
        run_git(dir, &["branch", "cascade-clean"]);
        run_git(
            dir,
            &["worktree", "add", ".worktrees/cascade-clean", "cascade-clean"],
        );
        let wt_path = dir.join(".worktrees/cascade-clean");

        let (prog_tx, _prog_rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let results = execute_action_with_remote(
            BranchAction::DeleteBranchAndRemoveWorktree,
            &["cascade-clean".into()],
            dir,
            "main",
            false,
            None,
            &prog_tx,
            &cancel,
        );

        assert!(results
            .iter()
            .any(|r| r.action == BranchAction::WorktreeRemove && r.success),
            "{results:?}"
        );
        assert!(results
            .iter()
            .any(|r| r.action == BranchAction::DeleteLocal && r.success),
            "{results:?}"
        );
        assert!(!wt_path.exists());
        assert!(git2::Repository::open(dir)
            .unwrap()
            .find_branch("cascade-clean", git2::BranchType::Local)
            .is_err());
    }

    #[test]
    fn execute_action_force_cascade_removes_dirty_worktree_and_branch() {
        let tmpdir = tempfile::tempdir().expect("temp repo");
        let dir = tmpdir.path();
        run_git(dir, &["init", "-b", "main"]);
        run_git(dir, &["config", "user.email", "test@example.com"]);
        run_git(dir, &["config", "user.name", "Test User"]);
        std::fs::write(dir.join("README.md"), "base\n").unwrap();
        run_git(dir, &["add", "README.md"]);
        run_git(dir, &["commit", "-m", "initial"]);
        run_git(dir, &["branch", "cascade-dirty"]);
        run_git(
            dir,
            &["worktree", "add", ".worktrees/cascade-dirty", "cascade-dirty"],
        );
        let wt_path = dir.join(".worktrees/cascade-dirty");
        std::fs::write(wt_path.join("untracked.txt"), "discard me\n").unwrap();

        let (prog_tx, _prog_rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let results = execute_action_with_remote(
            BranchAction::DeleteBranchAndRemoveWorktreeForce,
            &["cascade-dirty".into()],
            dir,
            "main",
            false,
            None,
            &prog_tx,
            &cancel,
        );

        assert!(results
            .iter()
            .any(|r| r.action == BranchAction::WorktreeForceRemove && r.success),
            "{results:?}"
        );
        assert!(results
            .iter()
            .any(|r| r.action == BranchAction::DeleteLocalForce && r.success),
            "{results:?}"
        );
        assert!(!wt_path.exists());
        assert!(git2::Repository::open(dir)
            .unwrap()
            .find_branch("cascade-dirty", git2::BranchType::Local)
            .is_err());
    }

    #[test]
    fn execute_action_cascade_rejects_primary_worktree() {
        let tmpdir = tempfile::tempdir().expect("temp repo");
        let dir = tmpdir.path();
        run_git(dir, &["init", "-b", "main"]);
        run_git(dir, &["config", "user.email", "test@example.com"]);
        run_git(dir, &["config", "user.name", "Test User"]);
        std::fs::write(dir.join("README.md"), "base\n").unwrap();
        run_git(dir, &["add", "README.md"]);
        run_git(dir, &["commit", "-m", "initial"]);

        let (prog_tx, _prog_rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let results = execute_action_with_remote(
            BranchAction::DeleteBranchAndRemoveWorktreeForce,
            &["main".into()],
            dir,
            "main",
            false,
            None,
            &prog_tx,
            &cancel,
        );

        assert!(results.iter().any(|result| matches!(
            &result.failure,
            Some(FailureCause::CheckedOutInWorktree { is_main: true, .. })
        )));
    }

    #[test]
    fn execute_action_worktree_remove_and_delete_branch_remote() {
        let base_tmp = tempfile::tempdir().expect("temp base dir");
        let base_dir = base_tmp.path();

        let remote_dir = base_dir.join("remote.git");
        std::fs::create_dir_all(&remote_dir).unwrap();
        run_git(&remote_dir, &["init", "--bare", "-b", "main"]);

        run_git(base_dir, &["clone", remote_dir.to_str().unwrap(), "work"]);
        let work_dir = base_dir.join("work");
        run_git(&work_dir, &["config", "user.name", "Test User"]);
        run_git(&work_dir, &["config", "user.email", "test@example.com"]);

        std::fs::write(work_dir.join("README.md"), "# Test\n").unwrap();
        run_git(&work_dir, &["add", "."]);
        run_git(&work_dir, &["commit", "-m", "Initial commit"]);
        run_git(&work_dir, &["push", "-u", "origin", "main"]);

        run_git(&work_dir, &["checkout", "-b", "wt-remote-branch"]);
        std::fs::write(work_dir.join("feature.txt"), "content\n").unwrap();
        run_git(&work_dir, &["add", "feature.txt"]);
        run_git(&work_dir, &["commit", "-m", "feature commit"]);
        run_git(&work_dir, &["push", "-u", "origin", "wt-remote-branch"]);
        run_git(&work_dir, &["checkout", "main"]);

        run_git(
            &work_dir,
            &[
                "worktree",
                "add",
                ".worktrees/wt-remote-branch",
                "wt-remote-branch",
            ],
        );
        let wt_path = work_dir.join(".worktrees").join("wt-remote-branch");
        assert!(wt_path.exists());

        let (prog_tx, _prog_rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let results = execute_action_with_remote(
            BranchAction::WorktreeRemoveAndDeleteBranchRemote,
            &[wt_path.to_string_lossy().to_string()],
            &work_dir,
            "main",
            false,
            None,
            &prog_tx,
            &cancel,
        );

        assert!(results
            .iter()
            .any(|r| r.action == BranchAction::WorktreeRemove && r.success));
        assert!(results
            .iter()
            .any(|r| r.action == BranchAction::DeleteLocal && r.success));
        assert!(results
            .iter()
            .any(|r| r.action == BranchAction::DeleteRemoteBranch && r.success));
        assert!(!wt_path.exists());

        let repo = git2::Repository::open(&work_dir).unwrap();
        assert!(repo
            .find_branch("wt-remote-branch", git2::BranchType::Local)
            .is_err());

        run_git(&work_dir, &["fetch", "--prune"]);
    assert!(repo
            .find_branch("origin/wt-remote-branch", git2::BranchType::Remote)
            .is_err());
    }

    #[test]
    fn execute_action_branch_name_cascade_remote_deletes_tracking_branch() {
        let base_tmp = tempfile::tempdir().expect("temp base dir");
        let base_dir = base_tmp.path();

        let remote_dir = base_dir.join("remote.git");
        std::fs::create_dir_all(&remote_dir).unwrap();
        run_git(&remote_dir, &["init", "--bare", "-b", "main"]);

        run_git(base_dir, &["clone", remote_dir.to_str().unwrap(), "work"]);
        let work_dir = base_dir.join("work");
        run_git(&work_dir, &["config", "user.name", "Test User"]);
        run_git(&work_dir, &["config", "user.email", "test@example.com"]);
        std::fs::write(work_dir.join("README.md"), "# Test\n").unwrap();
        run_git(&work_dir, &["add", "."]);
        run_git(&work_dir, &["commit", "-m", "Initial commit"]);
        run_git(&work_dir, &["push", "-u", "origin", "main"]);

        run_git(&work_dir, &["checkout", "-b", "branch-name-remote"]);
        std::fs::write(work_dir.join("feature.txt"), "content\n").unwrap();
        run_git(&work_dir, &["add", "feature.txt"]);
        run_git(&work_dir, &["commit", "-m", "feature commit"]);
        run_git(&work_dir, &["push", "-u", "origin", "branch-name-remote"]);
        run_git(&work_dir, &["checkout", "main"]);
        run_git(
            &work_dir,
            &[
                "worktree",
                "add",
                ".worktrees/branch-name-remote",
                "branch-name-remote",
            ],
        );
        let wt_path = work_dir.join(".worktrees/branch-name-remote");

        let (prog_tx, _prog_rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let results = execute_action_with_remote(
            BranchAction::DeleteBranchAndRemoveWorktreeRemote,
            &["branch-name-remote".into()],
            &work_dir,
            "main",
            false,
            Some("origin"),
            &prog_tx,
            &cancel,
        );

        assert!(results
            .iter()
            .any(|r| r.action == BranchAction::WorktreeRemove && r.success));
        assert!(results
            .iter()
            .any(|r| r.action == BranchAction::DeleteLocal && r.success));
        assert!(results
            .iter()
            .any(|r| r.action == BranchAction::DeleteRemoteBranch && r.success));
        assert!(!wt_path.exists());

        let repo = git2::Repository::open(&work_dir).unwrap();
        assert!(repo
            .find_branch("branch-name-remote", git2::BranchType::Local)
            .is_err());
        run_git(&work_dir, &["fetch", "--prune"]);
        assert!(repo
            .find_branch("origin/branch-name-remote", git2::BranchType::Remote)
            .is_err());
    }

    #[test]
    fn execute_action_remote_checkout_uses_the_selected_remote() {
        let base_tmp = tempfile::tempdir().expect("temp base dir");
        let base_dir = base_tmp.path();
        let remote_dir = base_dir.join("remote.git");
        std::fs::create_dir_all(&remote_dir).unwrap();
        run_git(&remote_dir, &["init", "--bare", "-b", "main"]);

        run_git(base_dir, &["clone", remote_dir.to_str().unwrap(), "work"]);
        let work_dir = base_dir.join("work");
        run_git(&work_dir, &["config", "user.name", "Test User"]);
        run_git(&work_dir, &["config", "user.email", "test@example.com"]);
        std::fs::write(work_dir.join("README.md"), "# Test\n").unwrap();
        run_git(&work_dir, &["add", "README.md"]);
        run_git(&work_dir, &["commit", "-m", "Initial commit"]);
        run_git(&work_dir, &["branch", "-M", "main"]);
        run_git(&work_dir, &["push", "origin", "main"]);
        run_git(&work_dir, &["remote", "rename", "origin", "upstream"]);

        run_git(&work_dir, &["checkout", "-b", "remote-checkout"]);
        std::fs::write(work_dir.join("feature.txt"), "content\n").unwrap();
        run_git(&work_dir, &["add", "feature.txt"]);
        run_git(&work_dir, &["commit", "-m", "feature commit"]);
        run_git(&work_dir, &["push", "-u", "upstream", "remote-checkout"]);
        run_git(&work_dir, &["checkout", "main"]);
        run_git(&work_dir, &["branch", "-D", "remote-checkout"]);
        run_git(&work_dir, &["fetch", "upstream"]);

        let (prog_tx, _prog_rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let results = execute_action_with_remote(
            BranchAction::CheckoutRemote,
            &["remote-checkout".into()],
            &work_dir,
            "main",
            false,
            Some("upstream"),
            &prog_tx,
            &cancel,
        );

        assert!(results.iter().any(|result| result.success), "{results:?}");
        let repo = git2::Repository::open(&work_dir).unwrap();
        assert!(repo
            .find_branch("remote-checkout", git2::BranchType::Local)
            .is_ok());

        run_git(&work_dir, &["checkout", "main"]);
        let deletion_results = execute_action_with_remote(
            BranchAction::DeleteLocalAndRemote,
            &["remote-checkout".into()],
            &work_dir,
            "main",
            false,
            Some("upstream"),
            &prog_tx,
            &cancel,
        );
        assert!(
            deletion_results
                .iter()
                .any(|result| result.action == BranchAction::DeleteRemoteBranch && result.success),
            "{deletion_results:?}"
        );
        assert!(deletion_results
            .iter()
            .any(|result| result.action == BranchAction::DeleteLocal && result.success));
    }
}
