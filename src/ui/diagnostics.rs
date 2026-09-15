//! Renderers for the Diagnostics overlay and its cache-audit report.

use ratatui::prelude::*;
use ratatui::style::Modifier;
use ratatui::widgets::Paragraph;

use crate::theme::Theme;
use crate::types::{CacheAudit, CategoryStat, DiagnosticAction};

use super::modal::{draw_modal_shell, ModalActionRow, ModalFooter, ModalScroll, ModalSpec};

/// Renders the Diagnostics menu: one selectable row per [`DiagnosticAction`].
pub fn draw_diagnostics_menu(frame: &mut Frame, cursor: usize, theme: &Theme) {
    let actions = DiagnosticAction::ALL;
    let areas = draw_modal_shell(
        frame,
        &ModalSpec::new(
            "Diagnostics",
            ModalFooter::hints(&[("j/k", "Navigate"), ("Enter", "Run"), ("Esc", "Close")]),
            54,
            actions.len() as u16 + 3,
        ),
        theme,
    );
    let mut scroll = ModalScroll::default();
    scroll.ensure_visible(cursor as u16, actions.len() as u16, areas.body.height);

    let lines: Vec<Line> = actions
        .iter()
        .enumerate()
        .map(|(i, action)| ModalActionRow::new(None, action.label(), "").render(i == cursor, theme))
        .collect();

    frame.render_widget(Paragraph::new(lines).scroll((scroll.offset, 0)), areas.body);
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
    let footer = if audit.is_clean() {
        ModalFooter::hints(&[("Esc", "Close")])
    } else {
        ModalFooter::hints(&[("f", "Fix & reload"), ("j/k", "Scroll"), ("Esc", "Close")])
    };
    let areas = draw_modal_shell(
        frame,
        &ModalSpec::new("Cache accuracy", footer, 72, 22),
        theme,
    );

    // Category lines are part of the scroll stream; the shell owns fixed chrome.
    let mut body: Vec<Line> = vec![
        category_line("Merge status", &audit.merge_status, theme),
        category_line("Ahead/behind", &audit.ahead_behind, theme),
        category_line("Merge base  ", &audit.merge_base, theme),
        Line::from(""),
    ];

    if !audit.discrepancies.is_empty() {
        body.push(Line::from(Span::styled(
            "Discrepancies",
            theme.modal_title.add_modifier(Modifier::BOLD),
        )));
        for d in &audit.discrepancies {
            body.push(Line::from(vec![
                Span::styled(
                    format!("  {:<28}", truncate(&d.branch, 28)),
                    theme.modal_branch,
                ),
                Span::styled(format!("{:<13}", d.kind.label()), theme.modal_secondary),
                Span::styled(d.cached.clone(), theme.modal_failure),
                Span::styled(" \u{2192} ", theme.modal_secondary),
                Span::styled(d.actual.clone(), theme.modal_success),
            ]));
        }
    }
    if !audit.orphans.is_empty() {
        if !body.is_empty() {
            body.push(Line::from(""));
        }
        body.push(Line::from(Span::styled(
            "Orphan entries (branch no longer exists)",
            theme.modal_title.add_modifier(Modifier::BOLD),
        )));
        for orphan in &audit.orphans {
            body.push(Line::from(Span::styled(
                format!("  {}", truncate(orphan, 44)),
                theme.modal_secondary,
            )));
        }
    }

    if audit.is_clean() {
        body.push(Line::from(Span::styled(
            format!(
                "\u{2713} Cache is accurate \u{2014} {} entries verified",
                audit.total_checked()
            ),
            theme.modal_success,
        )));
    }

    let mut modal_scroll = ModalScroll::default();
    modal_scroll.offset = scroll.min(u16::MAX as usize) as u16;
    modal_scroll.clamp(body.len() as u16, areas.body.height);
    frame.render_widget(
        Paragraph::new(body).scroll((modal_scroll.offset, 0)),
        areas.body,
    );
}

fn category_line(label: &str, stat: &CategoryStat, theme: &Theme) -> Line<'static> {
    let mut spans = vec![
        Span::styled(format!("  {label}  "), theme.modal_secondary),
        Span::styled(format!("{} verified", stat.verified), theme.modal_success),
    ];
    if stat.mismatched > 0 {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            format!("{} mismatched", stat.mismatched),
            theme.modal_failure,
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
            theme.modal_secondary,
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
