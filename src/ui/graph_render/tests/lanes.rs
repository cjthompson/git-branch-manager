use super::*;

#[test]
fn graph_lanes_use_distinct_foreground_colors() {
    let backend = TestBackend::new(80, 12);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut state = GraphState::new();
    state.apply_result(Ok(crate::git::graph::GraphSnapshot {
        source: GraphSource::Gleisbau,
        commits: vec![
            GraphCommit {
                oid: "1234567890abcdef".into(),
                summary: "lane zero".into(),
                parents: vec![],
                lane: Some(0),
                branch: None,
                refs: vec![],
                relationships: vec![],
                ..GraphCommit::default()
            },
            GraphCommit {
                oid: "abcdef1234567890".into(),
                summary: "lane one".into(),
                parents: vec![],
                lane: Some(1),
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
                graph: "  *".into(),
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
                &SymbolSet::ascii(),
            )
        })
        .unwrap();

    let buffer = terminal.backend().buffer();
    assert_eq!(buffer[(1, 2)].fg, Color::Cyan);
    assert_eq!(buffer[(3, 3)].fg, Color::Green);
    assert_ne!(buffer[(1, 2)].fg, buffer[(3, 3)].fg);
}

#[test]
fn graph_uses_circle_for_regular_commits_and_merge_symbol_for_merges() {
    let backend = TestBackend::new(80, 12);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut state = GraphState::new();
    state.apply_result(Ok(crate::git::graph::GraphSnapshot {
        source: GraphSource::Gleisbau,
        commits: vec![
            GraphCommit {
                oid: "1111111111111111".into(),
                summary: "regular".into(),
                parents: vec![],
                lane: Some(0),
                branch: None,
                refs: vec![],
                relationships: vec![],
                ..GraphCommit::default()
            },
            GraphCommit {
                oid: "2222222222222222".into(),
                summary: "merge".into(),
                parents: vec!["1111111111111111".into(), "3333333333333333".into()],
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
                &SymbolSet::powerline(),
            )
        })
        .unwrap();

    let buffer = terminal.backend().buffer();
    assert_eq!(buffer[(1, 2)].symbol(), "\u{25cf}");
    assert_eq!(buffer[(1, 3)].symbol(), "\u{f407}");
}

#[test]
fn powerline_merge_marker_is_one_cell_wide() {
    let marker = graph_symbol('*', true, &SymbolSet::powerline());

    assert_eq!(marker, "\u{f407}");
    assert_eq!(Span::raw(marker).width(), 1);
}

#[test]
fn powerline_touching_left_merge_preserves_connector_column() {
    let symbols = SymbolSet::powerline();
    let line = Line::from(graph_spans(
        "●<────╮",
        true,
        None,
        false,
        &Theme::dark(),
        &symbols,
    ));
    let rendered: String = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect();

    assert_eq!(rendered, "\u{f407}\u{25c0}────╮");
    assert_eq!(line.width(), 7);
}

#[test]
fn powerline_touching_left_merge_corner_stays_on_following_lane() {
    let symbols = SymbolSet::powerline();
    let merge = graph_spans("○<────┐", true, None, false, &Theme::dark(), &symbols);
    let following = graph_spans("●     │", false, None, false, &Theme::dark(), &symbols);
    let merge_corner = merge
        .iter()
        .position(|span| span.content.as_ref() == "┐")
        .expect("merge corner");
    let following_lane = following
        .iter()
        .position(|span| span.content.as_ref() == "│")
        .expect("following lane");

    assert_eq!(merge_corner, following_lane);
}

#[test]
fn powerline_touching_left_merge_aligns_row_width() {
    let symbols = SymbolSet::powerline();
    let line = compose_row(
        graph_spans("│●<────╮", true, None, false, &Theme::dark(), &symbols),
        Vec::new(),
        11,
        1,
        &Theme::dark(),
        &symbols,
    );
    let rendered: String = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect();

    assert_eq!(rendered, "│\u{f407}\u{25c0}────╮   │");
}

#[test]
fn powerline_right_merge_does_not_use_left_merge_width_adjustment() {
    let symbols = SymbolSet::powerline();
    let spans = graph_spans("●>────╮", true, None, false, &Theme::dark(), &symbols);
    let rendered: String = spans.iter().map(|span| span.content.as_ref()).collect();

    assert_eq!(rendered, "\u{f407}\u{25b6}────╮");
    assert_eq!(spans_width(&spans), Line::from(spans).width());
}

#[test]
fn powerline_touching_left_merge_keeps_commit_and_separator_aligned() {
    let symbols = SymbolSet::powerline();
    let theme = Theme::dark();
    let graph_width = 20;
    let ref_width = 1;
    let rows = [
        compose_row(
            {
                let mut spans = graph_spans("○<────┐", true, None, false, &theme, &symbols);
                spans.push(Span::raw(" "));
                spans.push(Span::raw("MERGE"));
                spans
            },
            Vec::new(),
            graph_width,
            ref_width,
            &theme,
            &symbols,
        ),
        compose_row(
            {
                let mut spans = graph_spans("●     │", false, None, false, &theme, &symbols);
                spans.push(Span::raw(" "));
                spans.push(Span::raw("NORMAL"));
                spans
            },
            Vec::new(),
            graph_width,
            ref_width,
            &theme,
            &symbols,
        ),
    ];
    let mut terminal = Terminal::new(TestBackend::new(40, 2)).unwrap();
    terminal
        .draw(|frame| frame.render_widget(Paragraph::new(rows.to_vec()), frame.area()))
        .unwrap();
    let rendered_rows: Vec<Vec<&str>> = terminal
        .backend()
        .buffer()
        .content()
        .chunks(40)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect())
        .collect();

    let merge_hash = rendered_rows[0]
        .iter()
        .position(|symbol| *symbol == "M")
        .expect("merge hash");
    let normal_hash = rendered_rows[1]
        .iter()
        .position(|symbol| *symbol == "N")
        .expect("normal hash");
    assert_eq!(merge_hash, normal_hash);
    assert_eq!(rendered_rows[0][graph_width as usize], "│");
    assert_eq!(rendered_rows[1][graph_width as usize], "│");
}

#[test]
fn graph_uses_active_theme_and_symbol_set() {
    let theme = Theme::light();
    let symbols = SymbolSet::unicode();
    let spans = graph_spans("●*", false, None, false, &theme, &symbols);

    assert_eq!(spans[0].content, symbols.graph_commit);
    assert_eq!(spans[0].style, theme.title);
    assert_eq!(spans[1].content, symbols.graph_commit);
    assert_eq!(spans[1].style, theme.title);
    assert_eq!(graph_symbol('○', true, &symbols), symbols.graph_merge);
    assert_eq!(graph_symbol('<', false, &symbols), "\u{25c0}");
    assert_eq!(graph_symbol('>', false, &symbols), "\u{25b6}");
    assert_eq!(graph_symbol('│', false, &SymbolSet::ascii()), "|");
    assert_eq!(graph_symbol('│', false, &symbols), "│");
}

#[test]
fn merge_connectors_keep_their_origin_lane_color() {
    let theme = Theme::dark();
    let symbols = SymbolSet::unicode();
    let merge = GraphCommit {
        oid: "merge".into(),
        summary: "merge".into(),
        parents: vec!["main-parent".into(), "feature-parent".into()],
        lane: Some(0),
        branch: None,
        refs: vec![],
        relationships: vec![],
        ..GraphCommit::default()
    };
    let lanes_by_oid = HashMap::from([("feature-parent", Some(3))]);
    let origin_lane = merge_origin_lane(Some(&merge), &lanes_by_oid);
    let spans = graph_spans("●<────╮", true, origin_lane, false, &theme, &symbols);

    assert_eq!(spans[0].style, theme.title);
    for span in &spans[1..] {
        assert_eq!(span.style, theme.remote_title);
    }

    let connector = graph_spans("╰─────┤", false, origin_lane, false, &theme, &symbols);
    for span in connector {
        assert_eq!(span.style, theme.remote_title);
    }
}
