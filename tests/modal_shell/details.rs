use chrono::Utc;
use git_branch_manager::git::commit_details::{
    ChangedCommitFile, CommitDetailMode, CommitDetails, CommitFileKind,
};
use git_branch_manager::git::graph::GraphCommit;
use git_branch_manager::{
    theme::Theme,
    types::{
        BranchAction, FailureCause,
        OperationResult,
    },
    ui::{
        menu::MenuItem,
        modal::ModalScroll,
        render::Overlay,
        results::ResultsFocus,
    },
};

use crate::support::*;

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

#[test]
fn branch_tip_details_render_message_metadata_log_and_files() {
    let details = CommitDetails {
        oid: "abcdef0123456789".into(),
        summary: "tip summary".into(),
        author_name: "Branch Author".into(),
        author_email: "author@example.com".into(),
        authored_at: Some(Utc::now().fixed_offset()),
        message_lines: vec![
            "message one".into(),
            "message two".into(),
            "message three".into(),
            "message four".into(),
        ],
        mode: CommitDetailMode::BranchTip {
            branch: "feature/details".into(),
            base: "main".into(),
            merge_base: "baseoid".into(),
            tip: "tipoid".into(),
        },
        files: vec![ChangedCommitFile {
            path: "aggregate.txt".into(),
            old_path: None,
            kind: CommitFileKind::Modified,
        }],
        branch_log: vec![
            "abc123 latest branch change".into(),
            "def456 earlier branch change".into(),
        ],
    };
    let mut overlay = Overlay::CommitDetails {
        commit: GraphCommit {
            oid: details.oid.clone(),
            summary: details.summary.clone(),
            ..GraphCommit::default()
        },
        details,
        items: vec![MenuItem {
            label: "Delete local".into(),
            shortcut: Some('d'),
            action: BranchAction::DeleteLocal,
            target: "feature/details".into(),
            remote: None,
            enabled: true,
            reason: None,
        }],
        cursor: 0,
        file_cursor: 0,
        focus: git_branch_manager::ui::commit_details::CommitDetailsFocus::Files,
        scroll: ModalScroll::default(),
    };
    let buffer = render_overlay_buffer(&mut overlay, 100, 32, &Theme::dark());
    let rendered = buffer
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    for expected in [
        "tip summary",
        "Branch Author",
        "author@example.com",
        "Date ",
        "Message",
        "message one",
        "message two",
        "message three",
        "message four",
        "Branch tip feature/details",
        "abc123 latest branch change",
        "aggregate.txt",
    ] {
        assert!(rendered.contains(expected), "missing {expected:?}");
    }
}

#[test]
fn commit_details_page_position_survives_render_until_focus_moves() {
    let details = CommitDetails {
        oid: "abcdef0123456789".into(),
        summary: "long log".into(),
        author_name: "Author".into(),
        author_email: "author@example.com".into(),
        authored_at: Some(Utc::now().fixed_offset()),
        message_lines: vec![
            "message at top".into(),
            "second line".into(),
            "third line".into(),
            "fourth line".into(),
        ],
        mode: CommitDetailMode::BranchTip {
            branch: "feature/long".into(),
            base: "main".into(),
            merge_base: "baseoid".into(),
            tip: "tipoid".into(),
        },
        files: vec![ChangedCommitFile {
            path: "selected.txt".into(),
            old_path: None,
            kind: CommitFileKind::Modified,
        }],
        branch_log: (0..36)
            .map(|index| format!("commit {index:02} change"))
            .collect(),
    };
    let mut overlay = Overlay::CommitDetails {
        commit: GraphCommit {
            oid: details.oid.clone(),
            summary: details.summary.clone(),
            ..GraphCommit::default()
        },
        details: details.clone(),
        items: vec![MenuItem {
            label: "Delete local".into(),
            shortcut: Some('d'),
            action: BranchAction::DeleteLocal,
            target: "feature/long".into(),
            remote: None,
            enabled: true,
            reason: None,
        }],
        cursor: 0,
        file_cursor: 0,
        focus: git_branch_manager::ui::commit_details::CommitDetailsFocus::Files,
        scroll: ModalScroll::default(),
    };
    let _ = render_overlay_buffer(&mut overlay, 100, 28, &Theme::dark());
    let Overlay::CommitDetails {
        scroll,
        file_cursor,
        focus,
        ..
    } = &mut overlay
    else {
        unreachable!()
    };
    *focus = git_branch_manager::ui::commit_details::CommitDetailsFocus::Files;
    *file_cursor = 0;
    scroll.page_up(u16::MAX);
    let top_buffer = render_overlay_buffer(&mut overlay, 100, 28, &Theme::dark());
    let Overlay::CommitDetails {
        scroll,
        file_cursor,
        ..
    } = &overlay
    else {
        unreachable!()
    };
    assert_eq!(scroll.offset, 0);
    assert_eq!(*file_cursor, 0);
    let top_text = top_buffer
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(top_text.contains("message at top"));
    assert!(top_text.contains("commit 00 change"));

    let Overlay::CommitDetails { focus, .. } = &mut overlay else {
        unreachable!()
    };
    *focus = git_branch_manager::ui::commit_details::CommitDetailsFocus::Actions;
    let action_buffer = render_overlay_buffer(&mut overlay, 100, 28, &Theme::dark());
    assert!(action_buffer
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>()
        .contains("Delete local"));
}
