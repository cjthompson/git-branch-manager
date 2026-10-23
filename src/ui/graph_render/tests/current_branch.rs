use super::*;

#[test]
fn graph_current_branch_commit_row_is_tinted_with_checked_row_background() {
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
                    ahead_of_base: None,
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
                    ahead_of_base: None,
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
    assert_ne!(Some(buffer[(1, 3)].bg), theme.checked_row.bg);
}

#[test]
fn graph_current_branch_state_column_abbreviates_WT_and_drops_count() {
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
                has_linked_worktree: true,
                is_current: true,
                tracking: Some(crate::git::graph::GraphRefTracking {
                    ahead: 3,
                    behind: 0,
                }),
                ahead_of_base: None,
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
    assert!(row.contains("HEAD \u{2191}W"), "got: {row}");
    assert!(!row.contains("WT"));
    let head_byte = row.find("HEAD").unwrap();
    let head_chars = row[..head_byte].chars().count();
    let chars_after: String = row.chars().skip(head_chars).take(7).collect();
    assert!(!chars_after.contains('3'), "got: {chars_after}");
}

#[test]
fn graph_current_branch_state_column_padding_matches_non_current_rows() {
    let backend = TestBackend::new(80, 12);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut state = GraphState::new();
    state.apply_result(Ok(crate::git::graph::GraphSnapshot {
        source: GraphSource::Gleisbau,
        commits: vec![
            GraphCommit {
                oid: "1111111111111111".into(),
                summary: "current commit".into(),
                parents: vec![],
                lane: Some(0),
                branch: None,
                refs: vec![GraphRef {
                    name: "zzcur".into(),
                    kind: GraphRefKind::LocalBranch,
                    has_linked_worktree: false,
                    is_current: true,
                    tracking: Some(crate::git::graph::GraphRefTracking {
                        ahead: 3,
                        behind: 0,
                    }),
                    ahead_of_base: None,
                }],
                relationships: vec![],
                ..GraphCommit::default()
            },
            GraphCommit {
                oid: "2222222222222222".into(),
                summary: "other commit".into(),
                parents: vec![],
                lane: Some(0),
                branch: None,
                refs: vec![GraphRef {
                    name: "zzoth".into(),
                    kind: GraphRefKind::LocalBranch,
                    has_linked_worktree: false,
                    is_current: false,
                    tracking: Some(crate::git::graph::GraphRefTracking {
                        ahead: 3,
                        behind: 0,
                    }),
                    ahead_of_base: None,
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
    let rows: Vec<String> = (1..=3)
        .map(|n| {
            buffer
                .content()
                .chunks(80)
                .nth(n)
                .unwrap()
                .iter()
                .map(|cell| cell.symbol())
                .collect()
        })
        .collect();
    let header_row = &rows[0];
    let current_row = &rows[1];
    let other_row = &rows[2];

    let char_index = |haystack: &str, needle: &str, label: &str| {
        let byte_index = haystack
            .find(needle)
            .unwrap_or_else(|| panic!("{label} not found: {haystack:?}"));
        haystack[..byte_index].chars().count()
    };

    let refs_index = char_index(header_row, "Refs", "Refs header label");
    let current_ref_index = char_index(current_row, "zzcur", "current row ref name");
    let other_ref_index = char_index(other_row, "zzoth", "non-current row ref name");

    assert_eq!(
        current_ref_index, refs_index,
        "current-branch row ref column should align with header: {current_row}"
    );
    assert_eq!(
        current_ref_index, other_ref_index,
        "current-branch row ref column should match non-current row: {current_row} vs {other_row}"
    );
}
