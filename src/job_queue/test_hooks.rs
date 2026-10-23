use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

use super::{ActionJob, ActionJobQueue, RunningJob};
use crate::git::graph::GraphRepositoryDelta;
use crate::types::{BranchAction, OperationResult, ProgressUpdate};

impl ActionJobQueue {
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
        partial_delete_risk: Arc<AtomicBool>,
    ) {
        let (_delta_tx, graph_delta_rx) = mpsc::channel();
        self.inject_running_with_graph_delta_for_test(job, op_rx, graph_delta_rx, progress_rx, cancel_flag, partial_delete_risk);
    }

    /// Inject a fake running job with an explicitly controlled repository
    /// observation receiver. Tests of the normal Graph route should use this
    /// seam or a real queued operation, never an implicit disconnected delta.
    pub fn inject_running_with_graph_delta_for_test(
        &mut self,
        job: ActionJob,
        op_rx: Receiver<Vec<OperationResult>>,
        graph_delta_rx: Receiver<Result<GraphRepositoryDelta, String>>,
        progress_rx: Receiver<ProgressUpdate>,
        cancel_flag: Arc<AtomicBool>,
        partial_delete_risk: Arc<AtomicBool>,
    ) {
        self.current = Some(RunningJob {
            job,
            op_rx,
            graph_delta_rx,
            progress_rx,
            cancel_flag,
            partial_delete_risk,
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

    pub fn queued_jobs_for_test(&self) -> Vec<(BranchAction, Vec<String>)> {
        self.queued
            .iter()
            .map(|job| (job.action, job.targets.clone()))
            .collect()
    }

    pub fn current_action(&self) -> Option<BranchAction> {
        self.current.as_ref().map(|r| r.job.action)
    }

    pub fn current_targets_for_test(&self) -> Option<&[String]> {
        self.current
            .as_ref()
            .map(|running| running.job.targets.as_slice())
    }

    pub fn current_partial_delete_risk(&self) -> bool {
        self.current
            .as_ref()
            .is_some_and(|r| r.partial_delete_risk.load(Ordering::Relaxed))
    }
}
