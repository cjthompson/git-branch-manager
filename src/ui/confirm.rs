use std::path::PathBuf;

use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use crate::theme::Theme;
use crate::types::{BranchAction, ChangedFile, MergeStatus};

use super::modal::{draw_modal_shell, ModalActionRow, ModalFooter, ModalScroll, ModalSpec};
use super::shared::abbreviate_path;

/// Structured safety facts collected before a local branch deletion.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeletePreflight {
    pub risks: Vec<DeleteRisk>,
}

/// One reason a local branch deletion needs additional review.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeleteRisk {
    UniqueCommits {
        branch: String,
        base: String,
        status: MergeStatus,
        merge_base_commit: Option<String>,
    },
    CheckedOut {
        branch: String,
        worktree: PathBuf,
        is_main: bool,
    },
    DirtyWorktree {
        worktree: PathBuf,
        files: Vec<ChangedFile>,
        omitted: usize,
    },
}

/// One selectable operation in the initial confirmation stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmChoice {
    pub accelerator: char,
    pub action: BranchAction,
    pub targets: Vec<String>,
    pub remote: Option<String>,
    pub command: String,
    pub description: String,
    pub destructive_target: Option<ConfirmTarget>,
}

/// The one exact target authorized by a destructive final confirmation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmTarget {
    Branch(String),
    Worktree(PathBuf),
    BranchWorktree { branch: String, worktree: PathBuf },
}

impl ConfirmTarget {
    /// The single queue target corresponding to this confirmation target.
    pub fn job_target(&self) -> String {
        match self {
            Self::Branch(branch) | Self::BranchWorktree { branch, .. } => branch.clone(),
            Self::Worktree(worktree) => worktree.to_string_lossy().into_owned(),
        }
    }
}

/// Confirmation advances explicitly before a recovery action can discard data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmStage {
    Initial,
    FinalDestructive {
        choice: ConfirmChoice,
        target: ConfirmTarget,
    },
}

/// Renders the typed confirmation state through the shared modal shell.
pub fn draw_confirm(
    frame: &mut Frame,
    preflight: &DeletePreflight,
    choices: &[ConfirmChoice],
    selected: usize,
    body_scroll: &mut ModalScroll,
    stage: &ConfirmStage,
    theme: &Theme,
) {
    let final_stage = matches!(stage, ConfirmStage::FinalDestructive { .. });
    let title = if final_stage {
        "Confirm destructive action"
    } else {
        "Confirm action"
    };
    let area = frame.area();
    let preferred_width = (area.width * 80 / 100).clamp(48, 100);
    let max_height = (area.height * 80 / 100).max(8).min(area.height);
    let areas = draw_modal_shell(
        frame,
        &ModalSpec::new(title, ModalFooter::hints(&[]), preferred_width, max_height),
        theme,
    );

    let path_width = areas.body.width.saturating_sub(20).max(12) as usize;
    let (lines, selected_row) = match stage {
        ConfirmStage::Initial => initial_lines(preflight, choices, selected, path_width, theme),
        ConfirmStage::FinalDestructive { choice, target } => {
            (final_lines(choice, target, theme), None)
        }
    };
    let total_rows = lines.len().min(u16::MAX as usize) as u16;
    let overflowing = total_rows > areas.body.height;
    let footer = confirm_footer(final_stage, overflowing, areas.footer);
    footer.render(theme, areas.footer, frame);
    body_scroll.keep_focus_visible(selected_row, total_rows, areas.body.height);

    frame.render_widget(
        Paragraph::new(lines).scroll((body_scroll.offset, 0)),
        areas.body,
    );
}

fn confirm_footer(final_stage: bool, overflowing: bool, area: Rect) -> ModalFooter<'static> {
    if final_stage {
        let full = if overflowing {
            vec![
                ("y/Enter", "Confirm"),
                ("n/Esc", "Cancel"),
                ("PgUp/Dn", "Page"),
            ]
        } else {
            vec![("y/Enter", "Confirm"), ("n/Esc", "Cancel")]
        };
        let compact = if overflowing {
            vec![("y/Enter", "Yes"), ("n/Esc", "No"), ("PgUp/Dn", "Scroll")]
        } else {
            vec![("y/Enter", "Yes"), ("n/Esc", "No")]
        };
        ModalFooter::adaptive_hints_with_compact(area, &full, &compact)
    } else {
        let full = if overflowing {
            vec![
                ("j/k", "Navigate"),
                ("Enter", "Select"),
                ("Esc", "Cancel"),
                ("PgUp/Dn", "Page"),
            ]
        } else {
            vec![("j/k", "Navigate"), ("Enter", "Select"), ("Esc", "Cancel")]
        };
        let compact = if overflowing {
            vec!["j/k", "Enter", "Esc", "PgUp/Dn"]
        } else {
            vec!["j/k", "Enter", "Esc"]
        };
        ModalFooter::adaptive_hints(area, &full, &compact)
    }
}

fn initial_lines(
    preflight: &DeletePreflight,
    choices: &[ConfirmChoice],
    selected: usize,
    path_width: usize,
    theme: &Theme,
) -> (Vec<Line<'static>>, Option<u16>) {
    let mut lines = vec![
        Line::from(Span::styled(
            "Review the target and choose an action.",
            theme.modal_secondary,
        )),
        Line::from(""),
    ];

    if let Some(default_choice) = choices.first() {
        lines.push(Line::from(Span::styled("Targets", theme.modal_title)));
        for target in &default_choice.targets {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(target.clone(), branch_style(target, theme)),
            ]));
        }
        lines.push(Line::from(""));
    }

    if !preflight.risks.is_empty() {
        lines.push(Line::from(Span::styled("Safety review", theme.modal_title)));
        for risk in &preflight.risks {
            push_risk_lines(&mut lines, risk, path_width, theme);
        }
        lines.push(Line::from(""));
    }

    lines.push(Line::from(Span::styled("Choices", theme.modal_title)));
    let selected = selected.min(choices.len().saturating_sub(1));
    let mut selected_row = None;
    for (index, choice) in choices.iter().enumerate() {
        if index == selected {
            selected_row = Some(lines.len().min(u16::MAX as usize) as u16);
        }
        lines.push(
            ModalActionRow::new(
                Some(choice.accelerator),
                choice.command.clone(),
                choice.description.clone(),
            )
            .render(index == selected, theme),
        );
    }

    (lines, selected_row)
}

fn push_risk_lines(
    lines: &mut Vec<Line<'static>>,
    risk: &DeleteRisk,
    path_width: usize,
    theme: &Theme,
) {
    match risk {
        DeleteRisk::UniqueCommits {
            branch,
            base,
            status,
            merge_base_commit,
        } => {
            lines.push(Line::from(vec![
                Span::styled("  Status: ", theme.modal_secondary),
                Span::styled(
                    merge_status_label(*status),
                    merge_status_style(*status, theme),
                ),
                Span::raw("  "),
                Span::styled(branch.clone(), branch_style(branch, theme)),
                Span::styled(" compared with ", theme.modal_secondary),
                Span::styled(base.clone(), branch_style(base, theme)),
            ]));
            if let Some(hash) = merge_base_commit {
                lines.push(Line::from(vec![
                    Span::styled("    Merge base: ", theme.modal_secondary),
                    Span::styled(hash.clone(), theme.modal_commit),
                ]));
            }
        }
        DeleteRisk::CheckedOut {
            branch,
            worktree,
            is_main,
        } => {
            let path = abbreviate_path(worktree, path_width);
            lines.push(Line::from(vec![
                Span::styled("  Checked out  ", theme.modal_warning),
                Span::styled(branch.clone(), branch_style(branch, theme)),
                Span::styled(" at ", theme.modal_secondary),
                Span::styled(path, theme.modal_worktree),
            ]));
            if *is_main {
                lines.push(Line::from(Span::styled(
                    "  PRIMARY WORKTREE - removal recovery is disabled",
                    theme.modal_warning,
                )));
            }
        }
        DeleteRisk::DirtyWorktree {
            worktree,
            files,
            omitted,
        } => {
            let path = abbreviate_path(worktree, path_width);
            lines.push(Line::from(Span::styled(
                format!("  DIRTY WORKTREE  {path}"),
                theme.modal_warning,
            )));
            for file in files {
                lines.push(Line::from(Span::styled(
                    format!("    {}: {}", file.kind.label(), file.path),
                    theme.modal_warning,
                )));
            }
            if *omitted > 0 {
                lines.push(Line::from(Span::styled(
                    format!("    +{omitted} more"),
                    theme.modal_warning,
                )));
            }
            if files.is_empty() && *omitted == 0 {
                lines.push(Line::from(Span::styled(
                    "    staged changes are present",
                    theme.modal_warning,
                )));
            }
        }
    }
}

fn final_lines(
    choice: &ConfirmChoice,
    target: &ConfirmTarget,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(Span::styled(
            "FINAL DESTRUCTIVE CONFIRMATION",
            theme.modal_warning,
        )),
        Line::from(vec![
            Span::styled("Action: ", theme.modal_secondary),
            Span::styled(choice.action.label(), theme.modal_command),
        ]),
    ];
    match target {
        ConfirmTarget::Branch(branch) => lines.push(Line::from(vec![
            Span::styled("Branch: ", theme.modal_secondary),
            Span::styled(branch.clone(), branch_style(branch, theme)),
        ])),
        ConfirmTarget::Worktree(worktree) => lines.push(Line::from(vec![
            Span::styled("Worktree: ", theme.modal_secondary),
            Span::styled(
                worktree.to_string_lossy().into_owned(),
                theme.modal_worktree,
            ),
        ])),
        ConfirmTarget::BranchWorktree { branch, worktree } => {
            lines.push(Line::from(vec![
                Span::styled("Branch: ", theme.modal_secondary),
                Span::styled(branch.clone(), branch_style(branch, theme)),
            ]));
            lines.push(Line::from(vec![
                Span::styled("Worktree: ", theme.modal_secondary),
                Span::styled(
                    worktree.to_string_lossy().into_owned(),
                    theme.modal_worktree,
                ),
            ]));
        }
    }
    if let Some(remote) = &choice.remote {
        lines.push(Line::from(vec![
            Span::styled("Remote: ", theme.modal_secondary),
            Span::styled(remote.clone(), theme.modal_branch),
        ]));
    }
    lines.extend([
        Line::from(vec![
            Span::styled("Command: ", theme.modal_secondary),
            Span::styled(choice.command.clone(), theme.modal_command),
        ]),
        Line::from(Span::styled(
            data_loss_text(choice.action),
            theme.modal_warning,
        )),
    ]);
    lines
}

fn merge_status_label(status: MergeStatus) -> &'static str {
    match status {
        MergeStatus::LikelySquashMerged => "Possible Squash Merge",
        MergeStatus::Unmerged => "Unmerged",
        _ => "Requires Review",
    }
}

fn merge_status_style(status: MergeStatus, theme: &Theme) -> Style {
    match status {
        MergeStatus::LikelySquashMerged => theme.squash_merged.add_modifier(Modifier::DIM),
        MergeStatus::Unmerged => theme.modal_failure,
        _ => theme.modal_warning,
    }
}

fn data_loss_text(action: BranchAction) -> &'static str {
    match action {
        BranchAction::DeleteLocalForce => {
            "DATA LOSS: unique commits on the exact target may be permanently destroyed."
        }
        BranchAction::WorktreeForceRemove | BranchAction::DeleteBranchAndRemoveWorktreeForce => {
            "DATA LOSS: uncommitted work and unique commits may be permanently destroyed."
        }
        _ => "DATA LOSS: removing this worktree and branch cannot be undone here.",
    }
}

fn branch_style(_name: &str, theme: &Theme) -> Style {
    theme.modal_branch
}

/// Renders a plain yes/no confirmation overlay -- unlike [`draw_confirm`],
/// not tied to a `BranchAction`/target list. Used for secondary
/// confirmations layered on top of an already-running job (e.g. "cancelling
/// now may leave the worktree partially deleted").
pub fn draw_confirm_cancel(frame: &mut Frame, message: &str, theme: &Theme) {
    let mut lines: Vec<Line> = message.lines().map(Line::from).collect();
    if lines.is_empty() {
        lines.push(Line::from(""));
    }

    let area = frame.area();
    let preferred_width = (area.width * 60 / 100).clamp(40, 80);
    let max_height = (area.height * 60 / 100).max(8).min(area.height);
    let areas = draw_modal_shell(
        frame,
        &ModalSpec::new(
            "Cancel running job?",
            ModalFooter::hints(&[("y/Enter", "Yes"), ("n/Esc", "No")]),
            preferred_width,
            max_height,
        ),
        theme,
    );

    frame.render_widget(Paragraph::new(lines), areas.body);
}
