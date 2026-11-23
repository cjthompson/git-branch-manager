use super::super::squash_arrows::plan_squash_arrows;
use super::*;
use crate::git::graph::{GraphRelationship, GraphSnapshot, RelationshipKind, RelationshipMatch};

const EXACT: RelationshipMatch = RelationshipMatch::Exact;

fn oid(name: &str) -> String {
    format!("{name:0<40}")
}

fn squash(destination: &str, source: &str, matching: RelationshipMatch) -> GraphRelationship {
    GraphRelationship {
        kind: RelationshipKind::SquashMerge,
        matching,
        destination_oid: destination.into(),
        destination_refs: vec!["main".into()],
        source_oid: source.into(),
        source_refs: vec!["feature/login".into()],
    }
}

fn commit(oid: &str, lane: usize, relationships: Vec<GraphRelationship>) -> GraphCommit {
    GraphCommit {
        oid: oid.into(),
        summary: format!("commit {}", &oid[..4]),
        parents: vec![],
        lane: Some(lane),
        relationships,
        ..GraphCommit::default()
    }
}

/// Builds a snapshot from `(graph, commit)` rows, one graph line per row.
fn snapshot(source: GraphSource, rows: Vec<(&str, Option<GraphCommit>)>) -> GraphSnapshot {
    let mut commits = Vec::new();
    let mut lines = Vec::new();
    for (graph, commit) in rows {
        let commit_index = commit.map(|commit| {
            commits.push(commit);
            commits.len() - 1
        });
        lines.push(GraphLine {
            graph: graph.into(),
            commit_index,
        });
    }
    GraphSnapshot {
        source,
        commits,
        lines,
        ref_counts: Default::default(),
        max_count: 500,
        includes_remotes: false,
        generation: None,
    }
}

/// Graph cells per line: x=1 skips the block border, y=2 skips border and header.
fn rendered_rows(
    snapshot: GraphSnapshot,
    arrows: bool,
    symbols: &SymbolSet,
    rows: u16,
    cols: u16,
) -> Vec<String> {
    let mut state = GraphState::new();
    state.set_show_squash_arrows(arrows);
    state.apply_result(Ok(snapshot));
    let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
    terminal
        .draw(|frame| render_graph_view(frame, frame.area(), &mut state, &Theme::dark(), symbols))
        .unwrap();
    let buffer = terminal.backend().buffer();
    (0..rows)
        .map(|line| {
            (0..cols)
                .map(|col| buffer[(1 + col, 2 + line)].symbol())
                .collect()
        })
        .collect()
}

/// Destination on lane 0, exact source tip on lane 1 two rows below, uniform width like gleisbau.
fn right_arrow_snapshot(source: GraphSource, matching: RelationshipMatch) -> GraphSnapshot {
    let dest = oid("dest");
    let tip = oid("tip");
    snapshot(
        source,
        vec![
            (
                "●  ",
                Some(commit(&dest, 0, vec![squash(&dest, &tip, matching)])),
            ),
            ("│  ", None),
            (
                "│ ●",
                Some(commit(&tip, 1, vec![squash(&dest, &tip, matching)])),
            ),
        ],
    )
}

#[test]
fn squash_arrows_draw_connector_from_destination_to_source_tip_on_the_right() {
    let ascii = SymbolSet::ascii();
    // Short graph strings: the arrow must pad the destination row past its end.
    let dest = oid("dest");
    let tip = oid("tip");
    let snapshot = snapshot(
        GraphSource::Gleisbau,
        vec![
            (
                "●",
                Some(commit(&dest, 0, vec![squash(&dest, &tip, EXACT)])),
            ),
            ("│", None),
            (
                "│ ●",
                Some(commit(&tip, 1, vec![squash(&dest, &tip, EXACT)])),
            ),
        ],
    );
    let mut planned_rows: Vec<usize> = plan_squash_arrows(&snapshot, &ascii)
        .keys()
        .copied()
        .collect();
    planned_rows.sort_unstable();
    assert_eq!(
        planned_rows,
        [0, 1],
        "one exact pairing plans the destination row and the vertical run, not the tip row"
    );

    let on = rendered_rows(snapshot.clone(), true, &ascii, 3, 3);
    let off = rendered_rows(snapshot, false, &ascii, 3, 3);

    assert_eq!(
        on[0], "~<.",
        "head next to the commit, corner over the tip lane"
    );
    assert_eq!(on[1], "| |", "vertical run in the source column");
    assert_eq!(on[2], "| ~");
    assert_eq!(
        on[2], off[2],
        "source tip row must render exactly as without arrows"
    );
}

#[test]
fn squash_arrows_leave_graph_untouched_when_option_is_off() {
    let ascii = SymbolSet::ascii();
    let rows = rendered_rows(
        right_arrow_snapshot(GraphSource::Gleisbau, EXACT),
        false,
        &ascii,
        3,
        3,
    );

    assert_eq!(rows, ["~  ", "|  ", "| ~"]);
}

#[test]
fn squash_arrows_skip_occupied_path_and_keep_destination_marker() {
    let dest = oid("dest");
    let tip = oid("tip");
    let snapshot = snapshot(
        GraphSource::Gleisbau,
        vec![
            (
                "●  ",
                Some(commit(&dest, 0, vec![squash(&dest, &tip, EXACT)])),
            ),
            ("│ │", None),
            (
                "│ ●",
                Some(commit(&tip, 1, vec![squash(&dest, &tip, EXACT)])),
            ),
        ],
    );

    let rows = rendered_rows(snapshot, true, &SymbolSet::ascii(), 3, 3);

    assert_eq!(
        rows[0], "~  ",
        "occupied column 2 on the path must drop the whole arrow"
    );
    assert_eq!(rows[1], "| |");
}

#[test]
fn squash_arrows_ignore_fuzzy_only_matches() {
    let snapshot = right_arrow_snapshot(
        GraphSource::Gleisbau,
        RelationshipMatch::Fuzzy {
            similarity_percent: 90,
        },
    );

    assert!(plan_squash_arrows(&snapshot, &SymbolSet::ascii()).is_empty());
    let rows = rendered_rows(snapshot, true, &SymbolSet::ascii(), 3, 3);
    assert_eq!(rows[0], "o  ");
}

#[test]
fn squash_arrows_skip_source_above_destination() {
    let dest = oid("dest");
    let tip = oid("tip");
    let snapshot = snapshot(
        GraphSource::Gleisbau,
        vec![
            (
                "│ ●",
                Some(commit(&tip, 1, vec![squash(&dest, &tip, EXACT)])),
            ),
            (
                "●  ",
                Some(commit(&dest, 0, vec![squash(&dest, &tip, EXACT)])),
            ),
        ],
    );

    let rows = rendered_rows(snapshot, true, &SymbolSet::ascii(), 2, 3);

    assert_eq!(rows, ["| ~", "~  "]);
}

#[test]
fn squash_arrows_skip_source_missing_from_loaded_lines() {
    let dest = oid("dest");
    let snapshot = snapshot(
        GraphSource::Gleisbau,
        vec![
            (
                "●  ",
                Some(commit(&dest, 0, vec![squash(&dest, &oid("gone"), EXACT)])),
            ),
            ("│  ", None),
            ("│ ●", Some(commit(&oid("other"), 1, vec![]))),
        ],
    );

    let rows = rendered_rows(snapshot, true, &SymbolSet::ascii(), 3, 3);

    assert_eq!(rows[0], "~  ");
}

#[test]
fn squash_arrows_skip_source_in_adjacent_column() {
    let dest = oid("dest");
    let tip = oid("tip");
    let snapshot = snapshot(
        GraphSource::Gleisbau,
        vec![
            (
                "●  ",
                Some(commit(&dest, 0, vec![squash(&dest, &tip, EXACT)])),
            ),
            (
                "│●",
                Some(commit(&tip, 0, vec![squash(&dest, &tip, EXACT)])),
            ),
        ],
    );

    let rows = rendered_rows(snapshot, true, &SymbolSet::ascii(), 2, 2);

    assert_eq!(rows, ["~ ", "|~"]);
}

#[test]
fn squash_arrows_use_round_corner_under_powerline_on_the_right() {
    let powerline = SymbolSet::powerline();
    let rows = rendered_rows(
        right_arrow_snapshot(GraphSource::Gleisbau, EXACT),
        true,
        &powerline,
        3,
        3,
    );

    assert_eq!(
        rows[0],
        format!(
            "{}{}╮",
            powerline.graph_squash_commit, powerline.graph_arrow_left
        )
    );
    assert_eq!(rows[1], "│ │");
}

#[test]
fn squash_arrows_use_round_corner_under_powerline_on_the_left() {
    let dest = oid("dest");
    let tip = oid("tip");
    let powerline = SymbolSet::powerline();
    let snapshot = snapshot(
        GraphSource::Gleisbau,
        vec![
            (
                "  ●",
                Some(commit(&dest, 1, vec![squash(&dest, &tip, EXACT)])),
            ),
            ("  │", None),
            (
                "●  ",
                Some(commit(&tip, 0, vec![squash(&dest, &tip, EXACT)])),
            ),
        ],
    );

    let rows = rendered_rows(snapshot, true, &powerline, 3, 3);

    assert_eq!(
        rows[0],
        format!(
            "╭{}{}",
            powerline.graph_arrow_right, powerline.graph_squash_commit
        )
    );
    assert_eq!(rows[1], "│ │");
}

#[test]
fn squash_arrows_skip_git_cli_fallback_snapshots() {
    let snapshot = right_arrow_snapshot(
        GraphSource::GitCliFallback {
            cause: "shallow repository".into(),
        },
        EXACT,
    );

    assert!(plan_squash_arrows(&snapshot, &SymbolSet::ascii()).is_empty());
}
