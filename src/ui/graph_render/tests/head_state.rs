use super::*;

#[test]
fn graph_current_branch_state_column_starts_with_HEAD_label() {
    let backend = TestBackend::new(80, 12);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut state = GraphState::new();
    state.apply_result(Ok(crate::git::graph::GraphSnapshot {
        source: GraphSource::Gleisbau,
        commits: vec![GraphCommit {
            oid: "1111111111111111".into(),
            summary: "feature commit".into(),
            parents: vec![],
            lane: Some(0),
            branch: None,
            refs: vec![GraphRef {
                name: "feature".into(),
                kind: GraphRefKind::LocalBranch,
                has_linked_worktree: false,
                is_current: true,
                tracking: Some(crate::git::graph::GraphRefTracking {
                    ahead: 3,
                    behind: 0,
                }),
            }],
            relationships: vec![],
            ..GraphCommit::default()
        }],
        lines: vec![GraphLine {
            graph: "*".into(),
            commit_index: Some(0),
        }],
        ref_counts: Default::default(),
        max_count: 500,
        includes_remotes: false,
        generation: None,
    }));
    terminal
        .draw(|frame| {
            render_graph_view(
                frame,
                frame.area(),
                &mut state,
                &Theme::dark(),
                &SymbolSet::unicode(),
            )
        })
        .unwrap();

    let theme = Theme::dark();
    let buffer = terminal.backend().buffer();
    let row: String = buffer
        .content()
        .chunks(80)
        .nth(2)
        .unwrap()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    let head_byte = row.find("HEAD").expect("HEAD label in State column");
    let head_index = row[..head_byte].chars().count();
    for (offset, expected) in "HEAD".chars().enumerate() {
        let cell = &buffer[((head_index + offset) as u16, 2)];
        assert_eq!(cell.symbol(), expected.to_string());
        assert_eq!(Some(cell.fg), theme.current_branch.fg);
    }
}

#[test]
fn graph_non_current_branch_state_column_does_not_render_HEAD_label() {
    let backend = TestBackend::new(80, 12);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut state = GraphState::new();
    state.apply_result(Ok(crate::git::graph::GraphSnapshot {
        source: GraphSource::Gleisbau,
        commits: vec![GraphCommit {
            oid: "1111111111111111".into(),
            summary: "feature commit".into(),
            parents: vec![],
            lane: Some(0),
            branch: None,
            refs: vec![GraphRef {
                name: "feature".into(),
                kind: GraphRefKind::LocalBranch,
                has_linked_worktree: false,
                is_current: false,
                tracking: Some(crate::git::graph::GraphRefTracking {
                    ahead: 3,
                    behind: 0,
                }),
            }],
            relationships: vec![],
            ..GraphCommit::default()
        }],
        lines: vec![GraphLine {
            graph: "*".into(),
            commit_index: Some(0),
        }],
        ref_counts: Default::default(),
        max_count: 500,
        includes_remotes: false,
        generation: None,
    }));
    terminal
        .draw(|frame| {
            render_graph_view(
                frame,
                frame.area(),
                &mut state,
                &Theme::dark(),
                &SymbolSet::unicode(),
            )
        })
        .unwrap();

    let buffer = terminal.backend().buffer();
    let row: String = buffer
        .content()
        .chunks(80)
        .nth(2)
        .unwrap()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(!row.contains("HEAD"), "got: {row}");
    assert!(row.contains("\u{2191}3"), "got: {row}");
}

#[test]
fn graph_detached_head_produces_no_tinted_rows() {
    let backend = TestBackend::new(80, 12);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut state = GraphState::new();
    state.apply_result(Ok(crate::git::graph::GraphSnapshot {
        source: GraphSource::Gleisbau,
        commits: vec![GraphCommit {
            oid: "1111111111111111".into(),
            summary: "main commit".into(),
            parents: vec![],
            lane: Some(0),
            branch: None,
            refs: vec![GraphRef {
                name: "main".into(),
                kind: GraphRefKind::LocalBranch,
                has_linked_worktree: false,
                is_current: false,
                tracking: None,
            }],
            relationships: vec![],
            ..GraphCommit::default()
        }],
        lines: vec![GraphLine {
            graph: "*".into(),
            commit_index: Some(0),
        }],
        ref_counts: Default::default(),
        max_count: 500,
        includes_remotes: false,
        generation: None,
    }));
    terminal
        .draw(|frame| {
            render_graph_view(
                frame,
                frame.area(),
                &mut state,
                &Theme::dark(),
                &SymbolSet::ascii(),
            )
        })
        .unwrap();

    let buffer = terminal.backend().buffer();
    let theme = Theme::dark();
    assert_ne!(Some(buffer[(1, 2)].bg), theme.checked_row.bg);
}

#[test]
fn graph_head_only_branch_row_is_tinted() {
    let backend = TestBackend::new(80, 12);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut state = GraphState::new();
    state.apply_result(Ok(crate::git::graph::GraphSnapshot {
        source: GraphSource::Gleisbau,
        commits: vec![
            GraphCommit {
                oid: "1111111111111111".into(),
                summary: "feature commit".into(),
                parents: vec![],
                lane: Some(0),
                branch: None,
                refs: vec![GraphRef {
                    name: "feature".into(),
                    kind: GraphRefKind::LocalBranch,
                    has_linked_worktree: false,
                    is_current: true,
                    tracking: None,
                }],
                relationships: vec![],
                ..GraphCommit::default()
            },
            GraphCommit {
                oid: "2222222222222222".into(),
                summary: "main commit".into(),
                parents: vec![],
                lane: Some(0),
                branch: None,
                refs: vec![GraphRef {
                    name: "main".into(),
                    kind: GraphRefKind::LocalBranch,
                    has_linked_worktree: false,
                    is_current: false,
                    tracking: None,
                }],
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
    terminal
        .draw(|frame| {
            render_graph_view(
                frame,
                frame.area(),
                &mut state,
                &Theme::dark(),
                &SymbolSet::ascii(),
            )
        })
        .unwrap();

    let buffer = terminal.backend().buffer();
    let theme = Theme::dark();
    assert_eq!(Some(buffer[(1, 2)].bg), theme.checked_row.bg);
}
