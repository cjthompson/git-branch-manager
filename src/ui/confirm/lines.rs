use ratatui::prelude::*;

use crate::theme::Theme;
use crate::types::{BranchAction, MergeStatus};
use crate::ui::modal::ModalActionRow;
use crate::ui::shared::abbreviate_path;

use super::{ConfirmChoice, ConfirmTarget, DeletePreflight, DeleteRisk};

pub(super) fn initial_lines(
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
        // Enter runs every safe `y` row, so list the union of their targets.
        let enter_group: Vec<&ConfirmChoice> = if default_choice.destructive_target.is_none() {
            choices
                .iter()
                .filter(|choice| {
                    choice.destructive_target.is_none()
                        && choice.accelerator == default_choice.accelerator
                })
                .collect()
        } else {
            vec![default_choice]
        };
        let mut shown_targets: Vec<&str> = Vec::new();
        for choice in enter_group {
            for target in &choice.targets {
                if !shown_targets.contains(&target.as_str()) {
                    shown_targets.push(target.as_str());
                }
            }
        }
        for target in shown_targets {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(target.to_string(), branch_style(target, theme)),
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

pub(super) fn final_lines(
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
