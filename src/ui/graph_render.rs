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
    let ref_width = ref_column_width(content_area.width);
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

fn ref_column_width(width: u16) -> u16 {
    if width >= 60 {
        30.min(width.saturating_sub(20))
    } else {
        18.min(width.saturating_sub(12)).max(1)
    }
}

fn render_graph_header(
    frame: &mut Frame,
    area: Rect,
    graph_width: u16,
    ref_width: u16,
    theme: &Theme,
    symbols: &SymbolSet,
) {
    let line = compose_row(
        vec![Span::styled(" Commits", theme.primary_text)],
        vec![Span::styled(" Refs", theme.primary_text)],
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
    state: &GraphState,
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
    let local_names: HashSet<&str> = snapshot
        .commits
        .iter()
        .flat_map(|commit| commit.refs.iter())
        .filter(|reference| reference.kind == GraphRefKind::LocalBranch)
        .map(|reference| reference.name.as_str())
        .collect();
    let mut pending_merge_lane = None;
    let lines: Vec<Line<'static>> = snapshot
        .lines
        .iter()
        .enumerate()
        .map(|(line_index, graph_line)| {
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
            let mut left_spans = graph_spans(
                &graph_line.graph,
                is_merge,
                connector_lane,
                selected,
                theme,
                symbols,
            );
            if let Some(commit) = commit {
                left_spans.push(Span::raw(" "));
                left_spans.push(Span::styled(
                    short_oid(&commit.oid),
                    selected_style(theme.squash_merged, selected, theme),
                ));
                left_spans.push(Span::styled(
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
            let right_spans = commit
                .map(|commit| graph_ref_spans(commit, &local_names, selected, theme, symbols))
                .unwrap_or_default();
            compose_row(
                left_spans,
                right_spans,
                graph_width,
                ref_width,
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

fn graph_ref_spans(
    commit: &GraphCommit,
    local_names: &HashSet<&str>,
    selected: bool,
    theme: &Theme,
    symbols: &SymbolSet,
) -> Vec<Span<'static>> {
    let mut refs: Vec<&GraphRef> = commit
        .refs
        .iter()
        .filter(|reference| match reference.kind {
            GraphRefKind::RemoteBranch => reference
                .name
                .split_once('/')
                .map(|(_, short_name)| !local_names.contains(short_name))
                .unwrap_or(true),
            _ => true,
        })
        .collect();
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

    let mut spans = Vec::new();
    for (index, reference) in refs.into_iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw("  "));
        }
        let style = selected_style(ref_style(reference.kind, theme), selected, theme);
        let scope = match reference.kind {
            GraphRefKind::LocalBranch => "L",
            GraphRefKind::RemoteBranch => "R",
            GraphRefKind::Tag => "T",
        };
        let lane = commit
            .lane
            .map(|lane| lane.to_string())
            .unwrap_or_else(|| "-".to_string());
        spans.push(Span::styled(format!(" {scope} {lane} "), style));
        spans.extend(ref_tracking_spans(reference, selected, theme, symbols));
        spans.extend(ref_status_spans(reference, selected, theme, symbols));
        spans.push(Span::styled(reference.name.clone(), style));
    }
    if spans.is_empty() {
        if let Some(branch) = commit.branch.as_ref() {
            let scope = match branch.kind {
                GraphRefKind::LocalBranch => "L",
                GraphRefKind::RemoteBranch => "R",
                GraphRefKind::Tag => "T",
            };
            let style = if branch.target_oid == commit.oid {
                ref_style(branch.kind, theme)
            } else {
                theme.secondary_text
            };
            spans.push(Span::styled(
                format!(" {scope} - {}", branch.name),
                selected_style(style, selected, theme),
            ));
        }
    }
    spans
}

fn commit_summary_style(commit: &GraphCommit, theme: &Theme) -> Style {
    match &commit.branch {
        Some(branch) if branch.target_oid != commit.oid => theme.secondary_text,
        _ => theme.primary_text,
    }
}

fn ref_tracking_spans(
    reference: &GraphRef,
    selected: bool,
    theme: &Theme,
    symbols: &SymbolSet,
) -> Vec<Span<'static>> {
    let Some(tracking) = reference.tracking.as_ref() else {
        return Vec::new();
    };
    let (text, style) = if tracking.ahead > 0 && tracking.behind == 0 {
        (
            format!("{}{} ", symbols.arrow_up, tracking.ahead),
            theme.ahead,
        )
    } else if tracking.behind > 0 && tracking.ahead == 0 {
        (
            format!("{}{} ", symbols.arrow_down, tracking.behind),
            theme.behind,
        )
    } else if tracking.ahead == 0 && tracking.behind == 0 {
        (format!("{} ", symbols.status_in_sync), theme.in_sync)
    } else {
        (format!("{} ", symbols.tracking_link), theme.in_sync)
    };
    vec![Span::styled(text, selected_style(style, selected, theme))]
}

fn ref_status_spans(
    reference: &GraphRef,
    selected: bool,
    theme: &Theme,
    symbols: &SymbolSet,
) -> Vec<Span<'static>> {
    let Some(status) = reference.status.as_ref() else {
        return Vec::new();
    };
    if status.is_base {
        return Vec::new();
    }
    let (text, style) =
        crate::ui::cells::compact_merge_status_parts(&status.merge_status, theme, symbols);
    vec![Span::styled(
        format!("{text} "),
        selected_style(style, selected, theme),
    )]
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
                    status: None,
                    tracking: None,
                }],
            }],
            lines: vec![GraphLine {
                graph: "*".into(),
                commit_index: Some(0),
            }],
            ref_counts: Default::default(),
            max_count: 500,
            includes_remotes: false,
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
                    status: None,
                    tracking: None,
                }],
            }],
            lines: vec![GraphLine {
                graph: "*".into(),
                commit_index: Some(0),
            }],
            ref_counts: Default::default(),
            max_count: 500,
            includes_remotes: false,
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
    fn graph_ref_lane_comes_from_its_commit() {
        let commit = GraphCommit {
            oid: "1234567890abcdef".into(),
            summary: "visible commit".into(),
            parents: vec![],
            lane: Some(3),
            branch: None,
            refs: vec![GraphRef {
                name: "main".into(),
                kind: GraphRefKind::LocalBranch,
                status: None,
                tracking: None,
            }],
        };
        let local_names = HashSet::from(["main"]);

        let rendered: String = graph_ref_spans(
            &commit,
            &local_names,
            false,
            &Theme::dark(),
            &SymbolSet::ascii(),
        )
        .iter()
        .map(|span| span.content.as_ref())
        .collect();

        assert!(rendered.contains(" L 3 "), "rendered ref: {rendered}");
    }

    #[test]
    fn graph_refs_are_ordered_and_show_tracking_and_merge_status() {
        use crate::git::graph::{GraphRefStatus, GraphRefTracking};
        use crate::types::MergeStatus;

        let theme = Theme::dark();
        let symbols = SymbolSet::ascii();
        let main = GraphRef {
            name: "main".into(),
            kind: GraphRefKind::LocalBranch,
            status: Some(GraphRefStatus {
                merge_status: MergeStatus::InSync,
                ahead: Some(0),
                behind: Some(0),
                is_current: true,
                is_base: false,
            }),
            tracking: Some(GraphRefTracking {
                remote_name: "origin/main".into(),
                ahead: 0,
                behind: 0,
            }),
        };
        let feature = GraphRef {
            name: "feature".into(),
            kind: GraphRefKind::LocalBranch,
            status: Some(GraphRefStatus {
                merge_status: MergeStatus::Unmerged,
                ahead: Some(2),
                behind: Some(0),
                is_current: false,
                is_base: false,
            }),
            tracking: Some(GraphRefTracking {
                remote_name: "origin/feature".into(),
                ahead: 2,
                behind: 0,
            }),
        };
        let same_remote = GraphRef {
            name: "origin/main".into(),
            kind: GraphRefKind::RemoteBranch,
            status: None,
            tracking: None,
        };
        let feature_remote = GraphRef {
            name: "origin/feature".into(),
            kind: GraphRefKind::RemoteBranch,
            status: None,
            tracking: None,
        };
        let remote_only = GraphRef {
            name: "origin/release".into(),
            kind: GraphRefKind::RemoteBranch,
            status: None,
            tracking: None,
        };
        let tag = GraphRef {
            name: "v0.3.0".into(),
            kind: GraphRefKind::Tag,
            status: None,
            tracking: None,
        };
        let commit = GraphCommit {
            oid: "main-tip".into(),
            summary: "main".into(),
            parents: vec![],
            lane: Some(0),
            branch: None,
            refs: vec![tag, remote_only, feature_remote, same_remote, feature, main],
        };
        let local_names = HashSet::from(["main", "feature"]);

        let text: String = graph_ref_spans(&commit, &local_names, false, &theme, &symbols)
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        assert!(text.contains("+2"), "local branch ahead count: {text}");
        assert!(text.contains("-"), "merge status symbol: {text}");
        assert!(
            !text.contains("origin/main"),
            "matching remote hidden: {text}"
        );
        assert!(
            text.contains("= "),
            "sync marker is present in matching rows: {text}"
        );
        assert!(text.find("main").unwrap() < text.find("origin/release").unwrap());
        assert!(text.find("origin/release").unwrap() < text.find("v0.3.0").unwrap());
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
                },
                GraphCommit {
                    oid: "abcdef1234567890".into(),
                    summary: "lane one".into(),
                    parents: vec![],
                    lane: Some(1),
                    branch: None,
                    refs: vec![],
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
                },
                GraphCommit {
                    oid: "2222222222222222".into(),
                    summary: "merge".into(),
                    parents: vec!["1111111111111111".into(), "3333333333333333".into()],
                    lane: Some(0),
                    branch: None,
                    refs: vec![],
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
