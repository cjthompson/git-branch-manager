use git_branch_manager::{
    theme::Theme,
    types::{
        BranchAction, ChangedFile, ChangedFileKind, FailureCause,
        OperationResult,
    },
    ui::{
        confirm::{ConfirmChoice, ConfirmStage, DeletePreflight, DeleteRisk},
        modal::ModalScroll,
        render::Overlay,
        results::ResultsFocus,
    },
};
use ratatui::style::Color;

use crate::support::*;

#[test]
fn every_built_in_theme_supplies_the_modal_semantic_palette() {
    for theme in [
        Theme::dark(),
        Theme::light(),
        Theme::solarized(),
        Theme::dracula(),
    ] {
        for (name, style) in [
            ("border", theme.modal_border),
            ("title", theme.modal_title),
            ("footer", theme.modal_footer),
            ("key", theme.modal_key),
            ("branch", theme.modal_branch),
            ("worktree", theme.modal_worktree),
            ("commit", theme.modal_commit),
            ("command", theme.modal_command),
            ("success", theme.modal_success),
            ("failure", theme.modal_failure),
            ("secondary", theme.modal_secondary),
            ("unavailable action", theme.modal_action_unavailable),
        ] {
            assert!(
                style.fg.is_some(),
                "{name} needs modal fg in {}",
                theme.name
            );
        }
        assert!(
            theme.modal_warning.fg.is_some(),
            "{} needs warning fg",
            theme.name
        );
        assert!(
            theme.modal_warning.bg.is_some(),
            "{} needs warning bg",
            theme.name
        );
        assert!(
            theme.modal_action_selected.bg.is_some(),
            "{} needs selected-row bg",
            theme.name
        );
    }
}

#[test]
fn built_in_themes_use_the_approved_accented_modal_palette() {
    let palettes = [
        (
            "dark",
            Theme::dark(),
            Color::Rgb(29, 59, 74),
            Color::Rgb(0, 114, 168),
        ),
        (
            "light",
            Theme::light(),
            Color::Rgb(219, 234, 246),
            Color::Rgb(133, 189, 232),
        ),
        (
            "solarized",
            Theme::solarized(),
            Color::Rgb(11, 61, 75),
            Color::Rgb(11, 111, 141),
        ),
        (
            "dracula",
            Theme::dracula(),
            Color::Rgb(59, 61, 79),
            Color::Rgb(106, 109, 157),
        ),
    ];

    for (name, theme, expected_surface, expected_selected) in palettes {
        assert_eq!(theme.name, name);
        assert_eq!(
            theme.modal_surface.bg,
            Some(expected_surface),
            "{name} theme's modal surface changed"
        );
        assert_eq!(
            theme.modal_action_selected.bg,
            Some(expected_selected),
            "{name} theme's selected action background changed"
        );
    }
}

#[test]
fn shared_shell_gives_every_theme_an_elevated_surface_and_legible_modal_semantics() {
    for theme in [
        Theme::dark(),
        Theme::light(),
        Theme::solarized(),
        Theme::dracula(),
    ] {
        let surface = theme
            .modal_surface
            .bg
            .expect("each built-in theme needs an explicit modal surface");
        assert_ne!(
            surface,
            Color::Reset,
            "{} theme's modal surface must not be the terminal default",
            theme.name
        );

        let (buffer, areas) = render_modal_semantic_sample(&theme);
        let blank_body_cell = &buffer[(areas.body.x, areas.body.y + 3)];
        assert_eq!(
            blank_body_cell.bg, surface,
            "{} theme's shared shell must paint its body surface",
            theme.name
        );

        for (role, text, style) in [
            ("title", "Modal semantic sample", theme.modal_title),
            ("footer", "Close", theme.modal_footer),
            ("footer key", "Esc", theme.modal_key),
            ("body key", "[r]", theme.modal_key),
            ("body command", "Run command", theme.modal_command),
            ("body detail", "Selected detail", theme.modal_secondary),
            ("unavailable key", "[u]", theme.modal_action_unavailable),
            (
                "unavailable command",
                "Unavailable action",
                theme.modal_action_unavailable,
            ),
            (
                "unavailable detail",
                "Cannot run now",
                theme.modal_action_unavailable,
            ),
            (
                "warning",
                "Warning: requires attention",
                theme.modal_warning,
            ),
        ] {
            assert_rendered_foreground(&buffer, text, style, role, theme.name);
        }

        for text in ["Modal semantic sample", "Close", "Esc"] {
            let (x, y) = text_cell(&buffer, text);
            let cell = &buffer[(x, y)];
            assert_eq!(
                cell.bg, surface,
                "{text} must retain the modal surface in {}",
                theme.name
            );
            assert_ne!(
                cell.fg, surface,
                "{text} must remain readable on the modal surface in {}",
                theme.name
            );
        }

        let selected_background = theme
            .modal_action_selected
            .bg
            .expect("selected actions need a background");
        assert_ne!(
            selected_background, surface,
            "{} theme's selected action must stand apart from the modal surface",
            theme.name
        );
        for text in ["[r]", "Run command", "Selected detail"] {
            let (x, y) = text_cell(&buffer, text);
            let cell = &buffer[(x, y)];
            assert_eq!(
                cell.bg, selected_background,
                "{text} must retain the selected action background in {}",
                theme.name
            );
            assert_ne!(
                cell.fg, selected_background,
                "{text} must remain readable while selected in {}",
                theme.name
            );
        }

        // Normal text and every selected action span use the normal-text
        // 4.5:1 threshold, including secondary detail.
        const NORMAL_TEXT_CONTRAST: f64 = 4.5;
        for (role, style) in [
            ("title", theme.modal_title),
            ("footer", theme.modal_footer),
            ("key", theme.modal_key),
            ("command", theme.modal_command),
            ("detail", theme.modal_secondary),
            ("unavailable", theme.modal_action_unavailable),
        ] {
            let ratio = contrast_ratio(style.fg.unwrap(), surface);
            assert!(
                ratio >= NORMAL_TEXT_CONTRAST,
                "{role} must meet {NORMAL_TEXT_CONTRAST}:1 on {} surface; measured {ratio:.4}:1",
                theme.name,
            );
        }

        for (role, style) in [
            ("key", theme.modal_key),
            ("command", theme.modal_command),
            ("detail", theme.modal_secondary),
        ] {
            let ratio = contrast_ratio(style.fg.unwrap(), selected_background);
            assert!(
                ratio >= NORMAL_TEXT_CONTRAST,
                "selected {role} must meet {NORMAL_TEXT_CONTRAST}:1 in {} theme; measured {ratio:.4}:1",
                theme.name,
            );
        }

        let (unavailable_x, unavailable_y) = text_cell(&buffer, "Unavailable action");
        let unavailable = &buffer[(unavailable_x, unavailable_y)];
        assert_eq!(
            unavailable.bg, surface,
            "unavailable actions must stay on the modal surface in {}",
            theme.name
        );
        assert_ne!(
            unavailable.fg, surface,
            "unavailable actions must remain visible in {}",
            theme.name
        );

        let (warning_x, warning_y) = text_cell(&buffer, "Warning: requires attention");
        let warning = &buffer[(warning_x, warning_y)];
        assert_eq!(warning.bg, theme.modal_warning.bg.unwrap());
        assert_ne!(
            warning.fg, warning.bg,
            "warnings need foreground contrast in {}",
            theme.name
        );
        assert_ne!(
            warning.bg, surface,
            "warnings need a distinct background in {}",
            theme.name
        );
        let warning_ratio = contrast_ratio(
            theme.modal_warning.fg.unwrap(),
            theme.modal_warning.bg.unwrap(),
        );
        assert!(
            warning_ratio >= NORMAL_TEXT_CONTRAST,
            "warning must meet {NORMAL_TEXT_CONTRAST}:1 in {} theme; measured {warning_ratio:.4}:1",
            theme.name,
        );
    }
}

#[test]
fn semantic_modal_styles_render_in_every_builtin_theme() {
    for theme in [
        Theme::dark(),
        Theme::light(),
        Theme::solarized(),
        Theme::dracula(),
    ] {
        let mut confirm = Overlay::Confirm {
            preflight: DeletePreflight {
                risks: vec![DeleteRisk::DirtyWorktree {
                    worktree: "/repo/.worktrees/dirty".into(),
                    files: vec![ChangedFile {
                        path: "changed.rs".to_string(),
                        kind: ChangedFileKind::Modified,
                    }],
                    omitted: 0,
                }],
            },
            choices: vec![ConfirmChoice {
                accelerator: 'y',
                action: BranchAction::DeleteLocal,
                targets: vec!["feature/semantic".to_string()],
                remote: None,
                command: "git branch -d -- feature/semantic".to_string(),
                description: "Preserve reachable history.".to_string(),
                destructive_target: None,
            }],
            selected: 0,
            body_scroll: ModalScroll::default(),
            stage: ConfirmStage::Initial,
        };
        let confirm_buffer = render_overlay_buffer(&mut confirm, 120, 30, &theme);
        for text in ["DIRTY WORKTREE", "modified: changed.rs"] {
            let (x, y) = text_cell(&confirm_buffer, text);
            assert_cell_style(&confirm_buffer, x, y, theme.modal_warning);
        }
        let (branch_x, branch_y) = text_cell(&confirm_buffer, "feature/semantic");
        assert_cell_foreground(&confirm_buffer, branch_x, branch_y, theme.modal_branch);
        let (command_x, command_y) = text_cell(&confirm_buffer, "git branch -d");
        assert_cell_foreground(&confirm_buffer, command_x, command_y, theme.modal_command);

        let mut results = Overlay::Results {
            results: vec![
                OperationResult::success("feature/ok", BranchAction::DeleteLocal, "deleted"),
                OperationResult::failure(
                    "feature/fail",
                    BranchAction::DeleteLocal,
                    FailureCause::NotMerged,
                    "not merged",
                ),
            ],
            selected_index: 0,
            expanded_index: Some(1),
            focus: ResultsFocus::Results,
            body_scroll: ModalScroll::default(),
        };
        let results_buffer = render_overlay_buffer(&mut results, 120, 30, &theme);
        let (success_x, success_y) = text_cell(&results_buffer, "OK");
        assert_cell_foreground(&results_buffer, success_x, success_y, theme.modal_success);
        let (failure_x, failure_y) = text_cell(&results_buffer, "FAIL");
        assert_cell_foreground(&results_buffer, failure_x, failure_y, theme.modal_failure);
        let (result_branch_x, result_branch_y) = text_cell(&results_buffer, "feature/fail");
        assert_cell_foreground(
            &results_buffer,
            result_branch_x,
            result_branch_y,
            theme.modal_branch,
        );
        let (detail_x, detail_y) = text_cell(&results_buffer, "Not merged into base branch;");
        assert_cell_foreground(&results_buffer, detail_x, detail_y, theme.modal_failure);
    }
}
