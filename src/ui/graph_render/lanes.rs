use std::collections::HashMap;

use ratatui::style::Style;
use ratatui::text::Span;

use crate::git::graph::GraphCommit;
use crate::symbols::SymbolSet;
use crate::theme::Theme;

pub(super) fn graph_spans(
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

pub(super) fn merge_origin_lane(
    commit: Option<&GraphCommit>,
    lanes_by_oid: &HashMap<&str, Option<usize>>,
) -> Option<usize> {
    let commit = commit?;
    commit
        .parents
        .get(1)
        .and_then(|parent| lanes_by_oid.get(parent.as_str()).copied().flatten())
}

pub(super) fn graph_contains_merge_connector(graph: &str) -> bool {
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
pub(super) fn graph_symbol(character: char, is_merge: bool, symbols: &SymbolSet) -> String {
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

pub(super) fn selected_style(style: Style, selected: bool, theme: &Theme) -> Style {
    if selected {
        theme.cursor.patch(style)
    } else {
        style
    }
}

pub(super) fn short_oid(oid: &str) -> String {
    oid.chars().take(7).collect()
}
