use std::collections::HashMap;

use ratatui::prelude::*;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::git::graph::{GraphCommit, GraphRefKind, GraphSource};
use crate::symbols::SymbolSet;
use crate::theme::Theme;
use crate::view::graph::{GraphPane, GraphRefScope, GraphState};
use crate::view::ViewId;

use super::shared::centered_rect;
use super::tab_bar::tab_bar_line;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Render the graph's commit pane and compact ref sidebar. The sidebar moves
/// below the graph on narrow terminals so the DAG never gets squeezed to zero
/// width.
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

    let [graph_area, sidebar_area] = split_graph_area(content_area);
    state.ensure_visible(graph_area.height as usize, sidebar_area.height as usize);
    render_commit_pane(frame, graph_area, state, theme, symbols);
    render_sidebar(frame, sidebar_area, state, theme, symbols);
}

fn split_graph_area(area: Rect) -> [Rect; 2] {
    if area.width >= 52 {
        let sidebar_width = 30.min(area.width.saturating_sub(20)).max(18);
        Layout::horizontal([Constraint::Min(1), Constraint::Length(sidebar_width)]).areas(area)
    } else {
        let sidebar_height = 6.min(area.height.saturating_sub(2)).max(1);
        Layout::vertical([Constraint::Min(1), Constraint::Length(sidebar_height)]).areas(area)
    }
}

fn render_commit_pane(
    frame: &mut Frame,
    area: Rect,
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
            let mut spans = graph_spans(
                &graph_line.graph,
                is_merge,
                connector_lane,
                selected,
                theme,
                symbols,
            );
            if let Some(commit) = commit {
                let text_style = if selected {
                    theme.cursor.patch(theme.primary_text)
                } else {
                    theme.primary_text
                };
                spans.push(Span::styled(
                    format!(" {} {}", short_oid(&commit.oid), commit.summary),
                    text_style,
                ));
                spans.extend(decoration_spans(commit, selected, theme));
            }

            if merge_origin_lane.is_some() {
                pending_merge_lane = merge_origin_lane;
            } else if commit.is_none()
                && pending_merge_lane.is_some()
                && graph_contains_merge_connector(&graph_line.graph)
            {
                pending_merge_lane = None;
            }
            Line::from(spans)
        })
        .collect();

    let block = Block::default()
        .title(" Commits ")
        .title_style(if state.focus() == GraphPane::Commits {
            theme.title
        } else {
            theme.secondary_text
        })
        .borders(Borders::RIGHT);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(
        Paragraph::new(lines).scroll((state.commit_offset() as u16, 0)),
        inner,
    );
}

fn graph_spans(
    graph: &str,
    is_merge: bool,
    merge_connector_lane: Option<usize>,
    selected: bool,
    theme: &Theme,
    symbols: &SymbolSet,
) -> Vec<Span<'static>> {
    let characters: Vec<char> = graph.chars().collect();
    let dropped_horizontal_column = (symbols.name == "powerline" && is_merge)
        .then(|| {
            characters
                .windows(3)
                .enumerate()
                .find_map(|(column, window)| {
                    (is_commit_marker(window[0])
                        && window[1] == '<'
                        && is_horizontal_connector(window[2]))
                    .then_some(column + 2)
                })
        })
        .flatten();
    let mut spans = Vec::with_capacity(characters.len() + 1);

    for (column, character) in characters.iter().copied().enumerate() {
        if dropped_horizontal_column == Some(column) {
            continue;
        }

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

        if symbols.name == "powerline"
            && is_merge
            && is_commit_marker(character)
            && characters.get(column + 1) == Some(&'<')
        {
            spans.push(Span::styled(" ", style));
        }
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

fn is_commit_marker(character: char) -> bool {
    matches!(character, '*' | '●' | 'o' | '○')
}

fn is_horizontal_connector(character: char) -> bool {
    matches!(character, '-' | '─' | '━' | '═')
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

fn render_sidebar(
    frame: &mut Frame,
    area: Rect,
    state: &GraphState,
    theme: &Theme,
    symbols: &SymbolSet,
) {
    let lines: Vec<Line<'static>> = (state.sidebar_offset()..state.sidebar_len())
        .map(|index| {
            let Some((reference, scope)) = state.sidebar_ref(index) else {
                return Line::from("");
            };
            let selected = state.sidebar_cursor() == index;
            let prefix = if selected { symbols.cursor_prefix } else { " " };
            let lane = reference
                .lane
                .map(|lane| format!("{lane:>2}"))
                .unwrap_or_else(|| "  ".to_string());
            let scope_marker = match scope {
                GraphRefScope::Local => "L",
                GraphRefScope::Remote => "R",
            };
            let style = if selected {
                theme.cursor.patch(match scope {
                    GraphRefScope::Local => theme.primary_text,
                    GraphRefScope::Remote => theme.remote_title,
                })
            } else {
                match scope {
                    GraphRefScope::Local => theme.primary_text,
                    GraphRefScope::Remote => theme.remote_title,
                }
            };
            Line::from(Span::styled(
                format!("{prefix} {scope_marker} {lane} {}", reference.name),
                style,
            ))
        })
        .collect();

    let block = Block::default()
        .title(" Refs ")
        .title_style(if state.focus() == GraphPane::Sidebar {
            theme.title
        } else {
            theme.secondary_text
        })
        .borders(Borders::LEFT);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(lines).scroll((0, 0)), inner);
}

fn short_oid(oid: &str) -> String {
    oid.chars().take(7).collect()
}

fn decoration_spans(
    commit: &crate::git::graph::GraphCommit,
    selected: bool,
    theme: &Theme,
) -> Vec<Span<'static>> {
    if commit.refs.is_empty() {
        return Vec::new();
    }
    let mut spans = vec![Span::styled(
        " (",
        selected_style(theme.secondary_text, selected, theme),
    )];
    for (index, reference) in commit.refs.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(
                ", ",
                selected_style(theme.secondary_text, selected, theme),
            ));
        }
        let prefix = match reference.kind {
            GraphRefKind::LocalBranch => "",
            GraphRefKind::RemoteBranch => "remote/",
            GraphRefKind::Tag => "tag/",
        };
        spans.push(Span::styled(
            format!("{prefix}{}", reference.name),
            decoration_style(reference.kind, selected, theme),
        ));
    }
    spans.push(Span::styled(
        ")",
        selected_style(theme.secondary_text, selected, theme),
    ));
    spans
}

fn decoration_style(kind: GraphRefKind, selected: bool, theme: &Theme) -> Style {
    let style = match kind {
        GraphRefKind::LocalBranch => theme.current_branch,
        GraphRefKind::RemoteBranch => theme.remote_title,
        GraphRefKind::Tag => theme.squash_merged,
    };
    selected_style(style, selected, theme)
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
    use crate::git::graph::{GraphCommit, GraphLine, GraphRef, GraphSidebar, GraphSource};
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
                refs: vec![GraphRef {
                    name: "main".into(),
                    kind: crate::git::graph::GraphRefKind::LocalBranch,
                }],
            }],
            lines: vec![GraphLine {
                graph: "*".into(),
                commit_index: Some(0),
            }],
            sidebar: GraphSidebar::default(),
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
        assert!(buffer.contains("visible commit"));
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
                    refs: vec![],
                },
                GraphCommit {
                    oid: "abcdef1234567890".into(),
                    summary: "lane one".into(),
                    parents: vec![],
                    lane: Some(1),
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
            sidebar: GraphSidebar::default(),
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
                    refs: vec![],
                },
                GraphCommit {
                    oid: "2222222222222222".into(),
                    summary: "merge".into(),
                    parents: vec!["1111111111111111".into(), "3333333333333333".into()],
                    lane: Some(0),
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
            sidebar: GraphSidebar::default(),
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
    fn powerline_right_merge_connector_is_spaced_and_width_preserving() {
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

        assert_eq!(rendered, "\u{f407} \u{25c0}───╮");
        assert_eq!(line.width(), 7);
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
