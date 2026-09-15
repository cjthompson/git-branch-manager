use std::borrow::Cow;

use ratatui::{
    prelude::*,
    widgets::{Clear, Paragraph},
};

use crate::{
    theme::Theme,
    ui::shared::{block_panel, centered_rect, key_hint_str},
};

/// The static content and preferred geometry of a modal panel.
pub struct ModalSpec<'a> {
    pub title: Line<'a>,
    pub footer: ModalFooter<'a>,
    pub preferred_width: u16,
    pub max_height: u16,
}

impl<'a> ModalSpec<'a> {
    pub fn new(
        title: impl Into<Line<'a>>,
        footer: ModalFooter<'a>,
        preferred_width: u16,
        max_height: u16,
    ) -> Self {
        Self {
            title: title.into(),
            footer,
            preferred_width,
            max_height,
        }
    }
}

/// The areas occupied by a rendered modal shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModalAreas {
    pub outer: Rect,
    pub body: Rect,
    pub footer: Rect,
}

/// Scroll state for a modal body viewport.
///
/// External callers should construct this with [`Default`] and adjust it with
/// the provided methods rather than using a struct literal. Its representation
/// may gain further internal scrolling state.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ModalScroll {
    pub offset: u16,
    focused_row: Option<u16>,
    last_total_rows: Option<u16>,
    last_viewport_rows: Option<u16>,
}

impl ModalScroll {
    /// Limits the offset to the final viewport that can contain body rows.
    pub fn clamp(&mut self, total_rows: u16, viewport_rows: u16) {
        self.offset = self.offset.min(total_rows.saturating_sub(viewport_rows));
    }

    /// Adjusts the offset so the selected body row remains visible.
    pub fn ensure_visible(&mut self, selected_row: u16, total_rows: u16, viewport_rows: u16) {
        self.clamp(total_rows, viewport_rows);
        if total_rows == 0 || viewport_rows == 0 {
            return;
        }

        let selected_row = selected_row.min(total_rows - 1);
        if selected_row < self.offset {
            self.offset = selected_row;
        } else if selected_row >= self.offset.saturating_add(viewport_rows) {
            self.offset = selected_row.saturating_add(1).saturating_sub(viewport_rows);
        }
        self.clamp(total_rows, viewport_rows);
    }

    /// Applies an explicit page movement without changing the active focus.
    pub fn page_down(&mut self, rows: u16) {
        self.offset = self.offset.saturating_add(rows);
    }

    /// Applies an explicit reverse page movement without changing the active focus.
    pub fn page_up(&mut self, rows: u16) {
        self.offset = self.offset.saturating_sub(rows);
    }

    /// Keeps a newly focused row visible without overriding an explicit page
    /// position while focus and body geometry are unchanged.
    pub fn keep_focus_visible(
        &mut self,
        focused_row: Option<u16>,
        total_rows: u16,
        viewport_rows: u16,
    ) {
        self.clamp(total_rows, viewport_rows);
        let geometry_changed = self.last_total_rows != Some(total_rows)
            || self.last_viewport_rows != Some(viewport_rows);
        if focused_row != self.focused_row || geometry_changed {
            if let Some(focused_row) = focused_row {
                self.ensure_visible(focused_row, total_rows, viewport_rows);
            }
            self.focused_row = focused_row;
        }
        self.last_total_rows = Some(total_rows);
        self.last_viewport_rows = Some(viewport_rows);
    }
}

/// One display-only command row in a modal action list.
#[derive(Debug, Clone)]
pub struct ModalActionRow<'a> {
    pub accelerator: Option<char>,
    pub command: Cow<'a, str>,
    pub description: Cow<'a, str>,
}

impl<'a> ModalActionRow<'a> {
    pub fn new(
        accelerator: Option<char>,
        command: impl Into<Cow<'a, str>>,
        description: impl Into<Cow<'a, str>>,
    ) -> Self {
        Self {
            accelerator,
            command: command.into(),
            description: description.into(),
        }
    }

    /// Renders the row with its semantic command and secondary-description styles.
    pub fn render(&self, selected: bool, theme: &Theme) -> Line<'static> {
        self.render_with_availability(selected, true, theme)
    }

    /// Renders the row with muted semantic spans when the action is unavailable.
    pub fn render_with_availability(
        &self,
        selected: bool,
        enabled: bool,
        theme: &Theme,
    ) -> Line<'static> {
        let (key_style, command_style, description_style) = if enabled {
            (theme.modal_key, theme.modal_command, theme.modal_secondary)
        } else {
            (
                theme.modal_action_unavailable,
                theme.modal_action_unavailable,
                theme.modal_action_unavailable,
            )
        };
        let mut spans = Vec::new();
        if let Some(accelerator) = self.accelerator {
            spans.push(Span::styled(format!("[{accelerator}]"), key_style));
            spans.push(Span::raw(" "));
        }
        spans.push(Span::styled(self.command.to_string(), command_style));
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            self.description.to_string(),
            description_style,
        ));

        Line::from(spans).style(if selected {
            theme.modal_action_selected
        } else {
            Style::default()
        })
    }
}

enum ModalFooterContent<'a> {
    Hints(Vec<(Cow<'a, str>, Cow<'a, str>)>),
    Line(Line<'a>),
}

/// The footer rendered in the final inner row of a modal panel.
pub struct ModalFooter<'a> {
    content: ModalFooterContent<'a>,
}

impl<'a> ModalFooter<'a> {
    pub fn line(line: Line<'a>) -> Self {
        Self {
            content: ModalFooterContent::Line(line),
        }
    }

    pub(crate) fn render(&self, theme: &Theme, area: Rect, frame: &mut Frame) {
        match &self.content {
            ModalFooterContent::Hints(hints) => {
                let mut spans = Vec::new();
                for (index, (key, label)) in hints.iter().enumerate() {
                    if index > 0 {
                        spans.push(Span::styled("  ", theme.modal_footer));
                    }
                    spans.extend(key_hint_str(key, label, theme));
                }
                frame.render_widget(Paragraph::new(Line::from(spans)), area);
            }
            ModalFooterContent::Line(line) => {
                frame.render_widget(Paragraph::new(line.clone()).style(theme.modal_footer), area);
            }
        }
    }
}

impl ModalFooter<'static> {
    pub fn hints(hints: &[(&str, &str)]) -> Self {
        Self {
            content: ModalFooterContent::Hints(
                hints
                    .iter()
                    .map(|(key, label)| {
                        (
                            Cow::Owned((*key).to_owned()),
                            Cow::Owned((*label).to_owned()),
                        )
                    })
                    .collect(),
            ),
        }
    }

    /// Builds a footer that keeps every supplied control within one shell row.
    ///
    /// Full hints are used when they fit the footer viewport. Otherwise the
    /// compact key-only grammar is used, so narrow modals retain each action
    /// rather than truncating a trailing hint.
    pub(crate) fn adaptive_hints(area: Rect, full: &[(&str, &str)], compact_keys: &[&str]) -> Self {
        let compact = compact_keys
            .iter()
            .map(|key| (*key, ""))
            .collect::<Vec<_>>();
        Self::adaptive_hints_with_compact(area, full, &compact)
    }

    /// Builds a footer with a concise labeled grammar when full hints do not fit.
    pub(crate) fn adaptive_hints_with_compact(
        area: Rect,
        full: &[(&str, &str)],
        compact: &[(&str, &str)],
    ) -> Self {
        if hint_width(full) <= area.width as usize {
            Self::hints(full)
        } else if hint_width(compact) <= area.width as usize {
            Self::hints(compact)
        } else {
            Self::hints(
                &compact
                    .iter()
                    .map(|(key, _)| (*key, ""))
                    .collect::<Vec<_>>(),
            )
        }
    }
}

fn hint_width(hints: &[(&str, &str)]) -> usize {
    hints
        .iter()
        .enumerate()
        .map(|(index, (key, label))| {
            let separator_width = usize::from(index > 0) * 2;
            separator_width + key.len() + label.len() + 3
        })
        .sum()
}

/// Clears and draws the shared panel, returning body and footer viewports.
pub fn draw_modal_shell(frame: &mut Frame, spec: &ModalSpec<'_>, theme: &Theme) -> ModalAreas {
    let outer = centered_rect(spec.preferred_width, spec.max_height, frame.area());
    frame.render_widget(Clear, outer);

    let panel = block_panel(theme)
        .border_style(theme.modal_border)
        .title(spec.title.clone())
        .title_style(theme.modal_title);
    let inner = panel.inner(outer);
    frame.render_widget(panel, outer);

    if inner.height == 0 {
        return ModalAreas {
            outer,
            body: Rect::new(inner.x, inner.y, inner.width, 0),
            footer: Rect::new(inner.x, inner.y, inner.width, 0),
        };
    }

    let footer = Rect::new(inner.x, inner.bottom() - 1, inner.width, 1);
    let body = Rect::new(inner.x, inner.y, inner.width, inner.height - 1);
    spec.footer.render(theme, footer, frame);

    ModalAreas {
        outer,
        body,
        footer,
    }
}
