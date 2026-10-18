use super::ladder::FULLY_COMPACT_LEVEL;
use super::*;
use crate::symbols::SymbolSet;
use crate::theme::Theme;
use crate::types::{BranchInfo, MergeStatus, TrackingStatus};
use crate::view::branches::BranchesViewDef;
use crate::view::list_state::ListState;
use crate::view::ViewId;
use chrono::Utc;
use ratatui::backend::TestBackend;

fn col(
    key: &'static str,
    min_width: u16,
    wide_width: Option<u16>,
    is_stretchy: bool,
) -> LadderColumn {
    LadderColumn {
        key,
        min_width,
        wide_width,
        is_stretchy,
    }
}

// A Branches-shaped column set mirroring `view::branches::BranchesViewDef`:
// Up (compact unless a gone upstream needs four cells), Branch (stretchy),
// A/B, PR, Age, and Merge.
fn branches_like_columns() -> Vec<LadderColumn> {
    vec![
        col("remote", 2, None, false),          // Up indicator
        col("name", 15, None, true),            // Branch (stretchy)
        col("ahead_behind", 3, Some(8), false), // A/B
        col("pr", 2, Some(9), false),           // PR
        col("age", 5, Some(14), false),         // Age
        col("merge", 5, Some(16), false),       // Merge
    ]
}

#[test]
fn demoted_at_level_matches_bl_022_priority_order() {
    assert!(!demoted_at_level("age", 0));
    assert!(!demoted_at_level("merge", 0));
    assert!(!demoted_at_level("ahead_behind", 0));
    assert!(!demoted_at_level("pr", 0));

    assert!(demoted_at_level("age", 1));
    assert!(!demoted_at_level("merge", 1));
    assert!(!demoted_at_level("ahead_behind", 1));
    assert!(!demoted_at_level("pr", 1));

    assert!(demoted_at_level("age", 2));
    assert!(demoted_at_level("merge", 2));
    assert!(!demoted_at_level("ahead_behind", 2));
    assert!(!demoted_at_level("pr", 2));

    // A/B and PR are demoted together, only at the final level.
    assert!(demoted_at_level("age", 3));
    assert!(demoted_at_level("merge", 3));
    assert!(demoted_at_level("ahead_behind", 3));
    assert!(demoted_at_level("pr", 3));
    // FULLY_COMPACT_LEVEL also demotes columns with no named tier.
    assert!(demoted_at_level("remote", 3));
    assert!(demoted_at_level("status", 3));
}

#[test]
fn stretchy_column_not_demoted_until_final_level() {
    for level in 0..FULLY_COMPACT_LEVEL {
        assert!(!demoted_at_level("name", level));
    }
    assert!(demoted_at_level("name", FULLY_COMPACT_LEVEL));
}

#[test]
fn wide_terminal_keeps_everything_wide() {
    let columns = branches_like_columns();
    let level = resolve_ladder_level(&columns, 160, 160);
    assert_eq!(level, 0);
}

#[test]
fn narrow_width_only_age_compacts() {
    let columns = branches_like_columns();
    let level = resolve_ladder_level(&columns, 100, 100);
    assert_eq!(level, 1);
}

#[test]
fn header_right_aligns_when_label_fits() {
    assert_eq!(header_alignment(true, 5, 5), Alignment::Right);
    assert_eq!(header_alignment(true, 5, 16), Alignment::Right);
}

#[test]
fn header_falls_back_to_left_when_too_narrow_for_label() {
    // Regression: at resolved width 4, right-aligning "Merge" (len 5)
    // used to drop the leading "M", rendering "erge".
    assert_eq!(header_alignment(true, 5, 4), Alignment::Left);
    assert_eq!(header_alignment(true, 5, 0), Alignment::Left);
}

#[test]
fn header_never_right_aligns_when_not_wanted() {
    assert_eq!(header_alignment(false, 5, 16), Alignment::Left);
}

#[test]
fn narrower_width_age_and_merge_compact() {
    let columns = branches_like_columns();
    let level = resolve_ladder_level(&columns, 80, 80);
    assert_eq!(level, 2);
}

#[test]
fn very_narrow_width_compacts_age_merge_and_ab_pr() {
    let columns = branches_like_columns();
    let level = resolve_ladder_level(&columns, 100, 50);
    assert_eq!(level, FULLY_COMPACT_LEVEL);
}

fn scroll_test_branch(name: &str) -> BranchInfo {
    BranchInfo {
        name: name.to_string(),
        is_current: false,
        is_base: false,
        tracking: TrackingStatus::Local,
        ahead: None,
        behind: None,
        last_commit_date: Utc::now(),
        merge_status: MergeStatus::Unmerged,
        base_branch: "main".into(),
        merge_base_commit: None,
        pr: None,
        squash_confidence: None,
    }
}

fn scroll_test_columns() -> Vec<ColumnDef<BranchInfo>> {
    vec![
        ColumnDef {
            key: "name",
            name: "Name",
            show_header: true,
            min_width: 12,
            content_min_width: None,
            wide_width: None,
            hide_below_width: None,
            compare: Some(|a, b| a.name.cmp(&b.name)),
        },
        ColumnDef {
            key: "extra",
            name: "Extra",
            show_header: true,
            min_width: 10,
            content_min_width: None,
            wide_width: Some(12),
            hide_below_width: Some(80),
            compare: None,
        },
        ColumnDef {
            key: "late",
            name: "Late",
            show_header: true,
            min_width: 10,
            content_min_width: None,
            wide_width: Some(12),
            hide_below_width: Some(100),
            compare: Some(|a, b| a.name.cmp(&b.name)),
        },
    ]
}

fn scroll_test_row(
    _item: &BranchInfo,
    _raw_index: usize,
    _is_selected: bool,
    _is_cursor_row: bool,
    visible_columns: &[usize],
    _context: &CellContext,
) -> Vec<Line<'static>> {
    visible_columns
        .iter()
        .map(|index| Line::from(format!("value-{index}")))
        .collect()
}

fn scroll_test_output(terminal: &Terminal<TestBackend>) -> String {
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

#[test]
fn horizontal_scrolling_reveals_all_columns_and_preserves_row_styles() {
    let theme = Theme::dark();
    let symbols = SymbolSet::ascii();
    let columns = scroll_test_columns();
    let mut state = ListState::new(vec![
        scroll_test_branch("cursor"),
        scroll_test_branch("selected"),
    ]);
    state.selected_mut()[1] = true;
    let mut terminal = Terminal::new(TestBackend::new(48, 8)).unwrap();

    terminal
        .draw(|frame| {
            let mut params = ListRenderParams {
                state: &mut state,
                columns: &columns,
                active_view: ViewId::Branches,
                render_row: scroll_test_row,
                theme: &theme,
                symbols: &symbols,
                horizontal_scrolling: true,
            };
            let area = frame.area();
            render_list_view(frame, area, &mut params);
        })
        .unwrap();
    assert!(state.max_horizontal_offset() > 0);

    let max_offset = state.max_horizontal_offset();
    state.set_horizontal_scroll_bounds(max_offset);
    for _ in 0..max_offset {
        state.scroll_right();
    }
    terminal
        .draw(|frame| {
            let mut params = ListRenderParams {
                state: &mut state,
                columns: &columns,
                active_view: ViewId::Branches,
                render_row: scroll_test_row,
                theme: &theme,
                symbols: &symbols,
                horizontal_scrolling: true,
            };
            let area = frame.area();
            render_list_view(frame, area, &mut params);
        })
        .unwrap();

    let output = scroll_test_output(&terminal);
    assert!(
        output.contains("Late"),
        "right header was not revealed: {output}"
    );
    assert!(
        output.contains("value-2"),
        "right cell was not revealed: {output}"
    );
    assert!(
        output.contains('━'),
        "horizontal scrollbar is missing: {output}"
    );
    assert!(
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .any(|cell| theme.checked_row.bg.is_some_and(|bg| cell.bg == bg)),
        "scrolling should preserve the checked-row styling"
    );
}

#[test]
fn horizontal_scrolling_disabled_keeps_responsive_column_hiding() {
    let theme = Theme::dark();
    let symbols = SymbolSet::ascii();
    let columns = scroll_test_columns();
    let mut state = ListState::new(vec![scroll_test_branch("feature/x")]);
    let mut terminal = Terminal::new(TestBackend::new(48, 8)).unwrap();

    terminal
        .draw(|frame| {
            let mut params = ListRenderParams {
                state: &mut state,
                columns: &columns,
                active_view: ViewId::Branches,
                render_row: scroll_test_row,
                theme: &theme,
                symbols: &symbols,
                horizontal_scrolling: false,
            };
            let area = frame.area();
            render_list_view(frame, area, &mut params);
        })
        .unwrap();

    let output = scroll_test_output(&terminal);
    assert!(
        !output.contains("Extra"),
        "hidden column appeared: {output}"
    );
    assert!(!output.contains("Late"), "hidden column appeared: {output}");
    assert_eq!(state.max_horizontal_offset(), 0);
}

#[test]
fn flat_width_below_70_forces_fully_compact_even_with_room() {
    let columns = branches_like_columns();
    let level = resolve_ladder_level(&columns, 69, 500);
    assert_eq!(level, FULLY_COMPACT_LEVEL);
}

#[test]
fn age_is_hidden_only_after_the_final_compact_width_rung() {
    let columns = BranchesViewDef.columns();

    assert_eq!(visible_column_indices(&columns, 60), vec![0, 1, 2, 3, 4, 5]);
    assert_eq!(visible_column_indices(&columns, 59), vec![0, 1, 2, 3, 5]);
}
