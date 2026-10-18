use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, TryRecvError};
use std::sync::Arc;

use super::execute::execute_action_with_remote;
use super::{
    ActionJob, ActionJobQueue, CompletionSummary, DrainingJob, JobEvent, JobPoll,
    JobStatusView, RunningJob,
};
use crate::git::graph::{GraphRepositoryDelta, GraphRepositoryState};
use crate::types::BranchAction;
use crate::view::ViewId;

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
        self.enqueue_or_start_with_remote(action, targets, None, return_view, None);
    }

    /// Like [`Self::enqueue_or_start`], while retaining the authoritative
    /// remote name for a single remote-branch action.
    ///
    /// `dispatch_path` overrides the queue's own `repo_path` for this job
    /// only (see [`ActionJob::dispatch_path`]) -- callers dispatching
    /// `Merge`/`SquashMerge`/`Rebase` against a specific worktree pass
    /// `Some(path)` (resolved by `App::dispatch_path_for`); every other
    /// caller passes `None`, preserving today's behavior of always running
    /// against the queue's own `repo_path` via the `unwrap_or_else` fallback
    /// in [`Self::start`].
    pub fn enqueue_or_start_with_remote(
        &mut self,
        action: BranchAction,
        targets: Vec<String>,
        remote: Option<String>,
        return_view: ViewId,
        dispatch_path: Option<PathBuf>,
    ) {
        if targets.is_empty() {
            return;
        }
        let job = ActionJob {
            action,
            targets,
            remote,
            return_view,
            dispatch_path,
        };
        if self.is_busy() {
            self.queued.push_back(job);
        } else {
            self.start(job);
        }
    }

    fn start(&mut self, job: ActionJob) {
        let repo_path = job
            .dispatch_path
            .clone()
            .unwrap_or_else(|| self.repo_path.clone());
        let graph_head_path = self.repo_path.clone();
        let base_branch = self.base_branch.clone();
        let item_names = job.targets.clone();
        let action = job.action;
        let remote = job.remote.clone();

        let (op_tx, op_rx) = mpsc::channel();
        let (graph_delta_tx, graph_delta_rx) = mpsc::channel();
        let (prog_tx, prog_rx) = mpsc::channel();
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let cancel_clone = Arc::clone(&cancel_flag);
        let partial_delete_risk = Arc::new(AtomicBool::new(false));
        let partial_delete_risk_clone = Arc::clone(&partial_delete_risk);

        std::thread::spawn(move || {
            let before = GraphRepositoryState::capture_with_head(&repo_path, &graph_head_path);
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
                &partial_delete_risk_clone,
            );
            let after = GraphRepositoryState::capture_with_head(&repo_path, &graph_head_path);
            let graph_delta = match (before, after) {
                (Ok(before), Ok(after)) => Ok(GraphRepositoryDelta::between(before, after)),
                (Err(error), _) | (_, Err(error)) => Err(error),
            };
            let _ = graph_delta_tx.send(graph_delta);
            let _ = op_tx.send(results);
        });

        self.current = Some(RunningJob {
            job,
            op_rx,
            graph_delta_rx,
            progress_rx: prog_rx,
            cancel_flag,
            partial_delete_risk,
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
                let DrainingJob { job, graph_delta_rx, .. } = self.draining.take().unwrap();
                let graph_delta = graph_delta_rx.try_recv().unwrap_or_else(|_| Err("repository state observation was unavailable".into()));
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
                    graph_delta,
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
                    let RunningJob { job, graph_delta_rx, .. } = self.current.take().unwrap();
                    let graph_delta = graph_delta_rx.try_recv().unwrap_or_else(|_| Err("repository state observation was unavailable".into()));
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
                        graph_delta,
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
            graph_delta_rx: running.graph_delta_rx,
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
}
