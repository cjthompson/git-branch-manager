use chrono::Utc;
use git_branch_manager::{
    config::Config,
    job_queue::JobStatusView,
    symbols::SymbolSet,
    theme::Theme,
    types::{
        BranchAction, BranchInfo, CacheAudit, ChangedFile, ChangedFileKind, FailureCause,
        OperationResult, ProgressUpdate, RemoteBranchInfo, TagInfo, WorktreeInfo,
    },
    ui::{
        confirm::{ConfirmChoice, ConfirmStage, ConfirmTarget, DeletePreflight, DeleteRisk},
        info_modal::{InfoHitRegion, InfoModalFocus, InfoModalRow},
        list_render::{CellContext, RowRenderer},
        menu::MenuItem,
        modal::{
            draw_modal_shell, ModalActionRow, ModalAreas, ModalFooter, ModalScroll, ModalSpec,
        },
        render::{draw, Overlay, RenderContext},
        results::ResultsFocus,
    },
    view::{
        column::ColumnDef, filter::merge_tokens, graph::GraphState, list_state::ListState, ViewId,
    },
};
use ratatui::{
    backend::TestBackend,
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Terminal,
};

struct BaseOverlayFixture {
    name: &'static str,
    overlay: Overlay,
    expected_title: &'static str,
    expected_footer: &'static str,
}

fn base_overlay_fixtures() -> Vec<BaseOverlayFixture> {
    let menu_item = || MenuItem {
        label: "Delete local".to_string(),
        shortcut: Some('d'),
        action: BranchAction::DeleteLocal,
        target: "feature/test".to_string(),
        remote: None,
        enabled: true,
        reason: None,
    };

    vec![
        BaseOverlayFixture {
            name: "Help",
            overlay: Overlay::Help { scroll: 0 },
            expected_title: "Help",
            expected_footer: "[j/k] Scroll  [PgUp/Dn] Page  [Esc] Close",
        },
        BaseOverlayFixture {
            name: "Menu",
            overlay: Overlay::Menu {
                items: vec![menu_item()],
                cursor: 0,
            },
            expected_title: "Actions",
            expected_footer: "[j/k] Navigate  [Enter] Select  [Esc] Close",
        },
        BaseOverlayFixture {
            name: "InfoModal",
            overlay: Overlay::InfoModal {
                items: vec![menu_item()],
                cursor: 0,
                info_cursor: 0,
                focus: InfoModalFocus::Actions,
                row: InfoModalRow::Tag(TagInfo {
                    name: "v1.0.0".to_string(),
                    commit_hash: "0123456789abcdef".to_string(),
                    date: Utc::now(),
                    message: None,
                    is_annotated: false,
                }),
            },
            expected_title: "v1.0.0",
            expected_footer: "[Tab] Switch  [j/k] Navigate  [Enter] Invoke  [Esc] Close",
        },
        BaseOverlayFixture {
            name: "Confirm",
            overlay: Overlay::Confirm {
                preflight: DeletePreflight { risks: Vec::new() },
                choices: vec![ConfirmChoice {
                    accelerator: 'y',
                    action: BranchAction::DeleteLocal,
                    targets: vec!["feature/test".to_string()],
                    remote: None,
                    command: "git branch -d -- feature/test".to_string(),
                    description: "Delete only this branch if Git reports it safe.".to_string(),
                    destructive_target: None,
                }],
                selected: 0,
                body_scroll: ModalScroll::default(),
                stage: ConfirmStage::Initial,
            },
            expected_title: "Confirm action",
            expected_footer: "[j/k] Navigate  [Enter] Select  [Esc] Cancel",
        },
        BaseOverlayFixture {
            name: "Executing",
            overlay: Overlay::Executing {
                label: "Deleting branches".to_string(),
                progress: Some(ProgressUpdate {
                    completed: 1,
                    total: 2,
                    current_item: "feature/test".to_string(),
                }),
            },
            expected_title: "Running",
            expected_footer: "[Esc] Cancel",
        },
        BaseOverlayFixture {
            name: "Results",
            overlay: Overlay::Results {
                results: vec![OperationResult::success(
                    "feature/test",
                    BranchAction::DeleteLocal,
                    "deleted",
                )],
                selected_index: 0,
                expanded_index: None,
                focus: ResultsFocus::Results,
                body_scroll: ModalScroll::default(),
            },
            expected_title: "Results",
            expected_footer: "[j/k] Navigate  [Enter] Expand  [Tab] Actions  [Esc] Close",
        },
        BaseOverlayFixture {
            name: "Settings",
            overlay: Overlay::Settings { cursor: 0 },
            expected_title: "Settings",
            expected_footer: "[j/k] Select  [Enter] Choose  [Esc] Close",
        },
        BaseOverlayFixture {
            name: "Filter",
            overlay: Overlay::Filter,
            expected_title: "Filters",
            expected_footer: "[j/k] Select  [Enter] Choose  [Esc] Close",
        },
        BaseOverlayFixture {
            name: "GraphOptions",
            overlay: Overlay::GraphOptions {
                cursor: 0,
                include_remotes: false,
            },
            expected_title: "Graph options",
            expected_footer: "[j/k] Navigate  [Space] Toggle  [Enter] Apply  [Esc] Cancel",
        },
        BaseOverlayFixture {
            name: "Diagnostics",
            overlay: Overlay::Diagnostics { cursor: 0 },
            expected_title: "Diagnostics",
            expected_footer: "[j/k] Navigate  [Enter] Run  [Esc] Close",
        },
        BaseOverlayFixture {
            name: "DiagnosticsReport",
            overlay: Overlay::DiagnosticsReport {
                audit: CacheAudit::default(),
                scroll: 0,
            },
            expected_title: "Cache accuracy",
            expected_footer: "[Esc] Close",
        },
    ]
}

fn noop_branch_row(
    _: &BranchInfo,
    _: usize,
    _: bool,
    _: bool,
    _: &[usize],
    _: &CellContext,
) -> Vec<ratatui::text::Line<'static>> {
    Vec::new()
}

fn noop_remote_row(
    _: &RemoteBranchInfo,
    _: usize,
    _: bool,
    _: bool,
    _: &[usize],
    _: &CellContext,
) -> Vec<ratatui::text::Line<'static>> {
    Vec::new()
}

fn noop_tag_row(
    _: &TagInfo,
    _: usize,
    _: bool,
    _: bool,
    _: &[usize],
    _: &CellContext,
) -> Vec<ratatui::text::Line<'static>> {
    Vec::new()
}

fn noop_worktree_row(
    _: &WorktreeInfo,
    _: usize,
    _: bool,
    _: bool,
    _: &[usize],
    _: &CellContext,
) -> Vec<ratatui::text::Line<'static>> {
    Vec::new()
}

fn render_overlay(overlay: &mut Overlay, width: u16, height: u16) -> String {
    let theme = Theme::dark();
    render_overlay_buffer(overlay, width, height, &theme)
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

fn render_overlay_buffer(overlay: &mut Overlay, width: u16, height: u16, theme: &Theme) -> Buffer {
    let symbols = SymbolSet::ascii();
    let config = Config::default();
    let mut branches = ListState::empty();
    let mut remotes = ListState::empty();
    let mut tags = ListState::empty();
    let mut worktrees = ListState::empty();
    let mut graph = GraphState::new();
    let branch_columns: Vec<ColumnDef<BranchInfo>> = Vec::new();
    let remote_columns: Vec<ColumnDef<RemoteBranchInfo>> = Vec::new();
    let tag_columns: Vec<ColumnDef<TagInfo>> = Vec::new();
    let worktree_columns: Vec<ColumnDef<WorktreeInfo>> = Vec::new();
    let filter_tokens = merge_tokens();
    let mut info_hit_regions = Vec::<InfoHitRegion>::new();
    let mut info_modal_scroll_offset = 0;
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();

    terminal
        .draw(|frame| {
            let mut ctx = RenderContext {
                active_view: ViewId::Branches,
                overlay: Some(overlay),
                toast: None,
                theme,
                symbols: &symbols,
                config: &config,
                info_copied_msg: None,
                info_hit_regions: &mut info_hit_regions,
                info_modal_scroll_offset: &mut info_modal_scroll_offset,
                job_status: JobStatusView {
                    visible: false,
                    current_label: None,
                    current_progress: None,
                    queued_count: 0,
                    targets_done: 0,
                    targets_total: 0,
                    summary: None,
                },
                branches: &mut branches,
                remotes: &mut remotes,
                tags: &mut tags,
                worktrees: &mut worktrees,
                graph: &mut graph,
                branch_columns: &branch_columns,
                remote_columns: &remote_columns,
                tag_columns: &tag_columns,
                worktree_columns: &worktree_columns,
                active_filter_tokens: &filter_tokens,
                render_branch_row: noop_branch_row as RowRenderer<BranchInfo>,
                render_remote_row: noop_remote_row as RowRenderer<RemoteBranchInfo>,
                render_tag_row: noop_tag_row as RowRenderer<TagInfo>,
                render_worktree_row: noop_worktree_row as RowRenderer<WorktreeInfo>,
            };
            draw(frame, &mut ctx);
        })
        .unwrap();

    terminal.backend().buffer().clone()
}

fn render_modal_semantic_sample(theme: &Theme) -> (Buffer, ModalAreas) {
    let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();
    let mut modal_areas = None;

    terminal
        .draw(|frame| {
            let areas = draw_modal_shell(
                frame,
                &ModalSpec::new(
                    "Modal semantic sample",
                    ModalFooter::hints(&[("Esc", "Close")]),
                    44,
                    10,
                ),
                theme,
            );
            modal_areas = Some(areas);
            frame.render_widget(
                Paragraph::new(vec![
                    ModalActionRow::new(Some('r'), "Run command", "Selected detail")
                        .render(true, theme),
                    ModalActionRow::new(Some('u'), "Unavailable action", "Cannot run now")
                        .render_with_availability(false, false, theme),
                    Line::from(Span::styled(
                        "Warning: requires attention",
                        theme.modal_warning,
                    )),
                ]),
                areas.body,
            );
        })
        .unwrap();

    (
        terminal.backend().buffer().clone(),
        modal_areas.expect("modal shell must report its areas"),
    )
}

#[test]
fn every_base_overlay_uses_the_shared_shell_and_footer() {
    for mut fixture in base_overlay_fixtures() {
        let rendered = render_overlay(&mut fixture.overlay, 80, 20);
        assert!(
            rendered.contains(fixture.expected_footer),
            "{} did not render expected footer: {}\n{rendered}",
            fixture.name,
            fixture.expected_footer,
        );
        assert!(
            !rendered.contains("(press "),
            "{} rendered parenthetical inline instruction: {rendered}",
            fixture.name,
        );
    }
}

#[test]
fn settings_and_filter_render_selected_action_rows_with_compact_controls() {
    let theme = Theme::dark();
    let mut settings = Overlay::Settings { cursor: 1 };
    let settings_buffer = render_overlay_buffer(&mut settings, 48, 8, &theme);
    let (settings_x, settings_y) = text_cell(&settings_buffer, "Theme");
    assert_eq!(
        settings_buffer[(settings_x, settings_y)].bg,
        theme.modal_action_selected.bg.unwrap(),
        "selected Settings row must use the action-list background"
    );
    let settings_rendered: String = settings_buffer
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(
        settings_rendered.contains("[j/k] Select  [Enter] Choose  [Esc] Close"),
        "{settings_rendered}"
    );

    let mut filter = Overlay::FilterSelection { cursor: 1 };
    let filter_buffer = render_overlay_buffer(&mut filter, 48, 8, &theme);
    let (filter_x, filter_y) = text_cell(&filter_buffer, "Squash-merged");
    assert_eq!(
        filter_buffer[(filter_x, filter_y)].bg,
        theme.modal_action_selected.bg.unwrap(),
        "selected Filter row must use the action-list background"
    );
    let filter_rendered: String = filter_buffer
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(
        filter_rendered.contains("[j/k] Select  [Enter] Choose  [Esc] Close"),
        "{filter_rendered}"
    );
}

#[test]
fn help_scroll_offset_does_not_render_an_empty_body_after_repeated_page_down() {
    let mut overlay = Overlay::Help { scroll: usize::MAX };
    let rendered = render_overlay(&mut overlay, 80, 20);

    assert!(
        rendered.contains("Quit"),
        "a stale Help offset must clamp to the final visible content, not render an empty body: {rendered}"
    );
    assert!(
        matches!(overlay, Overlay::Help { scroll } if scroll < 100),
        "rendering must also clamp the persisted Help offset: {overlay:?}"
    );
}

#[test]
fn constrained_menu_keeps_the_selected_action_and_shared_footer_visible() {
    let items = (0..10)
        .map(|index| MenuItem {
            label: format!("Action {index}"),
            shortcut: None,
            action: BranchAction::DeleteLocal,
            target: format!("feature/{index}"),
            remote: None,
            enabled: true,
            reason: None,
        })
        .collect();
    let mut overlay = Overlay::Menu { items, cursor: 9 };
    let rendered = render_overlay(&mut overlay, 80, 8);

    assert!(
        rendered.contains("Action 9"),
        "selected action is clipped: {rendered}"
    );
    assert!(
        rendered.contains("[j/k] Navigate  [Enter] Select  [Esc] Close"),
        "shared footer is clipped: {rendered}"
    );
}

#[test]
fn dirty_diagnostics_report_uses_the_fix_and_scroll_footer() {
    let mut overlay = Overlay::DiagnosticsReport {
        audit: CacheAudit {
            orphans: vec!["feature/stale".to_string()],
            ..CacheAudit::default()
        },
        scroll: 0,
    };
    let rendered = render_overlay(&mut overlay, 80, 20);

    assert!(
        rendered.contains("[f] Fix & reload  [j/k] Scroll  [Esc] Close"),
        "dirty report did not render its shared footer: {rendered}"
    );
}

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

#[test]
fn accordion_details_show_typed_summary_before_raw_git_message() {
    let raw_message = "fatal: raw git detail sentinel";
    let mut overlay = Overlay::Results {
        results: vec![OperationResult::failure(
            "feature/unmerged",
            BranchAction::DeleteLocal,
            FailureCause::NotMerged,
            raw_message,
        )],
        selected_index: 0,
        expanded_index: Some(0),
        focus: ResultsFocus::Actions {
            selected_index: 1,
            raw_details_visible: true,
        },
        body_scroll: ModalScroll::default(),
    };

    let rendered = render_overlay(&mut overlay, 100, 24);
    let typed_position = rendered
        .find("Not merged into base branch")
        .expect("typed summary must be rendered");
    let raw_position = rendered
        .find(raw_message)
        .expect("requested raw Git details must be rendered");

    assert!(
        typed_position < raw_position,
        "typed summary must precede the raw Git message: {rendered}"
    );
}

fn row_contents(buffer: &Buffer, y: u16) -> String {
    (0..buffer.area().width)
        .map(|x| buffer[(x, y)].symbol())
        .collect()
}

fn assert_cell_style(buffer: &Buffer, x: u16, y: u16, style: Style) {
    let cell = &buffer[(x, y)];
    assert_eq!(cell.fg, style.fg.unwrap_or(Color::Reset));
    assert_eq!(cell.bg, style.bg.unwrap_or(Color::Reset));
    assert_eq!(cell.modifier, style.add_modifier);
}

fn assert_cell_foreground(buffer: &Buffer, x: u16, y: u16, style: Style) {
    assert_eq!(
        buffer[(x, y)].fg,
        style.fg.unwrap_or(Color::Reset),
        "at ({x}, {y})"
    );
}

fn assert_rendered_foreground(
    buffer: &Buffer,
    text: &str,
    style: Style,
    role: &str,
    theme_name: &str,
) {
    let (x, y) = text_cell(buffer, text);
    assert_eq!(
        buffer[(x, y)].fg,
        style.fg.unwrap_or(Color::Reset),
        "{role} {text:?} has the wrong foreground in {theme_name}"
    );
}

fn contrast_ratio(foreground: Color, background: Color) -> f64 {
    let foreground_luminance = relative_luminance(foreground);
    let background_luminance = relative_luminance(background);
    (foreground_luminance.max(background_luminance) + 0.05)
        / (foreground_luminance.min(background_luminance) + 0.05)
}

fn relative_luminance(color: Color) -> f64 {
    fn linearize(channel: u8) -> f64 {
        let channel = f64::from(channel) / 255.0;
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    }

    let (red, green, blue) = match color {
        Color::Rgb(red, green, blue) => (red, green, blue),
        Color::Black => (0, 0, 0),
        Color::White => (255, 255, 255),
        other => panic!("contrast helper only supports RGB colors, got {other:?}"),
    };
    0.2126 * linearize(red) + 0.7152 * linearize(green) + 0.0722 * linearize(blue)
}

fn text_cell(buffer: &Buffer, text: &str) -> (u16, u16) {
    for y in 0..buffer.area().height {
        let row = row_contents(buffer, y);
        if let Some(byte_offset) = row.find(text) {
            return (row[..byte_offset].chars().count() as u16, y);
        }
    }
    panic!("did not render {text:?}");
}

#[test]
fn shell_pins_footer_when_body_is_taller_than_viewport() {
    let mut terminal = Terminal::new(TestBackend::new(40, 8)).unwrap();
    let theme = Theme::dark();
    let mut modal_areas = None;
    terminal
        .draw(|frame| {
            let areas = draw_modal_shell(
                frame,
                &ModalSpec::new("Example", ModalFooter::hints(&[("Esc", "Close")]), 30, 8),
                &theme,
            );
            modal_areas = Some(areas);
            frame.render_widget(
                Paragraph::new("one\ntwo\nthree\nfour\nfive\nsix"),
                areas.body,
            );
        })
        .unwrap();

    let buffer = terminal.backend().buffer();
    assert_eq!(modal_areas.unwrap().outer, Rect::new(5, 0, 30, 8));
    assert_eq!(modal_areas.unwrap().body, Rect::new(7, 1, 26, 5));
    assert_eq!(modal_areas.unwrap().footer, Rect::new(7, 6, 26, 1));
    assert!(row_contents(buffer, 0).contains("Example"));
    assert!(row_contents(buffer, 6).contains("[Esc] Close"));
    let footer_key = &buffer[(8, 6)];
    assert_eq!(footer_key.fg, theme.modal_key.fg.unwrap());
    assert_eq!(footer_key.bg, theme.modal_surface.bg.unwrap());
    assert_eq!(footer_key.modifier, theme.modal_key.add_modifier);
    assert!(row_contents(buffer, 5).contains("five"));
    assert!(!buffer
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>()
        .contains("six"));
}

#[test]
fn shell_keeps_body_and_footer_in_separate_rows_at_terminal_extremes() {
    let theme = Theme::dark();
    for (width, height) in [(48, 8), (80, 24), (160, 50)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let mut areas = None;
        terminal
            .draw(|frame| {
                areas = Some(draw_modal_shell(
                    frame,
                    &ModalSpec::new("Geometry", ModalFooter::hints(&[("Esc", "Close")]), 84, 22),
                    &theme,
                ));
            })
            .unwrap();

        let areas = areas.expect("shell must report its body and footer areas");
        assert_eq!(
            areas.body.bottom(),
            areas.footer.y,
            "body overlaps footer at {width}x{height}"
        );
        assert_eq!(areas.footer.height, 1, "footer height at {width}x{height}");
        assert!(
            areas.footer.bottom() <= areas.outer.bottom(),
            "footer escapes shell at {width}x{height}"
        );
    }
}

#[test]
fn scroll_clamps_and_keeps_selected_row_visible() {
    let mut scroll = ModalScroll::default();
    scroll.offset = 99;
    scroll.clamp(10, 3);
    assert_eq!(scroll.offset, 7);

    scroll.offset = 0;
    scroll.ensure_visible(9, 10, 3);
    assert_eq!(scroll.offset, 7);
    assert!(scroll.offset <= 9 && 9 < scroll.offset + 3);
}

#[test]
fn action_row_composes_key_command_and_description() {
    let theme = Theme::dark();
    let row = ModalActionRow::new(
        Some('r'),
        "Review forced removal",
        "Opens the final data-loss confirmation.",
    );
    let mut terminal = Terminal::new(TestBackend::new(80, 2)).unwrap();
    terminal
        .draw(|frame| {
            frame.render_widget(
                Paragraph::new(row.render(false, &theme)),
                Rect::new(0, 0, 80, 1),
            );
        })
        .unwrap();

    let buffer = terminal.backend().buffer();
    let contents = row_contents(buffer, 0);
    let key_x = contents.find("[r]").unwrap() as u16;
    let command_x = contents.find("Review forced removal").unwrap() as u16;
    let description_x = contents
        .find("Opens the final data-loss confirmation.")
        .unwrap() as u16;

    assert_cell_style(buffer, key_x, 0, theme.modal_key);
    assert_cell_style(buffer, command_x, 0, theme.modal_command);
    assert_cell_style(buffer, description_x, 0, theme.modal_secondary);
}

#[test]
fn unavailable_action_rows_mute_each_semantic_span_in_menu_and_info_modal() {
    for theme in [
        Theme::dark(),
        Theme::light(),
        Theme::solarized(),
        Theme::dracula(),
    ] {
        let enabled = MenuItem {
            label: "Delete local".to_string(),
            shortcut: Some('d'),
            action: BranchAction::DeleteLocal,
            target: "feature/test".to_string(),
            remote: None,
            enabled: true,
            reason: Some("available action".to_string()),
        };
        let unavailable = MenuItem {
            label: "Delete local unavailable".to_string(),
            shortcut: Some('u'),
            action: BranchAction::DeleteLocal,
            target: "feature/test".to_string(),
            remote: None,
            enabled: false,
            reason: Some("worktree is dirty".to_string()),
        };

        let mut menu = Overlay::Menu {
            items: vec![enabled.clone(), unavailable.clone()],
            cursor: 1,
        };
        let menu_buffer = render_overlay_buffer(&mut menu, 80, 10, &theme);
        for (available_text, unavailable_text) in [
            ("[d]", "[u]"),
            ("Delete local", "Delete local unavailable"),
            ("available action", "worktree is dirty"),
        ] {
            assert_unavailable_cell_differs(
                &menu_buffer,
                available_text,
                unavailable_text,
                theme.modal_action_unavailable,
                theme.modal_surface,
            );
        }

        let mut info_modal = Overlay::InfoModal {
            items: vec![enabled, unavailable],
            cursor: 1,
            info_cursor: 0,
            focus: InfoModalFocus::Actions,
            row: InfoModalRow::Tag(TagInfo {
                name: "v1.0.0".to_string(),
                commit_hash: "0123456789abcdef".to_string(),
                date: Utc::now(),
                message: None,
                is_annotated: false,
            }),
        };
        let info_buffer = render_overlay_buffer(&mut info_modal, 100, 24, &theme);
        for (available_text, unavailable_text) in [
            ("[d]", "[u]"),
            ("Delete local", "Delete local unavailable"),
            ("available action", "worktree is dirty"),
        ] {
            assert_unavailable_cell_differs(
                &info_buffer,
                available_text,
                unavailable_text,
                theme.modal_action_unavailable,
                theme.modal_surface,
            );
        }
    }
}

fn assert_unavailable_cell_differs(
    buffer: &Buffer,
    available_text: &str,
    unavailable_text: &str,
    unavailable_style: Style,
    surface_style: Style,
) {
    let (available_x, available_y) = text_cell(buffer, available_text);
    let (unavailable_x, unavailable_y) = text_cell(buffer, unavailable_text);
    let available = &buffer[(available_x, available_y)];
    let unavailable = &buffer[(unavailable_x, unavailable_y)];

    assert_eq!(
        unavailable.fg,
        unavailable_style.fg.unwrap_or(Color::Reset),
        "unavailable span foreground at ({unavailable_x}, {unavailable_y})"
    );
    assert_eq!(
        unavailable.bg,
        surface_style.bg.unwrap_or(Color::Reset),
        "disabled row must inherit the modal surface, not selected background"
    );
    assert!(
        unavailable
            .modifier
            .contains(unavailable_style.add_modifier),
        "unavailable span must include muted modifiers at ({unavailable_x}, {unavailable_y})"
    );
    assert_ne!(
        (available.fg, available.bg, available.modifier),
        (unavailable.fg, unavailable.bg, unavailable.modifier),
        "unavailable span must differ from enabled span"
    );
}

#[test]
fn modal_action_row_preserves_selection_background_across_key_command_and_detail_spans() {
    for theme in [
        Theme::dark(),
        Theme::light(),
        Theme::solarized(),
        Theme::dracula(),
    ] {
        let row = ModalActionRow::new(Some('r'), "Run command", "Secondary detail");
        let mut terminal = Terminal::new(TestBackend::new(80, 1)).unwrap();
        terminal
            .draw(|frame| {
                frame.render_widget(
                    Paragraph::new(row.render(true, &theme)),
                    Rect::new(0, 0, 80, 1),
                );
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let selected_bg = theme
            .modal_action_selected
            .bg
            .expect("modal selected rows require a background");
        for text in ["[r]", "Run command", "Secondary detail"] {
            let (x, y) = text_cell(buffer, text);
            assert_eq!(
                buffer[(x, y)].bg,
                selected_bg,
                "{} lost its selected background in {}",
                text,
                theme.name
            );
        }
    }
}

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

#[test]
fn all_overlays_keep_title_and_footer_chrome_at_terminal_extremes() {
    for (width, height) in [(48, 8), (80, 24), (160, 50)] {
        for mut fixture in base_overlay_fixtures() {
            let rendered = render_overlay(&mut fixture.overlay, width, height);
            assert!(
                rendered.contains(fixture.expected_title),
                "{} lost its title at {width}x{height}: {rendered}",
                fixture.name,
            );
            let first_footer_hint = fixture
                .expected_footer
                .split(' ')
                .next()
                .expect("footer hints are non-empty");
            assert!(
                rendered.contains(first_footer_hint),
                "{} lost its footer at {width}x{height}: {rendered}",
                fixture.name,
            );
            assert!(
                !rendered.contains("Press ") && !rendered.contains("press "),
                "{} restored an inline key instruction at {width}x{height}: {rendered}",
                fixture.name,
            );
        }
    }
}

#[test]
fn terminal_extremes_keep_focused_menu_confirm_and_results_rows_visible_without_footer_overlap() {
    for (width, height) in [(48, 8), (80, 24), (160, 50)] {
        let items = (0..10)
            .map(|index| MenuItem {
                label: format!("Menu {index}"),
                shortcut: None,
                action: BranchAction::DeleteLocal,
                target: format!("feature/{index}"),
                remote: None,
                enabled: true,
                reason: None,
            })
            .collect();
        let mut menu = Overlay::Menu { items, cursor: 9 };
        let menu_rendered = render_overlay(&mut menu, width, height);
        assert!(
            menu_rendered.contains("Menu 9"),
            "focused menu row was clipped at {width}x{height}: {menu_rendered}"
        );

        let choices = (0..10)
            .map(|index| ConfirmChoice {
                accelerator: 'y',
                action: BranchAction::DeleteLocal,
                targets: vec![format!("feature/{index}")],
                remote: None,
                command: format!("git branch -d -- feature/{index}"),
                description: "Keep the selected choice visible.".to_string(),
                destructive_target: None,
            })
            .collect();
        let mut confirm = Overlay::Confirm {
            preflight: DeletePreflight::default(),
            choices,
            selected: 9,
            body_scroll: ModalScroll::default(),
            stage: ConfirmStage::Initial,
        };
        let confirm_rendered = render_overlay(&mut confirm, width, height);
        assert!(
            confirm_rendered.contains("feature/9"),
            "focused confirmation row was clipped at {width}x{height}: {confirm_rendered}"
        );
        assert!(
            confirm_rendered.contains("[j/k]"),
            "confirmation footer overlapped the body at {width}x{height}: {confirm_rendered}"
        );

        let results = (0..10)
            .map(|index| {
                OperationResult::success(
                    format!("feature/{index}"),
                    BranchAction::DeleteLocal,
                    "deleted",
                )
            })
            .collect();
        let mut results = Overlay::Results {
            results,
            selected_index: 9,
            expanded_index: None,
            focus: ResultsFocus::Results,
            body_scroll: ModalScroll::default(),
        };
        let results_rendered = render_overlay(&mut results, width, height);
        assert!(
            results_rendered.contains("feature/9"),
            "focused results row was clipped at {width}x{height}: {results_rendered}"
        );
        assert!(
            results_rendered.contains("[j/k]"),
            "results footer overlapped the body at {width}x{height}: {results_rendered}"
        );
    }
}

#[test]
fn unicode_confirmation_targets_render_without_panicking_at_terminal_extremes() {
    for (width, height) in [(48, 8), (80, 24), (160, 50)] {
        let mut overlay = Overlay::Confirm {
            preflight: DeletePreflight::default(),
            choices: vec![ConfirmChoice {
                accelerator: 'y',
                action: BranchAction::DeleteLocal,
                targets: vec!["feature/日本語-🦀".to_string()],
                remote: None,
                command: "git branch -d -- feature/日本語-🦀".to_string(),
                description: "Unicode targets remain safe to display.".to_string(),
                destructive_target: None,
            }],
            selected: 0,
            body_scroll: ModalScroll::default(),
            stage: ConfirmStage::Initial,
        };
        let rendered = render_overlay(&mut overlay, width, height);
        assert!(rendered.contains("Confirm action"));
        assert!(rendered.contains("[j/k]"));
    }
}
