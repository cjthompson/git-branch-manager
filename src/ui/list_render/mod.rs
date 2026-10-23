mod ladder;
mod params;

pub use params::{CellContext, ListRenderParams, RowRenderer};

use ratatui::prelude::*;
use ratatui::text::Line;
use ratatui::widgets::{
    Block, Borders, Cell, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState, Table,
};

use crate::view::column::ColumnDef;
use crate::view::ViewItem;

use self::ladder::{
    demoted_at_level, header_alignment, resolve_ladder_level, visible_column_indices, LadderColumn,
};
use super::tab_bar::tab_bar_line;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Renders any list view generically.
///
/// The checkbox cell is automatically prepended; the `render_row` callback should
/// NOT include it. It receives visible column indices so it knows which cells to produce.
#[allow(clippy::too_many_arguments)]
pub fn render_list_view<T: ViewItem>(
    frame: &mut Frame,
    area: Rect,
    params: &mut ListRenderParams<T>,
) {
    let width = area.width as usize;
    let horizontal_scrolling = params.horizontal_scrolling;
    let columns = params.columns;
    let state = &mut *params.state;
    let display_indices = state.display_indices().to_vec();
    let theme = params.theme;
    let symbols = params.symbols;

    // Determine which columns are visible at this width
    let visible_col_indices = if horizontal_scrolling {
        (0..columns.len()).collect()
    } else {
        visible_column_indices(columns, area.width)
    };

    let visible_columns: Vec<&ColumnDef<T>> =
        visible_col_indices.iter().map(|&i| &columns[i]).collect();

    // Sort-direction arrow appended to the active sort column's header label.
    let sort_arrow = if state.sort_ascending() {
        "\u{25b2}"
    } else {
        "\u{25bc}"
    };

    // Build column widths: checkbox + visible columns. Branch (and the
    // Worktrees Branch column) stretches by name, so a preceding fixed
    // indicator such as Branches Up does not change which column grows.
    let highlight_width = symbols.cursor_prefix.len() as u16 + 1;

    // The stretchy column is identified by name rather than position:
    // "Branch" (Branches and Worktrees), "Name" (Remotes/Tags), and "Path"
    // (Worktrees). Matching by name means a fixed indicator can move ahead of
    // Branch without changing which column claims the priority width.
    let is_stretchy =
        |col: &ColumnDef<T>| -> bool { matches!(col.name, "Branch" | "Name" | "Path") };

    // Give the stretchy column priority via a staged compaction ladder
    // (BL-022 stage 1) instead of flipping every fixed column between wide
    // and compact at once: demote one priority tier at a time (Age, then
    // Merge, then A/B+PR together with everything else) until the stretchy
    // column gets at least as much space as it needs and at least as much
    // as the fixed columns combined. See `resolve_ladder_level` /
    // `demoted_at_level` above for the algorithm and the rationale for using
    // direct arithmetic instead of resolving a trial `Layout`.
    let effective_min_widths: Vec<u16> = visible_columns
        .iter()
        .map(|col| {
            display_indices
                .iter()
                .map(|&raw_idx| col.min_width_for(&state.items()[raw_idx]))
                .max()
                .unwrap_or(col.min_width)
        })
        .collect();
    let ladder_columns: Vec<LadderColumn> = visible_columns
        .iter()
        .zip(&effective_min_widths)
        .map(|(col, &min_width)| LadderColumn {
            key: col.key,
            min_width,
            wide_width: col.wide_width,
            is_stretchy: is_stretchy(col),
        })
        .collect();

    // The ladder always fits the viewport; scrolling only widens the logical table.
    let available = area.width.saturating_sub(2).saturating_sub(highlight_width) as u32;
    let viewport_level = resolve_ladder_level(&ladder_columns, area.width, available);
    let min_full_width_at = |level: u8| -> u32 {
        2u32 + highlight_width as u32
            + 3
            + visible_columns
                .iter()
                .zip(&effective_min_widths)
                .map(|(col, &min_width)| {
                    let col_width = if demoted_at_level(col.key, level) {
                        min_width
                    } else {
                        col.wide_width.unwrap_or(min_width)
                    };
                    col_width as u32
                })
                .sum::<u32>()
            + visible_columns.len() as u32
    };
    // Scroll only when even the compacted layout overflows the viewport. An
    // overflowing layout stays at the wide level: the compacted widths would
    // let Branch absorb the slack left by the fixed columns.
    let overflows = horizontal_scrolling && min_full_width_at(viewport_level) > area.width as u32;
    let (level, logical_width, compact) = if overflows {
        let logical_width = (area.width as u32)
            .max(120)
            .max(min_full_width_at(0))
            .min(u16::MAX as u32) as u16;
        (0u8, logical_width, false)
    } else {
        (viewport_level, area.width, width < 120)
    };
    let table_width = logical_width.saturating_sub(2);
    let [_highlight_area, columns_area] =
        Layout::horizontal([Constraint::Length(highlight_width), Constraint::Fill(0)])
            .areas(Rect::new(0, 0, table_width, 1));

    let mut widths: Vec<Constraint> = vec![Constraint::Length(3)]; // checkbox
    for (col, &min_width) in visible_columns.iter().zip(&effective_min_widths) {
        let col_width = if demoted_at_level(col.key, level) {
            min_width
        } else {
            col.wide_width.unwrap_or(min_width)
        };
        if is_stretchy(col) {
            widths.push(Constraint::Min(col_width));
        } else {
            widths.push(Constraint::Length(col_width));
        }
    }

    // Resolve the constraint widths once so row renderers can fit text to real
    // cells. This mirrors ratatui Table's width calculation: the table first
    // reserves highlight-symbol space, then applies column spacing.
    let resolved = Layout::horizontal(&widths).spacing(1).split(columns_area);
    // resolved[0] is the checkbox; resolved[1] is the first data column.
    let data_col_widths: Vec<u16> = resolved.iter().skip(1).map(|r| r.width).collect();
    let first_col_width = data_col_widths.first().copied().unwrap_or(0);

    // Build header row. Computed after `data_col_widths` (rather than up front)
    // so alignment can check each column's actual resolved width.
    let mut header_cells: Vec<Cell> = vec![Cell::from("")]; // checkbox header (empty)

    for (pos, &col_idx) in visible_col_indices.iter().enumerate() {
        let col = &columns[col_idx];
        let label = if !col.show_header {
            String::new()
        } else if state.sort_column() == Some(col_idx) && col.compare.is_some() {
            format!("{}{}", col.name, sort_arrow)
        } else {
            col.name.to_string()
        };
        let wants_right = col.name == "Age" || col_idx == columns.len() - 1;
        let resolved_width = data_col_widths.get(pos).copied().unwrap_or(0);
        let cell = if header_alignment(wants_right, label.chars().count(), resolved_width)
            == Alignment::Right
        {
            Cell::from(Line::from(label).alignment(Alignment::Right))
        } else {
            Cell::from(label)
        };
        header_cells.push(cell.style(theme.header));
    }

    let header = Row::new(header_cells).height(1);

    let ctx = CellContext {
        theme,
        symbols,
        area_width: logical_width,
        compact,
        data_col_widths,
        first_col_width,
    };

    let viewport_width = area.width.saturating_sub(2);
    let logical_viewport_width = logical_width.saturating_sub(2);
    let max_horizontal_offset = if horizontal_scrolling {
        logical_viewport_width.saturating_sub(viewport_width) as usize
    } else {
        0
    };
    state.set_horizontal_scroll_bounds(max_horizontal_offset);
    let overflow = max_horizontal_offset > 0;

    // Build rows from display indices
    if display_indices.is_empty() && state.loading {
        state.header_columns.clear();
        let tab_title = tab_bar_line(params.active_view, theme);
        let block = Block::default()
            .title(tab_title)
            .title_top(Line::from(format!(" v{VERSION} ")).right_aligned())
            .borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let loading =
            Paragraph::new(Span::styled("Loading...", theme.dim)).alignment(Alignment::Left);
        frame.render_widget(loading, inner);
        return;
    }

    let rows: Vec<Row> = display_indices
        .iter()
        .enumerate()
        .map(|(display_pos, &raw_idx)| {
            let item = &state.items()[raw_idx];
            let is_selected = state.selected()[raw_idx];
            let is_cursor = state.table_state().selected() == Some(display_pos);
            let is_pinned = item.is_pinned();

            // Build checkbox cell
            let (checkbox_text, checkbox_style) = if is_pinned {
                ("   ".to_string(), Style::default())
            } else if is_selected {
                (symbols.checkbox_on.to_string(), theme.selected)
            } else {
                (symbols.checkbox_off.to_string(), theme.secondary_text)
            };
            let checkbox_cell = Cell::from(Span::styled(checkbox_text, checkbox_style));

            // Get view-specific cells
            let mut cells = vec![checkbox_cell];
            cells.extend(
                (params.render_row)(
                    item,
                    raw_idx,
                    is_selected,
                    is_cursor,
                    &visible_col_indices,
                    &ctx,
                )
                .into_iter()
                .map(Cell::from),
            );

            if is_selected {
                Row::new(cells).style(theme.checked_row)
            } else {
                Row::new(cells)
            }
        })
        .collect();

    // Build block with tab bar title
    let tab_title = tab_bar_line(params.active_view, theme);
    let block = Block::default()
        .title(tab_title)
        .title_top(Line::from(format!(" v{VERSION} ")).right_aligned())
        .borders(Borders::ALL);

    let highlight_sym = format!("{} ", symbols.cursor_prefix);

    // Render the border on the real viewport. When horizontal scrolling is
    // needed, render the table into a full-width buffer and copy the visible
    // slice into the viewport inside this border.
    let inner_area = block.inner(area);
    frame.render_widget(block.clone(), area);

    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(theme.cursor)
        .highlight_symbol(highlight_sym)
        .highlight_spacing(ratatui::widgets::HighlightSpacing::Always);

    let reserve_scrollbar = overflow && inner_area.height >= 2 && inner_area.width > 0;
    let table_height = inner_area
        .height
        .saturating_sub(u16::from(reserve_scrollbar));

    if overflow {
        let virtual_area = Rect::new(0, 0, logical_width, area.height);
        let virtual_inner = block.inner(virtual_area);
        let virtual_table_area = Rect::new(
            virtual_inner.x,
            virtual_inner.y,
            virtual_inner.width,
            table_height,
        );
        let mut virtual_buffer = ratatui::buffer::Buffer::empty(virtual_area);
        ratatui::widgets::StatefulWidget::render(
            table,
            virtual_table_area,
            &mut virtual_buffer,
            state.table_state_mut(),
        );

        let source_x = virtual_inner
            .x
            .saturating_add(state.horizontal_offset().min(u16::MAX as usize) as u16);
        for row in 0..table_height {
            let source_y = virtual_table_area.y + row;
            let target_y = inner_area.y + row;
            for column in 0..inner_area.width {
                let from_x = source_x + column;
                let to_x = inner_area.x + column;
                frame
                    .buffer_mut()
                    .cell_mut((to_x, target_y))
                    .expect("list viewport cell is in the frame")
                    .clone_from(
                        virtual_buffer
                            .cell((from_x, source_y))
                            .expect("scrolled table cell is in its virtual buffer"),
                    );
            }
        }

        if reserve_scrollbar {
            let scrollbar_area = Rect::new(
                inner_area.x,
                inner_area.y + inner_area.height - 1,
                inner_area.width,
                1,
            );
            let scrollbar = Scrollbar::new(ScrollbarOrientation::HorizontalBottom)
                .track_symbol(Some("─"))
                .thumb_symbol("━")
                .begin_symbol(Some("◂"))
                .end_symbol(Some("▸"))
                .track_style(theme.dim)
                .thumb_style(theme.title);
            let mut scrollbar_state = ScrollbarState::new(logical_viewport_width as usize)
                .position(state.horizontal_offset())
                .viewport_content_length(inner_area.width as usize);
            frame.render_stateful_widget(scrollbar, scrollbar_area, &mut scrollbar_state);
        }
    } else {
        frame.render_stateful_widget(
            table,
            Rect::new(inner_area.x, inner_area.y, inner_area.width, table_height),
            state.table_state_mut(),
        );
    }

    // Keep exact bounded sort hit regions. Clipping the original cell ranges
    // against the rendered viewport makes sorting work after a horizontal
    // scroll even when non-sortable or hidden columns precede a header.
    let virtual_inner = block.inner(Rect::new(0, 0, logical_width, area.height));
    let source_view_start = virtual_inner
        .x
        .saturating_add(state.horizontal_offset().min(u16::MAX as usize) as u16);
    let source_view_end = source_view_start.saturating_add(inner_area.width);
    let mut sort_col_map: Vec<Option<usize>> = vec![None]; // checkbox
    sort_col_map.extend(
        visible_col_indices
            .iter()
            .map(|&col_idx| columns[col_idx].compare.is_some().then_some(col_idx)),
    );
    state.header_columns = resolved
        .iter()
        .enumerate()
        .filter_map(|(i, rect)| {
            let sort_idx = sort_col_map.get(i).copied().flatten()?;
            let source_start = virtual_inner.x.saturating_add(rect.x);
            let source_end = source_start.saturating_add(rect.width);
            let clipped_start = source_start.max(source_view_start);
            let clipped_end = source_end.min(source_view_end);
            (clipped_start < clipped_end).then(|| {
                let screen_start = inner_area.x + clipped_start - source_view_start;
                let screen_end = screen_start + clipped_end - clipped_start;
                (screen_start, screen_end, sort_idx)
            })
        })
        .collect();
}

#[cfg(test)]
mod tests;
