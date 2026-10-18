use std::collections::HashSet;

use ratatui::style::Style;
use ratatui::text::Span;

use crate::git::graph::{GraphCommit, GraphRef, GraphRefKind};
use crate::symbols::SymbolSet;
use crate::theme::Theme;

use super::lanes::selected_style;
use super::row::{graph_separator, spans_width, truncate_spans};

#[cfg(test)]
pub(super) fn ref_pane_spans(
    commit: &GraphCommit,
    ref_width: u16,
    selected: bool,
    theme: &Theme,
    symbols: &SymbolSet,
) -> Vec<Span<'static>> {
    let is_current = commit.refs.iter().any(|r| r.is_current);
    let (fixed, refs) = ref_pane_parts(commit, ref_width, is_current, selected, theme, symbols);
    let mut spans = fixed;
    for (index, reference) in refs.into_iter().enumerate() {
        if index > 0 {
            spans.push(ref_pane_space(2, selected, theme));
        }
        spans.push(reference);
    }
    spans
}

pub(super) fn ref_pane_parts(
    commit: &GraphCommit,
    ref_width: u16,
    is_current: bool,
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
                symbols.graph_local_ref
            } else {
                " "
            },
            selected_style(theme.merged, selected, theme),
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

    let state = ref_pane_state_spans(&refs, is_current, selected, theme, symbols);
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
    // The HEAD state value (STATE_WIDTH_HEAD) is 2 cells wider than the normal
    // STATE_WIDTH. Drop the inner padding space on each side of the state field
    // for that case so the total fixed-width section stays the same width as
    // every other row and the header, keeping the Refs column aligned.
    if !is_current {
        spans.push(ref_pane_space(1, selected, theme));
    }
    spans.extend(padded_ref_pane_state_spans(
        state,
        if is_current { STATE_WIDTH_HEAD } else { STATE_WIDTH },
        selected,
        theme,
    ));
    if !is_current {
        spans.push(ref_pane_space(1, selected, theme));
    }
    spans.push(Span::styled(
        graph_separator(symbols),
        selected_style(theme.secondary_text, selected, theme),
    ));
    spans.push(ref_pane_space(1, selected, theme));

    let squash_source_spans = possible_squash_source_spans(commit, selected, theme, symbols);
    if refs.is_empty() {
        let branch = commit.branch.as_ref().map(|branch| {
            Span::styled(
                branch.name.clone(),
                selected_style(theme.dim, selected, theme),
            )
        });
        let mut ref_spans: Vec<_> = branch.into_iter().collect();
        ref_spans.extend(squash_source_spans);
        return (spans, ref_spans);
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
    ref_spans.extend(squash_source_spans);
    (spans, ref_spans)
}

fn possible_squash_source_spans(
    commit: &GraphCommit,
    selected: bool,
    theme: &Theme,
    symbols: &SymbolSet,
) -> Vec<Span<'static>> {
    let Some(confidence) = commit.squash_match_confidence() else {
        return Vec::new();
    };
    let Some((first, rest)) = confidence.sources.split_first() else {
        return Vec::new();
    };
    let suffix = if rest.is_empty() {
        String::new()
    } else {
        format!(" +{}", rest.len())
    };
    let percent = if confidence.similarity_percent == 100 {
        String::new()
    } else {
        format!(" ~{}%", confidence.similarity_percent)
    };
    vec![Span::styled(
        format!("{} {first}{suffix}{percent}", symbols.status_squash_merged),
        selected_style(theme.squash_merged, selected, theme),
    )]
}

const STATE_WIDTH: usize = 5;
const STATE_WIDTH_HEAD: usize = 7;

pub(super) fn ref_pane_space(width: usize, selected: bool, theme: &Theme) -> Span<'static> {
    Span::styled(
        " ".repeat(width),
        selected_style(Style::default(), selected, theme),
    )
}

fn padded_ref_pane_state_spans(
    state: Vec<Span<'static>>,
    width: usize,
    selected: bool,
    theme: &Theme,
) -> Vec<Span<'static>> {
    let mut state = truncate_spans(state, width);
    let used = spans_width(&state);
    if used < width {
        state.push(ref_pane_space(width - used, selected, theme));
    }
    state
}

pub(super) fn commit_summary_style(commit: &GraphCommit, theme: &Theme) -> Style {
    match &commit.branch {
        Some(branch) if branch.target_oid != commit.oid => theme.secondary_text,
        _ => theme.primary_text,
    }
}

fn ref_pane_state_spans(
    refs: &[&GraphRef],
    is_current: bool,
    selected: bool,
    theme: &Theme,
    symbols: &SymbolSet,
) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    if is_current {
        spans.push(Span::styled(
            "HEAD",
            selected_style(theme.current_branch, selected, theme),
        ));
    }
    let mut content_started = false;

    if let Some(reference) = refs
        .iter()
        .filter(|r| r.kind == GraphRefKind::LocalBranch && r.tracking.is_some())
        .min_by_key(|r| r.name.as_str())
    {
        if let Some(tracking) = reference.tracking.as_ref() {
            let (text, style) = match (tracking.ahead, tracking.behind) {
                (0, 0) => (symbols.status_in_sync.to_string(), theme.in_sync),
                (_ahead, 0) if is_current => (symbols.arrow_up.to_string(), theme.ahead),
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
                (0, _behind) if is_current => (symbols.arrow_down.to_string(), theme.behind),
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
                _ if is_current => ("R".to_string(), theme.unmerged),
                _ => ("RB".to_string(), theme.unmerged),
            };
            let text = if is_current { format!(" {text}") } else { text };
            spans.push(Span::styled(text, selected_style(style, selected, theme)));
            content_started = true;
        }
    }
    if refs
        .iter()
        .any(|r| r.kind == GraphRefKind::LocalBranch && r.has_linked_worktree)
    {
        let text = if is_current {
            if content_started { "W" } else { " W" }
        } else if spans.is_empty() {
            "WT"
        } else {
            " WT"
        };
        spans.push(Span::styled(
            text,
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
