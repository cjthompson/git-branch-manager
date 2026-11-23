use super::*;
use crate::git::graph::{GraphRelationship, GraphSnapshot, RelationshipKind, RelationshipMatch};
use ratatui::buffer::Buffer;

#[test]
fn ref_pane_shows_compact_possible_squash_sources() {
    let commit = GraphCommit {
        oid: "landing".into(),
        summary: "squash landing".into(),
        parents: vec![],
        lane: Some(0),
        branch: Some(crate::git::graph::GraphBranchLabel {
            name: "main".into(),
            target_oid: "landing".into(),
            kind: GraphRefKind::LocalBranch,
        }),
        refs: vec![],
        relationships: vec![crate::git::graph::GraphRelationship {
            kind: crate::git::graph::RelationshipKind::SquashMerge,
            matching: crate::git::graph::RelationshipMatch::Exact,
            destination_oid: "landing".into(),
            destination_refs: vec!["main".into()],
            source_oid: "1111111111111111111111111111111111111111".into(),
            source_refs: vec!["feature/auth".into(), "feature/login".into()],
        }],
        ..GraphCommit::default()
    };

    let text: String = ref_pane_spans(&commit, 30, false, &Theme::dark(), &SymbolSet::ascii())
        .iter()
        .map(|span| span.content.as_ref())
        .collect();

    assert!(text.contains("main  ~ feature/auth +1"), "got: {text}");
}

#[test]
fn ref_pane_shows_fuzzy_squash_source_with_percent() {
    let commit = GraphCommit {
        oid: "near-match".into(),
        summary: "near squash landing".into(),
        parents: vec![],
        lane: Some(0),
        branch: None,
        refs: vec![],
        relationships: vec![crate::git::graph::GraphRelationship {
            kind: crate::git::graph::RelationshipKind::SquashMerge,
            matching: crate::git::graph::RelationshipMatch::Fuzzy {
                similarity_percent: 97,
            },
            destination_oid: "near-match".into(),
            destination_refs: vec!["main".into()],
            source_oid: "3333333333333333333333333333333333333333".into(),
            source_refs: vec!["feature/login".into()],
        }],
        ..GraphCommit::default()
    };

    let text: String =
        ref_pane_spans(&commit, 30, false, &Theme::dark(), &SymbolSet::ascii())
            .iter()
            .map(|span| span.content.as_ref())
            .collect();

    assert!(text.contains("~ feature/login ~97%"), "got: {text}");
}

#[test]
fn graph_renders_possible_squash_marker_in_commit_lane_with_selection_style() {
    let backend = TestBackend::new(80, 12);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut state = GraphState::new();
    state.apply_result(Ok(crate::git::graph::GraphSnapshot {
        source: GraphSource::Gleisbau,
        commits: vec![
            GraphCommit {
                oid: "1111111111111111".into(),
                summary: "regular".into(),
                parents: vec!["0000000000000000".into()],
                lane: Some(0),
                branch: None,
                refs: vec![],
                relationships: vec![],
                ..GraphCommit::default()
            },
            GraphCommit {
                oid: "2222222222222222".into(),
                summary: "possible squash".into(),
                parents: vec!["1111111111111111".into()],
                lane: Some(0),
                branch: None,
                refs: vec![],
                relationships: vec![crate::git::graph::GraphRelationship {
                    kind: crate::git::graph::RelationshipKind::SquashMerge,
                    matching: crate::git::graph::RelationshipMatch::Exact,
                    destination_oid: "2222222222222222".into(),
                    destination_refs: vec!["main".into()],
                    source_oid: "1111111111111111111111111111111111111111".into(),
                    source_refs: Vec::new(),
                }],
                ..GraphCommit::default()
            },
        ],
        lines: vec![
            GraphLine {
                graph: "*".into(),
                commit_index: Some(0),
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
    }));
    state.move_down();
    let theme = Theme::dark();
    terminal
        .draw(|frame| {
            render_graph_view(frame, frame.area(), &mut state, &theme, &SymbolSet::ascii())
        })
        .unwrap();

    let buffer = terminal.backend().buffer();
    assert_eq!(buffer[(1, 2)].symbol(), "o");
    assert_eq!(buffer[(1, 3)].symbol(), "~");
    let expected_style = selected_style(theme.squash_merged, true, &theme);
    let marker_style = buffer[(1, 3)].style();
    assert_eq!(marker_style.fg, expected_style.fg);
    assert_eq!(marker_style.bg, expected_style.bg);
    assert_eq!(marker_style.add_modifier, expected_style.add_modifier);
    assert_eq!(buffer[(2, 2)].symbol(), " ");
    assert_eq!(buffer[(2, 3)].symbol(), " ");
}

fn snapshot_with_commits(commits: Vec<GraphCommit>) -> GraphSnapshot {
    let lines = (0..commits.len())
        .map(|index| GraphLine {
            graph: "*".into(),
            commit_index: Some(index),
        })
        .collect();
    GraphSnapshot {
        source: GraphSource::Gleisbau,
        commits,
        lines,
        ref_counts: Default::default(),
        max_count: 500,
        includes_remotes: false,
        generation: None,
    }
}

fn branch_tip(oid: &str, summary: &str, relationships: Vec<GraphRelationship>) -> GraphCommit {
    GraphCommit {
        oid: oid.into(),
        summary: summary.into(),
        parents: vec!["0000000000000000".into()],
        lane: Some(0),
        relationships,
        ..GraphCommit::default()
    }
}

/// A squash landing on `main` and its source tip, each carrying the pairing as enrichment attaches it.
fn squash_pair_snapshot(matching: RelationshipMatch) -> GraphSnapshot {
    let relationship = GraphRelationship {
        kind: RelationshipKind::SquashMerge,
        matching,
        destination_oid: "2222222222222222".into(),
        destination_refs: vec!["main".into()],
        source_oid: "1111111111111111".into(),
        source_refs: vec!["feature/auth".into()],
    };
    snapshot_with_commits(vec![
        branch_tip("2222222222222222", "squash landing", vec![relationship.clone()]),
        branch_tip("1111111111111111", "feature tip", vec![relationship]),
    ])
}

fn render_graph_buffer(snapshot: GraphSnapshot, move_to_second_line: bool) -> Buffer {
    let backend = TestBackend::new(80, 12);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut state = GraphState::new();
    state.apply_result(Ok(snapshot));
    if move_to_second_line {
        state.move_down();
    }
    let theme = Theme::dark();
    terminal
        .draw(|frame| {
            render_graph_view(frame, frame.area(), &mut state, &theme, &SymbolSet::ascii())
        })
        .unwrap();
    terminal.backend().buffer().clone()
}

#[test]
fn graph_marks_exact_squash_source_tip_in_commit_lane() {
    let buffer = render_graph_buffer(squash_pair_snapshot(RelationshipMatch::Exact), false);
    assert_eq!(buffer[(1, 3)].symbol(), "~");
}

#[test]
fn graph_marks_fuzzy_squash_source_tip_in_commit_lane() {
    let buffer = render_graph_buffer(
        squash_pair_snapshot(RelationshipMatch::Fuzzy {
            similarity_percent: 97,
        }),
        false,
    );
    assert_eq!(buffer[(1, 3)].symbol(), "~");
}

#[test]
fn graph_leaves_unmatched_branch_tip_unmarked() {
    let buffer = render_graph_buffer(
        snapshot_with_commits(vec![branch_tip("3333333333333333", "unmatched tip", vec![])]),
        false,
    );
    assert_eq!(buffer[(1, 2)].symbol(), "o");
}

#[test]
fn graph_squash_source_tip_uses_squash_style_with_selection() {
    let theme = Theme::dark();
    let buffer = render_graph_buffer(squash_pair_snapshot(RelationshipMatch::Exact), true);
    let expected_style = selected_style(theme.squash_merged, true, &theme);
    let marker_style = buffer[(1, 3)].style();
    assert_eq!(marker_style.fg, expected_style.fg);
    assert_eq!(marker_style.bg, expected_style.bg);
    assert_eq!(marker_style.add_modifier, expected_style.add_modifier);
}

#[test]
fn graph_cherry_marker_wins_over_squash_source_tip_marker() {
    let squash_rel = GraphRelationship {
        kind: RelationshipKind::SquashMerge,
        matching: RelationshipMatch::Exact,
        destination_oid: "2222222222222222".into(),
        destination_refs: vec!["main".into()],
        source_oid: "1111111111111111".into(),
        source_refs: vec!["feature/auth".into()],
    };
    let cherry_rel = GraphRelationship {
        kind: RelationshipKind::CherryPick,
        matching: RelationshipMatch::Exact,
        destination_oid: "4444444444444444".into(),
        destination_refs: vec!["release".into()],
        source_oid: "1111111111111111".into(),
        source_refs: vec!["feature/auth".into()],
    };
    let buffer = render_graph_buffer(
        snapshot_with_commits(vec![branch_tip(
            "1111111111111111",
            "feature tip",
            vec![squash_rel, cherry_rel],
        )]),
        false,
    );
    assert_eq!(buffer[(1, 2)].symbol(), "c");
}
