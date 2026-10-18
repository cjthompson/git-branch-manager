use chrono::Utc;
use git_branch_manager::{
    theme::Theme,
    types::{
        BranchAction, TagInfo,
    },
    ui::{
        info_modal::{InfoModalFocus, InfoModalRow},
        menu::MenuItem,
        modal::{
            ModalActionRow, ModalScroll,
        },
        render::Overlay,
    },
};
use ratatui::{
    backend::TestBackend,
    layout::Rect,
    widgets::Paragraph,
    Terminal,
};

use crate::support::*;

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
