use super::*;

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
