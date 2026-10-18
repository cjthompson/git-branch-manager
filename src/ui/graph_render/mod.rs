mod lanes;
mod ref_pane;
mod row;
#[cfg(test)]
mod tests;

use std::collections::HashMap;

use ratatui::prelude::*;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::git::graph::GraphSource;
use crate::symbols::SymbolSet;
use crate::theme::Theme;
use crate::view::graph::GraphState;
use crate::view::ViewId;

use self::lanes::{
    graph_contains_merge_connector, graph_spans, merge_origin_lane, selected_style, short_oid,
};
use self::ref_pane::{commit_summary_style, ref_pane_parts};
use self::row::{
    compose_row, compose_scrolled_row, graph_separator, max_ref_scroll_offset, spans_width,
};
use super::modal::{draw_modal_shell, ModalActionRow, ModalFooter, ModalScroll, ModalSpec};
use super::tab_bar::tab_bar_line;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Render the graph and refs as one synchronized row stream. The right-hand
/// ref column has a stable width and is separated from the graph by one
/// vertical rule, so long commit summaries cannot push refs out of view.
pub fn render_graph_view(
    frame: &mut Frame,
    area: Rect,
    state: &mut GraphState,
    theme: &Theme,
    symbols: &SymbolSet,
) {
    let block = Block::default()
        .title(tab_bar_line(ViewId::Graph, theme))
        .title_top(Line::from(format!(" v{VERSION} ")).right_aligned())
        .borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    if state.is_loading() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled("Loading graph...", theme.dim))),
            inner,
        );
        return;
    }

    if let Some(error) = state.error() {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled("Graph unavailable", theme.error)),
                Line::from(Span::styled(error, theme.dim)),
                Line::from(Span::styled("Press r to retry", theme.secondary_text)),
            ]),
            inner,
        );
        return;
    }

    let Some(snapshot) = state.snapshot() else {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "Graph is waiting for its first load...",
                theme.dim,
            ))),
            inner,
        );
        return;
    };

    let fallback = match &snapshot.source {
        GraphSource::Gleisbau => None,
        GraphSource::GitCliFallback { cause } => Some(cause.as_str()),
    };
    let (banner_area, content_area) = if fallback.is_some() && inner.height > 1 {
        let [banner, content] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(inner);
        (Some(banner), content)
    } else {
        (None, inner)
    };

    // Render a one-line banner whenever the graph data was produced by the
    // git-CLI fallback path instead of gleisbau. `snapshot.source` carries the
    // underlying cause string (compile-time exclusion, runtime build error,
    // gleisbau disabled via config, etc.) — we surface it verbatim so the user
    // sees *why* they're on the degraded path without needing to scroll logs.
    // Suppressed on single-line viewports so the rows still get a chance to
    // render; on taller panes the banner takes the top row above the graph.
    if let (Some(banner_area), Some(cause)) = (banner_area, fallback) {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(" Git fallback: ", theme.error),
                Span::styled(cause, theme.dim),
            ])),
            banner_area,
        );
    }

    let [header_area, rows_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(content_area);
    let ref_width = ref_pane_width(content_area.width);
    let graph_width = content_area.width.saturating_sub(ref_width + 1);
    state.ensure_visible(rows_area.height as usize);
    render_graph_header(frame, header_area, graph_width, ref_width, theme, symbols);
    render_graph_rows(
        frame,
        rows_area,
        graph_width,
        ref_width,
        state,
        theme,
        symbols,
    );
}

fn ref_pane_width(content_width: u16) -> u16 {
    let available = content_width.saturating_sub(1);
    available.min((content_width / 3).max(3))
}

fn render_graph_header(
    frame: &mut Frame,
    area: Rect,
    graph_width: u16,
    ref_width: u16,
    theme: &Theme,
    symbols: &SymbolSet,
) {
    let separator = graph_separator(symbols);
    let label = match ref_width {
        0..=8 => "LRT".to_string(),
        9..=14 => format!("LRT{separator}State"),
        _ => format!("LRT {separator} State {separator} Refs"),
    };
    let line = compose_row(
        vec![Span::styled(" Commits", theme.primary_text)],
        vec![Span::styled(label, theme.primary_text)],
        graph_width,
        ref_width,
        theme,
        symbols,
    );
    frame.render_widget(Paragraph::new(line), area);
}

fn render_graph_rows(
    frame: &mut Frame,
    area: Rect,
    graph_width: u16,
    ref_width: u16,
    state: &mut GraphState,
    theme: &Theme,
    symbols: &SymbolSet,
) {
    let Some(snapshot) = state.snapshot() else {
        return;
    };

    let selected_line = state.selected_commit_line();
    let lanes_by_oid: HashMap<&str, Option<usize>> = snapshot
        .commits
        .iter()
        .map(|commit| (commit.oid.as_str(), commit.lane))
        .collect();
    let mut pending_merge_lane = None;
    let mut rows = Vec::new();
    for (line_index, graph_line) in snapshot.lines.iter().enumerate() {
        let selected = selected_line == Some(line_index);
        let commit = graph_line
            .commit_index
            .and_then(|commit_index| snapshot.commits.get(commit_index));
        let is_current_commit = commit.is_some_and(|commit| {
            commit
                .refs
                .iter()
                .any(|graph_ref| graph_ref.is_current)
        });
        let is_merge = commit.is_some_and(|commit| commit.parents.len() > 1)
            || graph_line
                .graph
                .chars()
                .any(|character| matches!(character, 'o' | '○'));
        let merge_origin_lane = merge_origin_lane(commit, &lanes_by_oid);
        let connector_lane = merge_origin_lane.or(pending_merge_lane);
        let mut dag = graph_spans(
            &graph_line.graph,
            is_merge,
            connector_lane,
            selected,
            theme,
            symbols,
        );
        if !is_merge && commit.is_some_and(|commit| commit.is_possible_squash_merge()) {
            if let Some(marker_index) = graph_line
                .graph
                .chars()
                .position(|character| matches!(character, '*' | '●' | 'o' | '○'))
            {
                dag[marker_index] = Span::styled(
                    symbols.graph_squash_commit,
                    selected_style(theme.squash_merged, selected, theme),
                );
            }
        }
        if !is_merge && commit.is_some_and(|commit| commit.is_cherry_picked_commit()) {
            if let Some(marker_index) = graph_line
                .graph
                .chars()
                .position(|character| matches!(character, '*' | '●' | 'o' | '○'))
            {
                dag[marker_index] = Span::styled(
                    symbols.graph_cherry_commit,
                    selected_style(theme.cherry_picked, selected, theme),
                );
            }
        }
        let mut detail = Vec::new();
        if let Some(commit) = commit {
            detail.push(Span::raw(" "));
            detail.push(Span::styled(
                short_oid(&commit.oid),
                selected_style(theme.squash_merged, selected, theme),
            ));
            if !commit.is_possible_squash_merge() && !commit.is_cherry_picked_commit() {
                if let Some(fuzzy) = commit.fuzzy_squash_match().as_ref() {
                    detail.push(Span::styled(
                        format!(" [fuzzy squash {}%]", fuzzy.similarity_percent),
                        selected_style(theme.secondary_text, selected, theme),
                    ));
                }
            }
            detail.push(Span::styled(
                format!(" {}", commit.summary),
                selected_style(commit_summary_style(commit, theme), selected, theme),
            ));
        }

        if merge_origin_lane.is_some() {
            pending_merge_lane = merge_origin_lane;
        } else if commit.is_none()
            && pending_merge_lane.is_some()
            && graph_contains_merge_connector(&graph_line.graph)
        {
            pending_merge_lane = None;
        }
        let (fixed_right, refs) = commit
            .map(|commit| ref_pane_parts(commit, ref_width, is_current_commit, selected, theme, symbols))
            .unwrap_or_default();
        rows.push((selected, is_current_commit, dag, detail, fixed_right, refs));
    }

    let max_offset = rows
        .iter()
        .map(|(_selected, _is_current_commit, dag, detail, fixed_right, refs)| {
            let detail_width = (graph_width as usize).saturating_sub(spans_width(dag));
            let refs_width = (ref_width as usize).saturating_sub(spans_width(fixed_right));
            spans_width(detail)
                .saturating_sub(detail_width)
                .max(max_ref_scroll_offset(refs, refs_width))
        })
        .max()
        .unwrap_or(0);
    state.clamp_horizontal_offset(max_offset);
    let horizontal_offset = state.horizontal_offset();
    let lines: Vec<Line<'static>> = rows
        .into_iter()
        .map(|(selected, is_current_commit, dag, detail, fixed_right, refs)| {
            compose_scrolled_row(
                selected,
                is_current_commit,
                dag,
                detail,
                fixed_right,
                refs,
                graph_width,
                ref_width,
                horizontal_offset,
                theme,
                symbols,
            )
        })
        .collect();

    frame.render_widget(
        Paragraph::new(lines).scroll((state.commit_offset() as u16, 0)),
        area,
    );
}

/// Draw the opt-in remote-ref/load-older controls for the Graph tab.
pub fn draw_graph_options(frame: &mut Frame, cursor: usize, include_remotes: bool, theme: &Theme) {
    let rows = [
        ModalActionRow::new(
            Some(' '),
            format!(
                "{} Include remote refs",
                if include_remotes { "[x]" } else { "[ ]" }
            ),
            "",
        ),
        ModalActionRow::new(None, "Load older history (+500 commits)", ""),
    ];
    let areas = draw_modal_shell(
        frame,
        &ModalSpec::new(
            "Graph options",
            ModalFooter::hints(&[
                ("j/k", "Navigate"),
                ("Space", "Toggle"),
                ("Enter", "Apply"),
                ("Esc", "Cancel"),
            ]),
            68,
            6,
        ),
        theme,
    );
    let mut scroll = ModalScroll::default();
    scroll.ensure_visible(cursor as u16, rows.len() as u16, areas.body.height);
    let lines = rows
        .iter()
        .enumerate()
        .map(|(index, row)| row.render(index == cursor, theme))
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines).scroll((scroll.offset, 0)), areas.body);
}
