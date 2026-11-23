use git_branch_manager::git::graph;
use git_branch_manager::symbols::SymbolSet;
use git_branch_manager::theme::Theme;
use git_branch_manager::ui::graph_render::render_graph_view;
use git_branch_manager::view::graph::GraphState;
use ratatui::backend::TestBackend;
use ratatui::Terminal;

use crate::common::*;

fn rendered_rows(snapshot: &graph::GraphSnapshot, arrows: bool) -> Vec<String> {
    let mut state = GraphState::new();
    state.set_show_squash_arrows(arrows);
    state.apply_result(Ok(snapshot.clone()));
    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
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
    terminal
        .backend()
        .buffer()
        .content()
        .chunks(120)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect())
        .collect()
}

fn row_containing<'a>(rows: &'a [String], short_oid: &str) -> &'a String {
    rows.iter()
        .find(|line| line.contains(short_oid))
        .unwrap_or_else(|| panic!("no rendered row contains {short_oid}"))
}

#[test]
fn squash_arrows_connect_paired_destination_to_exact_source_tip() {
    let (tmpdir, destination, exact, _high, _low) = paired_squash_fixture();
    let snapshot = graph::load_graph_with_squash_annotations(
        tmpdir.path(),
        graph::GraphLoadOptions {
            base_branch: Some("main".into()),
            ..tmpdir.graph_options()
        },
    )
    .expect("graph load should succeed");

    let with_arrows = rendered_rows(&snapshot, true);
    let without_arrows = rendered_rows(&snapshot, false);

    let landing = row_containing(&with_arrows, &destination[..7]);
    assert!(landing.contains("~<----."), "got: {landing}");
    assert!(
        !row_containing(&without_arrows, &destination[..7]).contains("<"),
        "arrows must not render while the option is off"
    );

    let tip_with = row_containing(&with_arrows, &exact[..7]);
    let tip_without = row_containing(&without_arrows, &exact[..7]);
    assert_eq!(
        tip_with, tip_without,
        "the source tip row must render identically with arrows on"
    );

    let arrow_rows = with_arrows
        .iter()
        .filter(|line| line.chars().take(10).any(|character| character == '<'))
        .count();
    assert_eq!(arrow_rows, 1, "the paired fixture draws exactly one arrow");
}
