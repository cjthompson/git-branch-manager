//! Top-level render dispatcher.
//!
//! This module defines the `Overlay` enum and `RenderContext` struct that the App
//! struct will populate in Phase 4, and provides the top-level `draw()` function
//! that dispatches to the appropriate view renderer and overlay.

mod context;
mod status_text;

pub use context::{Overlay, RenderContext};

use ratatui::prelude::*;

use crate::view::ViewId;

use self::status_text::default_status_text;
use super::commit_details::draw_commit_details;
use super::commit_diff::draw_commit_diff;
use super::confirm::{draw_confirm, draw_confirm_cancel};
use super::diagnostics::{draw_diagnostics_menu, draw_diagnostics_report};
use super::executing::draw_executing;
use super::filter_ui::{draw_filter, draw_filter_selected};
use super::graph_render::{draw_graph_options, render_graph_view};
use super::help::draw_help;
use super::info_modal::draw_info_modal;
use super::job_status::render_job_status;
use super::list_render::ListRenderParams;
use super::menu::draw_menu;
use super::results::draw_results;
use super::settings::{draw_settings, settings_rows};
use super::status_bar;
use super::toast::draw_toast;

/// Top-level draw function called by the event loop.
/// Dispatches to the appropriate view renderer + overlay.
pub fn draw(frame: &mut Frame, ctx: &mut RenderContext) {
    let area = frame.area();

    // Layout: main area + non-modal job-status area (only while a confirmed
    // action is running/queued/lingering) + status bar
    let job_rows: u16 = if ctx.job_status.visible { 2 } else { 0 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(job_rows),
            Constraint::Length(1),
        ])
        .split(area);

    let main_area = chunks[0];
    let job_area = chunks[1];
    let status_area = chunks[2];

    // Render active view's list
    match ctx.active_view {
        ViewId::Graph => {
            render_graph_view(frame, main_area, ctx.graph, ctx.theme, ctx.symbols);
        }
        ViewId::Branches => {
            let mut params = ListRenderParams {
                state: ctx.branches,
                columns: ctx.branch_columns,
                active_view: ViewId::Branches,
                render_row: ctx.render_branch_row,
                theme: ctx.theme,
                symbols: ctx.symbols,
                horizontal_scrolling: ctx.config.horizontal_scrolling == Some(true),
            };
            super::list_render::render_list_view(frame, main_area, &mut params);
        }
        ViewId::Remotes => {
            let mut params = ListRenderParams {
                state: ctx.remotes,
                columns: ctx.remote_columns,
                active_view: ViewId::Remotes,
                render_row: ctx.render_remote_row,
                theme: ctx.theme,
                symbols: ctx.symbols,
                horizontal_scrolling: ctx.config.horizontal_scrolling == Some(true),
            };
            super::list_render::render_list_view(frame, main_area, &mut params);
        }
        ViewId::Tags => {
            let mut params = ListRenderParams {
                state: ctx.tags,
                columns: ctx.tag_columns,
                active_view: ViewId::Tags,
                render_row: ctx.render_tag_row,
                theme: ctx.theme,
                symbols: ctx.symbols,
                horizontal_scrolling: ctx.config.horizontal_scrolling == Some(true),
            };
            super::list_render::render_list_view(frame, main_area, &mut params);
        }
        ViewId::Worktrees => {
            let mut params = ListRenderParams {
                state: ctx.worktrees,
                columns: ctx.worktree_columns,
                active_view: ViewId::Worktrees,
                render_row: ctx.render_worktree_row,
                theme: ctx.theme,
                symbols: ctx.symbols,
                horizontal_scrolling: ctx.config.horizontal_scrolling == Some(true),
            };
            super::list_render::render_list_view(frame, main_area, &mut params);
        }
    }

    // Render the non-modal job-status area, if visible
    if ctx.job_status.visible {
        render_job_status(frame, job_area, &ctx.job_status, ctx.theme);
    }

    // Render status bar (search, filter indicator, or normal)
    let search_active = match ctx.active_view {
        ViewId::Graph => false,
        ViewId::Branches => ctx.branches.search_active(),
        ViewId::Remotes => ctx.remotes.search_active(),
        ViewId::Tags => ctx.tags.search_active(),
        ViewId::Worktrees => ctx.worktrees.search_active(),
    };
    let search_query = match ctx.active_view {
        ViewId::Graph => String::new(),
        ViewId::Branches => ctx.branches.search_query().to_string(),
        ViewId::Remotes => ctx.remotes.search_query().to_string(),
        ViewId::Tags => ctx.tags.search_query().to_string(),
        ViewId::Worktrees => ctx.worktrees.search_query().to_string(),
    };
    let filter_query = match ctx.active_view {
        ViewId::Graph => String::new(),
        ViewId::Branches => ctx.branches.filter_query().to_string(),
        ViewId::Remotes => ctx.remotes.filter_query().to_string(),
        ViewId::Tags => ctx.tags.filter_query().to_string(),
        ViewId::Worktrees => ctx.worktrees.filter_query().to_string(),
    };

    if search_active {
        status_bar::render_search_bar(frame, status_area, &search_query, ctx.theme);
    } else if !filter_query.is_empty() {
        let (visible, total) = match ctx.active_view {
            ViewId::Graph => (0, 0),
            ViewId::Branches => (
                ctx.branches.display_indices().len(),
                ctx.branches.items().len(),
            ),
            ViewId::Remotes => (
                ctx.remotes.display_indices().len(),
                ctx.remotes.items().len(),
            ),
            ViewId::Tags => (ctx.tags.display_indices().len(), ctx.tags.items().len()),
            ViewId::Worktrees => (
                ctx.worktrees.display_indices().len(),
                ctx.worktrees.items().len(),
            ),
        };
        status_bar::render_filter_indicator(
            frame,
            status_area,
            &filter_query,
            visible,
            total,
            ctx.theme,
        );
    } else {
        let status_text = default_status_text(ctx);
        let items = status_bar::render_status_bar(frame, status_area, &status_text, ctx.theme);
        // Store status bar items for mouse handler
        let converted: Vec<(u16, u16, crossterm::event::KeyCode)> =
            items.iter().map(|i| (i.x_start, i.x_end, i.key)).collect();
        match ctx.active_view {
            ViewId::Graph => {}
            ViewId::Branches => ctx.branches.status_bar_items = converted,
            ViewId::Remotes => ctx.remotes.status_bar_items = converted,
            ViewId::Tags => ctx.tags.status_bar_items = converted,
            ViewId::Worktrees => ctx.worktrees.status_bar_items = converted,
        }
    }

    // Render overlay if present
    if let Some(overlay) = ctx.overlay.as_deref_mut() {
        match overlay {
            Overlay::Help { scroll } => {
                draw_help(frame, ctx.active_view, scroll, ctx.theme);
            }
            Overlay::Menu { items, cursor } => {
                draw_menu(frame, items, *cursor, ctx.theme, ctx.symbols);
            }
            Overlay::InfoModal {
                items,
                cursor,
                info_cursor,
                focus,
                row,
            } => {
                draw_info_modal(
                    frame,
                    row,
                    items,
                    *cursor,
                    *focus,
                    *info_cursor,
                    ctx.info_modal_scroll_offset,
                    ctx.info_copied_msg,
                    ctx.info_hit_regions,
                    ctx.theme,
                    ctx.symbols,
                );
            }
            Overlay::CommitDetails {
                commit,
                details,
                items,
                cursor,
                file_cursor,
                focus,
                scroll,
                ..
            } => draw_commit_details(
                frame,
                commit,
                details,
                items,
                *cursor,
                *file_cursor,
                *focus,
                scroll,
                ctx.theme,
                ctx.symbols,
            ),
            Overlay::CommitDiff { diff, scroll, .. } => {
                draw_commit_diff(frame, diff, scroll, ctx.theme);
            }
            Overlay::Confirm {
                preflight,
                choices,
                selected,
                body_scroll,
                stage,
            } => draw_confirm(
                frame,
                preflight,
                choices,
                *selected,
                body_scroll,
                stage,
                ctx.theme,
            ),
            Overlay::ConfirmCancelJob { message } => {
                draw_confirm_cancel(frame, message, ctx.theme);
            }
            Overlay::Executing { label, progress } => {
                draw_executing(frame, label, progress.as_ref(), ctx.theme);
            }
            Overlay::Results {
                results,
                selected_index,
                expanded_index,
                focus,
                body_scroll,
            } => draw_results(
                frame,
                results,
                selected_index,
                expanded_index,
                focus,
                body_scroll,
                ctx.theme,
            ),
            Overlay::Settings { cursor } => {
                let branch_sort = crate::view::sort_keys::display_string(
                    ctx.branch_columns,
                    ctx.config.sort_column_branches.as_deref(),
                    ctx.config.sort_asc_branches.unwrap_or(true),
                );
                let remote_sort = crate::view::sort_keys::display_string(
                    ctx.remote_columns,
                    ctx.config.sort_column_remotes.as_deref(),
                    ctx.config.sort_asc_remotes.unwrap_or(true),
                );
                let tag_sort = crate::view::sort_keys::display_string(
                    ctx.tag_columns,
                    ctx.config.sort_column_tags.as_deref(),
                    ctx.config.sort_asc_tags.unwrap_or(true),
                );
                let worktree_sort = crate::view::sort_keys::display_string(
                    ctx.worktree_columns,
                    ctx.config.sort_column_worktrees.as_deref(),
                    ctx.config.sort_asc_worktrees.unwrap_or(true),
                );
                let rows = settings_rows(
                    ctx.symbols,
                    ctx.theme,
                    ctx.config,
                    &branch_sort,
                    &remote_sort,
                    &tag_sort,
                    &worktree_sort,
                );
                draw_settings(frame, *cursor, &rows, ctx.theme);
            }
            Overlay::Filter => {
                let title = match ctx.active_view {
                    ViewId::Graph => "Graph Filters",
                    ViewId::Branches => "Filters",
                    ViewId::Remotes => "Remote Filters",
                    ViewId::Tags => "Tag Filters",
                    ViewId::Worktrees => "Worktree Filters",
                };
                draw_filter(
                    frame,
                    ctx.active_filter_tokens,
                    &filter_query,
                    title,
                    ctx.theme,
                );
            }
            Overlay::FilterSelection { cursor } => {
                let title = match ctx.active_view {
                    ViewId::Graph => "Graph Filters",
                    ViewId::Branches => "Filters",
                    ViewId::Remotes => "Remote Filters",
                    ViewId::Tags => "Tag Filters",
                    ViewId::Worktrees => "Worktree Filters",
                };
                draw_filter_selected(
                    frame,
                    ctx.active_filter_tokens,
                    &filter_query,
                    title,
                    *cursor,
                    ctx.theme,
                );
            }
            Overlay::GraphOptions {
                cursor,
                include_remotes,
            } => {
                draw_graph_options(frame, *cursor, *include_remotes, ctx.theme);
            }
            Overlay::Diagnostics { cursor } => {
                draw_diagnostics_menu(frame, *cursor, ctx.theme);
            }
            Overlay::DiagnosticsReport { audit, scroll } => {
                draw_diagnostics_report(frame, audit, *scroll, ctx.theme);
            }
        }
    }

    // Render toast if present -- positioned above whatever's reserved at the
    // bottom (status bar, plus the job-status area when it's showing).
    if let Some(toast) = ctx.toast {
        draw_toast(frame, toast, ctx.theme, 1 + job_rows);
    }
}

#[cfg(test)]
mod tests;
