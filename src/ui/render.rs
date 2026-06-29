//! Top-level render dispatcher.
//!
//! This module defines the `Overlay` enum and `RenderContext` struct that the App
//! struct will populate in Phase 4, and provides the top-level `draw()` function
//! that dispatches to the appropriate view renderer and overlay.

use ratatui::prelude::*;

use crate::config::Config;
use crate::symbols::SymbolSet;
use crate::theme::Theme;
use crate::types::*;
use crate::view::column::ColumnDef;
use crate::view::filter::FilterTokenDef;
use crate::view::list_state::ListState;
use crate::view::ViewId;
use crate::view::ViewItem;

use super::confirm::draw_confirm;
use super::diagnostics::{draw_diagnostics_menu, draw_diagnostics_report};
use super::executing::draw_executing;
use super::filter_ui::draw_filter;
use super::help::draw_help;
use super::info_modal::{draw_info_modal, InfoHitRegion, InfoModalRow};
use super::list_render::{ListRenderParams, RowRenderer};
use super::menu::{draw_menu, MenuItem};
use super::results::draw_results;
use super::settings::{draw_settings, settings_rows};
use super::status_bar;
use super::toast::{draw_toast, Toast};

/// Overlay state for the top-level renderer.
#[derive(Debug, Clone)]
pub enum Overlay {
    Help,
    Menu {
        items: Vec<MenuItem>,
        cursor: usize,
    },
    InfoModal {
        items: Vec<MenuItem>,
        cursor: usize,
        row: InfoModalRow,
        scroll_offset: u16,
    },
    Confirm {
        action: BranchAction,
        targets: Vec<String>,
    },
    Executing {
        label: String,
        progress: Option<ProgressUpdate>,
    },
    Results {
        results: Vec<OperationResult>,
    },
    Settings {
        cursor: usize,
    },
    Filter,
    /// Diagnostics menu: pick a debugging tool to run.
    Diagnostics {
        cursor: usize,
    },
    /// Result of a cache-accuracy audit, with an optional one-key fix.
    DiagnosticsReport {
        audit: CacheAudit,
        scroll: usize,
    },
}

/// Everything the renderer needs to draw one frame.
/// This avoids coupling to the full App struct (which is built in Phase 4).
pub struct RenderContext<'a> {
    pub active_view: ViewId,
    pub overlay: Option<&'a Overlay>,
    pub toast: Option<&'a Toast>,
    pub theme: &'a Theme,
    pub symbols: &'a SymbolSet,
    pub config: &'a Config,
    // Info modal: confirmation message + recorded click-to-copy hit regions
    pub info_copied_msg: Option<&'a str>,
    pub info_hit_regions: &'a mut Vec<InfoHitRegion>,
    // List states
    pub branches: &'a mut ListState<BranchInfo>,
    pub remotes: &'a mut ListState<RemoteBranchInfo>,
    pub tags: &'a mut ListState<TagInfo>,
    pub worktrees: &'a mut ListState<WorktreeInfo>,
    // Column definitions
    pub branch_columns: &'a [ColumnDef<BranchInfo>],
    pub remote_columns: &'a [ColumnDef<RemoteBranchInfo>],
    pub tag_columns: &'a [ColumnDef<TagInfo>],
    pub worktree_columns: &'a [ColumnDef<WorktreeInfo>],
    // Filter tokens (for filter overlay)
    pub active_filter_tokens: &'a [FilterTokenDef],
    // Row renderers
    pub render_branch_row: RowRenderer<BranchInfo>,
    pub render_remote_row: RowRenderer<RemoteBranchInfo>,
    pub render_tag_row: RowRenderer<TagInfo>,
    pub render_worktree_row: RowRenderer<WorktreeInfo>,
}

/// Top-level draw function called by the event loop.
/// Dispatches to the appropriate view renderer + overlay.
pub fn draw(frame: &mut Frame, ctx: &mut RenderContext) {
    let area = frame.area();

    // Layout: main area + status bar
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(area);

    let main_area = chunks[0];
    let status_area = chunks[1];

    // Render active view's list
    match ctx.active_view {
        ViewId::Branches => {
            let mut params = ListRenderParams {
                state: ctx.branches,
                columns: ctx.branch_columns,
                active_view: ViewId::Branches,
                render_row: ctx.render_branch_row,
                theme: ctx.theme,
                symbols: ctx.symbols,
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
            };
            super::list_render::render_list_view(frame, main_area, &mut params);
        }
    }

    // Render status bar (search, filter indicator, or normal)
    let search_active = match ctx.active_view {
        ViewId::Branches => ctx.branches.search_active(),
        ViewId::Remotes => ctx.remotes.search_active(),
        ViewId::Tags => ctx.tags.search_active(),
        ViewId::Worktrees => ctx.worktrees.search_active(),
    };
    let search_query = match ctx.active_view {
        ViewId::Branches => ctx.branches.search_query().to_string(),
        ViewId::Remotes => ctx.remotes.search_query().to_string(),
        ViewId::Tags => ctx.tags.search_query().to_string(),
        ViewId::Worktrees => ctx.worktrees.search_query().to_string(),
    };
    let filter_query = match ctx.active_view {
        ViewId::Branches => ctx.branches.filter_query().to_string(),
        ViewId::Remotes => ctx.remotes.filter_query().to_string(),
        ViewId::Tags => ctx.tags.filter_query().to_string(),
        ViewId::Worktrees => ctx.worktrees.filter_query().to_string(),
    };

    if search_active {
        status_bar::render_search_bar(frame, status_area, &search_query, ctx.theme);
    } else if !filter_query.is_empty() {
        let (visible, total) = match ctx.active_view {
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
            ViewId::Branches => ctx.branches.status_bar_items = converted,
            ViewId::Remotes => ctx.remotes.status_bar_items = converted,
            ViewId::Tags => ctx.tags.status_bar_items = converted,
            ViewId::Worktrees => ctx.worktrees.status_bar_items = converted,
        }
    }

    // Render overlay if present
    if let Some(overlay) = ctx.overlay {
        match overlay {
            Overlay::Help => {
                draw_help(frame, ctx.active_view, ctx.theme);
            }
            Overlay::Menu { items, cursor } => {
                let anchor = match ctx.active_view {
                    ViewId::Branches => {
                        ctx.branches.table_state().selected().unwrap_or(0) as u16 + 2
                    }
                    ViewId::Remotes => ctx.remotes.table_state().selected().unwrap_or(0) as u16 + 2,
                    ViewId::Tags => ctx.tags.table_state().selected().unwrap_or(0) as u16 + 2,
                    ViewId::Worktrees => {
                        ctx.worktrees.table_state().selected().unwrap_or(0) as u16 + 2
                    }
                };
                draw_menu(frame, items, *cursor, anchor, ctx.theme, ctx.symbols);
            }
            Overlay::InfoModal {
                items,
                cursor,
                row,
                scroll_offset,
            } => {
                draw_info_modal(
                    frame,
                    row,
                    items,
                    *cursor,
                    *scroll_offset,
                    ctx.info_copied_msg,
                    ctx.info_hit_regions,
                    ctx.theme,
                    ctx.symbols,
                );
            }
            Overlay::Confirm { action, targets } => {
                draw_confirm(frame, *action, targets, ctx.theme);
            }
            Overlay::Executing { label, progress } => {
                draw_executing(frame, label, progress.as_ref(), ctx.theme);
            }
            Overlay::Results { results } => {
                draw_results(frame, results, ctx.theme);
            }
            Overlay::Settings { cursor } => {
                let rows = settings_rows(ctx.symbols, ctx.theme, ctx.config);
                draw_settings(frame, *cursor, &rows, ctx.theme);
            }
            Overlay::Filter => {
                let title = match ctx.active_view {
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
            Overlay::Diagnostics { cursor } => {
                draw_diagnostics_menu(frame, *cursor, ctx.theme);
            }
            Overlay::DiagnosticsReport { audit, scroll } => {
                draw_diagnostics_report(frame, audit, *scroll, ctx.theme);
            }
        }
    }

    // Render toast if present
    if let Some(toast) = ctx.toast {
        draw_toast(frame, toast, ctx.theme);
    }
}

/// Build default status bar text based on the active view.
fn default_status_text(ctx: &RenderContext) -> String {
    match ctx.active_view {
        ViewId::Branches => format_branch_like(
            "branches",
            branch_like_summary(ctx.branches),
            "[/]search [\\]filter [c]heckout [d]el [D]el+remote [p]ush [f]etch [F2]diag [?]help [q]uit",
        ),
        ViewId::Remotes => format_branch_like(
            "remote branches",
            branch_like_summary(ctx.remotes),
            "[/]search [\\]filter [c]heckout [d]el [f]etch [?]help [q]uit",
        ),
        ViewId::Tags => {
            let total = ctx.tags.items().len();
            format!(
                " {} tags \u{2014} [/]search [\\]filter [d]el [D]el+remote [p]ush [f]etch [?]help [q]uit",
                total
            )
        }
        ViewId::Worktrees => {
            let total = ctx.worktrees.items().len();
            format!(
                " {} worktrees \u{2014} [/]search [d]el [D]force-del [f]etch [?]help [q]uit",
                total
            )
        }
    }
}

/// Counts `(total, selected, merged, squashed)` for a branch-like list view.
/// Works for any item type whose `ViewItem::merge_status` is populated.
fn branch_like_summary<T: ViewItem>(state: &ListState<T>) -> (usize, usize, usize, usize) {
    let total = state.items().len();
    let selected = state.selected().iter().filter(|&&s| s).count();
    let merged = state
        .items()
        .iter()
        .filter(|i| i.merge_status() == Some(&MergeStatus::Merged))
        .count();
    let squashed = state
        .items()
        .iter()
        .filter(|i| i.merge_status() == Some(&MergeStatus::SquashMerged))
        .count();
    (total, selected, merged, squashed)
}

/// Formats a branch-like status bar line from a noun, summary counts, and the
/// view's shortcut suffix. Branches and Remotes share this formatter; only the
/// noun and the shortcut list differ between them.
fn format_branch_like(
    noun: &str,
    summary: (usize, usize, usize, usize),
    shortcuts: &str,
) -> String {
    let (total, selected, merged, squashed) = summary;
    format!(
        " {total} {noun} | {selected} selected | {merged} merged | {squashed} squashed \u{2014} {shortcuts}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn branch(name: &str, status: MergeStatus) -> BranchInfo {
        BranchInfo {
            name: name.to_string(),
            is_current: false,
            is_base: false,
            tracking: TrackingStatus::Local,
            ahead: None,
            behind: None,
            last_commit_date: Utc::now(),
            merge_status: status,
            base_branch: "main".into(),
            merge_base_commit: None,
            pr: None,
        }
    }

    #[test]
    fn branch_like_summary_counts_statuses_and_selection() {
        let mut state = ListState::new(vec![
            branch("a", MergeStatus::Merged),
            branch("b", MergeStatus::SquashMerged),
            branch("c", MergeStatus::Unmerged),
            branch("d", MergeStatus::Merged),
        ]);
        // Select the first two.
        state.selected_mut()[0] = true;
        state.selected_mut()[1] = true;

        let (total, selected, merged, squashed) = branch_like_summary(&state);
        assert_eq!(total, 4);
        assert_eq!(selected, 2);
        assert_eq!(merged, 2);
        assert_eq!(squashed, 1);
    }

    #[test]
    fn branch_like_summary_empty() {
        let state: ListState<BranchInfo> = ListState::new(vec![]);
        assert_eq!(branch_like_summary(&state), (0, 0, 0, 0));
    }

    #[test]
    fn format_branch_like_preserves_shape() {
        let text = format_branch_like("branches", (4, 2, 2, 1), "[/]search [q]uit");
        assert_eq!(
            text,
            " 4 branches | 2 selected | 2 merged | 1 squashed \u{2014} [/]search [q]uit"
        );
    }
}
