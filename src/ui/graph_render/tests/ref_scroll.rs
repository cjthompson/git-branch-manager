use super::*;

#[test]
fn graph_places_refs_in_the_same_scrolling_row_as_the_commit() {
    let backend = TestBackend::new(80, 12);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut state = GraphState::new();
    state.apply_result(Ok(crate::git::graph::GraphSnapshot {
        source: GraphSource::Gleisbau,
        commits: vec![GraphCommit {
            oid: "1234567890abcdef".into(),
            summary: "visible commit".into(),
            parents: vec![],
            lane: Some(0),
            branch: None,
            refs: vec![GraphRef {
                name: "main".into(),
                kind: crate::git::graph::GraphRefKind::LocalBranch,
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

    let row: String = terminal
        .backend()
        .buffer()
        .content()
        .chunks(80)
        .nth(2)
        .unwrap()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    let separator = row.find('|').expect("graph/ref separator");
    let sha = row.find("1234567").expect("short commit hash");
    let ref_name = row[separator + 1..]
        .find("main")
        .map(|index| index + separator + 1)
        .expect("inline branch ref");

    assert!(sha < separator);
    assert!(ref_name > separator);
    assert!(!row.contains("(main)"));
}

#[test]
fn graph_horizontal_scroll_pins_dag_lrt_and_state_and_shifts_text_together() {
    let backend = TestBackend::new(100, 6);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut state = GraphState::new();
    state.apply_result(Ok(crate::git::graph::GraphSnapshot {
        source: GraphSource::Gleisbau,
        commits: vec![GraphCommit {
            oid: "1234567890abcdef".into(),
            summary: "long commit summary that continues beyond the visible commit region"
                .into(),
            parents: vec![],
            lane: Some(0),
            branch: None,
            refs: vec![GraphRef {
                name: "long-reference-name-that-continues".into(),
                kind: crate::git::graph::GraphRefKind::LocalBranch,
                has_linked_worktree: false,
                is_current: false,
                tracking: Some(crate::git::graph::GraphRefTracking {
                    ahead: 1,
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

    let render = |terminal: &mut Terminal<TestBackend>, state: &mut GraphState| {
        terminal
            .draw(|frame| {
                render_graph_view(
                    frame,
                    frame.area(),
                    state,
                    &Theme::dark(),
                    &SymbolSet::ascii(),
                )
            })
            .unwrap();
        terminal.backend().buffer().content()[200..300]
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    };
    let before = render(&mut terminal, &mut state);
    state.scroll_right();
    state.scroll_right();
    let after = render(&mut terminal, &mut state);

    assert_eq!(before.as_bytes()[0], after.as_bytes()[0]);
    assert_eq!(before.find('|'), after.find('|'));
    assert_eq!(before.find("+1"), after.find("+1"));
    assert_ne!(before, after);
    assert!(before.contains("long commit summary"));
    assert!(after.contains("ng commit summary"));
    assert!(before.contains("long-reference"));
    assert!(after.contains("ng-reference"));
}

#[test]
fn graph_horizontal_scroll_only_moves_truncated_refs() {
    let backend = TestBackend::new(80, 6);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut state = GraphState::new();
    state.apply_result(Ok(crate::git::graph::GraphSnapshot {
        source: GraphSource::Gleisbau,
        commits: vec![GraphCommit {
            oid: "1234567890abcdef".into(),
            summary: "visible commit".into(),
            parents: vec![],
            lane: Some(0),
            branch: None,
            refs: vec![
                GraphRef {
                    name: "main".into(),
                    kind: crate::git::graph::GraphRefKind::LocalBranch,
                    has_linked_worktree: false,
                    is_current: false,
                    tracking: None,
                },
                GraphRef {
                    name: "worktree-agent-68eec8ed8aa".into(),
                    kind: crate::git::graph::GraphRefKind::LocalBranch,
                    has_linked_worktree: false,
                    is_current: false,
                    tracking: None,
                },
            ],
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

    let render_row = |terminal: &mut Terminal<TestBackend>, state: &mut GraphState| {
        terminal
            .draw(|frame| {
                render_graph_view(
                    frame,
                    frame.area(),
                    state,
                    &Theme::dark(),
                    &SymbolSet::ascii(),
                )
            })
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .chunks(80)
            .nth(2)
            .unwrap()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    };

    let before = render_row(&mut terminal, &mut state);
    state.scroll_right();
    state.scroll_right();
    let after = render_row(&mut terminal, &mut state);

    let separator = before.find('|').expect("graph/ref separator");
    let refs_start = separator + 1 + 14;
    let before_refs = &before[refs_start..];
    let after_refs = &after[refs_start..];
    assert!(before_refs.starts_with("main"));
    assert!(after_refs.starts_with("main"));
    assert!(before_refs.contains("worktr"));
    assert!(after_refs.contains("rktree"));
    assert!(!after_refs.contains("worktr"));
}

#[test]
fn ref_pane_scroll_reaches_squash_source_after_long_real_refs() {
    let theme = Theme::dark();
    let refs = vec![
        Span::raw("main"),
        Span::raw("plan/p008-modal-system-redesign"),
        Span::styled("≈ plan/p007-likely-squash-merge-detection", theme.squash_merged),
    ];
    let source_offset = "plan/p008-modal-system-redesign  ".len();

    let text: String = render_ref_names(refs, 20, source_offset, false, &theme)
        .iter()
        .map(|span| span.content.as_ref())
        .collect();

    assert!(text.contains("main  ≈ plan/p007"), "got: {text}");
}
