use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Row, Table};

use crate::app::{App, View};
use git_branch_manager::git::github::PrStatus;
use git_branch_manager::types::MergeStatus;
use super::shared::{age_style, prefix_style, tab_bar_line, truncate};

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
    let short_status = width < 70;
    let hide_age = width < 60;

    let tab_title = tab_bar_line(&View::Worktrees, app.theme.title);
    let block = Block::default()
        .title(tab_title)
        .borders(Borders::ALL);

    // Header
    let checkbox_width: u16 = 3;

    // Sort indicator helper
    let sort_arrow = if app.worktree_sort_ascending { "\u{25b2}" } else { "\u{25bc}" };
    let sort_label = |col_index: usize, base: &str| -> String {
        if app.worktree_sort_column == Some(col_index) {
            format!("{}{}", base, sort_arrow)
        } else {
            base.to_string()
        }
    };

    let mut header_cells = vec![Cell::from(""), Cell::from(sort_label(0, "Branch")), Cell::from(sort_label(1, "Path"))];
    if !hide_ab {
        header_cells.push(Cell::from("A/B"));
        header_cells.push(Cell::from("PR"));
    }
    if !hide_age {
        header_cells.push(Cell::from(
            Line::from(sort_label(2, "Age")).alignment(Alignment::Right),
        ));
    }
    header_cells.push(Cell::from(
        Line::from(sort_label(3, "Status")).alignment(Alignment::Right),
    ));

    let header = Row::new(header_cells)
        .style(app.theme.header)
        .bottom_margin(0);

    // Sync table selection to cursor
    app.worktree_table_state.select(if app.worktrees.is_empty() {
        None
    } else {
        Some(app.worktree_cursor)
    });

    let is_ascii = app.symbols.cursor_prefix == ">";
    let ellipsis = if is_ascii { "..." } else { "\u{2026}" };

    // Column width calculation
    let highlight_width: u16 = app.symbols.cursor_prefix.len() as u16 + 1;
    // "squash-merged " (14) + symbol (1) = 15; short form "s ✗" = 4
    let status_min_width: u16 = if short_status { 4 } else { 15 };
    let age_width: u16 = if hide_age { 0 } else if compact_age { 5 } else { 14 };

    // PR column width: widest "#NNNN" across all worktrees, min = "PR".len()
    let max_pr_width: u16 = app.worktrees.iter()
        .filter_map(|wt| wt.pr.as_ref().map(|_| 5usize)) // "#NNNN" — placeholder; no PR number in WorktreeInfo
        .max()
        .unwrap_or(0)
        .max("PR".len()) as u16;

    // A/B column width
    let max_ab_width: u16 = app.worktrees.iter()
        .filter_map(|wt| {
            let a = wt.ahead.unwrap_or(0);
            let b = wt.behind.unwrap_or(0);
            if a > 0 || b > 0 {
                let mut s = String::new();
                if a > 0 { s.push_str(&a.to_string()); }
                if a > 0 && b > 0 { s.push(' '); }
                if b > 0 { s.push_str(&b.to_string()); }
                Some(s.len())
            } else {
                None
            }
        })
        .max()
        .unwrap_or(0)
        .max("A/B".len()) as u16;

    let ab_pr_width: u16 = if hide_ab { 0 } else { max_ab_width + 1 + max_pr_width + 1 };

    // Split remaining width evenly between Path and Branch columns
    let fixed_width = 2u16 // borders
        + highlight_width
        + checkbox_width
        + age_width
        + ab_pr_width
        + status_min_width
        + 3; // gaps between columns
    let remaining = main_area.width.saturating_sub(fixed_width) as usize;
    let branch_col_width = remaining / 2;
    let path_col_width = remaining - branch_col_width;

    let build_row = |i: usize| -> Row {
        let wt = &app.worktrees[i];
        let is_pinned = wt.is_pinned();

        // Path column — show relative to repo root, or just the last component
        let path_str = wt.path.to_string_lossy();
        let repo_str = app.repo_path.to_string_lossy();
        let rel_path = if path_str.starts_with(repo_str.as_ref()) {
            let stripped = path_str[repo_str.len()..].trim_start_matches('/');
            if stripped.is_empty() { "." } else { stripped }
        } else {
            path_str.as_ref()
        };

        let main_label = if is_pinned { " [main]" } else { "" };
        let display_path = truncate(rel_path, path_col_width.saturating_sub(main_label.len()), ellipsis);

        let path_style = if is_pinned {
            app.theme.pinned_row
        } else {
            app.theme.secondary_text
        };

        let path_spans = vec![
            Span::styled(display_path, path_style),
            Span::styled(main_label, app.theme.secondary_text),
        ];
        let path_cell = Cell::from(Line::from(path_spans));

        // Branch column
        let branch_text = wt.branch.as_deref().unwrap_or("(detached)");
        let wts_label = if !wt.wt_status.is_clean() {
            format!(" [{}]", wt.wt_status.summary())
        } else {
            String::new()
        };
        let branch_available = branch_col_width.saturating_sub(wts_label.len());
        let display_branch = truncate(branch_text, branch_available, ellipsis);
        let branch_style = if is_pinned {
            app.theme.pinned_row
        } else if wt.branch.is_none() {
            app.theme.secondary_text
        } else {
            app.theme.primary_text
        };
        let mut branch_spans: Vec<Span> = Vec::new();
        if wt.branch.is_some() && !is_pinned {
            if let Some((prefix_part, rest)) = display_branch.split_once('/') {
                if let Some(pstyle) = prefix_style(prefix_part) {
                    branch_spans.push(Span::styled(format!("{}/", prefix_part), pstyle));
                    branch_spans.push(Span::styled(rest.to_string(), branch_style));
                } else {
                    branch_spans.push(Span::styled(display_branch.clone(), branch_style));
                }
            } else {
                branch_spans.push(Span::styled(display_branch.clone(), branch_style));
            }
        } else {
            branch_spans.push(Span::styled(display_branch.clone(), branch_style));
        }
        branch_spans.push(Span::styled(wts_label, app.theme.secondary_text));
        let branch_cell = Cell::from(Line::from(branch_spans));

        // Checkbox column
        let is_selected = app.worktree_selected.get(i).copied().unwrap_or(false);
        let (checkbox_text, checkbox_style) = if is_pinned {
            (String::new(), app.theme.pinned_row)
        } else if is_selected {
            (app.symbols.checkbox_on.to_string(), app.theme.selected)
        } else {
            (app.symbols.checkbox_off.to_string(), app.theme.secondary_text)
        };

        let mut cells = vec![
            Cell::from(Span::styled(checkbox_text, checkbox_style)),
            branch_cell,
            path_cell,
        ];

        // Ahead/behind + PR columns
        if !hide_ab {
            let mut ab_spans: Vec<Span> = Vec::new();
            if is_pinned || wt.branch.is_none() {
                ab_spans.push(Span::styled("", app.theme.pinned_row));
            } else if let (Some(a), Some(b)) = (wt.ahead, wt.behind) {
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

            // PR column — WorktreeInfo only has PrStatus (no number), show icon
            let (pr_text, pr_style) = if is_pinned || wt.branch.is_none() {
                (String::new(), Style::default())
            } else {
                match &wt.pr {
                    Some(PrStatus::Draft) => ("PR".to_string(), app.theme.pr_draft),
                    Some(PrStatus::Open) => ("PR".to_string(), app.theme.pr_open),
                    Some(PrStatus::Merged) => ("PR".to_string(), app.theme.pr_merged),
                    Some(PrStatus::Closed) => ("PR".to_string(), app.theme.pr_closed),
                    None => (String::new(), Style::default()),
                }
            };
            cells.push(Cell::from(Span::styled(pr_text, pr_style)));
        }

        // Age column
        if !hide_age {
            let age = if compact_age {
                wt.age_short()
            } else {
                wt.age_display()
            };
            let a_style = if is_pinned {
                app.theme.pinned_row
            } else {
                age_style(&wt.age_date)
            };
            cells.push(Cell::from(
                Line::from(Span::styled(age, a_style)).alignment(Alignment::Right),
            ));
        }

        // Status column: merge status (same format as branch_list).
        // For pinned/detached rows, show the working tree status instead.
        let (status_text, status_style) = if is_pinned || wt.branch.is_none() {
            (wt.wt_status.summary(), app.theme.pinned_row)
        } else if short_status {
            match wt.merge_status {
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
                MergeStatus::Pending => ("p …".to_string(), app.theme.secondary_text),
            }
        } else {
            match wt.merge_status {
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
                MergeStatus::Pending => ("pending …".to_string(), app.theme.secondary_text),
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

    let all_rows: Vec<Row> = (0..app.worktrees.len()).map(build_row).collect();

    let mut widths: Vec<Constraint> = vec![
        Constraint::Length(checkbox_width),
        Constraint::Length(branch_col_width as u16),
        Constraint::Min(10),
    ];
    if !hide_ab {
        widths.push(Constraint::Length(max_ab_width));
        widths.push(Constraint::Length(max_pr_width));
    }
    if !hide_age {
        widths.push(Constraint::Length(age_width));
    }
    widths.push(Constraint::Length(status_min_width));

    let highlight_sym = format!("{} ", app.symbols.cursor_prefix);
    let inner_area = block.inner(main_area);
    frame.render_widget(block, main_area);

    let table = Table::new(all_rows, widths)
        .header(header)
        .row_highlight_style(app.theme.cursor)
        .highlight_symbol(highlight_sym);

    frame.render_stateful_widget(table, inner_area, &mut app.worktree_table_state);

    // Status bar
    {
        let total = app.worktrees.len();
        let progress = if app.worktree_enrich_total > 0
            && app.worktree_enrich_checked < app.worktree_enrich_total
        {
            format!(
                " | enriching {}/{}",
                app.worktree_enrich_checked, app.worktree_enrich_total
            )
        } else {
            String::new()
        };
        let status_text = if width < 80 {
            format!(
                " {} worktrees{} \u{2014} [d]rm [D]force-rm [f]etch [w]branches [?]help [q]uit",
                total, progress
            )
        } else {
            format!(
                " {} worktrees{} \u{2014} [Enter]menu [d]remove [D]force-remove [f]etch [w]branches [r]emotes [t]ags [?]help [q]uit",
                total, progress
            )
        };

        app.worktree_status_bar_items = super::shared::render_status_bar(
            frame,
            status_area,
            &status_text,
            app.theme.title.fg.unwrap_or(Color::White),
            app.theme.status_bar,
        );
    }

    super::shared::draw_toast(frame, app, area);
}
