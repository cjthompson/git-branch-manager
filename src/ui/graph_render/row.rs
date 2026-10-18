use ratatui::text::{Line, Span};

use crate::symbols::SymbolSet;
use crate::theme::Theme;

use super::ref_pane::ref_pane_space;

#[allow(clippy::too_many_arguments)]
pub(super) fn compose_scrolled_row(
    selected: bool,
    is_current_commit: bool,
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
    let line = Line::from(spans);
    if is_current_commit {
        line.style(theme.checked_row)
    } else {
        line
    }
}

pub(super) fn compose_row(
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

pub(super) fn truncate_spans(spans: Vec<Span<'static>>, width: usize) -> Vec<Span<'static>> {
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

pub(super) fn spans_width(spans: &[Span<'static>]) -> usize {
    spans.iter().map(Span::width).sum()
}

fn character_width(character: char) -> usize {
    Span::raw(character.to_string()).width()
}

pub(super) fn graph_separator(symbols: &SymbolSet) -> &'static str {
    if symbols.name == "ascii" {
        "|"
    } else {
        "│"
    }
}

pub(super) fn render_ref_names(
    refs: Vec<Span<'static>>,
    width: usize,
    offset: usize,
    selected: bool,
    theme: &Theme,
) -> Vec<Span<'static>> {
    let Some((first, remaining)) = refs.split_first() else {
        return Vec::new();
    };
    if first.width() >= width || remaining.is_empty() {
        return truncate_spans(skip_spans(vec![first.clone()], offset), width);
    }

    let mut rendered = vec![first.clone()];
    let remaining_width = width.saturating_sub(first.width() + 2);
    if remaining_width == 0 {
        return rendered;
    }
    rendered.push(ref_pane_space(2, selected, theme));
    rendered.extend(truncate_spans(
        skip_spans(joined_ref_spans(remaining.to_vec(), selected, theme), offset),
        remaining_width,
    ));
    rendered
}

pub(super) fn max_ref_scroll_offset(refs: &[Span<'static>], width: usize) -> usize {
    let Some((first, remaining)) = refs.split_first() else {
        return 0;
    };
    if first.width() >= width || remaining.is_empty() {
        return first.width().saturating_sub(width);
    }

    let remaining_width = width.saturating_sub(first.width() + 2);
    let separators_width = remaining.len().saturating_sub(1) * 2;
    spans_width(remaining)
        .saturating_add(separators_width)
        .saturating_sub(remaining_width)
}

fn joined_ref_spans(
    refs: Vec<Span<'static>>,
    selected: bool,
    theme: &Theme,
) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (index, reference) in refs.into_iter().enumerate() {
        if index > 0 {
            spans.push(ref_pane_space(2, selected, theme));
        }
        spans.push(reference);
    }
    spans
}
