use git_branch_manager::{
    config::Config,
    job_queue::JobStatusView,
    symbols::SymbolSet,
    theme::Theme,
    types::{
        BranchInfo, RemoteBranchInfo, TagInfo, WorktreeInfo,
    },
    ui::{
        info_modal::InfoHitRegion,
        list_render::RowRenderer,
        modal::{
            draw_modal_shell, ModalActionRow, ModalAreas, ModalFooter, ModalSpec,
        },
        render::{draw, Overlay, RenderContext},
    },
    view::{
        column::ColumnDef, filter::merge_tokens, graph::GraphState, list_state::ListState, ViewId,
    },
};
use ratatui::{
    backend::TestBackend,
    buffer::Buffer,
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Terminal,
};

use crate::fixtures::{noop_branch_row, noop_remote_row, noop_tag_row, noop_worktree_row};

pub(crate) fn render_overlay(overlay: &mut Overlay, width: u16, height: u16) -> String {
    let theme = Theme::dark();
    render_overlay_buffer(overlay, width, height, &theme)
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

pub(crate) fn render_overlay_buffer(overlay: &mut Overlay, width: u16, height: u16, theme: &Theme) -> Buffer {
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
                base_branch: "main",
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

pub(crate) fn render_modal_semantic_sample(theme: &Theme) -> (Buffer, ModalAreas) {
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

pub(crate) fn row_contents(buffer: &Buffer, y: u16) -> String {
    (0..buffer.area().width)
        .map(|x| buffer[(x, y)].symbol())
        .collect()
}

pub(crate) fn assert_cell_style(buffer: &Buffer, x: u16, y: u16, style: Style) {
    let cell = &buffer[(x, y)];
    assert_eq!(cell.fg, style.fg.unwrap_or(Color::Reset));
    assert_eq!(cell.bg, style.bg.unwrap_or(Color::Reset));
    assert_eq!(cell.modifier, style.add_modifier);
}

pub(crate) fn assert_cell_foreground(buffer: &Buffer, x: u16, y: u16, style: Style) {
    assert_eq!(
        buffer[(x, y)].fg,
        style.fg.unwrap_or(Color::Reset),
        "at ({x}, {y})"
    );
}

pub(crate) fn assert_rendered_foreground(
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

pub(crate) fn contrast_ratio(foreground: Color, background: Color) -> f64 {
    let foreground_luminance = relative_luminance(foreground);
    let background_luminance = relative_luminance(background);
    (foreground_luminance.max(background_luminance) + 0.05)
        / (foreground_luminance.min(background_luminance) + 0.05)
}

pub(crate) fn relative_luminance(color: Color) -> f64 {
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

pub(crate) fn text_cell(buffer: &Buffer, text: &str) -> (u16, u16) {
    for y in 0..buffer.area().height {
        let row = row_contents(buffer, y);
        if let Some(byte_offset) = row.find(text) {
            return (row[..byte_offset].chars().count() as u16, y);
        }
    }
    panic!("did not render {text:?}");
}

pub(crate) fn assert_unavailable_cell_differs(
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
