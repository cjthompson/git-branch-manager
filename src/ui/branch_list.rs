use chrono::{DateTime, Utc};
use crossterm::event::KeyCode;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};

use crate::app::App;
use git_branch_manager::git::github::PrStatus;
use git_branch_manager::types::{MergeStatus, TrackingStatus};

/// Returns a style for known branch name prefixes (text before the first `/`).
fn prefix_style(prefix: &str) -> Option<Style> {
    match prefix {
        "fix" => Some(Style::new().fg(Color::Red)),
        "feat" | "feature" => Some(Style::new().fg(Color::Green)),
        "chore" => Some(Style::new().fg(Color::Indexed(130))),
        "hotfix" => Some(Style::new().fg(Color::Magenta)),
        "release" => Some(Style::new().fg(Color::Cyan)),
        _ => None,
    }
}

/// Returns a color style based on how old a commit is.
fn age_style(date: &DateTime<Utc>) -> Style {
    let duration = Utc::now() - *date;
    let days = duration.num_days();

    if days < 7 {
        Style::new().fg(Color::Green)
    } else if days < 30 {
        Style::new().fg(Color::Yellow)
    } else if days < 90 {
        Style::new().fg(Color::Indexed(208)) // orange
    } else {
        Style::new().fg(Color::Red)
    }
}


pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    // Update terminal_rows so handle_mouse_click can detect status bar row
    app.terminal_rows = area.height;

    // Show loading screen while initial branch data is being fetched
    if app.loading {
        let block = Block::default()
            .title("git-branch-manager")
            .title_style(app.theme.title)
            .borders(Borders::ALL);
        let msg = format!("  {}", app.loading_message);
        let loading = Paragraph::new(msg)
            .style(app.theme.primary_text)
            .block(block);
        frame.render_widget(loading, area);
        return;
    }

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(area);

    let main_area = layout[0];
    let status_area = layout[1];

    // Responsive width thresholds
    let width = main_area.width as usize;
    let compact_age = width < 120;
    let hide_ab = width < 80;
    let short_status = width < 70;
    let hide_age = width < 60;

    // Main branch list
    let wt_status = app.working_tree_status.summary();
    let title = format!(
        "git-branch-manager \u{2014} base: {} [{}]",
        app.base_branch, wt_status
    );
    let block = Block::default()
        .title(title)
        .title_style(app.theme.title)
        .borders(Borders::ALL);

    // Sort indicator helper
    let sort_arrow = if app.sort_ascending { "\u{25b2}" } else { "\u{25bc}" };
    let sort_label = |col_index: usize, base: &str| -> String {
        if app.sort_column == Some(col_index) {
            format!("{}{}", base, sort_arrow)
        } else {
            base.to_string()
        }
    };

    // Header row — build dynamically based on visible columns
    let mut header_cells = vec![
        Cell::from(""),
        Cell::from(sort_label(0, "Branch")),
    ];
    if !hide_age {
        header_cells.push(Cell::from(
            Line::from(sort_label(1, "Age")).alignment(Alignment::Right),
        ));
    }
    if !hide_ab {
        header_cells.push(Cell::from(sort_label(2, "A/B")));
        header_cells.push(Cell::from("PR"));
    }
    header_cells.push(Cell::from(
        Line::from(sort_label(3, "Status")).alignment(Alignment::Right),
    ));

    let header = Row::new(header_cells)
        .style(app.theme.header)
        .bottom_margin(0);

    // Build filtered index list: only branches matching the search query
    let filtered_indices: Vec<usize> = app
        .branches
        .iter()
        .enumerate()
        .filter(|(_, branch)| app.matches_search(branch))
        .map(|(i, _)| i)
        .collect();

    // Split filtered indices into pinned and non-pinned groups, then combine pinned-first
    let pinned_indices: Vec<usize> = filtered_indices
        .iter()
        .copied()
        .filter(|&i| app.branches[i].is_pinned())
        .collect();
    let non_pinned_indices: Vec<usize> = filtered_indices
        .iter()
        .copied()
        .filter(|&i| !app.branches[i].is_pinned())
        .collect();

    // Combined display order: pinned first, then non-pinned
    let display_indices: Vec<usize> = pinned_indices
        .iter()
        .chain(non_pinned_indices.iter())
        .copied()
        .collect();

    // Map cursor (original branch index) to display row index for table_state
    let display_cursor = display_indices.iter().position(|&i| i == app.cursor);
    if let Some(row_idx) = display_cursor {
        app.table_state.select(Some(row_idx));
    }

    // Status min-width: "squash-merged " (14) + symbol (1) = 15; short form: 4
    let status_min_width: u16 = if short_status { 4 } else { 15 };

    // PR column: widest "#NNNN" across all branches, minimum = "PR".len() = 2
    let max_pr_width: u16 = app.branches.iter()
        .filter_map(|b| app.pr_map.get(&b.name).map(|info| format!("#{}", info.number).len()))
        .max()
        .unwrap_or(0)
        .max("PR".len()) as u16;

    // A/B column: widest ahead+behind string, minimum = "A/B".len() = 3
    let max_ab_width: u16 = app.branches.iter()
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

    // Name column available width (for ellipsis trimming)
    let highlight_width_for_name = app.symbols.cursor_prefix.len() as u16 + 1;
    let checkbox_width: u16 = 3;
    let age_width: u16 = if hide_age { 0 } else if compact_age { 5 } else { 14 };
    let ab_pr_width: u16 = if hide_ab { 0 } else { max_ab_width + 1 + max_pr_width + 1 };
    // widths count: checkbox + name + optional_age + optional_ab + optional_pr + status
    let gap_count: u16 = 2 + if hide_age { 0 } else { 1 } + if hide_ab { 0 } else { 2 } + 1;
    let name_col_width = main_area.width
        .saturating_sub(2) // borders
        .saturating_sub(highlight_width_for_name)
        .saturating_sub(checkbox_width)
        .saturating_sub(age_width)
        .saturating_sub(ab_pr_width)
        .saturating_sub(status_min_width)
        .saturating_sub(gap_count) as usize;

    // Helper closure to build a Row from a branch index
    let build_row = |i: usize| -> Row {
        let branch = &app.branches[i];
        let is_selected = app.selected[i];
        let is_pinned = branch.is_pinned();

        // Checkbox column — pinned rows show empty space
        let (checkbox_text, checkbox_style) = if is_pinned {
            ("   ".to_string(), Style::default())
        } else if is_selected {
            (app.symbols.checkbox_on.to_string(), app.theme.selected)
        } else {
            (app.symbols.checkbox_off.to_string(), app.theme.secondary_text)
        };

        // Branch name column
        let current_marker = if branch.is_current {
            format!("{} ", app.symbols.current_branch)
        } else {
            "  ".to_string()
        };

        let name_style = if branch.is_current {
            app.theme.current_branch
        } else if is_pinned {
            app.theme.pinned_row
        } else if is_selected {
            app.theme.selected
        } else {
            app.theme.primary_text
        };

        let ellipsis = if app.symbols.cursor_prefix == ">" { "..." } else { "\u{2026}" };
        let name_available = name_col_width.saturating_sub(current_marker.len());
        let display_name = if branch.name.len() > name_available && name_available > ellipsis.len() {
            format!("{}{}", &branch.name[..name_available - ellipsis.len()], ellipsis)
        } else if branch.name.len() > name_available {
            ellipsis.to_string()
        } else {
            branch.name.clone()
        };

        let tracking_text = match &branch.tracking {
            TrackingStatus::Tracked { remote_ref, gone } => {
                if *gone {
                    " (gone)".to_string()
                } else {
                    format!(" \u{2192} {}", remote_ref)
                }
            }
            TrackingStatus::Local => " (local)".to_string(),
        };

        let pinned_label = if branch.is_current && branch.is_base {
            " [base] [current]"
        } else if branch.is_base {
            " [base]"
        } else if branch.is_current {
            " [current]"
        } else {
            ""
        };

        // Build name spans — colorize known prefixes (e.g. fix/, feat/)
        let mut name_spans: Vec<Span> = Vec::new();
        if let Some((prefix_part, rest)) = display_name.split_once('/') {
            if let Some(pstyle) = prefix_style(prefix_part) {
                name_spans.push(Span::styled(
                    format!("{}{}/", current_marker, prefix_part),
                    pstyle,
                ));
                name_spans.push(Span::styled(rest.to_string(), name_style));
            } else {
                // Unknown prefix — render entire name in name_style
                name_spans.push(Span::styled(
                    format!("{}{}", current_marker, display_name),
                    name_style,
                ));
            }
        } else {
            // No slash — render entire name in name_style
            name_spans.push(Span::styled(
                format!("{}{}", current_marker, display_name),
                name_style,
            ));
        }
        name_spans.push(Span::styled(pinned_label, app.theme.secondary_text));
        name_spans.push(Span::styled(tracking_text, app.theme.secondary_text));

        let name_cell = Cell::from(Line::from(name_spans));

        // Build cells dynamically based on visible columns
        let mut cells = vec![
            Cell::from(Span::styled(checkbox_text.clone(), checkbox_style)),
            name_cell,
        ];

        // Age column
        if !hide_age {
            let age = if compact_age {
                branch.age_short()
            } else {
                branch.age_display()
            };
            let age_style = if is_pinned {
                app.theme.pinned_row
            } else {
                age_style(&branch.last_commit_date)
            };
            cells.push(Cell::from(
                Line::from(Span::styled(age, age_style)).alignment(Alignment::Right),
            ));
        }

        // Ahead/behind column
        if !hide_ab {
            let ahead_behind = match (branch.ahead, branch.behind) {
                (Some(a), Some(b)) if a > 0 || b > 0 => {
                    let mut parts = Vec::new();
                    if a > 0 {
                        parts.push(format!("{}{}", app.symbols.arrow_up, a));
                    }
                    if b > 0 {
                        parts.push(format!("{}{}", app.symbols.arrow_down, b));
                    }
                    parts.join("")
                }
                _ => String::new(),
            };
            let ab_style = if is_pinned {
                app.theme.pinned_row
            } else {
                app.theme.ahead_behind
            };
            cells.push(Cell::from(Span::styled(ahead_behind, ab_style)));

            // PR# column
            let pr_text = if is_pinned {
                String::new()
            } else {
                app.pr_map
                    .get(&branch.name)
                    .map(|info| format!("#{}", info.number))
                    .unwrap_or_default()
            };
            let pr_style = if is_pinned {
                app.theme.pinned_row
            } else if let Some(info) = app.pr_map.get(&branch.name) {
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

        // Status column — pinned rows don't show merge status (they are the base)
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

    // Build all rows in display order (pinned first, then non-pinned)
    let all_rows: Vec<Row> = display_indices.iter().map(|&i| build_row(i)).collect();

    // Dynamic column widths based on what's visible
    let mut widths: Vec<Constraint> = vec![
        Constraint::Length(3),              // checkbox always
        Constraint::Min(20),               // name
    ];
    if !hide_age {
        widths.push(Constraint::Length(if compact_age { 5 } else { 14 }));
    }
    if !hide_ab {
        widths.push(Constraint::Length(max_ab_width));  // A/B
        widths.push(Constraint::Length(max_pr_width));  // PR
    }
    widths.push(Constraint::Length(status_min_width)); // status

    // Compute header column x positions for mouse click sorting.
    // The table is inside a block with a 1-cell border on the left, so columns start at x=1.
    // The highlight symbol takes some space; ratatui adds it before the first column.
    // Sort column indices: checkbox=skip, name=0, age=1, A/B(ahead)=2, status=4
    {
        let mut col_positions: Vec<(u16, usize)> = Vec::new();
        // Account for left border (1) + highlight symbol width (cursor_prefix + space)
        let highlight_width = app.symbols.cursor_prefix.len() as u16 + 1;
        let x = main_area.x + 1 + highlight_width;

        // Map table column index to sort column index
        // col 0 = checkbox (no sort), col 1 = name (sort 0), then age/ab/status depending on visibility
        let mut sort_col_map: Vec<Option<usize>> = vec![None]; // checkbox = no sort
        sort_col_map.push(Some(0)); // name
        if !hide_age {
            sort_col_map.push(Some(1)); // age
        }
        if !hide_ab {
            sort_col_map.push(Some(2)); // A/B
            sort_col_map.push(None);    // PR (not sortable)
        }
        sort_col_map.push(Some(4)); // status

        // Resolve constraint widths using the main_area width minus borders and highlight
        let available = main_area.width.saturating_sub(2 + highlight_width);
        let resolved = Layout::horizontal(&widths).split(Rect::new(0, 0, available, 1));

        for (i, rect) in resolved.iter().enumerate() {
            if let Some(&Some(sort_idx)) = sort_col_map.get(i) {
                col_positions.push((x + rect.x, sort_idx));
            }
            // x advances based on resolved rect positions
        }

        app.header_columns = col_positions;
    }

    let highlight_sym = format!("{} ", app.symbols.cursor_prefix);

    // Render all rows in a single stateful table with cursor highlight.
    // Pinned rows appear first (sorted to top), cursor can move onto them.
    let inner_area = block.inner(main_area);
    frame.render_widget(block, main_area);

    let table = Table::new(all_rows, widths)
        .header(header)
        .row_highlight_style(app.theme.cursor)
        .highlight_symbol(highlight_sym);

    frame.render_stateful_widget(table, inner_area, &mut app.table_state);

    // Status bar / search bar
    if app.search_active {
        // Show search input — no clickable items
        app.status_bar_items.clear();
        let search_text = format!(" / {}_", app.search_query);
        let search_bar = Paragraph::new(search_text).style(app.theme.search_bar);
        frame.render_widget(search_bar, status_area);
    } else if !app.search_query.is_empty() {
        // Show active filter indicator in status bar — no clickable items
        app.status_bar_items.clear();
        let filter_text = format!(
            " filter: \"{}\" ({}/{} shown) \u{2014} [\\]filter [/]edit [Esc in /]clear",
            app.search_query, filtered_indices.len(), app.branches.len()
        );
        let status = Paragraph::new(filter_text).style(app.theme.search_bar);
        frame.render_widget(status, status_area);
    } else {
        let selected_count = app.selection_count();
        let total = app.branches.len();
        let merged_count = app
            .branches
            .iter()
            .filter(|b| b.merge_status == MergeStatus::Merged)
            .count();
        let squash_count = app
            .branches
            .iter()
            .filter(|b| b.merge_status == MergeStatus::SquashMerged)
            .count();
        let progress = if app.squash_total > 0 && app.squash_checked < app.squash_total {
            format!(" | checking {}/{}", app.squash_checked, app.squash_total)
        } else {
            String::new()
        };

        // Responsive status bar
        let status_text = if width < 80 {
            format!(
                " {}br {}sel {}m {}s{} \u{2014} [/]search [\\]filter [?]help [q]uit",
                total, selected_count, merged_count, squash_count, progress
            )
        } else {
            format!(
                " {} branches | {} selected | {} merged | {} squashed{} \u{2014} [/]search [\\]filter [c]heckout [d]el [D]el+remote [f]etch [?]help [q]uit",
                total, selected_count, merged_count, squash_count, progress
            )
        };

        // Build clickable status bar item regions.
        // Scan for [X]... patterns and record (x_start, x_end, KeyCode).
        // x_start is at the '[', x_end is after the last char of the word following ']'.
        {
            let mut items: Vec<(u16, u16, KeyCode)> = Vec::new();
            let chars: Vec<char> = status_text.chars().collect();
            let base_x = status_area.x;
            let mut i = 0;
            while i < chars.len() {
                if chars[i] == '[' && i + 2 < chars.len() && chars[i + 2] == ']' {
                    let key_char = chars[i + 1];
                    let key_code = match key_char {
                        '/' => KeyCode::Char('/'),
                        '?' => KeyCode::Char('?'),
                        'q' => KeyCode::Char('q'),
                        'c' => KeyCode::Char('c'),
                        'd' => KeyCode::Char('d'),
                        'D' => KeyCode::Char('D'),
                        'f' => KeyCode::Char('f'),
                        'E' => KeyCode::Char('E'),
                        _ => {
                            i += 1;
                            continue;
                        }
                    };
                    let x_start = base_x + i as u16;
                    // Find end: skip '[X]' then consume word chars (non-space, non-'[')
                    let mut j = i + 3;
                    while j < chars.len() && chars[j] != ' ' && chars[j] != '[' {
                        j += 1;
                    }
                    let x_end = base_x + j as u16;
                    items.push((x_start, x_end, key_code));
                    i = j;
                } else {
                    i += 1;
                }
            }
            app.status_bar_items = items;
        }

        // Build multi-span line: shortcut key tokens [X]word styled with theme.title fg,
        // surrounding text styled with status_bar.
        let key_style = Style::default()
            .fg(app.theme.title.fg.unwrap_or(Color::White))
            .bg(app.theme.status_bar.bg.unwrap_or(Color::Reset))
            .add_modifier(ratatui::style::Modifier::BOLD);
        let mut spans: Vec<Span> = Vec::new();
        let mut remaining = status_text.as_str();
        while let Some(open) = remaining.find('[') {
            // text before '['
            if open > 0 {
                spans.push(Span::styled(remaining[..open].to_string(), app.theme.status_bar));
            }
            remaining = &remaining[open..]; // remaining starts with '['
            if let Some(close) = remaining.find(']') {
                // '[' itself
                spans.push(Span::styled("[".to_string(), app.theme.status_bar));
                // the key letter(s) between '[' and ']'
                spans.push(Span::styled(remaining[1..close].to_string(), key_style));
                // ']' + following word (up to next space or end)
                let after_close = &remaining[close..]; // starts with ']'
                let word_end = after_close[1..].find(' ').map(|i| i + 1).unwrap_or(after_close.len());
                spans.push(Span::styled(after_close[..word_end].to_string(), app.theme.status_bar));
                remaining = &after_close[word_end..];
            } else {
                spans.push(Span::styled(remaining.to_string(), app.theme.status_bar));
                remaining = "";
            }
        }
        if !remaining.is_empty() {
            spans.push(Span::styled(remaining.to_string(), app.theme.status_bar));
        }
        let status = Paragraph::new(Line::from(spans)).style(app.theme.status_bar);
        frame.render_widget(status, status_area);
    }
}
