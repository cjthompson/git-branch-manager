//! Sequential background execution queue for confirmed `BranchAction`s.
//!
//! Replaces the old single-slot "one action in flight, modal overlay" model:
//! confirming an action while another is already running queues it instead of
//! blocking the UI, and everything runs non-modally. Jobs are strictly
//! sequential -- a new job's thread is never spawned until the previous one
//! has actually finished (see the `draining` handling in [`ActionJobQueue::poll`]),
//! since several actions (`Merge`, `Rebase`, `Checkout`, ...) mutate the single
//! shared working tree and would race if run concurrently.

mod cascade;
mod execute;
mod queue;
mod test_hooks;
#[cfg(test)]
mod tests;

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Receiver;
use std::sync::Arc;

use chrono::{DateTime, TimeDelta, Utc};

use crate::git::graph::GraphRepositoryDelta;
use crate::types::{BranchAction, OperationResult, ProgressUpdate};
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
    /// Overrides the queue's own `repo_path` for this job only — lets a
    /// caller dispatch a merge/squash/rebase against a specific worktree
    /// instead of always running from `ActionJobQueue::repo_path`. `None`
    /// means "use the queue's own `repo_path`" (every existing call site's
    /// current, unchanged behavior).
    pub dispatch_path: Option<PathBuf>,
}

struct RunningJob {
    job: ActionJob,
    op_rx: Receiver<Vec<OperationResult>>,
    graph_delta_rx: Receiver<Result<GraphRepositoryDelta, String>>,
    progress_rx: Receiver<ProgressUpdate>,
    cancel_flag: Arc<AtomicBool>,
    partial_delete_risk: Arc<AtomicBool>,
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
    graph_delta_rx: Receiver<Result<GraphRepositoryDelta, String>>,
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
                r.success || matches!(&r.failure, Some(crate::types::FailureCause::BranchNotFound))
            })
            .count();
        let failures: Vec<OperationResult> = results
            .iter()
            .filter(|r| {
                !r.success
                    && !matches!(&r.failure, Some(crate::types::FailureCause::BranchNotFound))
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
    pub graph_delta: Result<GraphRepositoryDelta, String>,
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
