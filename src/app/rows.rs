use super::*;

/// Get branch prefix style: extract prefix before first '/' and look up color.
pub(super) fn branch_prefix_style(name: &str, theme: &Theme) -> Style {
    let prefix = name.split('/').next().unwrap_or(name);
    prefix_style(prefix, theme).unwrap_or_default()
}

pub(super) fn visible_data_col_width(
    visible_cols: &[usize],
    ctx: &CellContext,
    col_idx: usize,
) -> Option<usize> {
    visible_cols
        .iter()
        .position(|&visible_col_idx| visible_col_idx == col_idx)
        .and_then(|visible_pos| ctx.data_col_widths.get(visible_pos))
        .map(|&width| width as usize)
}

pub(super) fn age_text_for_column(
    visible_cols: &[usize],
    ctx: &CellContext,
    col_idx: usize,
    long: String,
    short: String,
) -> String {
    let col_width = visible_data_col_width(visible_cols, ctx, col_idx);
    fit_text(long, short, col_width, ctx.compact)
}

// ---- Row Renderers ----

pub(crate) fn render_branch_row(
    item: &BranchInfo,
    _raw_idx: usize,
    _is_selected: bool,
    _is_cursor: bool,
    visible_cols: &[usize],
    ctx: &CellContext,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let theme = ctx.theme;
    let symbols = ctx.symbols;

    for &col_idx in visible_cols {
        match col_idx {
            0 => {
                let (text, style) = match &item.tracking {
                    TrackingStatus::Tracked { gone: false, .. } => {
                        (symbols.graph_remote_ref.to_string(), theme.remote_title)
                    }
                    TrackingStatus::Tracked { gone: true, .. } => {
                        ("gone".to_string(), theme.secondary_text)
                    }
                    TrackingStatus::Local => ("-".to_string(), theme.secondary_text),
                };
                lines.push(Line::from(Span::styled(text, style)));
            }
            1 => {
                // Branch name
                let style = if item.is_current {
                    theme.current_branch
                } else {
                    branch_prefix_style(&item.name, theme)
                };
                let prefix = if item.is_current {
                    format!("{} ", symbols.current_branch)
                } else {
                    String::new()
                };
                // For non-base branches, append base info
                let suffix = if item.is_base {
                    " [base]".to_string()
                } else {
                    match &item.merge_base_commit {
                        Some(hash) => format!(" ({} - {})", item.base_branch, hash),
                        None => String::new(),
                    }
                };
                let name = format!("{prefix}{}{suffix}", item.name);
                lines.push(Line::from(Span::styled(name, style)));
            }
            2 => {
                lines.push(ahead_behind_line(
                    item.ahead,
                    item.behind,
                    ctx,
                    visible_data_col_width(visible_cols, ctx, col_idx),
                ));
            }
            3 => {
                lines.push(pr_line(
                    item.pr.as_ref(),
                    ctx,
                    visible_data_col_width(visible_cols, ctx, col_idx),
                ));
            }
            4 => {
                let age = age_text_for_column(
                    visible_cols,
                    ctx,
                    col_idx,
                    item.age_display(),
                    item.age_short(),
                );
                lines.push(age_line(age, &item.last_commit_date, ctx));
            }
            5 => {
                lines.push(merge_status_line_for_branch(
                    &item.merge_status,
                    item.is_base,
                    ctx,
                    visible_data_col_width(visible_cols, ctx, col_idx),
                ));
            }
            _ => lines.push(Line::from("")),
        }
    }
    lines
}

pub(crate) fn render_remote_row(
    item: &RemoteBranchInfo,
    _raw_idx: usize,
    _is_selected: bool,
    _is_cursor: bool,
    visible_cols: &[usize],
    ctx: &CellContext,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let theme = ctx.theme;
    let symbols = ctx.symbols;

    for &col_idx in visible_cols {
        match col_idx {
            0 => {
                let text = if item.has_local {
                    symbols.graph_local_ref.to_string()
                } else {
                    "-".to_string()
                };
                let style = if item.has_local {
                    theme.merged
                } else {
                    theme.secondary_text
                };
                lines.push(Line::from(Span::styled(text, style)));
            }
            1 => {
                // Name: full remote branch name (e.g. "origin/feature/test")
                let prefix = item
                    .short_name
                    .split('/')
                    .next()
                    .unwrap_or(&item.short_name);
                let style = prefix_style(prefix, theme).unwrap_or(theme.primary_text);
                let name = if item.is_base {
                    format!("{} [base]", item.full_ref)
                } else {
                    item.full_ref.clone()
                };
                lines.push(Line::from(Span::styled(name, style)));
            }
            2 => {
                // Disjoint remotes share no history with base; their ahead/behind are
                // full history sizes (misleading), so show the disjoint marker instead.
                if item.disjoint {
                    lines.push(Line::from(Span::styled(
                        symbols.disjoint.to_string(),
                        theme.secondary_text,
                    )));
                } else {
                    lines.push(ahead_behind_line(
                        item.ahead,
                        item.behind,
                        ctx,
                        visible_data_col_width(visible_cols, ctx, col_idx),
                    ));
                }
            }
            3 => {
                lines.push(pr_line(
                    item.pr.as_ref(),
                    ctx,
                    visible_data_col_width(visible_cols, ctx, col_idx),
                ));
            }
            4 => {
                let age = age_text_for_column(
                    visible_cols,
                    ctx,
                    col_idx,
                    item.age_display(),
                    item.age_short(),
                );
                lines.push(age_line(age, &item.last_commit_date, ctx));
            }
            5 => {
                lines.push(merge_status_line(
                    &item.merge_status,
                    ctx,
                    visible_data_col_width(visible_cols, ctx, col_idx),
                ));
            }
            _ => lines.push(Line::from("")),
        }
    }
    lines
}

pub(crate) fn render_tag_row(
    item: &TagInfo,
    _raw_idx: usize,
    _is_selected: bool,
    _is_cursor: bool,
    visible_cols: &[usize],
    ctx: &CellContext,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let theme = ctx.theme;

    for &col_idx in visible_cols {
        match col_idx {
            0 => {
                // Tag name
                let style = branch_prefix_style(&item.name, theme);
                lines.push(Line::from(Span::styled(item.name.clone(), style)));
            }
            1 => {
                // Commit hash (short)
                let hash = if item.commit_hash.len() > 8 {
                    &item.commit_hash[..8]
                } else {
                    &item.commit_hash
                };
                lines.push(Line::from(Span::styled(
                    hash.to_string(),
                    theme.secondary_text,
                )));
            }
            2 => {
                let age = age_text_for_column(
                    visible_cols,
                    ctx,
                    col_idx,
                    item.age_display(),
                    item.age_short(),
                );
                lines.push(age_line(age, &item.date, ctx));
            }
            3 => {
                // Message
                let msg = item
                    .message
                    .as_deref()
                    .unwrap_or("")
                    .lines()
                    .next()
                    .unwrap_or("");
                let max_width = if ctx.area_width > 60 {
                    (ctx.area_width - 60) as usize
                } else {
                    20
                };
                let text = truncate(msg, max_width);
                lines.push(Line::from(Span::styled(text, theme.secondary_text)));
            }
            _ => lines.push(Line::from("")),
        }
    }
    lines
}

pub(crate) fn render_worktree_row(
    item: &WorktreeInfo,
    _raw_idx: usize,
    _is_selected: bool,
    _is_cursor: bool,
    visible_cols: &[usize],
    ctx: &CellContext,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let theme = ctx.theme;

    for &col_idx in visible_cols {
        match col_idx {
            0 => {
                // Path — abbreviate leading dirs / left-truncate so the END of
                // the path stays visible when the column is too narrow for it.
                let path_str = abbreviate_path(&item.path, ctx.first_col_width as usize);
                let style = if item.is_main {
                    theme.current_branch
                } else {
                    Style::default()
                };
                lines.push(Line::from(Span::styled(path_str, style)));
            }
            1 => {
                // Branch name
                let name = item.branch.as_deref().unwrap_or("[detached]");
                let style = prefix_style(name, theme).unwrap_or(theme.primary_text);
                let col_width = visible_data_col_width(visible_cols, ctx, col_idx);
                let display_name = match col_width {
                    Some(0) => String::new(),
                    Some(width) => truncate_left(name, width),
                    None => name.to_string(),
                };
                let mut line = Line::from(Span::styled(display_name, style));
                if col_width.is_some_and(|width| name.chars().count() > width) {
                    line = line.alignment(Alignment::Right);
                }
                lines.push(line);
            }
            2 => {
                // Working tree status — full words when wide, single letters when narrow.
                lines.push(worktree_status_line(
                    &item.wt_status,
                    ctx,
                    visible_data_col_width(visible_cols, ctx, col_idx),
                ));
            }
            3 => {
                let age = age_text_for_column(
                    visible_cols,
                    ctx,
                    col_idx,
                    item.age_display(),
                    item.age_short(),
                );
                lines.push(age_line(age, &item.age_date, ctx));
            }
            4 => {
                // Blank the merge cell for the base-branch worktree, mirroring
                // the Branches view (a branch can't be merged into itself).
                lines.push(merge_status_line_for_branch(
                    &item.merge_status,
                    item.is_base,
                    ctx,
                    visible_data_col_width(visible_cols, ctx, col_idx),
                ));
            }
            _ => lines.push(Line::from("")),
        }
    }
    lines
}
