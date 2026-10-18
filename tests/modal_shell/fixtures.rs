use chrono::Utc;
use git_branch_manager::git::commit_details::{
    ChangedCommitFile, CommitDetailMode, CommitDetails, CommitFileDiff, CommitFileKind,
};
use git_branch_manager::git::graph::GraphCommit;
use git_branch_manager::{
    types::{
        BranchAction, BranchInfo, CacheAudit,
        OperationResult, ProgressUpdate, RemoteBranchInfo, TagInfo, WorktreeInfo,
    },
    ui::{
        confirm::{ConfirmChoice, ConfirmStage, DeletePreflight},
        info_modal::{InfoModalFocus, InfoModalRow},
        list_render::CellContext,
        menu::MenuItem,
        modal::ModalScroll,
        render::Overlay,
        results::ResultsFocus,
    },
};

pub(crate) struct BaseOverlayFixture {
    pub(crate) name: &'static str,
    pub(crate) overlay: Overlay,
    pub(crate) expected_title: &'static str,
    pub(crate) expected_footer: &'static str,
}

pub(crate) fn base_overlay_fixtures() -> Vec<BaseOverlayFixture> {
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
            name: "CommitDetails",
            overlay: Overlay::CommitDetails {
                commit: GraphCommit::default(),
                details: CommitDetails {
                    oid: "abcdef0123456789".to_string(),
                    summary: "Commit summary".to_string(),
                    author_name: "Test User".to_string(),
                    author_email: "test@example.com".to_string(),
                    authored_at: None,
                    message_lines: vec![
                        "Subject".to_string(),
                        "".to_string(),
                        "Body".to_string(),
                        "Line 4".to_string(),
                    ],
                    mode: CommitDetailMode::Commit {
                        oid: "abcdef0123456789".to_string(),
                    },
                    files: vec![ChangedCommitFile {
                        path: "changed.txt".to_string(),
                        old_path: None,
                        kind: CommitFileKind::Modified,
                    }],
                    branch_log: Vec::new(),
                },
                items: vec![menu_item()],
                cursor: 0,
                file_cursor: 0,
                focus: git_branch_manager::ui::commit_details::CommitDetailsFocus::Files,
                scroll: ModalScroll::default(),
            },
            expected_title: "abcdef0 Commit summary",
            expected_footer: "[j/k] Navigate  [Enter] Diff/action  [Tab] Switch  [Esc] Close",
        },
        BaseOverlayFixture {
            name: "CommitDiff",
            overlay: Overlay::CommitDiff {
                commit: GraphCommit::default(),
                details: CommitDetails {
                    oid: "abcdef0123456789".to_string(),
                    summary: "Commit summary".to_string(),
                    author_name: String::new(),
                    author_email: String::new(),
                    authored_at: None,
                    message_lines: Vec::new(),
                    mode: CommitDetailMode::Commit {
                        oid: "abcdef0123456789".to_string(),
                    },
                    files: Vec::new(),
                    branch_log: Vec::new(),
                },
                items: Vec::new(),
                cursor: 0,
                file_cursor: 0,
                focus: git_branch_manager::ui::commit_details::CommitDetailsFocus::Files,
                details_scroll: ModalScroll::default(),
                diff: CommitFileDiff {
                    path: "changed.txt".to_string(),
                    patch: "@@ -1 +1 @@\n-old\n+new".to_string(),
                },
                scroll: ModalScroll::default(),
            },
            expected_title: "Diff: changed.txt",
            expected_footer: "[j/k] Scroll  [PgUp/Dn] Page  [Esc] Back",
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

pub(crate) fn noop_branch_row(
    _: &BranchInfo,
    _: usize,
    _: bool,
    _: bool,
    _: &[usize],
    _: &CellContext,
) -> Vec<ratatui::text::Line<'static>> {
    Vec::new()
}

pub(crate) fn noop_remote_row(
    _: &RemoteBranchInfo,
    _: usize,
    _: bool,
    _: bool,
    _: &[usize],
    _: &CellContext,
) -> Vec<ratatui::text::Line<'static>> {
    Vec::new()
}

pub(crate) fn noop_tag_row(
    _: &TagInfo,
    _: usize,
    _: bool,
    _: bool,
    _: &[usize],
    _: &CellContext,
) -> Vec<ratatui::text::Line<'static>> {
    Vec::new()
}

pub(crate) fn noop_worktree_row(
    _: &WorktreeInfo,
    _: usize,
    _: bool,
    _: bool,
    _: &[usize],
    _: &CellContext,
) -> Vec<ratatui::text::Line<'static>> {
    Vec::new()
}
