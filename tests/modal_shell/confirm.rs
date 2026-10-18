use git_branch_manager::{
    types::{
        BranchAction, ChangedFile, ChangedFileKind,
        OperationResult,
    },
    ui::{
        confirm::{ConfirmChoice, ConfirmStage, ConfirmTarget, DeletePreflight, DeleteRisk},
        modal::ModalScroll,
        render::Overlay,
        results::ResultsFocus,
    },
};

use crate::support::render_overlay;

#[test]
fn confirmation_dirty_warning_renders_five_files_and_exact_omitted_count() {
    let files = vec![
        ChangedFile {
            path: "modified-1.rs".to_string(),
            kind: ChangedFileKind::Modified,
        },
        ChangedFile {
            path: "modified-2.rs".to_string(),
            kind: ChangedFileKind::Modified,
        },
        ChangedFile {
            path: "modified-3.rs".to_string(),
            kind: ChangedFileKind::Modified,
        },
        ChangedFile {
            path: "modified-4.rs".to_string(),
            kind: ChangedFileKind::Modified,
        },
        ChangedFile {
            path: "untracked-1.rs".to_string(),
            kind: ChangedFileKind::Untracked,
        },
    ];
    let mut overlay = Overlay::Confirm {
        preflight: DeletePreflight {
            risks: vec![DeleteRisk::DirtyWorktree {
                worktree: "/repo/.worktrees/dirty".into(),
                files,
                omitted: 2,
            }],
        },
        choices: vec![ConfirmChoice {
            accelerator: 'y',
            action: BranchAction::DeleteLocal,
            targets: vec!["feature/dirty".to_string()],
            remote: None,
            command: "git branch -d -- feature/dirty".to_string(),
            description: "Let Git reject unsafe deletion.".to_string(),
            destructive_target: None,
        }],
        selected: 0,
        body_scroll: ModalScroll::default(),
        stage: ConfirmStage::Initial,
    };

    let rendered = render_overlay(&mut overlay, 100, 30);

    for expected in [
        "DIRTY WORKTREE",
        "/repo/.worktrees/dirty",
        "modified-1.rs",
        "modified-2.rs",
        "modified-3.rs",
        "modified-4.rs",
        "untracked-1.rs",
        "+2 more",
    ] {
        assert!(
            rendered.contains(expected),
            "missing {expected}: {rendered}"
        );
    }
    assert!(
        rendered.contains("[j/k] Navigate  [Enter] Select  [Esc] Cancel"),
        "confirmation did not use the shared footer: {rendered}"
    );
}

#[test]
fn constrained_confirmation_keeps_selected_choice_visible() {
    let risks = (0..8)
        .map(|index| DeleteRisk::UniqueCommits {
            branch: format!("feature/risk-{index}"),
            base: "main".to_string(),
            status: git_branch_manager::types::MergeStatus::Unmerged,
            merge_base_commit: None,
        })
        .collect();
    let mut overlay = Overlay::Confirm {
        preflight: DeletePreflight { risks },
        choices: vec![
            ConfirmChoice {
                accelerator: 'y',
                action: BranchAction::DeleteLocal,
                targets: vec!["feature/risk-0".to_string()],
                remote: None,
                command: "git branch -d -- feature/risk-0".to_string(),
                description: "Let Git reject unsafe deletion.".to_string(),
                destructive_target: None,
            },
            ConfirmChoice {
                accelerator: 'r',
                action: BranchAction::DeleteLocalForce,
                targets: vec!["feature/risk-0".to_string()],
                remote: None,
                command: "Review force-delete".to_string(),
                description: "Open final data-loss confirmation.".to_string(),
                destructive_target: Some(ConfirmTarget::Branch("feature/risk-0".to_string())),
            },
        ],
        selected: 1,
        body_scroll: ModalScroll::default(),
        stage: ConfirmStage::Initial,
    };

    let rendered = render_overlay(&mut overlay, 80, 8);

    assert!(
        rendered.contains("Review force-delete"),
        "selected choice is clipped: {rendered}"
    );
    assert!(
        matches!(overlay, Overlay::Confirm { body_scroll, .. } if body_scroll.offset > 0),
        "rendering must persist the corrected body offset: {overlay:?}"
    );
}

#[test]
fn final_confirmation_repeats_exact_target_action_and_confirm_cancel_footer() {
    let final_choice = ConfirmChoice {
        accelerator: 'r',
        action: BranchAction::DeleteLocalForce,
        targets: vec!["feature/unmerged".to_string()],
        remote: None,
        command: "git branch -D -- feature/unmerged".to_string(),
        description: "Permanently discard unique commits.".to_string(),
        destructive_target: Some(ConfirmTarget::Branch("feature/unmerged".to_string())),
    };
    let mut overlay = Overlay::Confirm {
        preflight: DeletePreflight { risks: Vec::new() },
        choices: vec![ConfirmChoice {
            accelerator: 'y',
            action: BranchAction::DeleteLocal,
            targets: vec!["feature/other".to_string()],
            remote: None,
            command: "git branch -d -- feature/other".to_string(),
            description: "Safe delete.".to_string(),
            destructive_target: None,
        }],
        selected: 0,
        body_scroll: ModalScroll::default(),
        stage: ConfirmStage::FinalDestructive {
            choice: final_choice,
            target: ConfirmTarget::Branch("feature/unmerged".to_string()),
        },
    };

    let rendered = render_overlay(&mut overlay, 90, 20);

    assert!(
        rendered.contains("FINAL DESTRUCTIVE CONFIRMATION"),
        "{rendered}"
    );
    assert!(rendered.contains("Force-delete local"), "{rendered}");
    assert!(rendered.contains("feature/unmerged"), "{rendered}");
    assert!(
        rendered.contains("git branch -D -- feature/unmerged"),
        "{rendered}"
    );
    assert!(!rendered.contains("feature/other"), "{rendered}");
    assert!(
        rendered.contains("[y/Enter] Confirm  [n/Esc] Cancel"),
        "final confirmation did not show confirm and cancel keys: {rendered}"
    );
    assert!(
        !rendered.contains("[PgUp/Dn]"),
        "a final confirmation without overflow must not advertise paging: {rendered}"
    );
}

#[test]
fn compact_confirm_and_results_footer_keeps_controls_and_pages_only_on_overflow() {
    let choices = (0..12)
        .map(|index| ConfirmChoice {
            accelerator: 'y',
            action: BranchAction::DeleteLocal,
            targets: vec![format!("feature/choice-{index}")],
            remote: None,
            command: format!("git branch -d -- feature/choice-{index}"),
            description: "Delete only this branch if Git reports it safe.".to_string(),
            destructive_target: None,
        })
        .collect();
    let mut overflowing_confirm = Overlay::Confirm {
        preflight: DeletePreflight::default(),
        choices,
        selected: 0,
        body_scroll: ModalScroll::default(),
        stage: ConfirmStage::Initial,
    };
    let compact_confirm = render_overlay(&mut overflowing_confirm, 48, 8);
    for control in ["[j/k]", "[Enter]", "[Esc]", "[PgUp/Dn]"] {
        assert!(
            compact_confirm.contains(control),
            "overflowing compact Confirm clipped {control}: {compact_confirm}"
        );
    }

    let mut normal_confirm = Overlay::Confirm {
        preflight: DeletePreflight::default(),
        choices: vec![ConfirmChoice {
            accelerator: 'y',
            action: BranchAction::DeleteLocal,
            targets: vec!["feature/only".to_string()],
            remote: None,
            command: "git branch -d -- feature/only".to_string(),
            description: "Delete only this branch if Git reports it safe.".to_string(),
            destructive_target: None,
        }],
        selected: 0,
        body_scroll: ModalScroll::default(),
        stage: ConfirmStage::Initial,
    };
    let normal_confirm = render_overlay(&mut normal_confirm, 120, 30);
    assert!(
        normal_confirm.contains("[j/k] Navigate  [Enter] Select  [Esc] Cancel"),
        "{normal_confirm}"
    );
    assert!(
        !normal_confirm.contains("[PgUp/Dn]"),
        "non-overflowing Confirm advertised paging: {normal_confirm}"
    );

    let results = (0..12)
        .map(|index| {
            OperationResult::success(
                format!("feature/result-{index}"),
                BranchAction::DeleteLocal,
                "deleted",
            )
        })
        .collect();
    let mut overflowing_results = Overlay::Results {
        results,
        selected_index: 0,
        expanded_index: None,
        focus: ResultsFocus::Results,
        body_scroll: ModalScroll::default(),
    };
    let compact_results = render_overlay(&mut overflowing_results, 48, 8);
    for control in ["[j/k]", "[Enter]", "[Tab]", "[Esc]", "[PgUp/Dn]"] {
        assert!(
            compact_results.contains(control),
            "overflowing compact Results clipped {control}: {compact_results}"
        );
    }

    let mut normal_results = Overlay::Results {
        results: vec![OperationResult::success(
            "feature/only",
            BranchAction::DeleteLocal,
            "deleted",
        )],
        selected_index: 0,
        expanded_index: None,
        focus: ResultsFocus::Results,
        body_scroll: ModalScroll::default(),
    };
    let normal_results = render_overlay(&mut normal_results, 120, 30);
    assert!(
        normal_results.contains("[j/k] Navigate  [Enter] Expand  [Tab] Actions  [Esc] Close"),
        "{normal_results}"
    );
    assert!(
        !normal_results.contains("[PgUp/Dn]"),
        "non-overflowing Results advertised paging: {normal_results}"
    );
}

#[test]
fn same_overlay_resize_keeps_confirm_and_results_focus_visible() {
    let choices = (0..12)
        .map(|index| ConfirmChoice {
            accelerator: 'y',
            action: BranchAction::DeleteLocal,
            targets: vec!["feature/target".to_string()],
            remote: None,
            command: format!("git branch -d -- feature/choice-{index}"),
            description: "Delete only this branch if Git reports it safe.".to_string(),
            destructive_target: None,
        })
        .collect();
    let mut confirm = Overlay::Confirm {
        preflight: DeletePreflight::default(),
        choices,
        selected: 8,
        body_scroll: ModalScroll::default(),
        stage: ConfirmStage::Initial,
    };
    let tall_confirm = render_overlay(&mut confirm, 120, 30);
    assert!(tall_confirm.contains("feature/choice-8"), "{tall_confirm}");
    let compact_confirm = render_overlay(&mut confirm, 48, 8);
    assert!(
        compact_confirm.contains("feature/choice-8"),
        "same Confirm state hid its focus after resize: {compact_confirm}"
    );

    let results = (0..12)
        .map(|index| {
            OperationResult::success(
                format!("feature/result-{index}"),
                BranchAction::DeleteLocal,
                "deleted",
            )
        })
        .collect();
    let mut results = Overlay::Results {
        results,
        selected_index: 8,
        expanded_index: None,
        focus: ResultsFocus::Results,
        body_scroll: ModalScroll::default(),
    };
    let tall_results = render_overlay(&mut results, 120, 30);
    assert!(tall_results.contains("feature/result-8"), "{tall_results}");
    let compact_results = render_overlay(&mut results, 48, 8);
    assert!(
        compact_results.contains("feature/result-8"),
        "same Results state hid its focus after resize: {compact_results}"
    );
}

#[test]
fn compact_final_confirmation_keeps_paged_body_and_scroll_hint() {
    let final_choice = ConfirmChoice {
        accelerator: 'r',
        action: BranchAction::DeleteLocalForce,
        targets: vec!["feature/unmerged".to_string()],
        remote: None,
        command: "git branch -D -- feature/unmerged".to_string(),
        description: "Permanently discard unique commits.".to_string(),
        destructive_target: Some(ConfirmTarget::Branch("feature/unmerged".to_string())),
    };
    let mut body_scroll = ModalScroll::default();
    body_scroll.page_down(5);
    let mut overlay = Overlay::Confirm {
        preflight: DeletePreflight { risks: Vec::new() },
        choices: vec![final_choice.clone()],
        selected: 0,
        body_scroll,
        stage: ConfirmStage::FinalDestructive {
            choice: final_choice,
            target: ConfirmTarget::Branch("feature/unmerged".to_string()),
        },
    };

    let rendered = render_overlay(&mut overlay, 48, 7);

    assert!(
        !rendered.contains("FINAL DESTRUCTIVE CONFIRMATION"),
        "render must retain the manually paged final body: {rendered}"
    );
    assert!(rendered.contains("DATA LOSS"), "{rendered}");
    assert!(
        rendered.contains("[y/Enter] Yes  [n/Esc] No  [PgUp/Dn] Scroll"),
        "compact final footer must expose confirm, cancel, and paging: {rendered}"
    );
}
