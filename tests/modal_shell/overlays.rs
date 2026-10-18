use git_branch_manager::{
    theme::Theme,
    types::{
        BranchAction, CacheAudit,
    },
    ui::{
        menu::MenuItem,
        render::Overlay,
    },
};

use crate::fixtures::base_overlay_fixtures;
use crate::support::*;

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
