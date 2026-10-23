use super::*;
use crate::git::graph::{GraphCommit, GraphLine, GraphRef, GraphRefKind};

fn snapshot() -> GraphSnapshot {
    GraphSnapshot {
        source: crate::git::graph::GraphSource::Gleisbau,
        commits: vec![
            GraphCommit {
                oid: "1111111111111111111111111111111111111111".into(),
                summary: "first".into(),
                parents: vec![],
                lane: Some(0),
                branch: None,
                refs: vec![GraphRef {
                    name: "main".into(),
                    kind: GraphRefKind::LocalBranch,
                    has_linked_worktree: false,
                    is_current: false,
                    tracking: None,
                    ahead_of_base: None,
                }],
                relationships: vec![],
                ..GraphCommit::default()
            },
            GraphCommit {
                oid: "2222222222222222222222222222222222222222".into(),
                summary: "second".into(),
                parents: vec![],
                lane: Some(0),
                branch: None,
                refs: vec![],
                relationships: vec![],
                ..GraphCommit::default()
            },
        ],
        lines: vec![
            GraphLine {
                graph: "*".into(),
                commit_index: Some(0),
            },
            GraphLine {
                graph: "|".into(),
                commit_index: None,
            },
            GraphLine {
                graph: "*".into(),
                commit_index: Some(1),
            },
        ],
        ref_counts: Default::default(),
        max_count: 500,
        includes_remotes: false,
        generation: None,
    }
}

#[test]
fn graph_navigation_uses_one_row_cursor() {
    let mut state = GraphState::new();
    state.apply_result(Ok(snapshot()));
    state.move_down();
    assert_eq!(state.commit_cursor(), 1);
}

#[test]
fn graph_options_and_older_history_update_state() {
    let mut state = GraphState::new();
    assert_eq!(state.max_count(), 500);
    assert_eq!(state.load_older_history(), 1000);
    state.set_include_remotes(true);
    assert!(state.includes_remotes());
    state.begin_load(state.max_count(), state.includes_remotes());
    assert!(state.is_loading());
}

#[test]
fn graph_and_ref_columns_share_one_scroll_offset() {
    let mut state = GraphState::new();
    let mut snapshot = snapshot();
    snapshot.lines = (0..6)
        .map(|index| GraphLine {
            graph: "*".into(),
            commit_index: Some(index % 2),
        })
        .collect();
    state.apply_result(Ok(snapshot));

    for _ in 0..4 {
        state.move_down();
    }
    state.ensure_visible(3);

    assert_eq!(state.commit_offset(), 2);
}

#[test]
fn graph_horizontal_scroll_is_independent_and_resets_on_result() {
    let mut state = GraphState::new();
    state.apply_result(Ok(snapshot()));
    state.move_down();
    state.scroll_right();
    state.scroll_right();

    assert_eq!(state.commit_cursor(), 1);
    assert_eq!(state.horizontal_offset(), 2);

    state.scroll_left();
    state.scroll_left();
    state.scroll_left();
    assert_eq!(state.horizontal_offset(), 0);

    state.scroll_right();
    state.apply_result(Ok(snapshot()));
    assert_eq!(state.horizontal_offset(), 0);

    state.scroll_right();
    state.apply_result(Err(GraphLoadError::Both {
        gleisbau: "failed".into(),
        fallback: "failed".into(),
    }));
    assert_eq!(state.horizontal_offset(), 0);
}

#[test]
fn graph_reload_retains_selected_commit_when_it_is_still_present() {
    let mut state = GraphState::new();
    state.apply_result(Ok(snapshot()));
    state.move_down();
    assert_eq!(state.selected_commit().unwrap().summary, "second");

    state.begin_load(500, false);
    state.apply_result(Ok(snapshot()));

    assert_eq!(state.commit_cursor(), 1);
    assert_eq!(state.selected_commit().unwrap().summary, "second");
}

#[test]
fn graph_focus_target_selects_refs_and_abbreviated_oids() {
    let mut state = GraphState::new();
    state.apply_result(Ok(snapshot()));

    assert!(state.focus_target("main"));
    assert_eq!(state.commit_cursor(), 0);
    assert!(state.focus_target("2222222"));
    assert_eq!(state.commit_cursor(), 1);
}

#[test]
fn graph_focus_target_waits_for_a_later_snapshot_when_loading() {
    let mut state = GraphState::new();
    state.begin_load(500, false);
    assert!(!state.focus_target("feature/new"));

    let mut loaded = snapshot();
    loaded.commits[1].refs.push(GraphRef {
        name: "feature/new".into(),
        kind: GraphRefKind::LocalBranch,
        has_linked_worktree: false,
        is_current: false,
        tracking: None,
        ahead_of_base: None,
    });
    state.apply_result(Ok(loaded));

    assert_eq!(state.commit_cursor(), 1);
}

#[test]
fn graph_apply_ref_delta_removes_single_ref_cleanly() {
    let mut state = GraphState::new();
    let mut snap = snapshot();
    // Give the first commit a "feature" ref so we can drop it.
    snap.commits[0].refs.push(GraphRef {
        name: "feature/x".into(),
        kind: GraphRefKind::LocalBranch,
        has_linked_worktree: false,
        is_current: false,
        tracking: None,
        ahead_of_base: None,
    });
    snap.ref_counts.local = 2;
    snap.generation = Some(1);
    state.apply_result(Ok(snap));

    let changed = state.apply_ref_delta(vec!["feature/x".to_string()]);

    assert!(changed, "apply_ref_delta should report a change");

    let snap_after = state.snapshot().unwrap();
    let commit = &snap_after.commits[0];
    assert!(
        !commit.refs.iter().any(|r| r.name == "feature/x"),
        "feature/x should be removed"
    );
    assert_eq!(snap_after.ref_counts.local, 1, "local ref count should decrement");
}

#[test]
fn graph_apply_ref_delta_leaves_generation_ownership_to_app() {
    let mut state = GraphState::new();
    let mut snap = snapshot();
    snap.commits[0].refs.push(GraphRef {
        name: "feature/y".into(),
        kind: GraphRefKind::LocalBranch,
        has_linked_worktree: false,
        is_current: false,
        tracking: None,
        ahead_of_base: None,
    });
    snap.generation = Some(1);
    state.apply_result(Ok(snap));

    let before_gen = state
        .snapshot()
        .unwrap()
        .generation
        .expect("snapshot should have a generation after apply_result");

    state.apply_ref_delta(vec!["feature/y".to_string()]);

    let after_gen = state.snapshot().unwrap().generation.unwrap();
    assert_eq!(after_gen, before_gen, "GraphState must not own App revisions");
}

#[test]
fn graph_apply_ref_delta_returns_false_when_no_snapshot() {
    let mut state = GraphState::new();
    let changed =
        state.apply_ref_delta(vec!["anything".to_string()]);
    assert!(!changed, "no snapshot loaded → no change");
}

#[test]
fn graph_apply_ref_delta_returns_false_when_ref_missing() {
    let mut state = GraphState::new();
    state.apply_result(Ok(snapshot()));

    let changed =
        state.apply_ref_delta(vec!["does/not/exist".to_string()]);
    assert!(!changed, "missing ref → no change");
}
