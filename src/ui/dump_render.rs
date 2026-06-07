//! Headless table rendering for the `--branches`/`--remotes`/`--tags`/`--worktrees`
//! dump flags. Serializes the same `Line` rows the TUI draws into plain or
//! ANSI-colored fixed-width text.

use std::io::IsTerminal;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;

use crate::cli::ColorChoice;
use crate::ui::list_render::{CellContext, RowRenderer};
use crate::view::column::ColumnDef;
use crate::view::ViewItem;

/// Width used for the synthetic terminal area when rendering a dump (wide enough
/// that no column hides and no compact short-forms trigger).
pub const DUMP_AREA_WIDTH: u16 = 200;

const RESET: &str = "\x1b[0m";

/// Build the SGR escape prefix for a style (modifiers, then fg, then bg).
/// Returns an empty string when the style carries no fg/bg/modifier.
pub(crate) fn sgr_prefix(style: &Style) -> String {
    let mut codes: Vec<String> = Vec::new();
    let m = style.add_modifier;
    if m.contains(Modifier::BOLD) {
        codes.push("1".into());
    }
    if m.contains(Modifier::DIM) {
        codes.push("2".into());
    }
    if m.contains(Modifier::ITALIC) {
        codes.push("3".into());
    }
    if m.contains(Modifier::UNDERLINED) {
        codes.push("4".into());
    }
    if let Some(fg) = style.fg.and_then(|c| color_code(c, true)) {
        codes.push(fg);
    }
    if let Some(bg) = style.bg.and_then(|c| color_code(c, false)) {
        codes.push(bg);
    }
    if codes.is_empty() {
        String::new()
    } else {
        format!("\x1b[{}m", codes.join(";"))
    }
}

/// SGR numeric code for a color. `fg` selects foreground (else +10 for background).
fn color_code(c: Color, fg: bool) -> Option<String> {
    let base = |n: u8| -> String {
        if fg {
            n.to_string()
        } else {
            (n + 10).to_string()
        }
    };
    let code = match c {
        Color::Black => base(30),
        Color::Red => base(31),
        Color::Green => base(32),
        Color::Yellow => base(33),
        Color::Blue => base(34),
        Color::Magenta => base(35),
        Color::Cyan => base(36),
        Color::Gray => base(37),
        Color::DarkGray => base(90),
        Color::LightRed => base(91),
        Color::LightGreen => base(92),
        Color::LightYellow => base(93),
        Color::LightBlue => base(94),
        Color::LightMagenta => base(95),
        Color::LightCyan => base(96),
        Color::White => base(97),
        Color::Indexed(n) => format!("{};5;{}", if fg { 38 } else { 48 }, n),
        Color::Rgb(r, g, b) => format!(
            "{};2;{};{};{}",
            if fg { 38 } else { 48 },
            r,
            g,
            b
        ),
        Color::Reset => return None,
    };
    Some(code)
}

/// Render a view to a fixed-width table string.
///
/// `rows` must already be in the intended display order. `base` is printed as a
/// `base: <branch>` header line when present (branches view).
pub fn render_table<T: ViewItem>(
    base: Option<&str>,
    rows: &[T],
    columns: &[ColumnDef<T>],
    render_row: RowRenderer<T>,
    ctx: &CellContext,
    color: ColorChoice,
) -> String {
    let colorize = match color {
        ColorChoice::Always => true,
        ColorChoice::Never => false,
        ColorChoice::Auto => std::io::stdout().is_terminal(),
    };

    let all_cols: Vec<usize> = (0..columns.len()).collect();

    // Render every row to its styled Lines exactly once.
    let rendered: Vec<Vec<Line<'static>>> = rows
        .iter()
        .enumerate()
        .map(|(idx, item)| render_row(item, idx, false, false, &all_cols, ctx))
        .collect();

    // Column widths: fixed (wide_width or min_width) for every column, except the
    // first — the TUI's stretchy Min-constrained column — which grows to fit its
    // content so names/paths are never truncated.
    let mut widths: Vec<usize> = columns
        .iter()
        .map(|c| c.wide_width.unwrap_or(c.min_width) as usize)
        .collect();
    if !columns.is_empty() {
        let mut w0 = columns[0].name.chars().count().max(widths[0]);
        for lines in &rendered {
            if let Some(first) = lines.first() {
                let vis: usize = first.spans.iter().map(|s| s.content.chars().count()).sum();
                w0 = w0.max(vis);
            }
        }
        widths[0] = w0;
    }

    let right_align = |idx: usize| columns[idx].name == "Age" || idx == columns.len() - 1;

    let mut out = String::new();
    if let Some(b) = base {
        out.push_str(&format!("base: {b}\n\n"));
    }

    let header_fields: Vec<String> = all_cols
        .iter()
        .map(|&i| pad_plain(columns[i].name, widths[i], right_align(i)))
        .collect();
    out.push_str(header_fields.join("  ").trim_end());
    out.push('\n');

    for lines in &rendered {
        let fields: Vec<String> = lines
            .iter()
            .enumerate()
            .map(|(i, line)| lay_out_cell(line, widths[i], right_align(i), colorize))
            .collect();
        out.push_str(fields.join("  ").trim_end());
        out.push('\n');
    }

    out
}

/// Pad/truncate plain text to `width`, right- or left-aligned.
fn pad_plain(text: &str, width: usize, right: bool) -> String {
    let len = text.chars().count();
    if len >= width {
        text.chars().take(width).collect()
    } else if right {
        format!("{}{}", " ".repeat(width - len), text)
    } else {
        format!("{}{}", text, " ".repeat(width - len))
    }
}

/// Render one `Line`'s spans to a fixed-width field, optionally ANSI-colored.
/// Padding is computed from the visible (un-escaped) text width.
/// On overflow (visible width >= `width`) the cell is truncated to `width`
/// visible characters and ANSI color is dropped regardless of `colorize`.
fn lay_out_cell(line: &Line<'static>, width: usize, right: bool, colorize: bool) -> String {
    let plain: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    let visible = plain.chars().count();

    if visible >= width {
        return plain.chars().take(width).collect();
    }

    let body = if colorize {
        line.spans
            .iter()
            .map(|s| {
                let prefix = sgr_prefix(&s.style);
                if prefix.is_empty() {
                    s.content.to_string()
                } else {
                    format!("{prefix}{}{RESET}", s.content)
                }
            })
            .collect::<String>()
    } else {
        plain
    };

    let pad = " ".repeat(width - visible);
    if right {
        format!("{pad}{body}")
    } else {
        format!("{body}{pad}")
    }
}

#[cfg(test)]
mod tests {
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
                name: "Name",
                min_width: 6,
                wide_width: None,
                hide_below_width: None,
                compare: None,
            },
            ColumnDef {
                name: "Age",
                min_width: 5,
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
                0 => Line::from(Span::styled(item.name.clone(), Style::new().fg(Color::Green))),
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
    fn render_table_always_emits_ansi() {
        let theme = Theme::dark();
        let symbols = SymbolSet::ascii();
        let ctx = CellContext {
            theme: &theme,
            symbols: &symbols,
            area_width: DUMP_AREA_WIDTH,
            compact: false,
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
        let ctx = CellContext { theme: &theme, symbols: &symbols, area_width: DUMP_AREA_WIDTH, compact: false };
        let long = "a-very-long-branch-name-exceeding-min-width";
        let rows = vec![Dummy { name: long.into(), pinned: false }];
        let cols = dummy_cols();
        let out = render_table(None, &rows, &cols, dummy_row, &ctx, ColorChoice::Never);
        assert!(out.contains(long), "first column must not be truncated: {out:?}");
    }
}
