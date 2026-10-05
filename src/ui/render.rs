//! Top-level render dispatcher.
//!
//! This module defines the `Overlay` enum and `RenderContext` struct that the App
//! struct will populate in Phase 4, and provides the top-level `draw()` function
//! that dispatches to the appropriate view renderer and overlay.

use ratatui::prelude::*;

use crate::config::Config;
use crate::git;
use crate::git::graph;
use crate::symbols::SymbolSet;
use crate::theme::Theme;
use crate::types::*;
use crate::view::column::ColumnDef;
use crate::view::filter::FilterTokenDef;
use crate::view::graph::GraphState;
use crate::view::list_state::ListState;
use crate::view::ViewId;
use crate::view::ViewItem;

use crate::job_queue::JobStatusView;

use super::commit_details::{draw_commit_details, CommitDetailsFocus};
use super::commit_diff::draw_commit_diff;
use super::confirm::{
    draw_confirm, draw_confirm_cancel, ConfirmChoice, ConfirmStage, DeletePreflight,
};
use super::diagnostics::{draw_diagnostics_menu, draw_diagnostics_report};
use super::executing::draw_executing;
use super::filter_ui::{draw_filter, draw_filter_selected};
use super::graph_render::{draw_graph_options, render_graph_view};
use super::help::draw_help;
use super::info_modal::{draw_info_modal, InfoHitRegion, InfoModalFocus, InfoModalRow};
use super::job_status::render_job_status;
use super::list_render::{ListRenderParams, RowRenderer};
use super::menu::{draw_menu, MenuItem};
use super::modal::ModalScroll;
use super::results::{draw_results, ResultsFocus};
use super::settings::{draw_settings, settings_rows};
use super::status_bar;
use super::toast::{draw_toast, Toast};

/// Overlay state for the top-level renderer.
#[derive(Debug, Clone)]
pub enum Overlay {
    Help {
        scroll: usize,
    },
    Menu {
        items: Vec<MenuItem>,
        cursor: usize,
    },
    InfoModal {
        items: Vec<MenuItem>,
        cursor: usize,
        info_cursor: usize,
        focus: InfoModalFocus,
        row: InfoModalRow,
    },
    CommitDetails {
        commit: graph::GraphCommit,
        details: git::commit_details::CommitDetails,
        items: Vec<MenuItem>,
        cursor: usize,
        file_cursor: usize,
        focus: CommitDetailsFocus,
        scroll: ModalScroll,
    },
    CommitDiff {
        commit: graph::GraphCommit,
        details: git::commit_details::CommitDetails,
        items: Vec<MenuItem>,
        cursor: usize,
        file_cursor: usize,
        focus: CommitDetailsFocus,
        details_scroll: ModalScroll,
        diff: git::commit_details::CommitFileDiff,
        scroll: ModalScroll,
    },
    Confirm {
        preflight: DeletePreflight,
        choices: Vec<ConfirmChoice>,
        selected: usize,
        body_scroll: ModalScroll,
        stage: ConfirmStage,
    },
    /// Secondary confirmation for cancelling a job that risks a partial
    /// worktree deletion. Not tied to a `BranchAction`/target list like
    /// `Confirm` -- just a plain message.
    ConfirmCancelJob {
        message: String,
    },
    Executing {
        label: String,
        progress: Option<ProgressUpdate>,
    },
    Results {
        results: Vec<OperationResult>,
        selected_index: usize,
        expanded_index: Option<usize>,
        focus: ResultsFocus,
        body_scroll: ModalScroll,
    },
    Settings {
        cursor: usize,
    },
    /// Legacy display-only filter variant retained for source compatibility.
    Filter,
    /// Stateful filter action list used by interactive App input.
    FilterSelection {
        cursor: usize,
    },
    GraphOptions {
        cursor: usize,
        include_remotes: bool,
    },
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

impl Overlay {
    /// Start a Results accordion with the first row selected and every row collapsed.
    pub fn results(results: Vec<OperationResult>) -> Self {
        Self::Results {
            results,
            selected_index: 0,
            expanded_index: None,
            focus: ResultsFocus::Results,
            body_scroll: ModalScroll::default(),
        }
    }
}

/// Everything the renderer needs to draw one frame.
/// This avoids coupling to the full App struct (which is built in Phase 4).
pub struct RenderContext<'a> {
    pub active_view: ViewId,
    pub overlay: Option<&'a mut Overlay>,
    pub toast: Option<&'a Toast>,
    pub theme: &'a Theme,
    pub symbols: &'a SymbolSet,
    pub config: &'a Config,
    // Info modal: confirmation message + recorded click-to-copy hit regions
    pub info_copied_msg: Option<&'a str>,
    pub info_hit_regions: &'a mut Vec<InfoHitRegion>,
    /// Scroll offset for the narrow-width info modal's combined info+actions
    /// scroll buffer. Corrected in place each frame to keep the current
    /// selection visible, mirroring ratatui's own TableState offset
    /// correction in ui/list_render.rs.
    pub info_modal_scroll_offset: &'a mut u16,
    // Non-modal status area for the confirmed-action job queue
    pub job_status: JobStatusView<'a>,
    // List states
    pub branches: &'a mut ListState<BranchInfo>,
    pub remotes: &'a mut ListState<RemoteBranchInfo>,
    pub tags: &'a mut ListState<TagInfo>,
    pub worktrees: &'a mut ListState<WorktreeInfo>,
    pub graph: &'a mut GraphState,
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

/// Build default status bar text based on the active view.
fn default_status_text(ctx: &RenderContext) -> String {
    match ctx.active_view {
        ViewId::Graph => {
            let commits = ctx.graph.snapshot().map(|snapshot| snapshot.commits.len()).unwrap_or(0);
            let (local, remote) = ctx
                .graph
                .snapshot()
                .map(|snapshot| (snapshot.ref_counts.local, snapshot.ref_counts.remote))
                .unwrap_or_default();
            format!(
                " {commits} commits | {local} local refs | {remote} remote refs — [j/k]move [g/G]home/end [Enter]details/actions [o]ptions [L]older [r]eload [?]help [q]uit"
            )
        }
        ViewId::Branches => format_branch_like(
            "branches",
            branch_like_summary(ctx.branches),
            "[/]search [\\]filter [g]graph [c]heckout [d]el [D]el+remote [p]ush [f]etch [F2]diag [?]help [q]uit",
        ),
        ViewId::Remotes => format_branch_like(
            "remote branches",
            branch_like_summary(ctx.remotes),
            "[/]search [\\]filter [g]graph [c]heckout [d]el [f]etch [?]help [q]uit",
        ),
        ViewId::Tags => {
            let total = ctx.tags.items().len();
            format!(
                " {} tags \u{2014} [/]search [\\]filter [g]graph [d]el [D]el+remote [p]ush [f]etch [?]help [q]uit",
                total
            )
        }
        ViewId::Worktrees => {
            let total = ctx.worktrees.items().len();
            format!(
                " {} worktrees \u{2014} [/]search [g]graph [d]el [D]force-del [f]etch [?]help [q]uit",
                total
            )
        }
    }
}

/// Counts `(total, selected, merged, squashed, cherry_picked)` for a branch-like list view.
/// Works for any item type whose `ViewItem::merge_status` is populated.
fn branch_like_summary<T: ViewItem>(state: &ListState<T>) -> (usize, usize, usize, usize, usize) {
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
    let cherry_picked = state
        .items()
        .iter()
        .filter(|i| i.merge_status() == Some(&MergeStatus::CherryPicked))
        .count();
    (total, selected, merged, squashed, cherry_picked)
}

/// Formats a branch-like status bar line from a noun, summary counts, and the
/// view's shortcut suffix. Branches and Remotes share this formatter; only the
/// noun and the shortcut list differ between them.
fn format_branch_like(
    noun: &str,
    summary: (usize, usize, usize, usize, usize),
    shortcuts: &str,
) -> String {
    let (total, selected, merged, squashed, cherry_picked) = summary;
    format!(
        " {total} {noun} | {selected} selected | {merged} merged | {squashed} squashed | {cherry_picked} cherry-picked \u{2014} {shortcuts}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn empty_renderer<T>(
        _: &T,
        _: usize,
        _: bool,
        _: bool,
        _: &[usize],
        _: &crate::ui::list_render::CellContext,
    ) -> Vec<Line<'static>> {
        Vec::new()
    }

    fn render_commit_details_overlay(commit: graph::GraphCommit) -> String {
        use crate::git::commit_details::{CommitDetailMode, CommitDetails};
        use crate::job_queue::JobStatusView;
        use crate::symbols::SymbolSet;
        use crate::theme::Theme;
        use crate::ui::info_modal::InfoHitRegion;
        use ratatui::{backend::TestBackend, Terminal};

        let oid = commit.oid.clone();
        let summary = commit.summary.clone();
        let mut overlay = Some(Overlay::CommitDetails {
            commit,
            details: CommitDetails {
                oid: oid.clone(),
                summary,
                author_name: String::new(),
                author_email: String::new(),
                authored_at: None,
                message_lines: Vec::new(),
                mode: CommitDetailMode::Commit { oid },
                files: Vec::new(),
                branch_log: Vec::new(),
            },
            items: Vec::new(),
            cursor: 0,
            file_cursor: 0,
            focus: CommitDetailsFocus::Files,
            scroll: ModalScroll::default(),
        });
        let theme = Theme::dark();
        let symbols = SymbolSet::ascii();
        let config = Config::default();
        let mut info_hit_regions: Vec<InfoHitRegion> = Vec::new();
        let mut info_modal_scroll_offset = 0;
        let mut branches = ListState::<BranchInfo>::empty();
        let mut remotes = ListState::<RemoteBranchInfo>::empty();
        let mut tags = ListState::<TagInfo>::empty();
        let mut worktrees = ListState::<WorktreeInfo>::empty();
        let mut graph = GraphState::new();
        let mut terminal = Terminal::new(TestBackend::new(120, 36)).unwrap();

        terminal
            .draw(|frame| {
                let mut context = RenderContext {
                    active_view: ViewId::Graph,
                    overlay: overlay.as_mut(),
                    toast: None,
                    theme: &theme,
                    symbols: &symbols,
                    config: &config,
                    info_copied_msg: None,
                    info_hit_regions: &mut info_hit_regions,
                    info_modal_scroll_offset: &mut info_modal_scroll_offset,
                    job_status: JobStatusView {
                        visible: false,
                        current_label: None,
                        current_progress: None,
                        queued_count: 0,
                        targets_done: 0,
                        targets_total: 0,
                        summary: None,
                    },
                    branches: &mut branches,
                    remotes: &mut remotes,
                    tags: &mut tags,
                    worktrees: &mut worktrees,
                    graph: &mut graph,
                    branch_columns: &[],
                    remote_columns: &[],
                    tag_columns: &[],
                    worktree_columns: &[],
                    active_filter_tokens: &[],
                    render_branch_row: empty_renderer::<BranchInfo>,
                    render_remote_row: empty_renderer::<RemoteBranchInfo>,
                    render_tag_row: empty_renderer::<TagInfo>,
                    render_worktree_row: empty_renderer::<WorktreeInfo>,
                };
                draw(frame, &mut context);
            })
            .unwrap();

        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

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
            squash_confidence: None,
        }
    }

    #[test]
    fn branch_like_summary_counts_statuses_and_selection() {
        let mut state = ListState::new(vec![
            branch("a", MergeStatus::Merged),
            branch("b", MergeStatus::SquashMerged),
            branch("c", MergeStatus::Unmerged),
            branch("d", MergeStatus::Merged),
            branch("e", MergeStatus::CherryPicked),
        ]);
        // Select the first two.
        state.selected_mut()[0] = true;
        state.selected_mut()[1] = true;

        let (total, selected, merged, squashed, cherry_picked) = branch_like_summary(&state);
        assert_eq!(total, 5);
        assert_eq!(selected, 2);
        assert_eq!(merged, 2);
        assert_eq!(squashed, 1);
        assert_eq!(cherry_picked, 1);
    }

    #[test]
    fn branch_like_summary_empty() {
        let state: ListState<BranchInfo> = ListState::new(vec![]);
        assert_eq!(branch_like_summary(&state), (0, 0, 0, 0, 0));
    }

    #[test]
    fn format_branch_like_preserves_shape() {
        let text = format_branch_like("branches", (4, 2, 2, 1, 1), "[/]search [q]uit");
        assert_eq!(
            text,
            " 4 branches | 2 selected | 2 merged | 1 squashed | 1 cherry-picked \u{2014} [/]search [q]uit"
        );
    }

    #[test]
    fn commit_details_overlay_renders_fuzzy_similarity_from_the_graph_commit() {
        use crate::git::graph::GraphCommit;
        let rendered = render_commit_details_overlay(GraphCommit {
            relationships: vec![crate::git::graph::GraphRelationship {
                kind: crate::git::graph::RelationshipKind::SquashMerge,
                matching: crate::git::graph::RelationshipMatch::Fuzzy {
                    similarity_percent: 84,
                },
                destination_oid: "abcdef1234567890".into(),
                destination_refs: vec!["main".into()],
                source_oid: "3333333333333333333333333333333333333333".into(),
                source_refs: Vec::new(),
            }],
            oid: "abcdef1234567890".into(),
            summary: "near squash landing".into(),
            ..GraphCommit::default()
        });
        assert!(
            rendered.contains("Possible squash merge (fuzzy): 84% similarity"),
            "commit details overlay should include fuzzy similarity metadata; got: {rendered}"
        );
    }

    #[test]
    fn commit_details_overlay_hides_fuzzy_metadata_without_a_match() {
        let rendered = render_commit_details_overlay(graph::GraphCommit {
            oid: "abcdef1234567890".into(),
            summary: "ordinary commit".into(),
            ..graph::GraphCommit::default()
        });
        assert!(!rendered.contains("Possible squash merge (fuzzy)"));
    }

    #[test]
    fn commit_details_overlay_keeps_exact_squash_and_cherry_classifications_ahead_of_fuzzy() {
        for (is_possible_squash_merge, is_cherry_picked_commit) in
            [(true, false), (false, true), (true, true)]
        {
            let rendered = render_commit_details_overlay(graph::GraphCommit {
                oid: "abcdef1234567890".into(),
                summary: "classified commit".into(),
                relationships: {
                    let mut pairs = vec![
                        crate::git::graph::GraphRelationship {
                            kind: crate::git::graph::RelationshipKind::SquashMerge,
                            matching: crate::git::graph::RelationshipMatch::Exact,
                            destination_oid: "abcdef1234567890".into(),
                            destination_refs: vec!["main".into()],
                            source_oid: "1111111111111111111111111111111111111111".into(),
                            source_refs: Vec::new(),
                        },
                        crate::git::graph::GraphRelationship {
                            kind: crate::git::graph::RelationshipKind::SquashMerge,
                            matching: crate::git::graph::RelationshipMatch::Fuzzy {
                                similarity_percent: 97,
                            },
                            destination_oid: "abcdef1234567890".into(),
                            destination_refs: vec!["main".into()],
                            source_oid: "3333333333333333333333333333333333333333".into(),
                            source_refs: Vec::new(),
                        },
                    ];
                    if !is_possible_squash_merge {
                        pairs.retain(|r| r.matching != crate::git::graph::RelationshipMatch::Exact);
                    }
                    pairs
                },
                is_cherry_picked_commit,
                ..graph::GraphCommit::default()
            });
            assert!(!rendered.contains("Possible squash merge (fuzzy)"));
        }
    }
}
