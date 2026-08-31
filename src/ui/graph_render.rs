use std::collections::{HashMap, HashSet};

use ratatui::prelude::*;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::git::graph::{GraphCommit, GraphRef, GraphRefKind, GraphSource};
use crate::symbols::SymbolSet;
use crate::theme::Theme;
use crate::view::graph::GraphState;
use crate::view::ViewId;

use super::shared::centered_rect;
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
        if !is_merge && commit.is_some_and(|commit| commit.is_possible_squash_merge) {
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
        let mut detail = Vec::new();
        if let Some(commit) = commit {
            detail.push(Span::raw(" "));
            detail.push(Span::styled(
                short_oid(&commit.oid),
                selected_style(theme.squash_merged, selected, theme),
            ));
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
            .map(|commit| ref_pane_parts(commit, ref_width, selected, theme, symbols))
            .unwrap_or_default();
        rows.push((selected, dag, detail, fixed_right, refs));
    }

    let max_offset = rows
        .iter()
        .map(|(_selected, dag, detail, fixed_right, refs)| {
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
        .map(|(selected, dag, detail, fixed_right, refs)| {
            compose_scrolled_row(
                selected,
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

#[allow(clippy::too_many_arguments)]
fn compose_scrolled_row(
    selected: bool,
    dag: Vec<Span<'static>>,
    detail: Vec<Span<'static>>,
    fixed_right: Vec<Span<'static>>,
    refs: Vec<Span<'static>>,
    graph_width: u16,
    ref_width: u16,
    offset: usize,
    theme: &Theme,
    symbols: &SymbolSet,
) -> Line<'static> {
    let dag = truncate_spans(dag, graph_width as usize);
    let detail_width = (graph_width as usize).saturating_sub(spans_width(&dag));
    let refs_width = (ref_width as usize).saturating_sub(spans_width(&fixed_right));
    let detail = truncate_spans(skip_spans(detail, offset), detail_width);
    let refs = render_ref_names(refs, refs_width, offset, selected, theme);
    let mut spans = dag;
    spans.extend(detail);
    let used = spans_width(&spans);
    spans.push(Span::raw(
        " ".repeat((graph_width as usize).saturating_sub(used)),
    ));
    spans.push(Span::styled(graph_separator(symbols), theme.secondary_text));
    spans.extend(fixed_right);
    spans.extend(refs);
    Line::from(spans)
}

fn compose_row(
    left: Vec<Span<'static>>,
    right: Vec<Span<'static>>,
    graph_width: u16,
    ref_width: u16,
    theme: &Theme,
    symbols: &SymbolSet,
) -> Line<'static> {
    let left = truncate_spans(left, graph_width as usize);
    let right = truncate_spans(right, ref_width as usize);
    // This is the width Ratatui will use when it places the spans in the
    // buffer. Keeping the graph geometry at its source columns means the
    // normal buffer width is also the correct padding width.
    let left_used = spans_width(&left);
    let mut spans = left;
    spans.push(Span::raw(
        " ".repeat((graph_width as usize).saturating_sub(left_used)),
    ));
    spans.push(Span::styled(graph_separator(symbols), theme.secondary_text));
    spans.extend(right);
    Line::from(spans)
}

fn truncate_spans(spans: Vec<Span<'static>>, width: usize) -> Vec<Span<'static>> {
    let mut remaining = width;
    let mut truncated = Vec::new();
    for span in spans {
        if remaining == 0 {
            break;
        }
        let span_width = span.width();
        if span_width <= remaining {
            remaining -= span_width;
            truncated.push(span);
            continue;
        }
        let mut content = String::new();
        for character in span.content.chars() {
            let character_width = character_width(character);
            if character_width > remaining {
                break;
            }
            content.push(character);
            remaining -= character_width;
        }
        if !content.is_empty() {
            truncated.push(Span::styled(content, span.style));
        }
    }
    truncated
}

fn skip_spans(spans: Vec<Span<'static>>, mut offset: usize) -> Vec<Span<'static>> {
    let mut skipped = Vec::new();
    for span in spans {
        if offset == 0 {
            skipped.push(span);
            continue;
        }
        let span_width = span.width();
        if offset >= span_width {
            offset -= span_width;
            continue;
        }
        let mut content = String::new();
        let mut remaining = offset;
        for character in span.content.chars() {
            let width = character_width(character);
            if remaining >= width {
                remaining -= width;
            } else {
                content.push(character);
            }
        }
        if !content.is_empty() {
            skipped.push(Span::styled(content, span.style));
        }
        offset = 0;
    }
    skipped
}

fn spans_width(spans: &[Span<'static>]) -> usize {
    spans.iter().map(Span::width).sum()
}

fn character_width(character: char) -> usize {
    Span::raw(character.to_string()).width()
}

fn graph_separator(symbols: &SymbolSet) -> &'static str {
    if symbols.name == "ascii" {
        "|"
    } else {
        "│"
    }
}

fn render_ref_names(
    refs: Vec<Span<'static>>,
    width: usize,
    offset: usize,
    selected: bool,
    theme: &Theme,
) -> Vec<Span<'static>> {
    let mut rendered = Vec::new();
    let mut remaining = width;

    for (index, reference) in refs.into_iter().enumerate() {
        let separator_width = if index == 0 { 0 } else { 2 };
        if remaining < separator_width {
            break;
        }

        let available = remaining - separator_width;
        if available == 0 {
            break;
        }

        if separator_width > 0 {
            rendered.push(ref_pane_space(separator_width, selected, theme));
        }

        let reference_width = reference.width();
        if reference_width <= available {
            rendered.push(reference);
            remaining = available - reference_width;
            continue;
        }

        let skip = offset.min(reference_width - available);
        rendered.extend(truncate_spans(skip_spans(vec![reference], skip), available));
        break;
    }

    rendered
}

fn max_ref_scroll_offset(refs: &[Span<'static>], width: usize) -> usize {
    let mut remaining = width;

    for (index, reference) in refs.iter().enumerate() {
        let separator_width = if index == 0 { 0 } else { 2 };
        if remaining < separator_width {
            break;
        }

        let available = remaining - separator_width;
        if available == 0 {
            break;
        }

        let reference_width = reference.width();
        if reference_width > available {
            return reference_width - available;
        }

        remaining = available - reference_width;
    }

    0
}

#[cfg(test)]
fn ref_pane_spans(
    commit: &GraphCommit,
    ref_width: u16,
    selected: bool,
    theme: &Theme,
    symbols: &SymbolSet,
) -> Vec<Span<'static>> {
    let (fixed, refs) = ref_pane_parts(commit, ref_width, selected, theme, symbols);
    let mut spans = fixed;
    for (index, reference) in refs.into_iter().enumerate() {
        if index > 0 {
            spans.push(ref_pane_space(2, selected, theme));
        }
        spans.push(reference);
    }
    spans
}

fn ref_pane_parts(
    commit: &GraphCommit,
    ref_width: u16,
    selected: bool,
    theme: &Theme,
    symbols: &SymbolSet,
) -> (Vec<Span<'static>>, Vec<Span<'static>>) {
    if ref_width == 0 {
        return (Vec::new(), Vec::new());
    }

    let mut refs: Vec<&GraphRef> = commit.refs.iter().collect();
    refs.sort_by_key(|reference| {
        (
            match reference.kind {
                GraphRefKind::LocalBranch => 0,
                GraphRefKind::RemoteBranch => 1,
                GraphRefKind::Tag => 2,
            },
            reference.name.as_str(),
        )
    });

    let mut spans = vec![
        Span::styled(
            if refs.iter().any(|r| r.kind == GraphRefKind::LocalBranch) {
                symbols.current_branch
            } else {
                " "
            },
            selected_style(theme.primary_text, selected, theme),
        ),
        Span::styled(
            if refs.iter().any(|r| r.kind == GraphRefKind::RemoteBranch) {
                symbols.graph_remote_ref
            } else {
                " "
            },
            selected_style(theme.remote_title, selected, theme),
        ),
        Span::styled(
            if refs.iter().any(|r| r.kind == GraphRefKind::Tag) {
                symbols.graph_tag_ref
            } else {
                " "
            },
            selected_style(theme.squash_merged, selected, theme),
        ),
    ];

    if ref_width <= 8 {
        return (spans, Vec::new());
    }

    let state = ref_pane_state_spans(&refs, selected, theme, symbols);
    if ref_width <= 14 {
        spans.push(Span::styled(
            graph_separator(symbols),
            selected_style(theme.secondary_text, selected, theme),
        ));
        spans.extend(state);
        return (spans, Vec::new());
    }

    spans.push(ref_pane_space(1, selected, theme));
    spans.push(Span::styled(
        graph_separator(symbols),
        selected_style(theme.secondary_text, selected, theme),
    ));
    spans.push(ref_pane_space(1, selected, theme));
    spans.extend(padded_ref_pane_state_spans(state, selected, theme));
    spans.push(ref_pane_space(1, selected, theme));
    spans.push(Span::styled(
        graph_separator(symbols),
        selected_style(theme.secondary_text, selected, theme),
    ));
    spans.push(ref_pane_space(1, selected, theme));

    if refs.is_empty() {
        let branch = commit.branch.as_ref().map(|branch| {
            Span::styled(
                branch.name.clone(),
                selected_style(theme.dim, selected, theme),
            )
        });
        return (spans, branch.into_iter().collect());
    }

    let local_names: HashSet<&str> = refs
        .iter()
        .filter(|r| r.kind == GraphRefKind::LocalBranch)
        .map(|r| r.name.as_str())
        .collect();
    let mut ref_spans = Vec::new();
    for reference in refs {
        if reference.kind == GraphRefKind::RemoteBranch
            && reference
                .name
                .split_once('/')
                .is_some_and(|(_, short)| local_names.contains(short))
        {
            continue;
        }
        ref_spans.push(Span::styled(
            reference.name.clone(),
            selected_style(ref_style(reference.kind, theme), selected, theme),
        ));
    }
    (spans, ref_spans)
}

const STATE_WIDTH: usize = 5;

fn ref_pane_space(width: usize, selected: bool, theme: &Theme) -> Span<'static> {
    Span::styled(
        " ".repeat(width),
        selected_style(Style::default(), selected, theme),
    )
}

fn padded_ref_pane_state_spans(
    state: Vec<Span<'static>>,
    selected: bool,
    theme: &Theme,
) -> Vec<Span<'static>> {
    let mut state = truncate_spans(state, STATE_WIDTH);
    let used = spans_width(&state);
    if used < STATE_WIDTH {
        state.push(ref_pane_space(STATE_WIDTH - used, selected, theme));
    }
    state
}

fn commit_summary_style(commit: &GraphCommit, theme: &Theme) -> Style {
    match &commit.branch {
        Some(branch) if branch.target_oid != commit.oid => theme.secondary_text,
        _ => theme.primary_text,
    }
}

fn ref_pane_state_spans(
    refs: &[&GraphRef],
    selected: bool,
    theme: &Theme,
    symbols: &SymbolSet,
) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    if let Some(reference) = refs
        .iter()
        .filter(|r| r.kind == GraphRefKind::LocalBranch && r.tracking.is_some())
        .min_by_key(|r| r.name.as_str())
    {
        if let Some(tracking) = reference.tracking.as_ref() {
            let (text, style) = match (tracking.ahead, tracking.behind) {
                (0, 0) => (symbols.status_in_sync.to_string(), theme.in_sync),
                (ahead, 0) => (
                    format!(
                        "{}{}",
                        symbols.arrow_up,
                        if ahead < 10 {
                            ahead.to_string()
                        } else {
                            String::new()
                        }
                    ),
                    theme.ahead,
                ),
                (0, behind) => (
                    format!(
                        "{}{}",
                        symbols.arrow_down,
                        if behind < 10 {
                            behind.to_string()
                        } else {
                            String::new()
                        }
                    ),
                    theme.behind,
                ),
                _ => ("RB".to_string(), theme.unmerged),
            };
            spans.push(Span::styled(text, selected_style(style, selected, theme)));
        }
    }
    if refs
        .iter()
        .any(|r| r.kind == GraphRefKind::LocalBranch && r.has_linked_worktree)
    {
        spans.push(Span::styled(
            if spans.is_empty() { "WT" } else { " WT" },
            selected_style(theme.primary_text, selected, theme),
        ));
    }
    spans
}

fn ref_style(kind: GraphRefKind, theme: &Theme) -> Style {
    match kind {
        GraphRefKind::LocalBranch => theme.primary_text,
        GraphRefKind::RemoteBranch => theme.remote_title,
        GraphRefKind::Tag => theme.squash_merged,
    }
}

fn graph_spans(
    graph: &str,
    is_merge: bool,
    merge_connector_lane: Option<usize>,
    selected: bool,
    theme: &Theme,
    symbols: &SymbolSet,
) -> Vec<Span<'static>> {
    let mut spans = Vec::new();

    for (column, character) in graph.chars().enumerate() {
        let lane = if is_merge_connector(character) {
            merge_connector_lane.unwrap_or(column / 2)
        } else {
            column / 2
        };
        let style = selected_style(graph_lane_style(lane, theme), selected, theme);
        spans.push(Span::styled(
            graph_symbol(character, is_merge, symbols),
            style,
        ));
    }

    spans
}

fn merge_origin_lane(
    commit: Option<&GraphCommit>,
    lanes_by_oid: &HashMap<&str, Option<usize>>,
) -> Option<usize> {
    let commit = commit?;
    commit
        .parents
        .get(1)
        .and_then(|parent| lanes_by_oid.get(parent.as_str()).copied().flatten())
}

fn graph_contains_merge_connector(graph: &str) -> bool {
    graph.chars().any(is_merge_connector)
}

fn is_merge_connector(character: char) -> bool {
    matches!(
        character,
        '<' | '>'
            | '-'
            | '/'
            | '\\'
            | '+'
            | '─'
            | '━'
            | '═'
            | '┌'
            | '┐'
            | '└'
            | '┘'
            | '╭'
            | '╮'
            | '╰'
            | '╯'
            | '├'
            | '┤'
            | '┬'
            | '┴'
            | '┼'
            | '┣'
            | '┫'
            | '┳'
            | '┻'
            | '╋'
            | '╠'
            | '╣'
            | '╦'
            | '╩'
            | '╬'
    )
}

/// Use semantic colors from the active theme for graph lanes. The palette
/// cycles after four lanes so large histories remain colorful without adding a
/// second, graph-specific theme configuration surface.
fn graph_lane_style(lane: usize, theme: &Theme) -> Style {
    match lane % 4 {
        0 => theme.title,
        1 => theme.ahead,
        2 => theme.behind,
        _ => theme.remote_title,
    }
}

/// Translate commit-point glyphs emitted by Gleisbau/git into the active
/// symbol set while preserving the graph engine's topology connectors.
fn graph_symbol(character: char, is_merge: bool, symbols: &SymbolSet) -> String {
    match character {
        '*' | '●' | 'o' | '○' => {
            if is_merge {
                symbols.graph_merge.to_string()
            } else {
                symbols.graph_commit.to_string()
            }
        }
        '<' => symbols.graph_arrow_left.to_string(),
        '>' => symbols.graph_arrow_right.to_string(),
        '│' | '┃' | '║' if symbols.name == "ascii" => "|".to_string(),
        '─' | '━' | '═' if symbols.name == "ascii" => "-".to_string(),
        '┼' | '╋' | '╬' if symbols.name == "ascii" => "+".to_string(),
        '└' | '╰' | '┗' | '╚' | '┘' | '╯' | '┛' | '╝' if symbols.name == "ascii" => {
            "'".to_string()
        }
        '┌' | '╭' | '┏' | '╔' | '┐' | '╮' | '┓' | '╗' if symbols.name == "ascii" => {
            ".".to_string()
        }
        '┤' | '├' | '┫' | '┣' | '╣' | '╠' if symbols.name == "ascii" => "|".to_string(),
        '┴' | '┬' | '┻' | '┳' | '╩' | '╦' if symbols.name == "ascii" => "+".to_string(),
        _ => character.to_string(),
    }
}

fn selected_style(style: Style, selected: bool, theme: &Theme) -> Style {
    if selected {
        theme.cursor.patch(style)
    } else {
        style
    }
}

fn short_oid(oid: &str) -> String {
    oid.chars().take(7).collect()
}

/// Draw the opt-in remote-ref/load-older controls for the Graph tab.
pub fn draw_graph_options(frame: &mut Frame, cursor: usize, include_remotes: bool, theme: &Theme) {
    let width = 46.min(frame.area().width);
    let height = 8.min(frame.area().height);
    let rect = centered_rect(width, height, frame.area());
    let selected = theme.cursor;
    let rows = [
        format!(
            "{} Include remote refs",
            if include_remotes { "[x]" } else { "[ ]" }
        ),
        "Load older history (+500 commits)".to_string(),
    ];
    let lines = rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            if index == cursor {
                Line::from(Span::styled(format!(" > {row}"), selected))
            } else {
                Line::from(format!("   {row}"))
            }
        })
        .chain([
            Line::from(""),
            Line::from(Span::styled(
                " Space toggle  Enter apply  Esc cancel",
                theme.dim,
            )),
        ])
        .collect::<Vec<_>>();
    let block = Block::default()
        .title(" Graph options ")
        .title_style(theme.title)
        .borders(Borders::ALL);
    frame.render_widget(Clear, rect);
    frame.render_widget(Paragraph::new(lines).block(block), rect);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::graph::{GraphCommit, GraphLine, GraphRef, GraphSource};
    use crate::view::graph::GraphState;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

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
                    tracking: None,
                }],
                is_possible_squash_merge: false,
                fuzzy_squash_match: None,
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
                    tracking: None,
                }],
                is_possible_squash_merge: false,
                fuzzy_squash_match: None,
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
                    tracking: Some(crate::git::graph::GraphRefTracking {
                        ahead: 1,
                        behind: 0,
                    }),
                }],
                is_possible_squash_merge: false,
                fuzzy_squash_match: None,
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
                        tracking: None,
                    },
                    GraphRef {
                        name: "worktree-agent-68eec8ed8aa".into(),
                        kind: crate::git::graph::GraphRefKind::LocalBranch,
                        has_linked_worktree: false,
                        tracking: None,
                    },
                ],
                is_possible_squash_merge: false,
                fuzzy_squash_match: None,
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
                    tracking: Some(crate::git::graph::GraphRefTracking {
                        ahead: 9,
                        behind: 0,
                    }),
                },
                GraphRef {
                    name: "origin/main".into(),
                    kind: GraphRefKind::RemoteBranch,
                    has_linked_worktree: false,
                    tracking: None,
                },
                GraphRef {
                    name: "v1".into(),
                    kind: GraphRefKind::Tag,
                    has_linked_worktree: false,
                    tracking: None,
                },
            ],
            is_possible_squash_merge: false,
            fuzzy_squash_match: None,
        };
        let text: String = ref_pane_spans(&commit, 30, false, &Theme::dark(), &SymbolSet::ascii())
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        assert!(text.starts_with("*@#"));
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
            is_possible_squash_merge: false,
            fuzzy_squash_match: None,
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
                tracking: None,
            }],
            is_possible_squash_merge: false,
            fuzzy_squash_match: None,
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
                    is_possible_squash_merge: false,
                    fuzzy_squash_match: None,
                },
                GraphCommit {
                    oid: "abcdef1234567890".into(),
                    summary: "lane one".into(),
                    parents: vec![],
                    lane: Some(1),
                    branch: None,
                    refs: vec![],
                    is_possible_squash_merge: false,
                    fuzzy_squash_match: None,
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
                    is_possible_squash_merge: false,
                    fuzzy_squash_match: None,
                },
                GraphCommit {
                    oid: "2222222222222222".into(),
                    summary: "merge".into(),
                    parents: vec!["1111111111111111".into(), "3333333333333333".into()],
                    lane: Some(0),
                    branch: None,
                    refs: vec![],
                    is_possible_squash_merge: false,
                    fuzzy_squash_match: None,
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
                    is_possible_squash_merge: false,
                    fuzzy_squash_match: None,
                },
                GraphCommit {
                    oid: "2222222222222222".into(),
                    summary: "possible squash".into(),
                    parents: vec!["1111111111111111".into()],
                    lane: Some(0),
                    branch: None,
                    refs: vec![],
                    is_possible_squash_merge: true,
                    fuzzy_squash_match: None,
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
            is_possible_squash_merge: false,
            fuzzy_squash_match: None,
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
}
