//! Headless table rendering for the `--branches`/`--remotes`/`--tags`/`--worktrees`
//! dump flags. Serializes the same `Line` rows the TUI draws into plain or
//! ANSI-colored fixed-width text.

use std::io::IsTerminal;

use ratatui::layout::Alignment;
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
        Color::Rgb(r, g, b) => format!("{};2;{};{};{}", if fg { 38 } else { 48 }, r, g, b),
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

    // Column widths: every column grows to fit the widest of its header label,
    // configured width, and rendered content. The dump is "fully enriched" — it
    // never truncates and never shows the TUI's narrow short-forms (those are a
    // responsive, terminal-width concern; here the synthetic width is unbounded).
    let mut widths: Vec<usize> = columns
        .iter()
        .map(|c| {
            let header_width = if c.show_header {
                c.name.chars().count()
            } else {
                0
            };
            header_width
                .max(c.wide_width.unwrap_or(c.min_width) as usize)
                .max(c.min_width_for_items(rows) as usize)
        })
        .collect();
    for lines in &rendered {
        for (i, line) in lines.iter().enumerate() {
            if let Some(w) = widths.get_mut(i) {
                let vis: usize = line.spans.iter().map(|s| s.content.chars().count()).sum();
                *w = (*w).max(vis);
            }
        }
    }

    // Header alignment uses the TUI's positional header rule (mirrors
    // `list_render::render_list_view`, which right-aligns the "Age" and last
    // columns' header labels regardless of the data's alignment).
    let header_right_align = |idx: usize| columns[idx].name == "Age" || idx == columns.len() - 1;

    let mut out = String::new();
    if let Some(b) = base {
        out.push_str(&format!("base: {b}\n\n"));
    }

    let header_fields: Vec<String> = all_cols
        .iter()
        .map(|&i| {
            let column = &columns[i];
            let label = if column.show_header { column.name } else { "" };
            pad_plain(label, widths[i], header_right_align(i))
        })
        .collect();
    out.push_str(header_fields.join("  ").trim_end());
    out.push('\n');

    for lines in &rendered {
        let fields: Vec<String> = lines
            .iter()
            .enumerate()
            .map(|(i, line)| {
                // Data cells derive alignment from each rendered Line's own
                // alignment (set by the render fns, e.g. age/merge-status use
                // Alignment::Right), not from the column's position. This keeps
                // left-aligned data (e.g. the Tags "Message" last column) left.
                let right = matches!(line.alignment, Some(Alignment::Right));
                lay_out_cell(line, widths[i], right, colorize)
            })
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
/// On overflow (visible width strictly greater than `width`) the cell is
/// truncated to `width` visible characters and ANSI color is dropped regardless
/// of `colorize`. An exact fit keeps color.
fn lay_out_cell(line: &Line<'static>, width: usize, right: bool, colorize: bool) -> String {
    let plain: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    let visible = plain.chars().count();

    if visible > width {
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
mod tests;
