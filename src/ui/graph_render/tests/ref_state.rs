use super::*;

#[test]
fn graph_ref_markers_are_width_safe() {
    let commit = GraphCommit {
        oid: "tip".into(),
        summary: "tip".into(),
        parents: vec![],
        lane: Some(0),
        branch: None,
        refs: vec![
            GraphRef {
                name: "main".into(),
                kind: GraphRefKind::LocalBranch,
                has_linked_worktree: true,
                is_current: false,
                tracking: Some(crate::git::graph::GraphRefTracking {
                    ahead: 9,
                    behind: 0,
                }),
            },
            GraphRef {
                name: "origin/main".into(),
                kind: GraphRefKind::RemoteBranch,
                has_linked_worktree: false,
                is_current: false,
                tracking: None,
            },
            GraphRef {
                name: "v1".into(),
                kind: GraphRefKind::Tag,
                has_linked_worktree: false,
                is_current: false,
                tracking: None,
            },
        ],
        relationships: vec![],
        ..GraphCommit::default()
    };
    let text: String =
        ref_pane_spans(&commit, 30, false, &Theme::dark(), &SymbolSet::ascii())
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
    assert!(text.starts_with("L@#"));
    assert!(text.contains("+9 WT"));
    assert!(!text.contains("origin/main"));
    assert!(!text.contains("Merged"));
}

#[test]
fn inferred_branch_name_starts_at_the_same_refs_column_as_a_live_ref() {
    let inferred = GraphCommit {
        oid: "inferred".into(),
        summary: "inferred".into(),
        parents: vec![],
        lane: Some(0),
        branch: Some(crate::git::graph::GraphBranchLabel {
            name: "main".into(),
            target_oid: "tip".into(),
            kind: GraphRefKind::LocalBranch,
        }),
        refs: vec![],
        relationships: vec![],
        ..GraphCommit::default()
    };
    let live = GraphCommit {
        oid: "live".into(),
        summary: "live".into(),
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
    };
    let theme = Theme::dark();
    let symbols = SymbolSet::ascii();
    let text = |commit: &GraphCommit| {
        ref_pane_spans(commit, 30, false, &theme, &symbols)
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>()
    };

    let inferred_start = text(&inferred).find("main").expect("inferred branch name");
    let live_start = text(&live).find("main").expect("live branch name");

    assert_eq!(
        inferred_start, live_start,
        "inferred branch labels must use the same Refs prefix as live refs"
    );
    assert_eq!(
        inferred_start, 14,
        "full-width Refs text starts after LRT and State"
    );
}

#[test]
fn ref_pane_markers_match_list_view_indicator_colors() {
    let theme = Theme::dracula();
    let symbols = SymbolSet::unicode();
    let reference = |name: &str, kind| GraphRef {
        name: name.into(),
        kind,
        has_linked_worktree: false,
        is_current: false,
        tracking: None,
    };
    let commit = GraphCommit {
        refs: vec![
            reference("feature", GraphRefKind::LocalBranch),
            reference("origin/feature", GraphRefKind::RemoteBranch),
        ],
        ..GraphCommit::default()
    };

    let (fixed, _) = ref_pane_parts(&commit, 8, false, false, &theme, &symbols);

    assert_eq!(fixed[0].content, symbols.graph_local_ref);
    assert_eq!(fixed[0].style, theme.merged);
    assert_eq!(fixed[1].content, symbols.graph_remote_ref);
    assert_eq!(fixed[1].style, theme.remote_title);
}
