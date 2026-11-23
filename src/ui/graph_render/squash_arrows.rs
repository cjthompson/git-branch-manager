use std::collections::{HashMap, HashSet};

use ratatui::text::Span;

use crate::git::graph::{GraphSnapshot, GraphSource, RelationshipKind, RelationshipMatch};
use crate::symbols::SymbolSet;
use crate::theme::Theme;

use super::lanes::{graph_lane_style, graph_symbol, selected_style};

/// One glyph of a squash arrow, placed by column on a graph row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ArrowCell {
    pub(super) col: usize,
    pub(super) glyph: String,
    pub(super) lane: usize,
}

struct Candidate {
    dest_line: usize,
    source_line: usize,
    dest_col: usize,
    source_col: usize,
    lane: usize,
}

/// Partial connectors would misread as topology, so each arrow is kept or dropped whole.
pub(super) fn plan_squash_arrows(
    snapshot: &GraphSnapshot,
    symbols: &SymbolSet,
) -> HashMap<usize, Vec<ArrowCell>> {
    let mut planned: HashMap<usize, Vec<ArrowCell>> = HashMap::new();
    if snapshot.source != GraphSource::Gleisbau {
        return planned;
    }

    let rows: Vec<Vec<char>> = snapshot
        .lines
        .iter()
        .map(|line| line.graph.chars().collect())
        .collect();
    let marker_col = |line_index: usize| {
        rows[line_index]
            .iter()
            .position(|character| matches!(character, '*' | '●' | 'o' | '○'))
    };
    let line_by_oid: HashMap<&str, usize> = snapshot
        .lines
        .iter()
        .enumerate()
        .filter_map(|(line_index, line)| {
            let commit = snapshot.commits.get(line.commit_index?)?;
            Some((commit.oid.as_str(), line_index))
        })
        .collect();

    let mut candidates = Vec::new();
    let mut seen = HashSet::new();
    for (line_index, line) in snapshot.lines.iter().enumerate() {
        let Some(commit) = line
            .commit_index
            .and_then(|index| snapshot.commits.get(index))
        else {
            continue;
        };
        let is_merge = commit.parents.len() > 1
            || rows[line_index]
                .iter()
                .any(|character| matches!(character, 'o' | '○'));
        if is_merge || !commit.is_possible_squash_merge() {
            continue;
        }
        let Some(dest_col) = marker_col(line_index) else {
            continue;
        };
        for relationship in &commit.relationships {
            if relationship.kind != RelationshipKind::SquashMerge
                || relationship.matching != RelationshipMatch::Exact
                || relationship.destination_oid != commit.oid
                || !seen.insert((commit.oid.as_str(), relationship.source_oid.as_str()))
            {
                continue;
            }
            let Some(&source_line) = line_by_oid.get(relationship.source_oid.as_str()) else {
                continue;
            };
            let Some(source_col) = marker_col(source_line) else {
                continue;
            };
            if source_line <= line_index || dest_col.abs_diff(source_col) < 2 {
                continue;
            }
            let lane = snapshot.lines[source_line]
                .commit_index
                .and_then(|index| snapshot.commits.get(index))
                .and_then(|source| source.lane)
                .unwrap_or(source_col / 2);
            candidates.push(Candidate {
                dest_line: line_index,
                source_line,
                dest_col,
                source_col,
                lane,
            });
        }
    }

    candidates.sort_by_key(|candidate| {
        (
            candidate.dest_col.abs_diff(candidate.source_col),
            candidate.dest_line,
            candidate.source_line,
        )
    });

    let mut claimed: HashSet<(usize, usize)> = HashSet::new();
    for candidate in &candidates {
        let cells = arrow_cells(candidate, symbols);
        let free = cells.iter().all(|(row, cell)| {
            !claimed.contains(&(*row, cell.col))
                && rows[*row]
                    .get(cell.col)
                    .is_none_or(|&character| character == ' ')
        });
        if !free {
            continue;
        }
        for (row, cell) in cells {
            claimed.insert((row, cell.col));
            planned.entry(row).or_default().push(cell);
        }
    }
    planned
}

/// The source row gets no cells, so its tip marker is never overdrawn.
fn arrow_cells(candidate: &Candidate, symbols: &SymbolSet) -> Vec<(usize, ArrowCell)> {
    let powerline = symbols.name == "powerline";
    let (dest, source) = (candidate.dest_col, candidate.source_col);
    let cell = |col: usize, character: char| ArrowCell {
        col,
        glyph: graph_symbol(character, false, symbols),
        lane: candidate.lane,
    };
    let dest_row = candidate.dest_line;
    let mut cells = Vec::new();
    if source > dest {
        cells.push((dest_row, cell(dest + 1, '<')));
        for col in dest + 2..source {
            cells.push((dest_row, cell(col, '─')));
        }
        let corner = if powerline { '╮' } else { '┐' };
        cells.push((dest_row, cell(source, corner)));
    } else {
        cells.push((dest_row, cell(dest - 1, '>')));
        for col in source + 1..dest - 1 {
            cells.push((dest_row, cell(col, '─')));
        }
        let corner = if powerline { '╭' } else { '┌' };
        cells.push((dest_row, cell(source, corner)));
    }
    for row in dest_row + 1..candidate.source_line {
        cells.push((row, cell(source, '│')));
    }
    cells
}

/// Pads the row when an arrow reaches past its existing graph width.
pub(super) fn apply_arrow_cells(
    dag: &mut Vec<Span<'static>>,
    cells: &[ArrowCell],
    selected: bool,
    theme: &Theme,
) {
    for cell in cells {
        if dag.len() <= cell.col {
            dag.resize_with(cell.col + 1, || Span::raw(" "));
        }
        let style = selected_style(graph_lane_style(cell.lane, theme), selected, theme);
        dag[cell.col] = Span::styled(cell.glyph.clone(), style);
    }
}
