use super::*;
use crate::theme::Theme;
use chrono::Utc;

#[test]
fn prefix_style_known_prefixes() {
    let theme = Theme::dark();
    assert!(prefix_style("fix", &theme).is_some());
    assert!(prefix_style("feat", &theme).is_some());
    assert!(prefix_style("feature", &theme).is_some());
    assert!(prefix_style("chore", &theme).is_some());
    assert!(prefix_style("hotfix", &theme).is_some());
    assert!(prefix_style("release", &theme).is_some());
}

#[test]
fn prefix_style_unknown_returns_none() {
    let theme = Theme::dark();
    assert!(prefix_style("unknown", &theme).is_none());
    assert!(prefix_style("main", &theme).is_none());
}

#[test]
fn age_style_recent() {
    let theme = Theme::dark();
    let recent = Utc::now() - chrono::Duration::days(1);
    let style = age_style(&recent, &theme);
    assert_eq!(style.fg, Some(Color::Green));
}

#[test]
fn age_style_week_old() {
    let theme = Theme::dark();
    let date = Utc::now() - chrono::Duration::days(10);
    let style = age_style(&date, &theme);
    assert_eq!(style.fg, Some(Color::Yellow));
}

#[test]
fn age_style_month_old() {
    let theme = Theme::dark();
    let date = Utc::now() - chrono::Duration::days(45);
    let style = age_style(&date, &theme);
    assert_eq!(style.fg, Some(Color::Indexed(208)));
}

#[test]
fn age_style_old() {
    let theme = Theme::dark();
    let date = Utc::now() - chrono::Duration::days(100);
    let style = age_style(&date, &theme);
    assert_eq!(style.fg, Some(Color::Red));
}

#[test]
fn truncate_short_string() {
    assert_eq!(truncate("hello", 10), "hello");
}

#[test]
fn truncate_exact_length() {
    assert_eq!(truncate("hello", 5), "hello");
}

#[test]
fn truncate_long_string() {
    let result = truncate("hello world", 6);
    assert_eq!(result, "hello\u{2026}");
}

#[test]
fn truncate_width_one() {
    assert_eq!(truncate("hello", 1), "\u{2026}");
}

#[test]
fn truncate_width_zero() {
    assert_eq!(truncate("hello", 0), "");
}

#[test]
fn truncate_left_short_string() {
    assert_eq!(truncate_left("hello", 10), "hello");
}

#[test]
fn truncate_left_keeps_tail() {
    // Keeps the last (max_width - 1) chars, prefixed with the ellipsis.
    assert_eq!(truncate_left("hello world", 6), "\u{2026}world");
}

#[test]
fn truncate_left_width_one() {
    assert_eq!(truncate_left("hello", 1), "\u{2026}");
}

#[test]
fn truncate_left_width_zero() {
    assert_eq!(truncate_left("hello", 0), "\u{2026}");
}

#[test]
fn truncate_left_handles_non_ascii_without_panicking() {
    // Regression for a latent panic in draw_executing (ui/executing.rs),
    // which calls truncate_left on a branch's current_item name. Branch
    // names can contain multi-byte UTF-8 (e.g. accented/CJK characters);
    // truncate_left must slice on char boundaries, not bytes.
    let s = "feature/日本語-emoji-🚀-café-branch";
    let result = truncate_left(s, 10);
    assert!(result.chars().count() <= 10, "got: {result:?}");
    assert!(result.starts_with('\u{2026}'), "got: {result:?}");

    // Also verify the short-circuit path (no truncation needed) doesn't panic.
    let short = "日本語";
    assert_eq!(truncate_left(short, 10), "日本語");
}

#[test]
fn abbreviate_path_fits_unchanged() {
    let p = std::path::Path::new("/Users/chris/dev/proj/feat");
    assert_eq!(abbreviate_path(p, 100), "/Users/chris/dev/proj/feat");
}

#[test]
fn abbreviate_path_zero_width() {
    let p = std::path::Path::new("/Users/chris/dev/proj/feat");
    assert_eq!(abbreviate_path(p, 0), "");
}

#[test]
fn abbreviate_path_abbreviates_leading_keeps_tail() {
    let p = std::path::Path::new("/Users/chris/dev/git-branch-manager/.claude/worktrees/feat");
    // Wide enough to keep the tail full but too narrow for the whole path.
    let result = abbreviate_path(p, 45);
    assert!(result.starts_with('/'), "got: {result:?}");
    assert!(result.ends_with("/feat"), "got: {result:?}");
    // Last component must be kept full (not abbreviated to "f").
    assert!(result.contains("/feat"), "got: {result:?}");
    assert!(result.chars().count() <= 45, "got: {result:?}");
}

#[test]
fn abbreviate_path_left_truncates_when_very_narrow() {
    let p = std::path::Path::new("/Users/chris/dev/git-branch-manager/.claude/worktrees/feat");
    let result = abbreviate_path(p, 8);
    assert!(result.starts_with('\u{2026}'), "got: {result:?}");
    assert!(result.chars().count() <= 8, "got: {result:?}");
}

#[test]
fn abbreviate_path_shortens_worktrees_before_clipping_tail() {
    // Real-world shape: long shared prefix, then `worktrees`, then a long
    // worktree name. At this width, keeping `worktrees` full would overflow,
    // so it must be abbreviated too — and the final name stays fully visible.
    let p = std::path::Path::new(
        "/Users/chris/workspace/zen/.claude/worktrees/idempotent-create-payroll-admin-rspec",
    );
    let result = abbreviate_path(p, 52);
    assert!(!result.contains("worktrees"), "got: {result:?}");
    assert!(
        result.contains("/w/"),
        "expected worktrees→w; got: {result:?}"
    );
    assert!(
        result.ends_with("idempotent-create-payroll-admin-rspec"),
        "tail must stay visible; got: {result:?}"
    );
    assert!(result.chars().count() <= 52, "got: {result:?}");
}

#[test]
fn abbreviate_path_single_component_narrow() {
    let p = std::path::Path::new("my-feature");
    let result = abbreviate_path(p, 5);
    // No parents to abbreviate → left-truncated, end visible, no panic.
    assert!(result.starts_with('\u{2026}'), "got: {result:?}");
    assert!(result.ends_with("ure"), "got: {result:?}");
    assert!(result.chars().count() <= 5, "got: {result:?}");
}

#[test]
fn centered_rect_basic() {
    let area = Rect::new(0, 0, 80, 24);
    let r = centered_rect(40, 10, area);
    assert_eq!(r.x, 20);
    assert_eq!(r.y, 7);
    assert_eq!(r.width, 40);
    assert_eq!(r.height, 10);
}

#[test]
fn centered_rect_larger_than_area() {
    let area = Rect::new(0, 0, 40, 10);
    let r = centered_rect(80, 20, area);
    assert_eq!(r.width, 40);
    assert_eq!(r.height, 10);
}

#[test]
fn centered_rect_pct_50() {
    let area = Rect::new(0, 0, 100, 50);
    let r = centered_rect_pct(50, 10, area);
    assert_eq!(r.width, 50);
    assert_eq!(r.x, 25);
}

#[test]
fn render_progress_bar_empty() {
    let bar = render_progress_bar(20, 0, 10);
    assert!(bar.starts_with('['));
    assert!(bar.ends_with(" 0/10"));
}

#[test]
fn render_progress_bar_full() {
    let bar = render_progress_bar(20, 10, 10);
    assert!(bar.ends_with(" 10/10"));
    // Fully filled: no spaces between the last '=' and the closing ']'.
    let inside = bar.split(']').next().unwrap();
    assert!(!inside.contains(' '));
}

#[test]
fn render_progress_bar_zero_total_does_not_panic() {
    let bar = render_progress_bar(20, 0, 0);
    assert!(bar.ends_with(" 0/0"));
}

#[test]
fn key_hint_produces_three_styled_spans() {
    let theme = Theme::dark();
    let spans = key_hint('d', "delete", &theme);

    assert_eq!(spans.len(), 3);

    assert_eq!(spans[0].content.as_ref(), "[");
    assert_eq!(spans[0].style, theme.modal_footer);

    assert_eq!(spans[1].content.as_ref(), "d");
    assert_eq!(spans[1].style, theme.modal_key);

    assert_eq!(spans[2].content.as_ref(), "] delete");
    assert_eq!(spans[2].style, theme.modal_footer);
}

#[test]
fn block_panel_inner_accounts_for_border_and_padding() {
    let theme = Theme::dark();
    let block = block_panel(&theme);

    // Borders::ALL removes 1 cell per side; padding adds 1 more on
    // left/right only (Padding::horizontal(1), no vertical padding).
    let area = Rect::new(0, 0, 20, 10);
    let inner = block.inner(area);
    assert_eq!(inner, Rect::new(2, 1, 16, 8));
}

#[test]
fn block_panel_draws_all_four_borders_with_modal_border_and_surface_styles() {
    let theme = Theme::dark();
    let block = block_panel(&theme);

    let area = Rect::new(0, 0, 10, 5);
    let mut buf = Buffer::empty(area);
    block.render(area, &mut buf);

    // Top-left corner: a border glyph styled with the theme's modal border fg.
    let corner = buf.cell((0, 0)).unwrap();
    assert_ne!(
        corner.symbol(),
        " ",
        "expected a border glyph at the top-left corner"
    );
    assert_eq!(corner.fg, theme.modal_border.fg.unwrap());
    assert_eq!(corner.bg, theme.modal_surface.bg.unwrap());

    // Borders::ALL: mid-point of every edge should be a non-blank glyph.
    assert_ne!(
        buf.cell((5, 0)).unwrap().symbol(),
        " ",
        "top border missing"
    );
    assert_ne!(
        buf.cell((5, 4)).unwrap().symbol(),
        " ",
        "bottom border missing"
    );
    assert_ne!(
        buf.cell((0, 2)).unwrap().symbol(),
        " ",
        "left border missing"
    );
    assert_ne!(
        buf.cell((9, 2)).unwrap().symbol(),
        " ",
        "right border missing"
    );
}
