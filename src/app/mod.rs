use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, MouseButton, MouseEventKind};
use ratatui::backend::CrosstermBackend;
use ratatui::prelude::*;
use ratatui::text::Line;
use ratatui::Terminal;

use git_branch_manager::config::Config;
use git_branch_manager::git::{
    branch, cache, capability, cherry_loader, commit_details, diagnostics, graph, operations,
    pr_loader, squash_loader, tags, worktree,
};
use git_branch_manager::job_queue::{ActionJobQueue, JobEvent};
use git_branch_manager::symbols::SymbolSet;
use git_branch_manager::theme::Theme;
use git_branch_manager::types::*;
use git_branch_manager::ui::cells::{
    age_line, ahead_behind_line, fit_text, merge_status_line, merge_status_line_for_branch,
    pr_line, worktree_status_line,
};
use git_branch_manager::ui::commit_details::CommitDetailsFocus;
use git_branch_manager::ui::confirm::{
    ConfirmChoice, ConfirmStage, ConfirmTarget, DeletePreflight, DeleteRisk,
};
use git_branch_manager::ui::info_modal::{InfoHitRegion, InfoModalFocus, InfoModalRow};
use git_branch_manager::ui::list_render::CellContext;
use git_branch_manager::ui::menu::MenuItem;
use git_branch_manager::ui::modal::ModalScroll;
use git_branch_manager::ui::render::{Overlay, RenderContext};
use git_branch_manager::ui::results::{result_actions, ResultsAction, ResultsFocus};
use git_branch_manager::ui::shared::{abbreviate_path, prefix_style, truncate, truncate_left};
use git_branch_manager::ui::toast::Toast;
use git_branch_manager::view::branches::BranchesViewDef;
use git_branch_manager::view::column::{apply_merge_status_if_confident, ColumnDef};
use git_branch_manager::view::filter::{FilterSet, FilterTokenDef};
use git_branch_manager::view::graph::GraphState;
use git_branch_manager::view::list_state::{self, ListState};
use git_branch_manager::view::remotes::RemotesViewDef;
use git_branch_manager::view::sort_keys;
use git_branch_manager::view::tags::TagsViewDef;
use git_branch_manager::view::worktrees::WorktreesViewDef;
use git_branch_manager::view::ViewId;

mod confirm;
mod construct;
mod delete_preflight;
mod drain;
mod event_loop;
mod helpers;
mod keys;
mod keys_results_settings;
mod keys_views;
mod loading;
mod menu;
mod menu_branch;
mod menu_graph;
mod menu_remote_tag;
mod menu_worktree;
mod mouse_search;
mod overlay_keys;
mod primary_code;
mod refresh;
mod rows;
mod settings;
#[cfg(test)]
mod tests;

pub(crate) use rows::{render_branch_row, render_remote_row, render_tag_row, render_worktree_row};
use helpers::*;


/// Messages sent by the background phase-1 thread.
pub enum Phase1Msg {
    /// Fast path: branch list + caches. Sent before merge detection.
    Fast(
        Vec<BranchInfo>,
        Box<cache::BranchCache>,
        Box<cache::BranchCache>,
    ),
    /// Slow path: per-branch merge status updates, sent after detect_merged_branches.
    MergeStatuses(Vec<(String, MergeStatus)>),
    /// Ahead/behind counts for tracked non-gone branches, sent after Fast.
    AheadBehind(Vec<(String, Option<u32>, Option<u32>)>),
    /// Merge-base commit hashes (short), sent after Fast.
    MergeBaseCommits(Vec<(String, String)>),
}

pub struct RemoteFetchCompletion {
    pub success: bool,
    pub graph_delta: Result<graph::GraphRepositoryDelta, String>,
}

pub struct App {
    // Core
    pub repo_path: PathBuf,
    pub base_branch: String,
    cache_root: cache::CacheRoot,
    #[cfg(test)]
    _test_cache_root: Option<tempfile::TempDir>,
    /// Root that settings saves resolve `config.toml` under; tests point it at a tempdir.
    config_root: PathBuf,
    pub config: Config,
    pub theme: Theme,
    pub symbols: SymbolSet,
    pub should_exit: bool,

    // View state -- 5 peers
    pub active_view: ViewId,
    pub graph: GraphState,
    pub branches: ListState<BranchInfo>,
    pub remotes: ListState<RemoteBranchInfo>,
    pub tags: ListState<TagInfo>,
    pub worktrees: ListState<WorktreeInfo>,

    // View definitions (columns + filter tokens)
    branch_columns: Vec<ColumnDef<BranchInfo>>,
    remote_columns: Vec<ColumnDef<RemoteBranchInfo>>,
    tag_columns: Vec<ColumnDef<TagInfo>>,
    worktree_columns: Vec<ColumnDef<WorktreeInfo>>,
    branch_filter_tokens: Vec<FilterTokenDef>,
    remote_filter_tokens: Vec<FilterTokenDef>,
    tag_filter_tokens: Vec<FilterTokenDef>,
    worktree_filter_tokens: Vec<FilterTokenDef>,

    // Overlay
    pub overlay: Option<Overlay>,
    /// Which view was active before the overlay opened (for Menu/Confirm return)
    pub return_view: ViewId,

    // Background channels
    pub squash_rx: Option<Receiver<SquashResult>>,
    pub squash_checked: usize,
    pub squash_total: usize,
    pub remote_squash_rx: Option<Receiver<SquashResult>>,
    pub cherry_rx: Option<Receiver<CherryResult>>,
    pub cherry_checked: usize,
    pub cherry_total: usize,
    pub remote_enrich_rx: Option<Receiver<RemoteEnrichResult>>,
    pub worktree_enrich_rx: Option<Receiver<WorktreeEnrichResult>>,
    pub pr_rx: Option<Receiver<PrMap>>,
    pub pr_map: PrMap,
    pub op_rx: Option<Receiver<Vec<OperationResult>>>,
    fetch_graph_delta_rx: Option<Receiver<Result<graph::GraphRepositoryDelta, String>>>,
    fetch_overlay_dismissed: bool,
    pub progress_rx: Option<Receiver<ProgressUpdate>>,
    pub progress: Option<ProgressUpdate>,
    /// Result channel for the background cache-accuracy audit.
    pub diag_rx: Option<Receiver<CacheAudit>>,
    /// Result channel for the silent, automatic launch-time cache verifier.
    /// Separate from `diag_rx`: receiving here must never open the manual
    /// Diagnostics review overlay.
    pub cache_verify_rx: Option<Receiver<CacheAudit>>,
    primary_code_check_request: Option<PrimaryCodeCheckRequest>,
    /// Branch names the launch-time verifier has already corrected in
    /// `self.branches` this session. Consulted by the `Phase1Msg::MergeStatuses`
    /// handler so a late-arriving stale cached status can't silently overwrite
    /// a verified correction, regardless of which one lands first.
    pub verified_branches: HashSet<String>,
    pub remote_fetch_rx: Option<Receiver<RemoteFetchCompletion>>,
    pub tag_load_rx: Option<Receiver<Vec<TagInfo>>>,
    pub worktree_load_rx: Option<Receiver<Vec<WorktreeInfo>>>,
    #[allow(clippy::type_complexity)]
    pub remote_load_rx: Option<
        Receiver<(
            Vec<RemoteBranchInfo>,
            Vec<(String, String, Option<String>)>,
            cache::BranchCache,
        )>,
    >,
    pub phase1_rx: Option<Receiver<Phase1Msg>>,
    pub graph_rx: Option<Receiver<Result<graph::GraphSnapshot, graph::GraphLoadError>>>,
    graph_update_rx: Option<Receiver<graph::GraphUpdateMsg>>,
    /// Result channel for the asynchronous squash-merge enrichment spawned
    /// after each successful structural graph load. The current reload
    /// generation lives in `graph_generation`; enrichment messages whose
    /// generation does not match are dropped (they belong to a stale
    /// snapshot the user no longer sees).
    pub graph_enrich_rx: Option<Receiver<graph::GraphEnrichmentMsg>>,
    pub commit_details_rx:
        Option<Receiver<(String, Result<commit_details::CommitDetails, String>)>>,
    pub commit_details_target: Option<String>,
    pub commit_file_diff_rx: Option<
        Receiver<(
            (String, String),
            Result<commit_details::CommitFileDiff, String>,
        )>,
    >,
    pub commit_file_diff_target: Option<(String, String)>,
    /// Bumped for graph loads and enrichment-invalidating updates. Both
    /// snapshots and enrichment use this generation, so stale results
    /// cannot overwrite a newer snapshot.
    pub graph_generation: u64,
    graph_generation_signal: Arc<AtomicU64>,
    graph_revision: u64,
    graph_repository_state: Option<graph::GraphRepositoryState>,
    pending_graph_deltas: Vec<Result<graph::GraphRepositoryDelta, String>>,
    graph_update_invalidates_enrichment: bool,
    staged_graph_snapshot: Option<graph::GraphSnapshot>,
    reconciling_initial_graph: bool,

    // Cache (used for R-key cache clearing)
    #[allow(dead_code)]
    pub cache: cache::BranchCache,

    // Toast
    pub toast: Option<Toast>,

    // Operation cancellation (fetch / cache-audit only; confirmed actions use job_queue)
    pub cancel_flag: Option<Arc<AtomicBool>>,

    // Sequential background queue for confirmed actions (delete, push, merge, ...)
    pub job_queue: ActionJobQueue,

    // Terminal dimensions (for mouse handling)
    pub terminal_rows: u16,

    // Info modal: click-to-copy hit regions (recorded each frame) and the
    // confirmation message shown after a successful copy.
    pub info_hit_regions: Vec<InfoHitRegion>,
    pub info_copied_msg: Option<String>,
    // Info modal: scroll offset for the narrow-layout combined info+actions
    // pane. Lives outside `Overlay::InfoModal` (like info_hit_regions/
    // info_copied_msg) because render.rs needs to correct it in place each
    // frame, and RenderContext.overlay is an immutable reference.
    pub info_modal_scroll_offset: u16,

    // Whether remote fetch has been done this session
    pub remote_fetched: bool,

    // Whether the repo has any remote configured (e.g. origin)
    pub has_configured_remote: bool,

    // Fingerprint of the previous branch refresh inputs. Diagnostic spans use
    // this to show whether a full refresh recomputed identical branch tips.
    last_branch_fingerprint: Option<u64>,
}

struct PrimaryCodeCheckResult {
    display_ref: String,
    base_branch: String,
    primary_reference: Option<String>,
    primary_oid: Option<String>,
    outcome: Result<PrimaryBranchCodeMatch, String>,
}

struct PrimaryCodeCheckRequest {
    receiver: Receiver<PrimaryCodeCheckResult>,
    cancel: Arc<AtomicBool>,
}

fn branch_input_fingerprint(repo: &git2::Repository, base_branch: &str) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut tips: Vec<(String, String)> = Vec::new();
    if let Ok(branches) = repo.branches(Some(git2::BranchType::Local)) {
        for (branch, _) in branches.flatten() {
            if let (Ok(Some(name)), Some(oid)) = (branch.name(), branch.get().target()) {
                tips.push((name.to_string(), oid.to_string()));
            }
        }
    }

    tips.sort();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    base_branch.hash(&mut hasher);
    tips.hash(&mut hasher);
    hasher.finish()
}

// ---- Watchdog helpers ----

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Copy `text` to the host system clipboard. Returns an error string on failure.
fn copy_to_clipboard(text: &str) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    clipboard
        .set_text(text.to_string())
        .map_err(|e| e.to_string())
}

// ---- Generic channel drain helper ----

fn drain_channel<T>(rx: &mut Option<Receiver<T>>, max_per_tick: usize, dirty: &mut bool) -> Vec<T> {
    let Some(receiver) = rx.as_ref() else {
        return vec![];
    };
    let mut results = Vec::new();

    for _ in 0..max_per_tick {
        match receiver.try_recv() {
            Ok(item) => results.push(item),
            Err(std::sync::mpsc::TryRecvError::Empty) => break,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                *rx = None;
                break;
            }
        }
    }

    if !results.is_empty() {
        *dirty = true;
    }
    results
}
