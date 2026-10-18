mod lines;

use std::path::PathBuf;

use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use crate::theme::Theme;
use crate::types::{BranchAction, ChangedFile, MergeStatus};

use super::modal::{draw_modal_shell, ModalFooter, ModalScroll, ModalSpec};

use lines::{final_lines, initial_lines};

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
