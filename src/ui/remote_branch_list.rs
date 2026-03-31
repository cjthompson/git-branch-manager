use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};

use crate::app::{App, View};
use git_branch_manager::git::github::PrStatus;
use git_branch_manager::types::MergeStatus;
use super::shared::{age_style, prefix_style, tab_bar_line};

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    app.terminal_rows = area.height;

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(area);

    let main_area = layout[0];
    let status_area = layout[1];

    let width = main_area.width as usize;
    let compact_age = width < 120;
    let hide_ab = width < 80;
    let hide_local = width < 80;
    let short_status = width < 70;
    let hide_age = width < 60;

    let tab_title = tab_bar_line(&View::RemoteBranches, app.theme.remote_title);
    let block = Block::default()
        .title(tab_title)
        .borders(Borders::ALL);

    let sort_arrow = if app.remote_sort_ascending { "\u{25b2}" } else { "\u{25bc}" };
    let sort_label = |col_index: usize, base: &str| -> String {
        if app.remote_sort_column == Some(col_index) {
            format!("{}{}", base, sort_arrow)
        } else {
            base.to_string()
        }
    };

    let mut header_cells = vec![
        Cell::from(""),
        Cell::from(sort_label(0, "Remote")),
    ];
    if !hide_local {
        header_cells.push(Cell::from("Local"));
    }
    if !hide_ab {
        header_cells.push(Cell::from("A/B"));
        header_cells.push(Cell::from("PR"));
    }
    if !hide_age {
        header_cells.push(Cell::from(
            Line::from(sort_label(1, "Age")).alignment(Alignment::Right),
        ));
    }
    header_cells.push(Cell::from(
        Line::from(sort_label(2, "Status")).alignment(Alignment::Right),
    ));

    let header = Row::new(header_cells)
        .style(app.theme.remote_header)
        .bottom_margin(0);

    let display_indices = app.filtered_remote_indices();

    let display_cursor = display_indices
        .iter()
        .position(|&i| i == app.remote_cursor);
    if let Some(row_idx) = display_cursor {
        app.remote_table_state.select(Some(row_idx));
    }

    let status_min_width: u16 = if short_status { 4 } else { 15 };
    let local_col_width: u16 = 5;

    let max_pr_width: u16 = app.remote_branches.iter()
        .filter_map(|b| app.pr_map.get(&b.short_name).map(|info| format!("#{}", info.number).len()))
        .max()
        .unwrap_or(0)
        .max("PR".len()) as u16;

    let max_ab_width: u16 = app.remote_branches.iter()
        .filter_map(|b| {
            let a = b.ahead.unwrap_or(0);
            let bk = b.behind.unwrap_or(0);
            if a > 0 || bk > 0 {
                let mut s = String::new();
                if a > 0 { s.push_str(&a.to_string()); }
                if a > 0 && bk > 0 { s.push(' '); }
                if bk > 0 { s.push_str(&bk.to_string()); }
                Some(s.len())
            } else {
                None
            }
        })
        .max()
        .unwrap_or(0)
        .max("A/B".len()) as u16;

    let highlight_width_for_name = app.symbols.cursor_prefix.len() as u16 + 1;
    let checkbox_width: u16 = 3;
    let age_width: u16 = if hide_age { 0 } else if compact_age { 5 } else { 14 };
    let local_width: u16 = if hide_local { 0 } else { local_col_width };
    let ab_pr_width: u16 = if hide_ab { 0 } else { max_ab_width + 1 + max_pr_width + 1 };
    let gap_count: u16 = 2
        + if hide_age { 0 } else { 1 }
        + if hide_local { 0 } else { 1 }
        + if hide_ab { 0 } else { 2 }
        + 1;
    let name_col_width = main_area
        .width
        .saturating_sub(2)
        .saturating_sub(highlight_width_for_name)
        .saturating_sub(checkbox_width)
        .saturating_sub(age_width)
        .saturating_sub(local_width)
        .saturating_sub(ab_pr_width)
        .saturating_sub(status_min_width)
        .saturating_sub(gap_count) as usize;

    let is_ascii = app.symbols.cursor_prefix == ">";
    let local_yes = if is_ascii { "Y" } else { "\u{2713}" };
    let local_no = if is_ascii { "-" } else { "\u{2014}" };

    let build_row = |i: usize| -> Row {
        let branch = &app.remote_branches[i];
        let is_selected = app.remote_selected.get(i).copied().unwrap_or(false);
        let is_pinned = branch.is_pinned();

        let (checkbox_text, checkbox_style) = if is_pinned {
            ("   ".to_string(), Style::default())
        } else if is_selected {
            (app.symbols.checkbox_on.to_string(), app.theme.selected)
        } else {
            (app.symbols.checkbox_off.to_string(), app.theme.secondary_text)
        };

        let name_style = if is_pinned {
            app.theme.pinned_row
        } else if is_selected {
            app.theme.selected
        } else {
            app.theme.primary_text
        };

        let ellipsis = if is_ascii { "..." } else { "\u{2026}" };
        let remote_prefix = format!("{}/", branch.remote);
        let name_available = name_col_width.saturating_sub(remote_prefix.len());
        let display_name =
            if branch.short_name.len() > name_available && name_available > ellipsis.len() {
                format!(
                    "{}{}",
                    &branch.short_name[..name_available - ellipsis.len()],
                    ellipsis
                )
            } else if branch.short_name.len() > name_available {
                ellipsis.to_string()
            } else {
                branch.short_name.clone()
            };

        let pinned_label = if is_pinned { " [base]" } else { "" };

        let mut name_spans: Vec<Span> = Vec::new();
        if is_pinned {
            name_spans.push(Span::styled(remote_prefix, app.theme.pinned_row));
        } else {
            name_spans.push(Span::styled(remote_prefix, app.theme.secondary_text));
        }
        if let Some((prefix_part, rest)) = display_name.split_once('/') {
            if let Some(pstyle) = prefix_style(prefix_part) {
                if is_pinned {
                    name_spans.push(Span::styled(
                        format!("{}/", prefix_part),
                        app.theme.pinned_row,
                    ));
                    name_spans.push(Span::styled(rest.to_string(), app.theme.pinned_row));
                } else {
                    name_spans.push(Span::styled(format!("{}/", prefix_part), pstyle));
                    name_spans.push(Span::styled(rest.to_string(), name_style));
                }
            } else {
                name_spans.push(Span::styled(display_name.clone(), name_style));
            }
        } else {
            name_spans.push(Span::styled(display_name.clone(), name_style));
        }
        name_spans.push(Span::styled(pinned_label, app.theme.secondary_text));

        let name_cell = Cell::from(Line::from(name_spans));

        let mut cells = vec![
            Cell::from(Span::styled(checkbox_text, checkbox_style)),
            name_cell,
        ];

        if !hide_local {
            let (local_text, local_style) = if is_pinned {
                (String::new(), app.theme.pinned_row)
            } else if branch.has_local {
                (local_yes.to_string(), app.theme.merged)
            } else {
                (local_no.to_string(), app.theme.secondary_text)
            };
            cells.push(Cell::from(Span::styled(local_text, local_style)));
        }

        if !hide_ab {
            let mut ab_spans: Vec<Span> = Vec::new();
            if is_pinned {
                ab_spans.push(Span::styled("", app.theme.pinned_row));
            } else if let (Some(a), Some(b)) = (branch.ahead, branch.behind) {
                if a > 0 {
                    ab_spans.push(Span::styled(
                        format!("{}{}", app.symbols.arrow_up, a),
                        app.theme.merged,
                    ));
                }
                if b > 0 {
                    ab_spans.push(Span::styled(
                        format!("{}{}", app.symbols.arrow_down, b),
                        app.theme.unmerged,
                    ));
                }
            }
            cells.push(Cell::from(Line::from(ab_spans)));

            // PR column — look up by short_name in pr_map
            let pr_text = if is_pinned {
                String::new()
            } else {
                app.pr_map
                    .get(&branch.short_name)
                    .map(|info| format!("#{}", info.number))
                    .unwrap_or_default()
            };
            let pr_style = if is_pinned {
                app.theme.pinned_row
            } else if let Some(info) = app.pr_map.get(&branch.short_name) {
                match info.status {
                    PrStatus::Draft => app.theme.pr_draft,
                    PrStatus::Open => app.theme.pr_open,
                    PrStatus::Merged => app.theme.pr_merged,
                    PrStatus::Closed => app.theme.pr_closed,
                }
            } else {
                Style::default()
            };
            cells.push(Cell::from(Span::styled(pr_text, pr_style)));
        }

        if !hide_age {
            let age = if compact_age {
                branch.age_short()
            } else {
                branch.age_display()
            };
            let a_style = if is_pinned {
                app.theme.pinned_row
            } else {
                age_style(&branch.last_commit_date)
            };
            cells.push(Cell::from(
                Line::from(Span::styled(age, a_style)).alignment(Alignment::Right),
            ));
        }

        let (status_text, status_style) = if is_pinned {
            (String::new(), app.theme.pinned_row)
        } else if short_status {
            match branch.merge_status {
                MergeStatus::Merged => (
                    format!("m {}", app.symbols.status_merged),
                    app.theme.merged,
                ),
                MergeStatus::SquashMerged => (
                    format!("s {}", app.symbols.status_squash_merged),
                    app.theme.squash_merged,
                ),
                MergeStatus::Unmerged => (
                    format!("u {}", app.symbols.status_unmerged),
                    app.theme.unmerged,
                ),
            }
        } else {
            match branch.merge_status {
                MergeStatus::Merged => (
                    format!("merged {}", app.symbols.status_merged),
                    app.theme.merged,
                ),
                MergeStatus::SquashMerged => (
                    format!("squash-merged {}", app.symbols.status_squash_merged),
                    app.theme.squash_merged,
                ),
                MergeStatus::Unmerged => (
                    format!("unmerged {}", app.symbols.status_unmerged),
                    app.theme.unmerged,
                ),
            }
        };
        cells.push(Cell::from(
            Line::from(Span::styled(status_text, status_style)).alignment(Alignment::Right),
        ));

        if is_selected {
            Row::new(cells).style(app.theme.checked_row)
        } else {
            Row::new(cells)
        }
    };

    let all_rows: Vec<Row> = display_indices.iter().map(|&i| build_row(i)).collect();

    let mut widths: Vec<Constraint> = vec![
        Constraint::Length(3),
        Constraint::Min(20),
    ];
    if !hide_local {
        widths.push(Constraint::Length(local_col_width));
    }
    if !hide_ab {
        widths.push(Constraint::Length(max_ab_width));
        widths.push(Constraint::Length(max_pr_width));
    }
    if !hide_age {
        widths.push(Constraint::Length(if compact_age { 5 } else { 14 }));
    }
    widths.push(Constraint::Length(status_min_width));

    {
        let mut col_positions: Vec<(u16, usize)> = Vec::new();
        let highlight_width = app.symbols.cursor_prefix.len() as u16 + 1;
        let x = main_area.x + 1 + highlight_width;

        let mut sort_col_map: Vec<Option<usize>> = vec![None];
        sort_col_map.push(Some(0));
        if !hide_local {
            sort_col_map.push(None);
        }
        if !hide_ab {
            sort_col_map.push(None); // A/B
            sort_col_map.push(None); // PR
        }
        if !hide_age {
            sort_col_map.push(Some(1));
        }
        sort_col_map.push(Some(2));

        let available = main_area.width.saturating_sub(2 + highlight_width);
        let resolved = Layout::horizontal(&widths).split(Rect::new(0, 0, available, 1));

        for (i, rect) in resolved.iter().enumerate() {
            if let Some(&Some(sort_idx)) = sort_col_map.get(i) {
                col_positions.push((x + rect.x, sort_idx));
            }
        }

        app.remote_header_columns = col_positions;
    }

    let highlight_sym = format!("{} ", app.symbols.cursor_prefix);

    let inner_area = block.inner(main_area);
    frame.render_widget(block, main_area);

    let table = Table::new(all_rows, widths)
        .header(header)
        .row_highlight_style(app.theme.cursor)
        .highlight_symbol(highlight_sym);

    frame.render_stateful_widget(table, inner_area, &mut app.remote_table_state);

    if app.remote_search_active {
        app.remote_status_bar_items.clear();
        let search_text = format!(" / {}_", app.remote_search_query);
        let search_bar = Paragraph::new(search_text).style(app.theme.search_bar);
        frame.render_widget(search_bar, status_area);
    } else if !app.remote_search_query.is_empty() {
        app.remote_status_bar_items.clear();
        let filter_text = format!(
            " filter: \"{}\" ({}/{} shown) \u{2014} [\\]filter [/]edit [Esc in /]clear",
            app.remote_search_query,
            display_indices.len(),
            app.remote_branches.len()
        );
        let status = Paragraph::new(filter_text).style(app.theme.search_bar);
        frame.render_widget(status, status_area);
    } else {
        let selected_count = app.remote_selected.iter().filter(|&&s| s).count();
        let total = app.remote_branches.len();
        let merged_count = app
            .remote_branches
            .iter()
            .filter(|b| b.merge_status == MergeStatus::Merged)
            .count();
        let squash_count = app
            .remote_branches
            .iter()
            .filter(|b| b.merge_status == MergeStatus::SquashMerged)
            .count();
        let progress = if app.remote_squash_total > 0
            && app.remote_squash_checked < app.remote_squash_total
        {
            format!(
                " | checking {}/{}",
                app.remote_squash_checked, app.remote_squash_total
            )
        } else if app.remote_enrich_total > 0
            && app.remote_enrich_checked < app.remote_enrich_total
        {
            format!(
                " | enriching {}/{}",
                app.remote_enrich_checked, app.remote_enrich_total
            )
        } else {
            String::new()
        };

        let status_text = if width < 80 {
            format!(
                " {}br {}sel {}m {}s{} \u{2014} [/]search [d]el [c]heckout [q]uit",
                total, selected_count, merged_count, squash_count, progress
            )
        } else {
            format!(
                " {} remote branches | {} selected | {} merged | {} squashed{} \u{2014} [/]search [\\]filter [d]el [c]heckout [f]etch [?]help [q]uit",
                total, selected_count, merged_count, squash_count, progress
            )
        };

        app.remote_status_bar_items = super::shared::render_status_bar(
            frame,
            status_area,
            &status_text,
            app.theme.remote_title.fg.unwrap_or(Color::White),
            app.theme.status_bar,
        );
    }

    super::shared::draw_toast(frame, app, area);
}
