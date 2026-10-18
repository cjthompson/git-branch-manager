use crate::config::Config;
use crate::git;
use crate::git::graph;
use crate::job_queue::JobStatusView;
use crate::symbols::SymbolSet;
use crate::theme::Theme;
use crate::types::*;
use crate::view::column::ColumnDef;
use crate::view::filter::FilterTokenDef;
use crate::view::graph::GraphState;
use crate::view::list_state::ListState;
use crate::view::ViewId;

use crate::ui::commit_details::CommitDetailsFocus;
use crate::ui::confirm::{ConfirmChoice, ConfirmStage, DeletePreflight};
use crate::ui::info_modal::{InfoHitRegion, InfoModalFocus, InfoModalRow};
use crate::ui::list_render::RowRenderer;
use crate::ui::menu::MenuItem;
use crate::ui::modal::ModalScroll;
use crate::ui::results::ResultsFocus;
use crate::ui::toast::Toast;

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
