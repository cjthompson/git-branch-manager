use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use super::modal::{draw_modal_shell, ModalActionRow, ModalFooter, ModalScroll, ModalSpec};
use crate::theme::Theme;
use crate::types::{FailureCause, OperationResult};

/// Keyboard focus within the Results accordion.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ResultsFocus {
    #[default]
    Results,
    Actions {
        selected_index: usize,
        raw_details_visible: bool,
    },
}

impl ResultsFocus {
    fn raw_details_visible(self) -> bool {
        matches!(
            self,
            Self::Actions {
                raw_details_visible: true,
                ..
            }
        )
    }
}

/// An action available for exactly one expanded operation result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultsAction {
    ReviewForceDeletion,
    ReviewWorktreeRemoval,
    ViewRawGitDetails,
}

impl ResultsAction {
    fn label(self) -> &'static str {
        match self {
            Self::ReviewForceDeletion => "Review force deletion",
            Self::ReviewWorktreeRemoval => "Review worktree removal",
            Self::ViewRawGitDetails => "View raw Git details",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::ReviewForceDeletion => {
                "Open final confirmation for this branch's unique commits."
            }
            Self::ReviewWorktreeRemoval => {
                "Open final confirmation for this linked worktree and branch."
            }
            Self::ViewRawGitDetails => "Show or hide the underlying Git message.",
        }
    }
}

/// Derive actions from the selected row's typed result only.
pub fn result_actions(result: &OperationResult) -> Vec<ResultsAction> {
    let mut actions = Vec::with_capacity(2);
    match result.failure.as_ref() {
        Some(FailureCause::NotMerged) => actions.push(ResultsAction::ReviewForceDeletion),
        Some(FailureCause::CheckedOutInWorktree { is_main: false, .. }) => {
            actions.push(ResultsAction::ReviewWorktreeRemoval);
        }
        Some(FailureCause::BranchNotFound)
        | Some(FailureCause::CheckedOutInWorktree { is_main: true, .. })
        | Some(FailureCause::Other { .. })
        | None => {}
    }
    actions.push(ResultsAction::ViewRawGitDetails);
    actions
}

/// Render operation results as a one-open accordion in the shared modal shell.
pub fn draw_results(
    frame: &mut Frame,
    results: &[OperationResult],
    selected_index: &mut usize,
    expanded_index: &mut Option<usize>,
    focus: &mut ResultsFocus,
    body_scroll: &mut ModalScroll,
    theme: &Theme,
) {
    clamp_state(results, selected_index, expanded_index, focus);

    let area = frame.area();
    let preferred_width = (area.width * 85 / 100).clamp(50, 110);
    let max_height = (area.height * 80 / 100).max(8).min(area.height);
    let areas = draw_modal_shell(
        frame,
        &ModalSpec::new(
            "Results",
            ModalFooter::hints(&[]),
            preferred_width,
            max_height,
        ),
        theme,
    );

    let (lines, focused_row) =
        results_lines(results, *selected_index, *expanded_index, *focus, theme);
    let total_rows = lines.len().min(u16::MAX as usize) as u16;
    let overflowing = total_rows > areas.body.height;
    let footer = results_footer(*focus, results.is_empty(), overflowing, areas.footer);
    footer.render(theme, areas.footer, frame);
    body_scroll.keep_focus_visible(focused_row, total_rows, areas.body.height);

    frame.render_widget(
        Paragraph::new(lines).scroll((body_scroll.offset, 0)),
        areas.body,
    );
}

fn results_footer(
    focus: ResultsFocus,
    empty: bool,
    overflowing: bool,
    area: Rect,
) -> ModalFooter<'static> {
    let mut full = match focus {
        ResultsFocus::Results if empty => vec![("Esc", "Close")],
        ResultsFocus::Results => vec![
            ("j/k", "Navigate"),
            ("Enter", "Expand"),
            ("Tab", "Actions"),
            ("Esc", "Close"),
        ],
        ResultsFocus::Actions { .. } => vec![
            ("j/k", "Navigate"),
            ("Enter", "Invoke"),
            ("Tab", "Results"),
            ("Esc", "Close"),
        ],
    };
    let mut compact = match focus {
        ResultsFocus::Results if empty => vec!["Esc"],
        ResultsFocus::Results => vec!["j/k", "Enter", "Tab", "Esc"],
        ResultsFocus::Actions { .. } => vec!["j/k", "Enter", "Tab", "Esc"],
    };
    if overflowing {
        full.push(("PgUp/Dn", "Page"));
        compact.push("PgUp/Dn");
    }
    ModalFooter::adaptive_hints(area, &full, &compact)
}

fn clamp_state(
    results: &[OperationResult],
    selected_index: &mut usize,
    expanded_index: &mut Option<usize>,
    focus: &mut ResultsFocus,
) {
    if results.is_empty() {
        *selected_index = 0;
        *expanded_index = None;
        *focus = ResultsFocus::Results;
        return;
    }

    *selected_index = (*selected_index).min(results.len() - 1);
    *expanded_index = expanded_index.filter(|index| *index < results.len());
    let ResultsFocus::Actions {
        selected_index: action_index,
        raw_details_visible,
    } = *focus
    else {
        return;
    };
    let Some(expanded_index) = *expanded_index else {
        *focus = ResultsFocus::Results;
        return;
    };
    let actions = result_actions(&results[expanded_index]);
    if actions.is_empty() {
        *focus = ResultsFocus::Results;
    } else {
        *focus = ResultsFocus::Actions {
            selected_index: action_index.min(actions.len() - 1),
            raw_details_visible,
        };
    }
}

fn results_lines(
    results: &[OperationResult],
    selected_index: usize,
    expanded_index: Option<usize>,
    focus: ResultsFocus,
    theme: &Theme,
) -> (Vec<Line<'static>>, Option<u16>) {
    if results.is_empty() {
        return (
            vec![Line::from(Span::styled(
                "No operation results.",
                theme.modal_secondary,
            ))],
            None,
        );
    }

    let mut lines = Vec::new();
    let mut focused_row = None;
    for (index, result) in results.iter().enumerate() {
        let expanded = expanded_index == Some(index);
        if index == selected_index && matches!(focus, ResultsFocus::Results) {
            focused_row = Some(lines.len().min(u16::MAX as usize) as u16);
        }
        lines.push(result_header_line(
            result,
            index == selected_index,
            expanded,
            theme,
        ));

        if expanded {
            push_typed_details(&mut lines, result, theme);
            let actions = result_actions(result);
            for (action_index, action) in actions.into_iter().enumerate() {
                let selected = matches!(
                    focus,
                    ResultsFocus::Actions {
                        selected_index,
                        ..
                    } if selected_index == action_index
                );
                if selected {
                    focused_row = Some(lines.len().min(u16::MAX as usize) as u16);
                }
                lines.push(
                    ModalActionRow::new(None, action.label(), action.description())
                        .render(selected, theme),
                );
            }
            if focus.raw_details_visible() {
                lines.push(Line::from(vec![
                    Span::styled("Raw Git details: ", theme.modal_secondary),
                    Span::styled(raw_git_message(result).to_string(), theme.modal_failure),
                ]));
            }
        }
    }

    (lines, focused_row)
}

fn result_header_line(
    result: &OperationResult,
    selected: bool,
    expanded: bool,
    theme: &Theme,
) -> Line<'static> {
    let (status, status_style) = if result.success {
        ("OK", theme.modal_success)
    } else {
        ("FAIL", theme.modal_failure)
    };
    let disclosure = if expanded { "[-]" } else { "[+]" };
    Line::from(vec![
        Span::styled(disclosure, theme.modal_key),
        Span::raw(" "),
        Span::styled(status, status_style),
        Span::raw("  "),
        Span::styled(result.branch_name.clone(), theme.modal_branch),
        Span::raw("  "),
        Span::styled(typed_summary(result), theme.modal_secondary),
    ])
    .style(if selected {
        theme.modal_action_selected
    } else {
        Style::default()
    })
}

fn push_typed_details(lines: &mut Vec<Line<'static>>, result: &OperationResult, theme: &Theme) {
    lines.push(Line::from(vec![
        Span::raw("    "),
        Span::styled("Cause: ", theme.modal_secondary),
        Span::styled(
            typed_detail(result),
            if result.success {
                theme.modal_success
            } else {
                theme.modal_failure
            },
        ),
    ]));

    if let Some(FailureCause::CheckedOutInWorktree {
        worktree_path,
        is_main,
    }) = result.failure.as_ref()
    {
        lines.push(Line::from(vec![
            Span::raw("    "),
            Span::styled("Worktree: ", theme.modal_secondary),
            Span::styled(
                worktree_path.to_string_lossy().into_owned(),
                theme.modal_worktree,
            ),
        ]));
        if *is_main {
            lines.push(Line::from(Span::styled(
                "    Primary worktree recovery is disabled.",
                theme.modal_warning,
            )));
        }
    }
}

fn typed_summary(result: &OperationResult) -> &'static str {
    if result.success {
        return "Completed";
    }
    match result.failure.as_ref() {
        Some(FailureCause::BranchNotFound) => "Branch not found",
        Some(FailureCause::NotMerged) => "Not merged into base branch",
        Some(FailureCause::CheckedOutInWorktree { is_main: false, .. }) => {
            "Checked out in linked worktree"
        }
        Some(FailureCause::CheckedOutInWorktree { is_main: true, .. }) => {
            "Checked out in primary worktree"
        }
        Some(FailureCause::Other { .. }) => "Unclassified Git failure",
        None => "Git operation failed",
    }
}

fn typed_detail(result: &OperationResult) -> &'static str {
    if result.success {
        return "The operation completed successfully.";
    }
    match result.failure.as_ref() {
        Some(FailureCause::BranchNotFound) => "The branch no longer exists.",
        Some(FailureCause::NotMerged) => {
            "Not merged into base branch; force deletion can discard unique commits."
        }
        Some(FailureCause::CheckedOutInWorktree { is_main: false, .. }) => {
            "The branch is checked out in a linked worktree."
        }
        Some(FailureCause::CheckedOutInWorktree { is_main: true, .. }) => {
            "The branch is checked out in the primary worktree."
        }
        Some(FailureCause::Other { .. }) => "Git reported an unclassified failure.",
        None => "Git reported a failure without a typed classification.",
    }
}

fn raw_git_message(result: &OperationResult) -> &str {
    match result.failure.as_ref() {
        Some(FailureCause::Other { raw_message }) if !raw_message.is_empty() => raw_message,
        _ => &result.message,
    }
}
