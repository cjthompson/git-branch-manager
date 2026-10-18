use git_branch_manager::{
    theme::Theme,
    types::{
        BranchAction,
        OperationResult,
    },
    ui::{
        confirm::{ConfirmChoice, ConfirmStage, DeletePreflight},
        menu::MenuItem,
        modal::{
            draw_modal_shell, ModalFooter, ModalScroll, ModalSpec,
        },
        render::Overlay,
        results::ResultsFocus,
    },
};
use ratatui::{
    backend::TestBackend,
    layout::Rect,
    widgets::Paragraph,
    Terminal,
};

use crate::fixtures::base_overlay_fixtures;
use crate::support::*;

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
