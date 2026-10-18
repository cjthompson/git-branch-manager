use super::*;
use chrono::{DateTime, Utc};

use crate::symbols::SymbolSet;
use crate::theme::Theme;
use crate::ui::list_render::CellContext;
use crate::view::column::ColumnDef;
use crate::view::ViewItem;
use ratatui::style::{Color, Style};
use ratatui::text::Span;

#[test]
fn sgr_empty_style_is_blank() {
    assert_eq!(sgr_prefix(&Style::default()), "");
}

#[test]
fn sgr_named_fg() {
    assert_eq!(sgr_prefix(&Style::new().fg(Color::Green)), "\x1b[32m");
}

#[test]
fn sgr_modifier_then_color() {
    assert_eq!(
        sgr_prefix(&Style::new().fg(Color::Red).add_modifier(Modifier::BOLD)),
        "\x1b[1;31m"
    );
}

#[test]
fn sgr_indexed() {
    assert_eq!(
        sgr_prefix(&Style::new().fg(Color::Indexed(141))),
        "\x1b[38;5;141m"
    );
}

#[derive(Clone)]
struct Dummy {
    name: String,
    pinned: bool,
}

impl ViewItem for Dummy {
    fn display_name(&self) -> &str {
        &self.name
    }
    fn is_pinned(&self) -> bool {
        self.pinned
    }
    fn last_commit_date(&self) -> &DateTime<Utc> {
        static EPOCH: std::sync::OnceLock<DateTime<Utc>> = std::sync::OnceLock::new();
        EPOCH.get_or_init(DateTime::default)
    }
}

fn dummy_cols() -> Vec<ColumnDef<Dummy>> {
    vec![
        ColumnDef {
            key: "name",
            name: "Name",
            show_header: true,
            min_width: 6,
            content_min_width: None,
            wide_width: None,
            hide_below_width: None,
            compare: None,
        },
        ColumnDef {
            key: "age",
            name: "Age",
            show_header: true,
            min_width: 5,
            content_min_width: None,
            wide_width: None,
            hide_below_width: None,
            compare: None,
        },
    ]
}

fn dummy_row(
    item: &Dummy,
    _i: usize,
    _s: bool,
    _c: bool,
    cols: &[usize],
    _ctx: &CellContext,
) -> Vec<Line<'static>> {
    cols.iter()
        .map(|&c| match c {
            0 => Line::from(Span::styled(
                item.name.clone(),
                Style::new().fg(Color::Green),
            )),
            _ => Line::from("2d"),
        })
        .collect()
}

#[test]
fn render_table_plain_no_ansi() {
    let theme = Theme::dark();
    let symbols = SymbolSet::ascii();
    let ctx = CellContext {
        theme: &theme,
        symbols: &symbols,
        area_width: DUMP_AREA_WIDTH,
        compact: false,
        data_col_widths: Vec::new(),
        first_col_width: DUMP_AREA_WIDTH,
    };
    let rows = vec![Dummy {
        name: "main".into(),
        pinned: true,
    }];
    let cols = dummy_cols();
    let out = render_table(
        Some("main"),
        &rows,
        &cols,
        dummy_row,
        &ctx,
        ColorChoice::Never,
    );
    assert!(out.starts_with("base: main\n\n"));
    assert!(out.contains("Name"));
    assert!(out.contains("main"));
    assert!(!out.contains('\x1b'), "Never must not emit ANSI: {out:?}");
}

#[test]
fn render_table_keeps_hidden_header_column_data() {
    let theme = Theme::dark();
    let symbols = SymbolSet::ascii();
    let ctx = CellContext {
        theme: &theme,
        symbols: &symbols,
        area_width: DUMP_AREA_WIDTH,
        compact: false,
        data_col_widths: Vec::new(),
        first_col_width: DUMP_AREA_WIDTH,
    };
    let rows = vec![Dummy {
        name: "main".into(),
        pinned: true,
    }];
    let mut cols = dummy_cols();
    cols[0].name = "Up";
    cols[0].show_header = false;

    let out = render_table(None, &rows, &cols, dummy_row, &ctx, ColorChoice::Never);
    let header = out.lines().next().expect("header line");
    let data = out.lines().nth(1).expect("row line");

    assert!(
        !header.contains("Up"),
        "hidden header should be blank: {out:?}"
    );
    assert!(header.contains("Age"), "other headers remain: {out:?}");
    assert!(data.contains("main"), "hidden column data remains: {out:?}");
}

#[test]
fn render_table_always_emits_ansi() {
    let theme = Theme::dark();
    let symbols = SymbolSet::ascii();
    let ctx = CellContext {
        theme: &theme,
        symbols: &symbols,
        area_width: DUMP_AREA_WIDTH,
        compact: false,
        data_col_widths: Vec::new(),
        first_col_width: DUMP_AREA_WIDTH,
    };
    let rows = vec![Dummy {
        name: "main".into(),
        pinned: true,
    }];
    let cols = dummy_cols();
    let out = render_table(None, &rows, &cols, dummy_row, &ctx, ColorChoice::Always);
    assert!(
        out.contains("\x1b[32m"),
        "Always must color the green name: {out:?}"
    );
}

#[test]
fn render_table_first_column_not_truncated() {
    let theme = Theme::dark();
    let symbols = SymbolSet::ascii();
    let ctx = CellContext {
        theme: &theme,
        symbols: &symbols,
        area_width: DUMP_AREA_WIDTH,
        compact: false,
        data_col_widths: Vec::new(),
        first_col_width: DUMP_AREA_WIDTH,
    };
    let long = "a-very-long-branch-name-exceeding-min-width";
    let rows = vec![Dummy {
        name: long.into(),
        pinned: false,
    }];
    let cols = dummy_cols();
    let out = render_table(None, &rows, &cols, dummy_row, &ctx, ColorChoice::Never);
    assert!(
        out.contains(long),
        "first column must not be truncated: {out:?}"
    );
}

#[test]
fn render_table_exact_fit_keeps_color() {
    let theme = Theme::dark();
    let symbols = SymbolSet::ascii();
    let ctx = CellContext {
        theme: &theme,
        symbols: &symbols,
        area_width: DUMP_AREA_WIDTH,
        compact: false,
        data_col_widths: Vec::new(),
        first_col_width: DUMP_AREA_WIDTH,
    };
    // Single row → first column auto-sizes to exactly this name's width.
    let rows = vec![Dummy {
        name: "exactly-this-wide".into(),
        pinned: false,
    }];
    let cols = dummy_cols();
    let out = render_table(None, &rows, &cols, dummy_row, &ctx, ColorChoice::Always);
    assert!(
        out.contains("\x1b[32m"),
        "exact-fit first column must keep color: {out:?}"
    );
    assert!(
        out.contains("exactly-this-wide"),
        "name must appear in full: {out:?}"
    );
}
