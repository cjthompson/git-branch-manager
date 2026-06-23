//! Renderers for the Diagnostics overlay and its cache-audit report.

use ratatui::prelude::*;
use ratatui::style::Modifier;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::theme::Theme;
use crate::types::{CacheAudit, CategoryStat, DiagnosticAction};

use super::shared::centered_rect;

/// Renders the Diagnostics menu: one selectable row per [`DiagnosticAction`].
pub fn draw_diagnostics_menu(frame: &mut Frame, cursor: usize, theme: &Theme) {
    let area = frame.area();
    let actions = DiagnosticAction::ALL;
    let width = 48u16.min(area.width);
    let height = (actions.len() as u16 + 4).min(area.height); // +4 borders + instructions
    let rect = centered_rect(width, height, area);

    let block = Block::default()
        .title(" Diagnostics ")
        .title_style(theme.title)
        .borders(Borders::ALL);
    let inner = block.inner(rect);
    frame.render_widget(Clear, rect);
    frame.render_widget(block, rect);

    let mut lines: Vec<Line> = actions
        .iter()
        .enumerate()
        .map(|(i, action)| {
            let style = if i == cursor {
                theme.cursor
            } else {
                Style::default()
            };
            Line::from(Span::styled(format!("  {}", action.label()), style))
        })
        .collect();

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Enter run   Esc close",
        theme.dim,
    )));

    frame.render_widget(Paragraph::new(lines), inner);
}

/// Renders the cache-audit report: per-category breakdown, then any
/// discrepancies and orphan rows, with an adaptive footer. `scroll` is the
/// number of body lines scrolled past (the header and footer stay fixed).
pub fn draw_diagnostics_report(
    frame: &mut Frame,
    audit: &CacheAudit,
    scroll: usize,
    theme: &Theme,
) {
    let area = frame.area();
    let width = (area.width * 70 / 100).max(54).min(area.width);
    let height = (area.height * 70 / 100).max(10).min(area.height);
    let rect = centered_rect(width, height, area);

    let block = Block::default()
        .title(" Cache accuracy ")
        .title_style(theme.title)
        .borders(Borders::ALL);
    let inner = block.inner(rect);
    frame.render_widget(Clear, rect);
    frame.render_widget(block, rect);

    // Fixed header: one line per category breakdown.
    let header: Vec<Line> = vec![
        category_line("Merge status", &audit.merge_status, theme),
        category_line("Ahead/behind", &audit.ahead_behind, theme),
        category_line("Merge base  ", &audit.merge_base, theme),
        Line::from(""),
    ];

    // Scrollable body: discrepancies, then orphans.
    let mut body: Vec<Line> = Vec::new();
    if !audit.discrepancies.is_empty() {
        body.push(Line::from(Span::styled(
            "Discrepancies",
            theme.title.add_modifier(Modifier::BOLD),
        )));
        for d in &audit.discrepancies {
            body.push(Line::from(vec![
                Span::styled(format!("  {:<28}", truncate(&d.branch, 28)), Style::default()),
                Span::styled(format!("{:<13}", d.kind.label()), theme.dim),
                Span::styled(d.cached.clone(), theme.error),
                Span::styled(" \u{2192} ", theme.dim),
                Span::styled(d.actual.clone(), theme.merged),
            ]));
        }
    }
    if !audit.orphans.is_empty() {
        if !body.is_empty() {
            body.push(Line::from(""));
        }
        body.push(Line::from(Span::styled(
            "Orphan entries (branch no longer exists)",
            theme.title.add_modifier(Modifier::BOLD),
        )));
        for orphan in &audit.orphans {
            body.push(Line::from(Span::styled(
                format!("  {}", truncate(orphan, 44)),
                theme.dim,
            )));
        }
    }

    // Footer adapts to whether anything needs fixing.
    let key_style = Style::default().fg(theme.accent_fg());
    let footer: Line = if audit.is_clean() {
        Line::from(vec![
            Span::styled(
                format!("\u{2713} Cache is accurate \u{2014} {} entries verified   ", audit.total_checked()),
                theme.merged,
            ),
            Span::styled("Esc", key_style),
            Span::styled(" close", theme.dim),
        ])
    } else {
        Line::from(vec![
            Span::styled("f", key_style),
            Span::styled(" fix & reload   ", theme.dim),
            Span::styled("Esc", key_style),
            Span::styled(" close", theme.dim),
        ])
    };

    // Compose: header + windowed body + blank + footer, fitting `inner.height`.
    let footer_block = 2u16; // blank separator + footer line
    let body_capacity = inner
        .height
        .saturating_sub(header.len() as u16)
        .saturating_sub(footer_block) as usize;

    let max_scroll = body.len().saturating_sub(body_capacity);
    let scroll = scroll.min(max_scroll);
    let visible_body = body
        .into_iter()
        .skip(scroll)
        .take(body_capacity)
        .collect::<Vec<_>>();

    let mut lines = header;
    lines.extend(visible_body);
    lines.push(Line::from(""));
    lines.push(footer);

    frame.render_widget(Paragraph::new(lines), inner);
}

fn category_line(label: &str, stat: &CategoryStat, theme: &Theme) -> Line<'static> {
    let mut spans = vec![
        Span::styled(format!("  {label}  "), Style::default()),
        Span::styled(format!("{} verified", stat.verified), theme.merged),
    ];
    if stat.mismatched > 0 {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            format!("{} mismatched", stat.mismatched),
            theme.error,
        ));
    }
    if stat.skipped > 0 {
        // Deduplicate reasons while preserving first-seen order.
        let mut seen = std::collections::HashSet::new();
        let unique_reasons: Vec<&str> = stat
            .skip_reasons
            .iter()
            .filter(|&&r| seen.insert(r))
            .copied()
            .collect();
        let reasons_str = unique_reasons.join("; ");
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            format!("{} skipped ({})", stat.skipped, reasons_str),
            theme.dim,
        ));
    }
    Line::from(spans)
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let kept: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{kept}\u{2026}")
    }
}
