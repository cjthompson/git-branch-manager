mod execute_local;
mod execute_remote;
mod queue;

use super::*;
use super::execute::execute_action_with_remote;
use crate::types::FailureCause;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Sender};
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
        dispatch_path: None,
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
    q.inject_running_for_test(
        j,
        op_rx,
        prog_rx,
        Arc::clone(&cancel_flag),
        Arc::new(AtomicBool::new(false)),
    );
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
