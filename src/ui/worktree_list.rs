use chrono::{DateTime, Utc};
use crossterm::event::KeyCode;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table};

use crate::app::App;
use git_branch_manager::git::github::PrStatus;
use git_branch_manager::types::MergeStatus;

/// Returns a color style based on how old a date is.
fn age_style(date: &DateTime<Utc>) -> Style {
    let days = (Utc::now() - *date).num_days();
    if days < 7 {
        Style::new().fg(Color::Green)
    } else if days < 30 {
        Style::new().fg(Color::Yellow)
    } else if days < 90 {
        Style::new().fg(Color::Indexed(208))
    } else {
        Style::new().fg(Color::Red)
    }
}

/// Truncates `s` to fit within `max_chars`, appending `ellipsis` if truncated.
fn truncate(s: &str, max_chars: usize, ellipsis: &str) -> String {
    if s.len() <= max_chars {
        s.to_string()
    } else if max_chars > ellipsis.len() {
        format!("{}{}", &s[..max_chars - ellipsis.len()], ellipsis)
    } else {
        ellipsis.to_string()
    }
}

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
    let hide_ab = width < 90;
    let short_status = width < 70;
    let hide_age = width < 60;

    let title = format!(
        "git-branch-manager \u{2014} worktrees (base: {})",
        app.base_branch
    );
    let block = Block::default()
        .title(title)
        .title_style(app.theme.title)
        .borders(Borders::ALL);

    // Header
    let mut header_cells = vec![Cell::from("Path"), Cell::from("Branch")];
    if !hide_age {
        header_cells.push(Cell::from(
            Line::from("Age").alignment(Alignment::Right),
        ));
    }
    if !hide_ab {
        header_cells.push(Cell::from("A/B"));
        header_cells.push(Cell::from("PR"));
    }
    header_cells.push(Cell::from(
        Line::from("Status").alignment(Alignment::Right),
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
        + age_width
        + ab_pr_width
        + status_min_width
        + 3; // gaps between columns
    let remaining = main_area.width.saturating_sub(fixed_width) as usize;
    let path_col_width = remaining / 2;
    let branch_col_width = remaining - path_col_width;

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
        let branch_cell = Cell::from(Line::from(vec![
            Span::styled(display_branch, branch_style),
            Span::styled(wts_label, app.theme.secondary_text),
        ]));

        let mut cells = vec![path_cell, branch_cell];

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

        // Ahead/behind + PR columns
        if !hide_ab {
            let ahead_behind = if is_pinned || wt.branch.is_none() {
                String::new()
            } else {
                match (wt.ahead, wt.behind) {
                    (Some(a), Some(b)) if a > 0 || b > 0 => {
                        let mut parts = Vec::new();
                        if a > 0 { parts.push(format!("{}{}", app.symbols.arrow_up, a)); }
                        if b > 0 { parts.push(format!("{}{}", app.symbols.arrow_down, b)); }
                        parts.join("")
                    }
                    _ => String::new(),
                }
            };
            let ab_style = if is_pinned {
                app.theme.pinned_row
            } else {
                app.theme.ahead_behind
            };
            cells.push(Cell::from(Span::styled(ahead_behind, ab_style)));

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
            }
        };
        cells.push(Cell::from(
            Line::from(Span::styled(status_text, status_style)).alignment(Alignment::Right),
        ));

        Row::new(cells)
    };

    let all_rows: Vec<Row> = (0..app.worktrees.len()).map(build_row).collect();

    let mut widths: Vec<Constraint> = vec![
        Constraint::Length(path_col_width as u16),
        Constraint::Min(10),
    ];
    if !hide_age {
        widths.push(Constraint::Length(age_width));
    }
    if !hide_ab {
        widths.push(Constraint::Length(max_ab_width));
        widths.push(Constraint::Length(max_pr_width));
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
        let status_text = if width < 80 {
            format!(
                " {} worktrees \u{2014} [d]rm [D]force-rm [w]branches [?]help [q]uit",
                total
            )
        } else {
            format!(
                " {} worktrees \u{2014} [Enter]menu [d]remove [D]force-remove [w]branches [r]emotes [t]ags [?]help [q]uit",
                total
            )
        };

        // Build clickable regions
        {
            let mut items: Vec<(u16, u16, KeyCode)> = Vec::new();
            let chars: Vec<char> = status_text.chars().collect();
            let base_x = status_area.x;
            let mut i = 0;
            while i < chars.len() {
                if chars[i] == '[' && i + 2 < chars.len() && chars[i + 2] == ']' {
                    let key_char = chars[i + 1];
                    let key_code = match key_char {
                        'd' => KeyCode::Char('d'),
                        'D' => KeyCode::Char('D'),
                        'w' => KeyCode::Char('w'),
                        'r' => KeyCode::Char('r'),
                        't' => KeyCode::Char('t'),
                        '?' => KeyCode::Char('?'),
                        'q' => KeyCode::Char('q'),
                        _ => { i += 1; continue; }
                    };
                    let x_start = base_x + i as u16;
                    let mut j = i + 3;
                    while j < chars.len() && chars[j] != ' ' && chars[j] != '[' {
                        j += 1;
                    }
                    items.push((x_start, base_x + j as u16, key_code));
                    i = j;
                } else {
                    i += 1;
                }
            }
            app.worktree_status_bar_items = items;
        }

        // Render with key characters highlighted
        let key_style = Style::default()
            .fg(app.theme.title.fg.unwrap_or(Color::White))
            .bg(app.theme.status_bar.bg.unwrap_or(Color::Reset))
            .add_modifier(ratatui::style::Modifier::BOLD);
        let mut spans: Vec<Span> = Vec::new();
        let mut remaining = status_text.as_str();
        while let Some(open) = remaining.find('[') {
            if open > 0 {
                spans.push(Span::styled(remaining[..open].to_string(), app.theme.status_bar));
            }
            remaining = &remaining[open..];
            if let Some(close) = remaining.find(']') {
                spans.push(Span::styled("[".to_string(), app.theme.status_bar));
                spans.push(Span::styled(remaining[1..close].to_string(), key_style));
                let after_close = &remaining[close..];
                let word_end = after_close[1..]
                    .find(|c: char| c == ' ' || c == '[')
                    .map(|idx| idx + 1)
                    .unwrap_or(after_close.len());
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

    // Loading toast — shown while worktrees are being fetched
    if app.worktree_loading {
        let msg = " Loading worktrees\u{2026} ";
        let toast_width = msg.len() as u16 + 2;
        let toast_height: u16 = 3;
        let x = area.width.saturating_sub(toast_width).saturating_sub(1);
        let y = area.height.saturating_sub(toast_height).saturating_sub(2);
        let toast_area = Rect::new(x, y, toast_width, toast_height);

        let toast = Paragraph::new(msg)
            .style(app.theme.toast_text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(app.theme.toast_border),
            )
            .alignment(Alignment::Center);
        frame.render_widget(Clear, toast_area);
        frame.render_widget(toast, toast_area);
    }
}
