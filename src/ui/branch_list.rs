use chrono::{DateTime, Utc};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};

use crate::app::App;
use git_branch_manager::types::{MergeStatus, TrackingStatus};
use super::theme;

/// Returns a style for known branch name prefixes (text before the first `/`).
fn prefix_style(prefix: &str) -> Option<Style> {
    match prefix {
        "fix" => Some(Style::new().fg(Color::Red)),
        "feat" | "feature" => Some(Style::new().fg(Color::Green)),
        "chore" => Some(Style::new().fg(Color::Yellow)),
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

/// Trim a branch name to fit within max_len characters, using the given strategy.
fn trim_name(name: &str, max_len: usize, strategy: &str) -> String {
    if name.len() <= max_len {
        return name.to_string();
    }
    if max_len <= 1 {
        return "\u{2026}".to_string();
    }
    match strategy {
        "start" => format!("\u{2026}{}", &name[name.len().saturating_sub(max_len - 1)..]),
        "middle" => {
            let half = (max_len.saturating_sub(1)) / 2;
            format!(
                "{}\u{2026}{}",
                &name[..half],
                &name[name.len().saturating_sub(half)..]
            )
        }
        _ => format!("{}\u{2026}", &name[..max_len.saturating_sub(1)]),
    }
}

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(area);

    let main_area = layout[0];
    let status_area = layout[1];

    // Responsive width thresholds
    let width = main_area.width as usize;
    let compact_age = width < 120;
    let do_trim_names = width < 100;
    let hide_ab = width < 80;
    let short_status = width < 70;
    let hide_age = width < 60;

    // Max branch name length when trimming is active
    let name_max_len = if do_trim_names { 30 } else { usize::MAX };

    // Main branch list
    let wt_status = app.working_tree_status.summary();
    let title = format!(
        "git-branch-manager \u{2014} base: {} [{}]",
        app.base_branch, wt_status
    );
    let block = Block::default()
        .title(title)
        .title_style(theme::TITLE_STYLE)
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
        header_cells.push(Cell::from(sort_label(1, "Age")));
    }
    if !hide_ab {
        header_cells.push(Cell::from(sort_label(2, "A/B")));
    }
    header_cells.push(Cell::from(sort_label(3, "Status")));

    let header = Row::new(header_cells)
        .style(theme::HEADER_STYLE)
        .bottom_margin(0);

    // Build table rows
    let rows: Vec<Row> = app
        .branches
        .iter()
        .enumerate()
        .map(|(i, branch)| {
            let is_selected = app.selected[i];
            let is_pinned = branch.is_pinned();

            // Checkbox column — pinned rows show empty space
            let (checkbox_text, checkbox_style) = if is_pinned {
                ("   ".to_string(), Style::default())
            } else if is_selected {
                (app.symbols.checkbox_on.to_string(), theme::SELECTED_STYLE)
            } else {
                (app.symbols.checkbox_off.to_string(), theme::SECONDARY_TEXT)
            };

            // Branch name column
            let current_marker = if branch.is_current {
                format!("{} ", app.symbols.current_branch)
            } else {
                "  ".to_string()
            };

            let name_style = if branch.is_current {
                theme::CURRENT_BRANCH_STYLE
            } else if is_pinned {
                theme::PINNED_ROW_STYLE
            } else if is_selected {
                theme::SELECTED_STYLE
            } else {
                theme::PRIMARY_TEXT
            };

            let display_name = if do_trim_names {
                trim_name(&branch.name, name_max_len, &app.trim_strategy)
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
            name_spans.push(Span::styled(pinned_label, theme::SECONDARY_TEXT));
            name_spans.push(Span::styled(tracking_text, theme::SECONDARY_TEXT));

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
                    theme::PINNED_ROW_STYLE
                } else {
                    age_style(&branch.last_commit_date)
                };
                cells.push(Cell::from(Span::styled(age, age_style)));
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
                    theme::PINNED_ROW_STYLE
                } else {
                    theme::AHEAD_BEHIND_STYLE
                };
                cells.push(Cell::from(Span::styled(ahead_behind, ab_style)));
            }

            // Status column — pinned rows don't show merge status (they are the base)
            let (status_text, status_style) = if is_pinned {
                (String::new(), theme::PINNED_ROW_STYLE)
            } else if short_status {
                match branch.merge_status {
                    MergeStatus::Merged => ("m".into(), theme::MERGED_STYLE),
                    MergeStatus::SquashMerged => ("s".into(), theme::SQUASH_MERGED_STYLE),
                    MergeStatus::Unmerged => ("u".into(), theme::UNMERGED_STYLE),
                }
            } else {
                match branch.merge_status {
                    MergeStatus::Merged => ("merged".into(), theme::MERGED_STYLE),
                    MergeStatus::SquashMerged => ("squash-merged".into(), theme::SQUASH_MERGED_STYLE),
                    MergeStatus::Unmerged => ("unmerged".into(), theme::UNMERGED_STYLE),
                }
            };
            cells.push(Cell::from(Span::styled(status_text, status_style)));

            Row::new(cells)
        })
        .collect();

    // Dynamic column widths based on what's visible
    let mut widths: Vec<Constraint> = vec![
        Constraint::Length(3),  // checkbox always
        Constraint::Min(20),   // name
    ];
    if !hide_age {
        widths.push(Constraint::Length(if compact_age { 5 } else { 14 }));
    }
    if !hide_ab {
        widths.push(Constraint::Length(6));
    }
    widths.push(Constraint::Length(if short_status { 3 } else { 14 })); // status

    let highlight_sym = format!("{} ", app.symbols.cursor_prefix);
    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .row_highlight_style(theme::CURSOR_STYLE)
        .highlight_symbol(highlight_sym);

    frame.render_stateful_widget(table, main_area, &mut app.table_state);

    // Status bar
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
            " {}br {}sel {}m {}s{} \u{2014} [?]help [q]uit",
            total, selected_count, merged_count, squash_count, progress
        )
    } else {
        format!(
            " {} branches | {} selected | {} merged | {} squashed{} \u{2014} [c]heckout [d]el [D]el+remote [f]etch [?]help [q]uit",
            total, selected_count, merged_count, squash_count, progress
        )
    };
    let status = Paragraph::new(status_text).style(theme::STATUS_BAR_STYLE);
    frame.render_widget(status, status_area);
}
