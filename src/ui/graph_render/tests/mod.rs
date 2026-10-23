mod current_branch;
mod fuzzy;
mod head_state;
mod lanes;
mod ref_scroll;
mod ref_state;
mod squash_refs;

use super::lanes::{graph_spans, graph_symbol, merge_origin_lane, selected_style};
use super::ref_pane::{ref_pane_parts, ref_pane_spans};
use super::row::{compose_row, render_ref_names, spans_width};
use super::{draw_graph_options, render_graph_view};
use crate::git::graph::{GraphCommit, GraphLine, GraphRef, GraphRefKind, GraphSource};
use crate::symbols::SymbolSet;
use crate::theme::Theme;
use crate::view::graph::GraphState;
use ratatui::backend::TestBackend;
use ratatui::style::Color;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Terminal;
use std::collections::HashMap;

fn fallback_state() -> GraphState {
    let mut state = GraphState::new();
    state.apply_result(Ok(crate::git::graph::GraphSnapshot {
        source: GraphSource::GitCliFallback {
            cause: "shallow repository".into(),
        },
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
    state
}

fn render_commit_lines(
    commit: GraphCommit,
    width: u16,
    horizontal_offset: usize,
) -> Vec<String> {
    let mut state = GraphState::new();
    state.apply_result(Ok(crate::git::graph::GraphSnapshot {
        source: GraphSource::Gleisbau,
        commits: vec![commit],
        lines: vec![GraphLine {
            graph: "*".into(),
            commit_index: Some(0),
        }],
        ref_counts: Default::default(),
        max_count: 500,
        includes_remotes: false,
        generation: None,
    }));
    for _ in 0..horizontal_offset {
        state.scroll_right();
    }

    let mut terminal = Terminal::new(TestBackend::new(width, 8)).unwrap();
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
        .chunks(width as usize)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect())
        .collect()
}

fn fuzzy_commit(score: u8) -> GraphCommit {
    GraphCommit {
        relationships: vec![crate::git::graph::GraphRelationship {
            kind: crate::git::graph::RelationshipKind::SquashMerge,
            matching: crate::git::graph::RelationshipMatch::Fuzzy {
                similarity_percent: score,
            },
            destination_oid: "abcdef0123456789".into(),
            destination_refs: vec!["main".into()],
            source_oid: "3333333333333333333333333333333333333333".into(),
            source_refs: Vec::new(),
        }],
        oid: "abcdef0123456789".into(),
        summary: "fuzzy landing".into(),
        ..GraphCommit::default()
    }
}

#[test]
fn narrow_terminal_keeps_graph_renderable_and_shows_fallback_banner() {
    let backend = TestBackend::new(40, 12);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut state = fallback_state();
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
    let buffer: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(buffer.contains("Git fallback"));
    assert!(buffer.contains("visible"));
}

#[test]
fn graph_options_render_remote_toggle_and_history_control() {
    let backend = TestBackend::new(60, 12);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| draw_graph_options(frame, 0, true, &Theme::dark()))
        .unwrap();
    let buffer: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(buffer.contains("[x] Include remote refs"));
    assert!(buffer.contains("Load older history"));
}
