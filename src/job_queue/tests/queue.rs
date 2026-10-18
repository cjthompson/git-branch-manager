use super::*;

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
    assert_eq!(q.current_action(), Some(BranchAction::DeleteLocal));
}

#[test]
fn poll_completion_advances_queue() {
    let mut q = queue();
    let (op_tx, _prog_tx, _cancel) = inject(&mut q, job(BranchAction::DeleteLocal, &["a"]));
    q.enqueue_or_start(BranchAction::Push, vec!["b".into()], ViewId::Branches);
    assert_eq!(q.queued_len_for_test(), 1);

    op_tx
        .send(vec![OperationResult::success("a", BranchAction::DeleteLocal, "ok")])
        .unwrap();

    let poll = q.poll();
    assert!(poll.event.is_some());
    assert_eq!(poll.event.unwrap().action, BranchAction::DeleteLocal);
    assert_eq!(q.queued_len_for_test(), 0);
    assert_eq!(q.current_action(), Some(BranchAction::Push));
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
    assert_eq!(q.current_action(), Some(BranchAction::Push));
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
    assert_eq!(q.current_action(), Some(BranchAction::Push));

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
    assert_eq!(q.current_action(), Some(BranchAction::DeleteLocal));
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
