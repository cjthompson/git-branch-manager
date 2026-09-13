use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, MouseButton, MouseEventKind};
use ratatui::backend::CrosstermBackend;
use ratatui::prelude::*;
use ratatui::text::Line;
use ratatui::Terminal;

use git_branch_manager::config::Config;
use git_branch_manager::git::{
    branch, cache, cherry_loader, diagnostics, graph, operations, pr_loader, squash_loader, tags,
    worktree,
};
use git_branch_manager::job_queue::{ActionJobQueue, JobEvent};
use git_branch_manager::symbols::SymbolSet;
use git_branch_manager::theme::Theme;
use git_branch_manager::types::*;
use git_branch_manager::ui::cells::{
    age_line, ahead_behind_line, fit_text, merge_status_line, merge_status_line_for_branch,
    pr_line, worktree_status_line,
};
use git_branch_manager::ui::info_modal::{InfoHitRegion, InfoModalFocus, InfoModalRow};
use git_branch_manager::ui::list_render::CellContext;
use git_branch_manager::ui::menu::MenuItem;
use git_branch_manager::ui::render::{ConfirmExtraKey, Overlay, RenderContext};
use git_branch_manager::ui::shared::{abbreviate_path, prefix_style, truncate, truncate_left};
use git_branch_manager::ui::toast::Toast;
use git_branch_manager::view::branches::BranchesViewDef;
use git_branch_manager::view::column::ColumnDef;
use git_branch_manager::view::filter::{FilterSet, FilterTokenDef};
use git_branch_manager::view::graph::GraphState;
use git_branch_manager::view::list_state::{self, ListState};
use git_branch_manager::view::remotes::RemotesViewDef;
use git_branch_manager::view::sort_keys;
use git_branch_manager::view::tags::TagsViewDef;
use git_branch_manager::view::worktrees::WorktreesViewDef;
use git_branch_manager::view::ViewId;

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

pub struct App {
    // Core
    pub repo_path: PathBuf,
    pub base_branch: String,
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
    pub progress_rx: Option<Receiver<ProgressUpdate>>,
    pub progress: Option<ProgressUpdate>,
    /// Result channel for the background cache-accuracy audit.
    pub diag_rx: Option<Receiver<CacheAudit>>,
    /// Result channel for the silent, automatic launch-time cache verifier.
    /// Separate from `diag_rx`: receiving here must never open the manual
    /// Diagnostics review overlay.
    pub cache_verify_rx: Option<Receiver<CacheAudit>>,
    /// Branch names the launch-time verifier has already corrected in
    /// `self.branches` this session. Consulted by the `Phase1Msg::MergeStatuses`
    /// handler so a late-arriving stale cached status can't silently overwrite
    /// a verified correction, regardless of which one lands first.
    pub verified_branches: HashSet<String>,
    pub remote_fetch_rx: Option<Receiver<bool>>,
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
    /// Result channel for the asynchronous squash-merge enrichment spawned
    /// after each successful structural graph load. The current reload
    /// generation lives in `graph_generation`; enrichment messages whose
    /// generation does not match are dropped (they belong to a stale
    /// snapshot the user no longer sees).
    pub graph_enrich_rx: Option<Receiver<graph::GraphEnrichmentMsg>>,
    /// Bumped once per `spawn_graph_load` call. Both the structural snapshot
    /// we just received and any in-flight enrichment are tagged with it, so
    /// a stale enrichment result cannot overwrite a newer snapshot.
    pub graph_generation: u64,

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

impl App {
    pub fn new(repo_path: PathBuf, base_branch: String, config: Config) -> Self {
        let theme = Theme::from_name(config.theme.as_deref().unwrap_or("dark"));
        let symbols = SymbolSet::from_name(config.symbols.as_deref().unwrap_or("auto"));

        let branch_def = BranchesViewDef;
        let remote_def = RemotesViewDef;
        let tag_def = TagsViewDef;
        let worktree_def = WorktreesViewDef;

        let branch_cols = branch_def.columns();
        let remote_cols = remote_def.columns();
        let tag_cols = tag_def.columns();
        let worktree_cols = worktree_def.columns();

        let branch_sort_col = config
            .sort_column_branches
            .as_deref()
            .and_then(|k| sort_keys::index_for_key(&branch_cols, k));
        let branch_sort_asc = config.sort_asc_branches.unwrap_or(true);
        let remote_sort_col = config
            .sort_column_remotes
            .as_deref()
            .and_then(|k| sort_keys::index_for_key(&remote_cols, k));
        let remote_sort_asc = config.sort_asc_remotes.unwrap_or(true);
        let tag_sort_col = config
            .sort_column_tags
            .as_deref()
            .and_then(|k| sort_keys::index_for_key(&tag_cols, k));
        let tag_sort_asc = config.sort_asc_tags.unwrap_or(true);
        let worktree_sort_col = config
            .sort_column_worktrees
            .as_deref()
            .and_then(|k| sort_keys::index_for_key(&worktree_cols, k));
        let worktree_sort_asc = config.sort_asc_worktrees.unwrap_or(true);
        let include_remotes = config.include_remotes.unwrap_or(false);

        let mut branch_state = ListState::empty();
        branch_state.loading = true;
        branch_state.set_sort(branch_sort_col, branch_sort_asc);

        let mut remote_state = ListState::empty();
        remote_state.set_sort(remote_sort_col, remote_sort_asc);

        let mut tag_state = ListState::empty();
        tag_state.set_sort(tag_sort_col, tag_sort_asc);

        let mut worktree_state = ListState::empty();
        worktree_state.set_sort(worktree_sort_col, worktree_sort_asc);

        let remote_fetched = config.auto_fetch == Some(true);
        let cache = cache::BranchCache::load(&repo_path);
        let has_configured_remote = git2::Repository::open(&repo_path)
            .and_then(|r| r.remotes())
            .map(|r| !r.is_empty())
            .unwrap_or(false);
        let job_queue = ActionJobQueue::new(repo_path.clone(), base_branch.clone());

        Self {
            repo_path,
            base_branch,
            config,
            theme,
            symbols,
            should_exit: false,
            active_view: ViewId::Graph,
            graph: {
                let mut graph = GraphState::new();
                graph.set_include_remotes(include_remotes);
                graph
            },
            branches: branch_state,
            remotes: remote_state,
            tags: tag_state,
            worktrees: worktree_state,
            branch_columns: branch_def.columns(),
            remote_columns: remote_def.columns(),
            tag_columns: tag_def.columns(),
            worktree_columns: worktree_def.columns(),
            branch_filter_tokens: branch_def.filter_tokens(),
            remote_filter_tokens: remote_def.filter_tokens(),
            tag_filter_tokens: tag_def.filter_tokens(),
            worktree_filter_tokens: worktree_def.filter_tokens(),
            overlay: None,
            return_view: ViewId::Branches,
            squash_rx: None,
            squash_checked: 0,
            squash_total: 0,
            remote_squash_rx: None,
            cherry_rx: None,
            cherry_checked: 0,
            cherry_total: 0,
            remote_enrich_rx: None,
            worktree_enrich_rx: None,
            pr_rx: None,
            pr_map: PrMap::new(),
            op_rx: None,
            progress_rx: None,
            progress: None,
            diag_rx: None,
            cache_verify_rx: None,
            verified_branches: HashSet::new(),
            remote_fetch_rx: None,
            tag_load_rx: None,
            worktree_load_rx: None,
            remote_load_rx: None,
            phase1_rx: None,
            graph_rx: None,
            graph_enrich_rx: None,
            graph_generation: 0,
            cache,
            toast: None,
            cancel_flag: None,
            job_queue,
            terminal_rows: 0,
            info_hit_regions: Vec::new(),
            info_copied_msg: None,
            info_modal_scroll_offset: 0,
            remote_fetched,
            has_configured_remote,
            last_branch_fingerprint: None,
        }
    }

    // ---- Event Loop ----

    pub fn run(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    ) -> std::io::Result<()> {
        // Apply initial sort
        list_state::apply_sort(&mut self.branches, &self.branch_columns);

        // Force a full redraw on the first frame by drawing once, then
        // inserting a resize event so ratatui marks the entire buffer dirty.
        terminal.clear()?;

        // Watchdog: logs to /tmp/gbm-watchdog.log if the main loop stalls >2s
        let tick_ms = Arc::new(AtomicU64::new(now_ms()));
        {
            let watchdog_tick = Arc::clone(&tick_ms);
            std::thread::spawn(move || loop {
                std::thread::sleep(Duration::from_secs(1));
                let stall = now_ms().saturating_sub(watchdog_tick.load(Ordering::Relaxed));
                if stall > 2000 {
                    let _ = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open("/tmp/gbm-watchdog.log")
                        .and_then(|mut f| {
                            use std::io::Write;
                            writeln!(f, "UI stall: {stall}ms")
                        });
                }
            });
        }

        let mut needs_redraw = true;

        loop {
            tick_ms.store(now_ms(), Ordering::Relaxed);
            if self.drain_channels() {
                needs_redraw = true;
            }

            if needs_redraw {
                terminal.draw(|frame| {
                    self.terminal_rows = frame.area().height;
                    let mut ctx = self.build_render_context();
                    git_branch_manager::ui::render::draw(frame, &mut ctx);
                })?;
                needs_redraw = false;
            }

            if self.should_exit {
                return Ok(());
            }

            if event::poll(Duration::from_millis(50))? {
                let ev = event::read()?;
                needs_redraw = true;
                self.handle_event(ev);
            }
        }
    }

    fn build_render_context(&mut self) -> RenderContext<'_> {
        let active_filter_tokens: &[FilterTokenDef] = match self.active_view {
            ViewId::Graph => &[],
            ViewId::Branches => &self.branch_filter_tokens,
            ViewId::Remotes => &self.remote_filter_tokens,
            ViewId::Tags => &self.tag_filter_tokens,
            ViewId::Worktrees => &self.worktree_filter_tokens,
        };

        RenderContext {
            active_view: self.active_view,
            overlay: self.overlay.as_ref(),
            toast: self.toast.as_ref(),
            theme: &self.theme,
            symbols: &self.symbols,
            config: &self.config,
            info_copied_msg: self.info_copied_msg.as_deref(),
            info_hit_regions: &mut self.info_hit_regions,
            info_modal_scroll_offset: &mut self.info_modal_scroll_offset,
            graph: &mut self.graph,
            job_status: self.job_queue.render_data(),
            branches: &mut self.branches,
            remotes: &mut self.remotes,
            tags: &mut self.tags,
            worktrees: &mut self.worktrees,
            branch_columns: &self.branch_columns,
            remote_columns: &self.remote_columns,
            tag_columns: &self.tag_columns,
            worktree_columns: &self.worktree_columns,
            active_filter_tokens,
            render_branch_row,
            render_remote_row,
            render_tag_row,
            render_worktree_row,
        }
    }

    // ---- Channel Draining ----

    fn drain_channels(&mut self) -> bool {
        let mut dirty = false;

        for result in drain_channel(&mut self.graph_rx, 1, &mut dirty) {
            // Stamp the snapshot with the current reload generation so the
            // enrichment that follows can be matched back to it. The
            // generation is also the gate we check below in the enrich
            // drain — stale enrichment from a previous load is dropped
            // rather than overwriting the newer snapshot's markers.
            let result = match result {
                Ok(mut snapshot) => {
                    snapshot.generation = Some(self.graph_generation);
                    Ok(snapshot)
                }
                err => err,
            };
            self.graph.apply_result(result);
            self.clear_toast();

            if let Some(snapshot) = self.graph.snapshot().cloned() {
                self.graph_enrich_rx = Some(graph::spawn_possible_squash_enrichment(
                    snapshot,
                    self.repo_path.clone(),
                    Some(self.base_branch.clone()),
                    self.graph_generation,
                ));
            }
        }

        for msg in drain_channel(&mut self.graph_enrich_rx, 1, &mut dirty) {
            // Drop enrichment that belongs to a previous reload — the
            // user already moved past that snapshot. A failed/cancelled
            // enrichment just never sends; if the receiver is dropped
            // mid-compute, the worker thread exits on its next `tx.send`.
            if msg.generation == self.graph_generation {
                self.graph.apply_squash_enrichment(&msg.updates);
            }
        }

        // Phase-1 messages (fast metadata first, merge statuses second)
        for msg in drain_channel(&mut self.phase1_rx, 2, &mut dirty) {
            match msg {
                Phase1Msg::Fast(branches, cache_for_app, _cache_for_squash) => {
                    self.cache = *cache_for_app;
                    self.branches.set_items(branches);
                    self.branches.loading = false;
                    list_state::apply_sort(&mut self.branches, &self.branch_columns);
                    // Squash checker and PR loader are spawned after merge statuses arrive.
                }
                Phase1Msg::MergeStatuses(updates) => {
                    let update_map: std::collections::HashMap<String, MergeStatus> =
                        updates.into_iter().collect();
                    for b in self.branches.items_mut() {
                        // A branch the launch-time cache verifier already corrected
                        // wins regardless of arrival order — this restore path can
                        // otherwise reintroduce the exact stale status the verifier
                        // just fixed (it restores from cache, which is why the
                        // verifier exists in the first place).
                        if self.verified_branches.contains(&b.name) {
                            continue;
                        }
                        if let Some(&new_status) = update_map.get(&b.name) {
                            b.merge_status = new_status;
                        }
                    }
                    self.branches.rebuild_display_indices();

                    // Now spawn squash checker on the updated (merged-filtered) set.
                    let repo_path = self.repo_path.clone();
                    let base_branch = self.base_branch.clone();
                    let cache_for_squash = cache::BranchCache::load(&repo_path);
                    let cache_for_cherry = cache::BranchCache::load(&repo_path);
                    if let Ok(repo) = git2::Repository::open(&repo_path) {
                        // MergeBaseCommits is sent before MergeStatuses, so merge_base_commit
                        // is populated here. A Pending branch with no merge base is disjoint
                        // from base and can't be squash-merged. list_branches_fast marked every
                        // non-pinned branch Pending without knowing merge bases, so resolve the
                        // disjoint ones to Unmerged now — otherwise they'd sit at Pending forever
                        // (they're excluded from the squash candidate set below).
                        for b in self.branches.items_mut() {
                            if b.merge_status == MergeStatus::Pending
                                && !b.is_base
                                && !b.is_current
                                && b.merge_base_commit.is_none()
                            {
                                b.merge_status = MergeStatus::Unmerged;
                            }
                        }
                        self.branches.rebuild_display_indices();

                        // Connected branches carry their precomputed merge base into the squash
                        // check so it never re-derives it via the unbounded `git merge-base` walk.
                        let candidates: Vec<(String, String, Option<String>)> = self
                            .branches
                            .items()
                            .iter()
                            .filter(|b| {
                                b.merge_status == MergeStatus::Pending
                                    && !b.is_base
                                    && !b.is_current
                                    && b.merge_base_commit.is_some()
                            })
                            .filter_map(|b| {
                                branch::get_commit_hash(&repo, &b.name)
                                    .map(|hash| (b.name.clone(), hash, b.merge_base_commit.clone()))
                            })
                            .collect();

                        self.squash_total = candidates.len();
                        self.squash_checked = 0;
                        let cherry_candidates = candidates.clone();
                        if !candidates.is_empty() {
                            self.squash_rx = Some(squash_loader::spawn_squash_checker(
                                repo_path.clone(),
                                base_branch.clone(),
                                candidates,
                                cache_for_squash,
                            ));
                        }

                        self.cherry_total = cherry_candidates.len();
                        self.cherry_checked = 0;
                        if !cherry_candidates.is_empty() {
                            self.cherry_rx = Some(cherry_loader::spawn_cherry_checker(
                                repo_path.clone(),
                                base_branch.clone(),
                                cherry_candidates,
                                cache_for_cherry,
                            ));
                        }
                    }
                    self.pr_rx = Some(pr_loader::spawn_pr_loader(repo_path));
                    // Branch merge statuses just changed; re-correlate worktrees
                    // if they're already loaded.
                    self.refresh_worktree_merge_status();
                }
                Phase1Msg::AheadBehind(updates) => {
                    for (name, ahead, behind) in updates {
                        if let Some(b) = self
                            .branches
                            .items_mut()
                            .iter_mut()
                            .find(|b| b.name == name)
                        {
                            b.ahead = ahead;
                            b.behind = behind;
                        }
                    }
                    self.branches.rebuild_display_indices();
                }
                Phase1Msg::MergeBaseCommits(updates) => {
                    for (name, hash) in updates {
                        if let Some(b) = self
                            .branches
                            .items_mut()
                            .iter_mut()
                            .find(|b| b.name == name)
                        {
                            b.merge_base_commit = Some(hash);
                        }
                    }
                    self.branches.rebuild_display_indices();
                }
            }
        }

        // Squash-merge results (cap 32 per tick)
        let squash_results = drain_channel(&mut self.squash_rx, 32, &mut dirty);
        let had_squash_results = !squash_results.is_empty();
        for result in squash_results {
            self.squash_checked += 1;
            if let Some(b) = self
                .branches
                .items_mut()
                .iter_mut()
                .find(|b| b.name == result.branch_name)
            {
                b.merge_status = result.status;
            }
            // Propagate squash-merge status to the matching remote branch. The remote
            // enricher marks ahead>0 branches as Unmerged; squash detection on the local
            // branch is authoritative for squash-merged remotes.
            if !matches!(result.status, MergeStatus::Unmerged | MergeStatus::Pending) {
                if let Some(r) = self
                    .remotes
                    .items_mut()
                    .iter_mut()
                    .find(|r| r.short_name == result.branch_name)
                {
                    r.merge_status = result.status;
                }
            }
        }
        if had_squash_results {
            // Squash detection just resolved branch statuses; re-correlate worktrees.
            self.refresh_worktree_merge_status();
            self.remotes.rebuild_display_indices();
        }

        // Cherry-pick results (cap 32 per tick, mirrors squash cadence)
        let cherry_results = drain_channel(&mut self.cherry_rx, 32, &mut dirty);
        let had_cherry_results = !cherry_results.is_empty();
        for result in cherry_results {
            self.cherry_checked += 1;
            if let Some(b) = self
                .branches
                .items_mut()
                .iter_mut()
                .find(|b| b.name == result.branch_name)
            {
                b.merge_status = result.status;
            }
            if !matches!(result.status, MergeStatus::Unmerged | MergeStatus::Pending) {
                if let Some(r) = self
                    .remotes
                    .items_mut()
                    .iter_mut()
                    .find(|r| r.short_name == result.branch_name)
                {
                    r.merge_status = result.status;
                }
            }
        }
        if had_cherry_results {
            self.refresh_worktree_merge_status();
            self.remotes.rebuild_display_indices();
        }

        // Remote squash-merge results
        for result in drain_channel(&mut self.remote_squash_rx, 32, &mut dirty) {
            if let Some(b) = self
                .remotes
                .items_mut()
                .iter_mut()
                .find(|b| b.full_ref == result.branch_name)
            {
                b.merge_status = result.status;
            }
        }

        // Remote enrichment
        for result in drain_channel(&mut self.remote_enrich_rx, 32, &mut dirty) {
            if let Some(b) = self
                .remotes
                .items_mut()
                .iter_mut()
                .find(|b| b.full_ref == result.full_ref)
            {
                b.merge_status = result.merge_status;
                b.ahead = result.ahead;
                b.behind = result.behind;
                b.disjoint = result.disjoint;
            }
        }

        // Worktree enrichment
        for result in drain_channel(&mut self.worktree_enrich_rx, 32, &mut dirty) {
            if let Some(wt) = self.worktrees.items_mut().get_mut(result.index) {
                wt.wt_status = result.wt_status;
                wt.age_date = result.age_date;
            }
        }

        // PR map (one-shot)
        for map in drain_channel(&mut self.pr_rx, 1, &mut dirty) {
            self.pr_map = map;
            // Push PR data into branch items
            for branch in self.branches.items_mut() {
                branch.pr = self.pr_map.get(&branch.name).cloned();
            }
            // Push PR data into remote items (keyed by short_name)
            for remote in self.remotes.items_mut() {
                remote.pr = self.pr_map.get(&remote.short_name).cloned();
            }
            self.branches.rebuild_display_indices();
            self.remotes.rebuild_display_indices();
        }

        // Tag loading (one-shot)
        for items in drain_channel(&mut self.tag_load_rx, 1, &mut dirty) {
            self.tags.set_items(items);
            self.tags.loading = false;
            list_state::apply_sort(&mut self.tags, &self.tag_columns);
            self.clear_toast();
        }

        // Remote loading (one-shot)
        for (remotes, candidates, remote_cache) in
            drain_channel(&mut self.remote_load_rx, 1, &mut dirty)
        {
            self.remotes.set_items(remotes);
            self.remotes.loading = false;
            list_state::apply_sort(&mut self.remotes, &self.remote_columns);
            self.clear_toast();

            // Spawn remote enrichment
            let unmerged: Vec<RemoteBranchInfo> = self
                .remotes
                .items()
                .iter()
                .filter(|b| !b.is_base)
                .cloned()
                .collect();
            if !unmerged.is_empty() {
                self.remote_enrich_rx = Some(branch::spawn_remote_enricher(
                    self.repo_path.clone(),
                    self.base_branch.clone(),
                    unmerged,
                ));
            }

            // Spawn remote squash checker
            if !candidates.is_empty() {
                self.remote_squash_rx = Some(squash_loader::spawn_squash_checker(
                    self.repo_path.clone(),
                    self.base_branch.clone(),
                    candidates,
                    remote_cache,
                ));
            }
        }

        // Worktree loading (one-shot)
        for items in drain_channel(&mut self.worktree_load_rx, 1, &mut dirty) {
            self.worktrees.set_items(items);
            self.worktrees.loading = false;
            list_state::apply_sort(&mut self.worktrees, &self.worktree_columns);
            self.clear_toast();

            // Branches may already be loaded; correlate merge status now.
            self.refresh_worktree_merge_status();

            // Spawn worktree enrichment
            let rx = worktree::enrich_worktrees(self.worktrees.items().to_vec());
            self.worktree_enrich_rx = Some(rx);
        }

        // Operation results (one-shot) -- fetch only; confirmed-action jobs
        // are handled by self.job_queue below, non-modally.
        for results in drain_channel(&mut self.op_rx, 1, &mut dirty) {
            self.cancel_flag = None;
            self.progress_rx = None;
            self.progress = None;
            self.refresh_after_operation();
            self.overlay = Some(Overlay::Results { results });
        }

        // Confirmed-action job queue (delete, push, merge, worktree ops, ...)
        let job_poll = self.job_queue.poll();
        if job_poll.dirty {
            dirty = true;
        }
        if let Some(JobEvent {
            action,
            remote,
            results,
            failures,
            return_view,
            ..
        }) = job_poll.event
        {
            self.refresh_after_job(action, return_view);

            // Plan P005 §5: when the job produced any typed failures,
            // auto-open the Results overlay so the user sees the cause
            // and can press `!`/`r` to recover without scrolling past
            // successes. Success-only completion stays non-modal — the
            // transient `CompletionSummary` in the status area is the
            // only feedback. `failures` already excludes `BranchNotFound`
            // (treated as "already gone" success — see §9).
            if !failures.is_empty() {
                self.overlay = Some(Overlay::Results { results: failures });
            }

            // When DeleteLocalAndRemote completes, immediately filter confirmed remote
            // deletions from the in-memory remotes list so they don't appear until a fetch.
            if action == BranchAction::DeleteLocalAndRemote {
                let selected_remote = remote.as_deref().unwrap_or("origin");
                let successfully_deleted: Vec<String> = results
                    .iter()
                    .filter(|r| r.success && r.action == BranchAction::DeleteRemoteBranch)
                    .map(|r| r.branch_name.clone())
                    .collect();

                if !successfully_deleted.is_empty() {
                    let deleted_set: std::collections::HashSet<_> =
                        successfully_deleted.into_iter().collect();
                    self.remotes.set_items(
                        self.remotes
                            .items()
                            .iter()
                            .filter(|remote| {
                                remote.remote != selected_remote
                                    || !deleted_set.contains(&remote.short_name)
                            })
                            .cloned()
                            .collect(),
                    );
                    list_state::apply_sort(&mut self.remotes, &self.remote_columns);
                }
            }
        }

        // Cache-audit result (one-shot)
        for audit in drain_channel(&mut self.diag_rx, 1, &mut dirty) {
            self.cancel_flag = None;
            self.progress_rx = None;
            self.progress = None;
            self.overlay = Some(Overlay::DiagnosticsReport { audit, scroll: 0 });
        }

        // Silent, automatic launch-time cache verification (one-shot). The
        // audit has already been applied to disk by `spawn_cache_verifier`;
        // this only patches live state. No overlay, no toast — fully silent.
        for audit in drain_channel(&mut self.cache_verify_rx, 1, &mut dirty) {
            if audit.is_clean() {
                continue;
            }
            for d in &audit.discrepancies {
                if let Some(b) = self
                    .branches
                    .items_mut()
                    .iter_mut()
                    .find(|b| b.name == d.branch)
                {
                    match &d.fix {
                        CacheFix::Status { status, .. } => {
                            b.merge_status = *status;
                            self.verified_branches.insert(d.branch.clone());
                        }
                        CacheFix::AheadBehind { ahead, behind, .. } => {
                            b.ahead = Some(*ahead);
                            b.behind = Some(*behind);
                        }
                        CacheFix::MergeBase { merge_base, .. } => {
                            b.merge_base_commit = merge_base.clone();
                        }
                    }
                }
            }
            self.branches.rebuild_display_indices();
            self.refresh_worktree_merge_status();
        }

        // Progress updates
        for update in drain_channel(&mut self.progress_rx, 32, &mut dirty) {
            self.progress = Some(update.clone());
            if let Some(Overlay::Executing { progress, .. }) = &mut self.overlay {
                *progress = Some(update);
            }
        }

        // Remote fetch completion
        for success in drain_channel(&mut self.remote_fetch_rx, 1, &mut dirty) {
            if success {
                self.remote_fetched = true;
                // Reload remote branches if we're on that view
                if self.active_view == ViewId::Remotes {
                    self.spawn_remote_load();
                }
            }
            self.clear_toast();
        }

        // Expire toast
        if let Some(ref toast) = self.toast {
            if toast.is_expired() {
                self.toast = None;
                dirty = true;
            }
        }

        if dirty {
            self.refresh_open_graph_menu();
        }

        dirty
    }

    // ---- Event Dispatch ----

    fn handle_event(&mut self, event: Event) {
        match event {
            Event::Key(key) => {
                if key.kind != KeyEventKind::Press {
                    return;
                }
                self.handle_key(key);
            }
            Event::Mouse(mouse) => self.handle_mouse(mouse),
            _ => {}
        }
    }

    fn handle_key(&mut self, key: KeyEvent) {
        // If search is active, route to search handler first
        if self.is_search_active() {
            self.handle_search_key(key);
            return;
        }

        // Route to overlay handler if overlay is active
        if self.overlay.is_some() {
            self.handle_overlay_key(key);
            return;
        }

        // Global keys (work in every view, take priority)
        match key.code {
            KeyCode::Char('q') => {
                self.should_exit = true;
                return;
            }
            KeyCode::Char('?') => {
                self.overlay = Some(Overlay::Help);
                return;
            }
            KeyCode::Char(',') => {
                self.overlay = Some(Overlay::Settings { cursor: 0 });
                return;
            }
            KeyCode::Char('T') => {
                self.theme = self.theme.next();
                self.save_config();
                return;
            }
            KeyCode::Char('Y') => {
                self.symbols = self.symbols.next();
                self.reload_graph_for_symbol_change();
                self.save_config();
                return;
            }
            KeyCode::Tab => {
                if key
                    .modifiers
                    .contains(crossterm::event::KeyModifiers::SHIFT)
                {
                    self.active_view = self.active_view.prev();
                } else {
                    self.active_view = self.active_view.next();
                }
                self.ensure_view_loaded();
                return;
            }
            KeyCode::BackTab => {
                self.active_view = self.active_view.prev();
                self.ensure_view_loaded();
                return;
            }
            KeyCode::Char('/') => {
                self.toggle_search();
                return;
            }
            KeyCode::Char('\\') => {
                self.overlay = Some(Overlay::Filter);
                return;
            }
            KeyCode::F(2) => {
                self.return_view = self.active_view;
                self.overlay = Some(Overlay::Diagnostics { cursor: 0 });
                return;
            }
            KeyCode::Char('x') => {
                self.job_queue.cancel_current();
                return;
            }
            KeyCode::Char('X') => {
                self.job_queue.clear_queued();
                return;
            }
            _ => {}
        }

        if self.active_view == ViewId::Graph {
            self.handle_graph_key(key);
            return;
        }

        // Common navigation/selection keys (work in every view)
        if self.handle_common_list_key(key) {
            return;
        }

        // View-specific keys
        match self.active_view {
            ViewId::Graph => {
                self.handle_graph_key(key);
            }
            ViewId::Branches => self.handle_branches_key(key),
            ViewId::Remotes => self.handle_remotes_key(key),
            ViewId::Tags => self.handle_tags_key(key),
            ViewId::Worktrees => self.handle_worktrees_key(key),
        }
    }

    /// Keys shared by all 4 views: navigation, selection, sorting.
    /// Returns true if the key was handled.
    fn handle_common_list_key(&mut self, key: KeyEvent) -> bool {
        macro_rules! with_state {
            ($op:expr) => {
                match self.active_view {
                    ViewId::Branches => $op(&mut self.branches),
                    ViewId::Remotes => $op(&mut self.remotes),
                    ViewId::Tags => $op(&mut self.tags),
                    ViewId::Worktrees => $op(&mut self.worktrees),
                    ViewId::Graph => {}
                }
            };
        }

        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                with_state!(list_state::nav_down);
                true
            }
            KeyCode::Char('k') | KeyCode::Up => {
                with_state!(list_state::nav_up);
                true
            }
            KeyCode::PageDown => {
                with_state!(|s| list_state::nav_page_down(s, 20));
                true
            }
            KeyCode::PageUp => {
                with_state!(|s| list_state::nav_page_up(s, 20));
                true
            }
            KeyCode::Home => {
                with_state!(list_state::nav_home);
                true
            }
            KeyCode::End => {
                with_state!(list_state::nav_end);
                true
            }
            KeyCode::Char(' ') => {
                with_state!(list_state::select_toggle);
                true
            }
            KeyCode::Char('a') => {
                with_state!(list_state::select_all);
                true
            }
            KeyCode::Char('n') => {
                with_state!(list_state::deselect_all);
                true
            }
            KeyCode::Char('i') => {
                with_state!(list_state::invert_selection);
                true
            }
            KeyCode::Char('m') => {
                with_state!(list_state::select_merged);
                true
            }
            KeyCode::Char('s') => {
                self.cycle_sort();
                true
            }
            KeyCode::Char('S') => {
                self.toggle_sort_direction();
                true
            }
            KeyCode::Enter => {
                self.open_context_menu();
                true
            }
            _ => false,
        }
    }

    // ---- View-specific key handlers ----

    fn handle_branches_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('d') => self.delete_selected_branches(false),
            KeyCode::Char('D') => self.delete_selected_branches(true),
            KeyCode::Char('p') => self.push_selected_branches(),
            KeyCode::Char('R') => self.clear_cache_and_refresh(),
            KeyCode::Char('f') => self.start_fetch(false),
            KeyCode::Char('F') => self.start_fetch(true),
            KeyCode::Char('r') => {
                self.active_view = ViewId::Remotes;
                self.ensure_view_loaded();
            }
            KeyCode::Char('t') => {
                self.active_view = ViewId::Tags;
                self.ensure_view_loaded();
            }
            KeyCode::Char('w') => {
                self.active_view = ViewId::Worktrees;
                self.ensure_view_loaded();
            }
            _ => {}
        }
    }

    fn handle_graph_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => self.open_context_menu(),
            KeyCode::Char('j') | KeyCode::Down => self.graph.move_down(),
            KeyCode::Char('k') | KeyCode::Up => self.graph.move_up(),
            KeyCode::PageDown => self.graph.page_down(),
            KeyCode::PageUp => self.graph.page_up(),
            KeyCode::Char('h') | KeyCode::Left => self.graph.scroll_left(),
            KeyCode::Char('l') | KeyCode::Right => self.graph.scroll_right(),
            KeyCode::Home | KeyCode::Char('g') => self.graph.home(),
            KeyCode::End | KeyCode::Char('G') => self.graph.end(),
            KeyCode::Char('L') => self.load_older_graph(),
            KeyCode::Char('o') => self.open_graph_options(),
            KeyCode::Char('r') => self.reload_graph(),
            _ => {}
        }
    }

    fn handle_remotes_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('d') => self.delete_selected_remote_branches(),
            KeyCode::Char('f') => self.start_fetch(false),
            KeyCode::Char('F') => self.start_fetch(true),
            KeyCode::Char('b') | KeyCode::Char('r') | KeyCode::Esc => {
                self.active_view = ViewId::Branches;
            }
            KeyCode::Char('t') => {
                self.active_view = ViewId::Tags;
                self.ensure_view_loaded();
            }
            KeyCode::Char('w') => {
                self.active_view = ViewId::Worktrees;
                self.ensure_view_loaded();
            }
            _ => {}
        }
    }

    fn handle_tags_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('d') => self.delete_selected_tags(false),
            KeyCode::Char('D') => self.delete_selected_tags(true),
            KeyCode::Char('p') => self.push_selected_tags(),
            KeyCode::Char('f') => self.start_fetch(false),
            KeyCode::Char('F') => self.start_fetch(true),
            KeyCode::Char('b') | KeyCode::Char('t') | KeyCode::Esc => {
                self.active_view = ViewId::Branches;
            }
            KeyCode::Char('r') => {
                self.active_view = ViewId::Remotes;
                self.ensure_view_loaded();
            }
            KeyCode::Char('w') => {
                self.active_view = ViewId::Worktrees;
                self.ensure_view_loaded();
            }
            _ => {}
        }
    }

    fn handle_worktrees_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('d') => self.remove_selected_worktrees(false),
            KeyCode::Char('D') => self.remove_selected_worktrees(true),
            KeyCode::Char('f') => self.start_fetch(false),
            KeyCode::Char('F') => self.start_fetch(true),
            KeyCode::Char('b') | KeyCode::Char('w') | KeyCode::Esc => {
                self.active_view = ViewId::Branches;
            }
            KeyCode::Char('r') => {
                self.active_view = ViewId::Remotes;
                self.ensure_view_loaded();
            }
            KeyCode::Char('t') => {
                self.active_view = ViewId::Tags;
                self.ensure_view_loaded();
            }
            _ => {}
        }
    }

    // ---- Overlay key handling ----

    fn handle_overlay_key(&mut self, key: KeyEvent) {
        let overlay = self.overlay.take();
        match overlay {
            Some(Overlay::Help) => {
                // Any key closes help
            }
            Some(Overlay::Confirm {
                action,
                targets,
                remote,
                reason,
                extra_keys,
            }) => match key.code {
                KeyCode::Char('y') | KeyCode::Enter => {
                    self.job_queue.enqueue_or_start_with_remote(
                        action,
                        targets,
                        remote,
                        self.return_view,
                    );
                }
                KeyCode::Char('n') | KeyCode::Esc => {
                    // Cancel -- don't put overlay back
                }
                KeyCode::Char(c) => {
                    // Plan P005 §6: an extra-key press (e.g. `!` for
                    // force-delete, `r` for worktree cascade) swaps the
                    // pending action and re-installs the overlay with
                    // the same pre-flight context so the user can
                    // iterate without leaving the confirm flow.
                    if let Some(extra) = extra_keys.iter().find(|e| e.key == c) {
                        let swapped = extra.action;
                        let extra_targets = extra.targets.clone();
                        self.overlay = Some(Overlay::Confirm {
                            action: swapped,
                            targets: extra_targets,
                            remote,
                            reason,
                            extra_keys,
                        });
                    } else {
                        self.overlay = Some(Overlay::Confirm {
                            action,
                            targets,
                            remote,
                            reason,
                            extra_keys,
                        });
                    }
                }
                _ => {
                    self.overlay = Some(Overlay::Confirm {
                        action,
                        targets,
                        remote,
                        reason,
                        extra_keys,
                    });
                }
            },
            Some(Overlay::Menu { cursor, items }) => match key.code {
                KeyCode::Char('j') | KeyCode::Down => {
                    let mut new_cursor = cursor + 1;
                    while new_cursor < items.len() && !items[new_cursor].enabled {
                        new_cursor += 1;
                    }
                    if new_cursor < items.len() {
                        self.overlay = Some(Overlay::Menu {
                            cursor: new_cursor,
                            items,
                        });
                    } else {
                        self.overlay = Some(Overlay::Menu { cursor, items });
                    }
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    let mut new_cursor = cursor;
                    loop {
                        if new_cursor == 0 {
                            break;
                        }
                        new_cursor -= 1;
                        if items[new_cursor].enabled {
                            break;
                        }
                    }
                    self.overlay = Some(Overlay::Menu {
                        cursor: new_cursor,
                        items,
                    });
                }
                KeyCode::Enter => {
                    if let Some(item) = items.get(cursor) {
                        if item.enabled {
                            self.execute_menu_action(item.clone());
                        } else {
                            self.overlay = Some(Overlay::Menu { cursor, items });
                        }
                    }
                }
                KeyCode::Esc | KeyCode::Char('q') => {} // close
                KeyCode::Char(c) => {
                    if let Some((_, item)) = items
                        .iter()
                        .enumerate()
                        .find(|(_, mi)| mi.shortcut == Some(c) && mi.enabled)
                    {
                        self.execute_menu_action(item.clone());
                    } else {
                        self.overlay = Some(Overlay::Menu { cursor, items });
                    }
                }
                _ => {
                    self.overlay = Some(Overlay::Menu { cursor, items });
                }
            },
            Some(Overlay::InfoModal {
                cursor,
                info_cursor,
                focus,
                items,
                row,
            }) => match key.code {
                KeyCode::Tab | KeyCode::BackTab => {
                    self.overlay = Some(Overlay::InfoModal {
                        cursor,
                        info_cursor,
                        focus: if focus == InfoModalFocus::Info {
                            InfoModalFocus::Actions
                        } else {
                            InfoModalFocus::Info
                        },
                        items,
                        row,
                    });
                }
                KeyCode::Char('j') | KeyCode::Down if focus == InfoModalFocus::Actions => {
                    let mut new_cursor = cursor + 1;
                    while new_cursor < items.len() && !items[new_cursor].enabled {
                        new_cursor += 1;
                    }
                    if new_cursor < items.len() {
                        self.overlay = Some(Overlay::InfoModal {
                            cursor: new_cursor,
                            info_cursor,
                            focus,
                            items,
                            row,
                        });
                    } else {
                        self.overlay = Some(Overlay::InfoModal {
                            cursor,
                            info_cursor,
                            focus,
                            items,
                            row,
                        });
                    }
                }
                KeyCode::Char('k') | KeyCode::Up if focus == InfoModalFocus::Actions => {
                    let mut new_cursor = cursor;
                    loop {
                        if new_cursor == 0 {
                            break;
                        }
                        new_cursor -= 1;
                        if items[new_cursor].enabled {
                            break;
                        }
                    }
                    self.overlay = Some(Overlay::InfoModal {
                        cursor: new_cursor,
                        info_cursor,
                        focus,
                        items,
                        row,
                    });
                }
                KeyCode::Char('u') | KeyCode::PageUp => {
                    self.info_modal_scroll_offset = self.info_modal_scroll_offset.saturating_sub(5);
                    self.overlay = Some(Overlay::InfoModal {
                        cursor,
                        info_cursor,
                        focus,
                        items,
                        row,
                    });
                }
                KeyCode::Char('d') | KeyCode::PageDown => {
                    self.info_modal_scroll_offset = self.info_modal_scroll_offset.saturating_add(5);
                    self.overlay = Some(Overlay::InfoModal {
                        cursor,
                        info_cursor,
                        focus,
                        items,
                        row,
                    });
                }
                KeyCode::Down if focus == InfoModalFocus::Info => {
                    let info_cursor = info_cursor
                        .saturating_add(1)
                        .min(row.info_field_count().saturating_sub(1));
                    self.overlay = Some(Overlay::InfoModal {
                        cursor,
                        info_cursor,
                        focus,
                        items,
                        row,
                    });
                }
                KeyCode::Up if focus == InfoModalFocus::Info => {
                    let info_cursor = info_cursor.saturating_sub(1);
                    self.overlay = Some(Overlay::InfoModal {
                        cursor,
                        info_cursor,
                        focus,
                        items,
                        row,
                    });
                }
                KeyCode::Enter if focus == InfoModalFocus::Info => {
                    if let Some((label, value)) = row.info_field(info_cursor) {
                        self.copy_info_value(label, value);
                    }
                    self.overlay = Some(Overlay::InfoModal {
                        cursor,
                        info_cursor,
                        focus,
                        items,
                        row,
                    });
                }
                KeyCode::Char('y') if focus == InfoModalFocus::Info => {
                    if let Some((label, value)) = row.info_field(info_cursor) {
                        self.copy_info_value(label, value);
                    }
                    self.overlay = Some(Overlay::InfoModal {
                        cursor,
                        info_cursor,
                        focus,
                        items,
                        row,
                    });
                }
                KeyCode::Enter if focus == InfoModalFocus::Actions => {
                    if let Some(item) = items.get(cursor) {
                        if item.enabled {
                            self.execute_menu_action(item.clone());
                        } else {
                            self.overlay = Some(Overlay::InfoModal {
                                cursor,
                                info_cursor,
                                focus,
                                items,
                                row,
                            });
                        }
                    }
                }
                KeyCode::Esc | KeyCode::Char('q') => {} // close
                KeyCode::Char(c) if focus == InfoModalFocus::Actions => {
                    if let Some((_, item)) = items
                        .iter()
                        .enumerate()
                        .find(|(_, mi)| mi.shortcut == Some(c) && mi.enabled)
                    {
                        self.execute_menu_action(item.clone());
                    } else {
                        self.overlay = Some(Overlay::InfoModal {
                            cursor,
                            info_cursor,
                            focus,
                            items,
                            row,
                        });
                    }
                }
                _ => {
                    self.overlay = Some(Overlay::InfoModal {
                        cursor,
                        info_cursor,
                        focus,
                        items,
                        row,
                    });
                }
            },
            Some(Overlay::Results { results }) => match key.code {
                KeyCode::Enter | KeyCode::Esc => {
                    // Operation completion already refreshed the backing view;
                    // closing results should only reveal the refreshed list.
                }
                KeyCode::Char('!') => {
                    let targets: Vec<String> = results
                        .iter()
                        .filter(|result| {
                            matches!(&result.failure, Some(FailureCause::NotMerged))
                        })
                        .map(|result| result.branch_name.clone())
                        .collect();
                    if targets.is_empty() {
                        self.overlay = Some(Overlay::Results { results });
                    } else {
                        self.job_queue.enqueue_or_start(
                            BranchAction::DeleteLocalForce,
                            targets,
                            self.return_view,
                        );
                    }
                }
                KeyCode::Char('r') => {
                    let targets: Vec<String> = results
                        .iter()
                        .filter(|result| {
                            matches!(
                                &result.failure,
                                Some(FailureCause::CheckedOutInWorktree {
                                    is_main: false,
                                    ..
                                })
                            )
                        })
                        .map(|result| result.branch_name.clone())
                        .collect();
                    if targets.is_empty() {
                        self.overlay = Some(Overlay::Results { results });
                    } else {
                        self.job_queue.enqueue_or_start(
                            BranchAction::DeleteBranchAndRemoveWorktree,
                            targets,
                            self.return_view,
                        );
                    }
                }
                _ => {
                    self.overlay = Some(Overlay::Results { results });
                }
            },
            Some(Overlay::Executing { label, progress }) => {
                if key.code == KeyCode::Esc {
                    if let Some(flag) = &self.cancel_flag {
                        flag.store(true, Ordering::Relaxed);
                    }
                    // Option 3: drop receivers so UI recovers immediately;
                    // the background thread will fail on its next send and exit.
                    self.op_rx = None;
                    self.diag_rx = None;
                    self.progress_rx = None;
                    self.progress = None;
                    self.cancel_flag = None;
                    // overlay stays None (already taken at top of function)
                    return;
                }
                // overlay was taken; put it back for any other key
                self.overlay = Some(Overlay::Executing { label, progress });
            }
            Some(Overlay::Settings { cursor }) => {
                self.handle_settings_key(key, cursor);
            }
            Some(Overlay::Filter) => {
                self.handle_filter_key(key);
            }
            Some(Overlay::GraphOptions {
                cursor,
                include_remotes,
            }) => match key.code {
                KeyCode::Char('j') | KeyCode::Down => {
                    self.overlay = Some(Overlay::GraphOptions {
                        cursor: (cursor + 1).min(1),
                        include_remotes,
                    });
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.overlay = Some(Overlay::GraphOptions {
                        cursor: cursor.saturating_sub(1),
                        include_remotes,
                    });
                }
                KeyCode::Char(' ') => {
                    self.overlay = Some(Overlay::GraphOptions {
                        cursor,
                        include_remotes: !include_remotes,
                    });
                }
                KeyCode::Enter => {
                    self.apply_graph_options(cursor, include_remotes);
                    self.save_config();
                }
                KeyCode::Esc | KeyCode::Char('q') => {}
                _ => {
                    self.overlay = Some(Overlay::GraphOptions {
                        cursor,
                        include_remotes,
                    });
                }
            },
            Some(Overlay::Diagnostics { cursor }) => {
                let count = DiagnosticAction::ALL.len();
                match key.code {
                    KeyCode::Char('j') | KeyCode::Down => {
                        self.overlay = Some(Overlay::Diagnostics {
                            cursor: (cursor + 1).min(count.saturating_sub(1)),
                        });
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        self.overlay = Some(Overlay::Diagnostics {
                            cursor: cursor.saturating_sub(1),
                        });
                    }
                    KeyCode::Enter => {
                        if let Some(action) = DiagnosticAction::ALL.get(cursor) {
                            match action {
                                DiagnosticAction::VerifyCache => self.run_cache_audit(),
                            }
                        }
                    }
                    KeyCode::Esc | KeyCode::Char('q') => {} // close
                    _ => {
                        self.overlay = Some(Overlay::Diagnostics { cursor });
                    }
                }
            }
            Some(Overlay::DiagnosticsReport { audit, scroll }) => match key.code {
                KeyCode::Char('j') | KeyCode::Down => {
                    self.overlay = Some(Overlay::DiagnosticsReport {
                        audit,
                        scroll: scroll + 1,
                    });
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.overlay = Some(Overlay::DiagnosticsReport {
                        audit,
                        scroll: scroll.saturating_sub(1),
                    });
                }
                KeyCode::Char('f') if !audit.is_clean() => {
                    self.apply_cache_fix(audit);
                }
                KeyCode::Esc | KeyCode::Char('q') => {} // close
                _ => {
                    self.overlay = Some(Overlay::DiagnosticsReport { audit, scroll });
                }
            },
            None => {}
        }
    }

    fn handle_settings_key(&mut self, key: KeyEvent, cursor: usize) {
        const NUM_ROWS: usize = 8;

        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.overlay = Some(Overlay::Settings {
                    cursor: (cursor + 1).min(NUM_ROWS - 1),
                });
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.overlay = Some(Overlay::Settings {
                    cursor: cursor.saturating_sub(1),
                });
            }
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Char(' ') => {
                match cursor {
                    0 => {
                        self.symbols = self.symbols.next();
                        self.reload_graph_for_symbol_change();
                    }
                    1 => {
                        self.theme = self.theme.next();
                    }
                    2 => {
                        self.cycle_view_sort(ViewId::Branches, true);
                    }
                    3 => {
                        self.cycle_view_sort(ViewId::Remotes, true);
                    }
                    4 => {
                        self.cycle_view_sort(ViewId::Tags, true);
                    }
                    5 => {
                        self.cycle_view_sort(ViewId::Worktrees, true);
                    }
                    6 => {
                        self.config.auto_fetch = Some(self.config.auto_fetch != Some(true));
                    }
                    7 => {
                        self.config.load_worktrees_on_launch =
                            Some(self.config.load_worktrees_on_launch != Some(true));
                    }
                    _ => {}
                }
                self.save_config();
                self.overlay = Some(Overlay::Settings { cursor });
            }
            KeyCode::Left | KeyCode::Char('h') => {
                match cursor {
                    0 => {
                        // backward = next() twice (3-cycle)
                        self.symbols = self.symbols.next();
                        self.symbols = self.symbols.next();
                        self.reload_graph_for_symbol_change();
                    }
                    1 => {
                        // backward = next() 3 times (4-cycle)
                        self.theme = self.theme.next();
                        self.theme = self.theme.next();
                        self.theme = self.theme.next();
                    }
                    2 => {
                        self.cycle_view_sort(ViewId::Branches, false);
                    }
                    3 => {
                        self.cycle_view_sort(ViewId::Remotes, false);
                    }
                    4 => {
                        self.cycle_view_sort(ViewId::Tags, false);
                    }
                    5 => {
                        self.cycle_view_sort(ViewId::Worktrees, false);
                    }
                    6 => {
                        self.config.auto_fetch = Some(self.config.auto_fetch != Some(true));
                    }
                    7 => {
                        self.config.load_worktrees_on_launch =
                            Some(self.config.load_worktrees_on_launch != Some(true));
                    }
                    _ => {}
                }
                self.save_config();
                self.overlay = Some(Overlay::Settings { cursor });
            }
            KeyCode::Esc => {
                // Close settings (overlay already taken)
            }
            _ => {
                self.overlay = Some(Overlay::Settings { cursor });
            }
        }
    }

    fn handle_filter_key(&mut self, key: KeyEvent) {
        let active_tokens = match self.active_view {
            ViewId::Graph => &[] as &[FilterTokenDef],
            ViewId::Branches => &self.branch_filter_tokens,
            ViewId::Remotes => &self.remote_filter_tokens,
            ViewId::Tags => &self.tag_filter_tokens,
            ViewId::Worktrees => &self.worktree_filter_tokens,
        };

        match key.code {
            KeyCode::Esc | KeyCode::Char('\\') => {
                // Close filter (overlay already taken); only Esc / \ dismiss it.
            }
            KeyCode::Char('c') => {
                // Clear all filters, but keep the modal open.
                self.set_active_filter(String::new());
                self.overlay = Some(Overlay::Filter);
            }
            KeyCode::Char(ch) => {
                // Toggle the matching filter token (if any). Either way the modal
                // stays open so the user can adjust several filters in a row.
                if let Some(token_def) = active_tokens.iter().find(|t| t.key == ch) {
                    let current = self.active_filter_query();
                    let new = FilterSet::toggle_token(&current, token_def.token);
                    self.set_active_filter(new);
                }
                self.overlay = Some(Overlay::Filter);
            }
            _ => {
                self.overlay = Some(Overlay::Filter);
            }
        }
    }

    // ---- Search ----

    fn is_search_active(&self) -> bool {
        match self.active_view {
            ViewId::Graph => false,
            ViewId::Branches => self.branches.search_active(),
            ViewId::Remotes => self.remotes.search_active(),
            ViewId::Tags => self.tags.search_active(),
            ViewId::Worktrees => self.worktrees.search_active(),
        }
    }

    fn toggle_search(&mut self) {
        match self.active_view {
            ViewId::Graph => {}
            ViewId::Branches => self.branches.set_search_active(true),
            ViewId::Remotes => self.remotes.set_search_active(true),
            ViewId::Tags => self.tags.set_search_active(true),
            ViewId::Worktrees => self.worktrees.set_search_active(true),
        }
    }

    fn handle_search_key(&mut self, key: KeyEvent) {
        macro_rules! with_state {
            ($state:expr, $code:expr) => {
                match $code {
                    KeyCode::Esc => {
                        $state.set_search_query(String::new());
                        $state.set_search_active(false);
                    }
                    KeyCode::Enter => {
                        $state.set_search_active(false);
                    }
                    KeyCode::Backspace => {
                        let mut q = $state.search_query().to_string();
                        q.pop();
                        $state.set_search_query(q);
                    }
                    KeyCode::Char(c) => {
                        let mut q = $state.search_query().to_string();
                        q.push(c);
                        $state.set_search_query(q);
                    }
                    _ => {}
                }
            };
        }
        match self.active_view {
            ViewId::Graph => {}
            ViewId::Branches => with_state!(self.branches, key.code),
            ViewId::Remotes => with_state!(self.remotes, key.code),
            ViewId::Tags => with_state!(self.tags, key.code),
            ViewId::Worktrees => with_state!(self.worktrees, key.code),
        }
    }

    // ---- Mouse handling ----

    fn handle_mouse(&mut self, mouse: crossterm::event::MouseEvent) {
        // The info modal handles left-clicks (click a value to copy it) and
        // mouse-wheel scrolling of its combined info+actions pane.
        if matches!(self.overlay, Some(Overlay::InfoModal { .. })) {
            match mouse.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    self.handle_info_modal_click(mouse.column, mouse.row);
                }
                MouseEventKind::ScrollDown => {
                    self.info_modal_scroll_offset =
                        self.info_modal_scroll_offset.saturating_add(3);
                }
                MouseEventKind::ScrollUp => {
                    self.info_modal_scroll_offset =
                        self.info_modal_scroll_offset.saturating_sub(3);
                }
                _ => {}
            }
            return;
        }

        // Don't handle mouse in other overlays
        if self.overlay.is_some() {
            return;
        }

        match mouse.kind {
            MouseEventKind::ScrollDown => {
                macro_rules! with_state {
                    ($op:expr) => {
                        match self.active_view {
                            ViewId::Graph => self.graph.move_down(),
                            ViewId::Branches => $op(&mut self.branches),
                            ViewId::Remotes => $op(&mut self.remotes),
                            ViewId::Tags => $op(&mut self.tags),
                            ViewId::Worktrees => $op(&mut self.worktrees),
                        }
                    };
                }
                with_state!(list_state::nav_down);
            }
            MouseEventKind::ScrollUp => {
                macro_rules! with_state {
                    ($op:expr) => {
                        match self.active_view {
                            ViewId::Graph => self.graph.move_up(),
                            ViewId::Branches => $op(&mut self.branches),
                            ViewId::Remotes => $op(&mut self.remotes),
                            ViewId::Tags => $op(&mut self.tags),
                            ViewId::Worktrees => $op(&mut self.worktrees),
                        }
                    };
                }
                with_state!(list_state::nav_up);
            }
            MouseEventKind::Down(MouseButton::Left) => {
                self.handle_left_click(mouse.column, mouse.row);
            }
            MouseEventKind::Down(MouseButton::Right) => {
                self.handle_right_click(mouse.column, mouse.row);
            }
            _ => {}
        }
    }

    /// A left-click inside the info modal: if it landed on a value, copy that
    /// value to the host clipboard and show a confirmation in the modal.
    fn handle_info_modal_click(&mut self, x: u16, y: u16) {
        let hit = self
            .info_hit_regions
            .iter()
            .find(|r| {
                x >= r.rect.x
                    && x < r.rect.x + r.rect.width
                    && y >= r.rect.y
                    && y < r.rect.y + r.rect.height
            })
            .map(|r| (r.label.clone(), r.value.clone()));

        if let Some((label, value)) = hit {
            self.copy_info_value(label, value);
        }
    }

    fn copy_info_value(&mut self, label: String, value: String) {
        self.info_copied_msg = Some(match copy_to_clipboard(&value) {
            Ok(()) => format!("{label} copied to clipboard"),
            Err(e) => format!("Clipboard error: {e}"),
        });
    }

    fn handle_left_click(&mut self, x: u16, y: u16) {
        // Header row click (y == 1): sort by column
        if y == 1 {
            let clicked_col = self.find_header_click(x);
            if let Some(col) = clicked_col {
                match self.active_view {
                    ViewId::Graph => {}
                    ViewId::Branches => list_state::sort_by_column_click(
                        &mut self.branches,
                        &self.branch_columns,
                        col,
                    ),
                    ViewId::Remotes => list_state::sort_by_column_click(
                        &mut self.remotes,
                        &self.remote_columns,
                        col,
                    ),
                    ViewId::Tags => {
                        list_state::sort_by_column_click(&mut self.tags, &self.tag_columns, col)
                    }
                    ViewId::Worktrees => list_state::sort_by_column_click(
                        &mut self.worktrees,
                        &self.worktree_columns,
                        col,
                    ),
                }
            }
        } else if self.terminal_rows > 0 && y == self.terminal_rows - 1 {
            // Status bar click
            let items = match self.active_view {
                ViewId::Graph => Vec::new(),
                ViewId::Branches => self.branches.status_bar_items.clone(),
                ViewId::Remotes => self.remotes.status_bar_items.clone(),
                ViewId::Tags => self.tags.status_bar_items.clone(),
                ViewId::Worktrees => self.worktrees.status_bar_items.clone(),
            };
            for &(x_start, x_end, key) in &items {
                if x >= x_start && x < x_end {
                    self.handle_key(KeyEvent::new(key, crossterm::event::KeyModifiers::NONE));
                    break;
                }
            }
        } else if y >= 2 {
            // Click on a data row
            macro_rules! click_row {
                ($state:expr) => {{
                    let scroll_offset = $state.table_state().offset();
                    let clicked_display_row = (y - 2) as usize + scroll_offset;
                    if let Some(&raw_idx) = $state.display_indices().get(clicked_display_row) {
                        $state.set_cursor(raw_idx);
                        $state.table_state_mut().select(Some(clicked_display_row));
                    }
                }};
            }
            match self.active_view {
                ViewId::Graph => {}
                ViewId::Branches => click_row!(self.branches),
                ViewId::Remotes => click_row!(self.remotes),
                ViewId::Tags => click_row!(self.tags),
                ViewId::Worktrees => click_row!(self.worktrees),
            }
        }
    }

    fn find_header_click(&self, x: u16) -> Option<usize> {
        let header_columns = match self.active_view {
            ViewId::Graph => &[] as &[(u16, usize)],
            ViewId::Branches => &self.branches.header_columns,
            ViewId::Remotes => &self.remotes.header_columns,
            ViewId::Tags => &self.tags.header_columns,
            ViewId::Worktrees => &self.worktrees.header_columns,
        };
        if header_columns.is_empty() {
            return None;
        }
        for (i, &(col_x, sort_idx)) in header_columns.iter().enumerate() {
            let next_x = if i + 1 < header_columns.len() {
                header_columns[i + 1].0
            } else {
                u16::MAX
            };
            if x >= col_x && x < next_x {
                return Some(sort_idx);
            }
        }
        None
    }

    fn handle_right_click(&mut self, _x: u16, y: u16) {
        if y < 2 {
            return;
        }

        macro_rules! move_cursor {
            ($state:expr) => {{
                let scroll_offset = $state.table_state().offset();
                let clicked_display_row = (y - 2) as usize + scroll_offset;
                if let Some(&raw_idx) = $state.display_indices().get(clicked_display_row) {
                    $state.set_cursor(raw_idx);
                    $state.table_state_mut().select(Some(clicked_display_row));
                    true
                } else {
                    false
                }
            }};
        }

        let moved = match self.active_view {
            ViewId::Graph => false,
            ViewId::Branches => move_cursor!(self.branches),
            ViewId::Remotes => move_cursor!(self.remotes),
            ViewId::Tags => move_cursor!(self.tags),
            ViewId::Worktrees => move_cursor!(self.worktrees),
        };

        if moved {
            self.open_context_menu();
        }
    }

    // ---- Context Menu Building ----

    fn open_context_menu(&mut self) {
        let items = self.build_menu_items();
        let Some(row) = self.build_info_modal_row() else {
            return;
        };
        let focus = if items.is_empty() {
            InfoModalFocus::Info
        } else {
            InfoModalFocus::Actions
        };
        self.return_view = self.active_view;
        // Clear any stale copy confirmation from a previous opening.
        self.info_copied_msg = None;
        self.info_modal_scroll_offset = 0;
        self.overlay = Some(Overlay::InfoModal {
            items,
            cursor: 0,
            info_cursor: 0,
            focus,
            row,
        });
    }

    fn build_menu_items(&self) -> Vec<MenuItem> {
        match self.active_view {
            ViewId::Graph => self.build_graph_menu(),
            ViewId::Branches => self.build_branch_menu(),
            ViewId::Remotes => self.build_remote_menu(),
            ViewId::Tags => self.build_tag_menu(),
            ViewId::Worktrees => self.build_worktree_menu(),
        }
    }

    fn build_info_modal_row(&self) -> Option<InfoModalRow> {
        match self.active_view {
            ViewId::Graph => self
                .graph
                .selected_commit()
                .cloned()
                .map(InfoModalRow::GraphCommit),
            ViewId::Branches => self
                .branches
                .cursor_item()
                .cloned()
                .map(InfoModalRow::Branch),
            ViewId::Remotes => self
                .remotes
                .cursor_item()
                .cloned()
                .map(InfoModalRow::Remote),
            ViewId::Tags => self.tags.cursor_item().cloned().map(InfoModalRow::Tag),
            ViewId::Worktrees => self
                .worktrees
                .cursor_item()
                .cloned()
                .map(InfoModalRow::Worktree),
        }
    }

    fn build_graph_menu(&self) -> Vec<MenuItem> {
        let Some(commit) = self.graph.selected_commit() else {
            return vec![];
        };
        self.build_graph_menu_for(commit)
    }

    fn build_graph_menu_for(&self, commit: &graph::GraphCommit) -> Vec<MenuItem> {
        let mut groups = Vec::new();
        for reference in &commit.refs {
            let items = match reference.kind {
                graph::GraphRefKind::LocalBranch => self
                    .branches
                    .items()
                    .iter()
                    .find(|branch| branch.name == reference.name)
                    .map(|branch| self.build_branch_menu_for(branch)),
                graph::GraphRefKind::RemoteBranch => self
                    .remotes
                    .items()
                    .iter()
                    .find(|branch| branch.full_ref == reference.name)
                    .map(|branch| self.build_remote_menu_for(branch)),
                graph::GraphRefKind::Tag => self
                    .tags
                    .items()
                    .iter()
                    .find(|tag| tag.name == reference.name)
                    .map(|tag| self.build_tag_menu_for(tag)),
            };
            if let Some(items) = items {
                groups.push((reference.name.clone(), items));
            }
        }

        let has_multiple_refs = groups.len() > 1;
        groups
            .into_iter()
            .flat_map(|(reference, items)| {
                items.into_iter().map(move |mut item| {
                    item.label = format!("{reference}: {}", item.label);
                    if has_multiple_refs {
                        item.shortcut = None;
                    }
                    item
                })
            })
            .collect()
    }

    fn refresh_open_graph_menu(&mut self) {
        let Some(old_commit) = (match self.overlay.as_ref() {
            Some(Overlay::InfoModal {
                row: InfoModalRow::GraphCommit(commit),
                ..
            }) => Some(commit.clone()),
            _ => None,
        }) else {
            return;
        };
        let commit = self.graph.selected_commit().cloned().unwrap_or(old_commit);
        let items = self.build_graph_menu_for(&commit);

        let Some(Overlay::InfoModal {
            items: old_items,
            cursor,
            info_cursor,
            focus,
            row: _,
        }) = self.overlay.take()
        else {
            unreachable!();
        };
        let selected = old_items
            .get(cursor)
            .map(|item| (item.action, item.target.as_str(), item.remote.as_deref()));
        let cursor = selected
            .and_then(|(action, target, remote)| {
                items.iter().position(|item| {
                    item.action == action
                        && item.target == target
                        && item.remote.as_deref() == remote
                })
            })
            .unwrap_or(0);
        let focus = if items.is_empty() {
            InfoModalFocus::Info
        } else {
            focus
        };
        let row = InfoModalRow::GraphCommit(commit);
        let info_cursor = info_cursor.min(row.info_field_count().saturating_sub(1));
        self.overlay = Some(Overlay::InfoModal {
            items,
            cursor,
            info_cursor,
            focus,
            row,
        });
    }

    fn build_branch_menu(&self) -> Vec<MenuItem> {
        let Some(branch) = self.branches.cursor_item() else {
            return vec![];
        };
        self.build_branch_menu_for(branch)
    }

    fn build_branch_menu_for(&self, branch: &BranchInfo) -> Vec<MenuItem> {
        let has_remote = matches!(
            &branch.tracking,
            TrackingStatus::Tracked { gone: false, .. }
        );
        let tracking_remote = match &branch.tracking {
            TrackingStatus::Tracked { remote_ref, .. } => remote_ref
                .split_once('/')
                .map(|(remote, _)| remote.to_owned()),
            TrackingStatus::Local => None,
        };
        let is_ahead = branch.ahead.is_some_and(|a| a > 0);
        let is_behind = branch.behind.is_some_and(|b| b > 0);
        let has_pr = self.pr_map.contains_key(&branch.name);

        vec![
            MenuItem {
                label: "Checkout".into(),
                enabled: !branch.is_current,
                reason: if branch.is_current {
                    Some("current".into())
                } else {
                    None
                },
                shortcut: Some('c'),
                action: BranchAction::Checkout,
                target: branch.name.clone(),
                remote: None,
            },
            MenuItem {
                label: "Delete local".into(),
                enabled: !branch.is_base && !branch.is_current,
                reason: if branch.is_current {
                    Some("current".into())
                } else if branch.is_base {
                    Some("base".into())
                } else {
                    None
                },
                shortcut: Some('d'),
                action: BranchAction::DeleteLocal,
                target: branch.name.clone(),
                remote: None,
            },
            MenuItem {
                label: "Delete local + remote".into(),
                enabled: !branch.is_base && !branch.is_current && has_remote,
                reason: if branch.is_current {
                    Some("current".into())
                } else if branch.is_base {
                    Some("base".into())
                } else if !has_remote {
                    Some("no remote".into())
                } else {
                    None
                },
                shortcut: Some('D'),
                action: BranchAction::DeleteLocalAndRemote,
                target: branch.name.clone(),
                remote: tracking_remote,
            },
            MenuItem {
                label: "Force-delete local".into(),
                enabled: !branch.is_base && !branch.is_current,
                reason: if branch.is_current {
                    Some("current".into())
                } else if branch.is_base {
                    Some("base".into())
                } else {
                    None
                },
                shortcut: Some('!'),
                action: BranchAction::DeleteLocalForce,
                target: branch.name.clone(),
                remote: None,
            },
            MenuItem {
                label: "Fast-forward".into(),
                enabled: !branch.is_current && has_remote,
                reason: if branch.is_current {
                    Some("current".into())
                } else if !has_remote {
                    Some("no remote".into())
                } else {
                    None
                },
                shortcut: Some('f'),
                action: BranchAction::FastForward,
                target: branch.name.clone(),
                remote: None,
            },
            MenuItem {
                label: "Push".into(),
                enabled: is_ahead || (!has_remote && self.has_configured_remote),
                reason: if !self.has_configured_remote {
                    Some("no remote".into())
                } else if has_remote && !is_ahead {
                    Some("not ahead".into())
                } else {
                    None
                },
                shortcut: Some('p'),
                action: BranchAction::Push,
                target: branch.name.clone(),
                remote: None,
            },
            MenuItem {
                label: "Force push".into(),
                enabled: is_ahead && is_behind,
                reason: if !has_remote {
                    Some("no remote".into())
                } else if !is_ahead {
                    Some("not ahead".into())
                } else if !is_behind {
                    Some("not behind".into())
                } else {
                    None
                },
                shortcut: Some('P'),
                action: BranchAction::ForcePush,
                target: branch.name.clone(),
                remote: None,
            },
            MenuItem {
                label: "Pull".into(),
                enabled: is_behind && has_remote,
                reason: if !has_remote {
                    Some("no remote".into())
                } else if !is_behind {
                    Some("not behind".into())
                } else {
                    None
                },
                shortcut: Some('l'),
                action: BranchAction::Pull,
                target: branch.name.clone(),
                remote: None,
            },
            MenuItem {
                label: "Merge into base".into(),
                enabled: !branch.is_base && !branch.is_current,
                reason: if branch.is_current {
                    Some("current".into())
                } else if branch.is_base {
                    Some("base".into())
                } else {
                    None
                },
                shortcut: Some('m'),
                action: BranchAction::Merge,
                target: branch.name.clone(),
                remote: None,
            },
            MenuItem {
                label: "Squash merge into base".into(),
                enabled: !branch.is_base && !branch.is_current,
                reason: if branch.is_current {
                    Some("current".into())
                } else if branch.is_base {
                    Some("base".into())
                } else {
                    None
                },
                shortcut: Some('s'),
                action: BranchAction::SquashMerge,
                target: branch.name.clone(),
                remote: None,
            },
            MenuItem {
                label: "Rebase onto base".into(),
                enabled: !branch.is_base && !branch.is_current,
                reason: if branch.is_current {
                    Some("current".into())
                } else if branch.is_base {
                    Some("base".into())
                } else {
                    None
                },
                shortcut: Some('r'),
                action: BranchAction::Rebase,
                target: branch.name.clone(),
                remote: None,
            },
            MenuItem {
                label: "Create worktree".into(),
                enabled: !branch.is_current,
                reason: if branch.is_current {
                    Some("current".into())
                } else {
                    None
                },
                shortcut: Some('w'),
                action: BranchAction::Worktree,
                target: branch.name.clone(),
                remote: None,
            },
            MenuItem {
                label: "Open PR in browser".into(),
                enabled: has_pr,
                reason: if !has_pr { Some("no PR".into()) } else { None },
                shortcut: Some('o'),
                action: BranchAction::ViewRemotePR,
                target: branch.name.clone(),
                remote: None,
            },
        ]
    }

    fn build_remote_menu(&self) -> Vec<MenuItem> {
        let Some(branch) = self.remotes.cursor_item() else {
            return vec![];
        };
        self.build_remote_menu_for(branch)
    }

    fn build_remote_menu_for(&self, branch: &RemoteBranchInfo) -> Vec<MenuItem> {
        let pinned = branch.is_pinned();
        let has_local = branch.has_local;
        let has_pr = self.pr_map.contains_key(&branch.short_name);

        vec![
            MenuItem {
                label: "Checkout".into(),
                enabled: !pinned && !has_local,
                reason: if pinned {
                    Some("base".into())
                } else if has_local {
                    Some("local exists".into())
                } else {
                    None
                },
                shortcut: Some('c'),
                action: BranchAction::CheckoutRemote,
                target: branch.short_name.clone(),
                remote: Some(branch.remote.clone()),
            },
            MenuItem {
                label: "Delete remote branch".into(),
                enabled: !pinned,
                reason: if pinned { Some("base".into()) } else { None },
                shortcut: Some('d'),
                action: BranchAction::DeleteRemoteBranch,
                target: branch.short_name.clone(),
                remote: Some(branch.remote.clone()),
            },
            MenuItem {
                label: "Delete remote + local".into(),
                enabled: !pinned && has_local,
                reason: if pinned {
                    Some("base".into())
                } else if !has_local {
                    Some("no local".into())
                } else {
                    None
                },
                shortcut: Some('D'),
                action: BranchAction::DeleteRemoteAndLocal,
                target: branch.short_name.clone(),
                remote: Some(branch.remote.clone()),
            },
            MenuItem {
                label: "Fetch remote".into(),
                enabled: !pinned,
                reason: if pinned { Some("base".into()) } else { None },
                shortcut: Some('f'),
                action: BranchAction::FetchRemote,
                target: branch.short_name.clone(),
                remote: Some(branch.remote.clone()),
            },
            MenuItem {
                label: "Pull remote".into(),
                enabled: !pinned && has_local,
                reason: if pinned {
                    Some("base".into())
                } else if !has_local {
                    Some("no local".into())
                } else {
                    None
                },
                shortcut: Some('l'),
                action: BranchAction::PullRemote,
                target: branch.short_name.clone(),
                remote: Some(branch.remote.clone()),
            },
            MenuItem {
                label: "Merge into current".into(),
                enabled: !pinned,
                reason: if pinned { Some("base".into()) } else { None },
                shortcut: Some('m'),
                action: BranchAction::MergeRemoteIntoCurrent,
                target: branch.short_name.clone(),
                remote: Some(branch.remote.clone()),
            },
            MenuItem {
                label: "Cherry-pick latest".into(),
                enabled: !pinned,
                reason: if pinned { Some("base".into()) } else { None },
                shortcut: Some('p'),
                action: BranchAction::CherryPickRemote,
                target: branch.short_name.clone(),
                remote: Some(branch.remote.clone()),
            },
            MenuItem {
                label: "View PR in browser".into(),
                enabled: has_pr && !pinned,
                reason: if pinned {
                    Some("base".into())
                } else if !has_pr {
                    Some("no PR".into())
                } else {
                    None
                },
                shortcut: Some('o'),
                action: BranchAction::ViewRemotePR,
                target: branch.short_name.clone(),
                remote: Some(branch.remote.clone()),
            },
        ]
    }

    fn build_tag_menu(&self) -> Vec<MenuItem> {
        let Some(tag) = self.tags.cursor_item() else {
            return vec![];
        };
        self.build_tag_menu_for(tag)
    }

    fn build_tag_menu_for(&self, tag: &TagInfo) -> Vec<MenuItem> {
        vec![
            MenuItem {
                label: "Delete tag".into(),
                enabled: true,
                reason: None,
                shortcut: Some('d'),
                action: BranchAction::DeleteTag,
                target: tag.name.clone(),
                remote: None,
            },
            MenuItem {
                label: "Delete tag (local + remote)".into(),
                enabled: true,
                reason: None,
                shortcut: Some('D'),
                action: BranchAction::DeleteTagAndRemote,
                target: tag.name.clone(),
                remote: None,
            },
            MenuItem {
                label: "Push tag to remote".into(),
                enabled: true,
                reason: None,
                shortcut: Some('p'),
                action: BranchAction::PushTag,
                target: tag.name.clone(),
                remote: None,
            },
        ]
    }

    fn build_worktree_menu(&self) -> Vec<MenuItem> {
        let Some(wt) = self.worktrees.cursor_item() else {
            return vec![];
        };
        let is_main = wt.is_main;
        let is_dirty = !wt.wt_status.is_clean();
        let is_detached = wt.branch.is_none();
        let is_base = wt.is_base;
        let can_delete_branch = !is_main && !is_dirty && !is_detached && !is_base;
        let delete_branch_reason = if is_main {
            Some("main worktree".into())
        } else if is_dirty {
            Some("dirty".into())
        } else if is_detached {
            Some("detached HEAD".into())
        } else if is_base {
            Some("base branch".into())
        } else {
            None
        };
        let has_remote = wt.branch.as_deref().is_some_and(|name| {
            self.branches.items().iter().any(|b| {
                b.name == name && matches!(b.tracking, TrackingStatus::Tracked { gone: false, .. })
            })
        });
        let can_delete_branch_remote = can_delete_branch && has_remote;
        let delete_branch_remote_reason = delete_branch_reason
            .clone()
            .or((!has_remote).then(|| "no remote".into()));
        let target = wt.path.to_string_lossy().to_string();

        vec![
            MenuItem {
                label: "Remove worktree".into(),
                enabled: !is_main && !is_dirty,
                reason: if is_main {
                    Some("main worktree".into())
                } else if is_dirty {
                    Some("dirty".into())
                } else {
                    None
                },
                shortcut: Some('d'),
                action: BranchAction::WorktreeRemove,
                target: target.clone(),
                remote: None,
            },
            MenuItem {
                label: "Force remove worktree".into(),
                enabled: !is_main,
                reason: if is_main {
                    Some("main worktree".into())
                } else {
                    None
                },
                shortcut: Some('D'),
                action: BranchAction::WorktreeForceRemove,
                target: target.clone(),
                remote: None,
            },
            MenuItem {
                label: "Remove worktree + branch".into(),
                enabled: can_delete_branch,
                reason: delete_branch_reason.clone(),
                shortcut: Some('b'),
                action: BranchAction::WorktreeRemoveAndDeleteBranch,
                target: target.clone(),
                remote: None,
            },
            MenuItem {
                label: "Remove worktree + branch (local + remote)".into(),
                enabled: can_delete_branch_remote,
                reason: delete_branch_remote_reason,
                shortcut: Some('B'),
                action: BranchAction::WorktreeRemoveAndDeleteBranchRemote,
                target,
                remote: None,
            },
        ]
    }

    fn execute_menu_action(&mut self, item: MenuItem) {
        let action = item.action;
        // View PR -- fire and forget, no confirm
        if action == BranchAction::ViewRemotePR {
            let name = item.target;
            let repo_path = self.repo_path.clone();
            std::thread::spawn(move || {
                let _ = std::process::Command::new("gh")
                    .args(["pr", "view", "--web", &name])
                    .current_dir(&repo_path)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn();
            });
            return;
        }

        let targets = vec![item.target];
        if action == BranchAction::DeleteLocal {
            let (reason, extra_keys) = self.build_delete_preflight(&targets);
            self.open_confirm_with_reason(
                action,
                self.return_view,
                targets,
                item.remote,
                reason,
                extra_keys,
            );
        } else {
            self.open_confirm_with_reason(
                action,
                self.return_view,
                targets,
                item.remote,
                None,
                Vec::new(),
            );
        }
    }

    // ---- View-level action helpers ----

    fn delete_selected_branches(&mut self, include_remote: bool) {
        let targets = self.get_selected_branch_names();
        let action = if include_remote {
            BranchAction::DeleteLocalAndRemote
        } else {
            BranchAction::DeleteLocal
        };
        // Plan P005 §6: walk the targets for pre-flight conditions
        // (unmerged commits, checked out in a non-main worktree) so the
        // Confirm overlay can show the user *why* plain delete is risky
        // and offer a one-key recovery (`!` force-delete, `r` cascade).
        // Only meaningful for local-only deletes — `DeleteLocalAndRemote`
        // already implies force-delete semantics on the remote side.
        let (reason, extra_keys) = if include_remote {
            (None, Vec::new())
        } else {
            self.build_delete_preflight(&targets)
        };
        self.open_confirm_with_reason(
            action,
            ViewId::Branches,
            targets,
            None,
            reason,
            extra_keys,
        );
    }

    /// Build the Confirm overlay's pre-flight reason block and
    /// alternate-action keys (plan P005 §6). Returns `(None, [])` when
    /// no target triggers a pre-flight — the overlay renders the
    /// classic `[y]es [n]o` only.
    fn build_delete_preflight(
        &self,
        targets: &[String],
    ) -> (Option<String>, Vec<ConfirmExtraKey>) {
        let mut reasons: Vec<String> = Vec::new();
        let mut unmerged_targets = Vec::new();
        let mut worktree_targets = Vec::new();
        let mut force_cascade = false;

        // Derive a target width for path abbreviation from the terminal's
        // current size, mirroring confirm.rs's ~60%-of-modal-width sizing,
        // so long worktree paths in the reason block fit on one line
        // instead of relying solely on word-wrap. Falls back to a sane
        // default (80 cols) if the terminal size can't be queried.
        let term_width = crossterm::terminal::size().map(|(w, _)| w).unwrap_or(80);
        let target_width = ((term_width as usize * 60 / 100).saturating_sub(4)).max(20);

        for name in targets {
            let branch = self
                .branches
                .items()
                .iter()
                .find(|branch| branch.name == *name);
            if let Some(branch) = branch {
                match branch.merge_status {
                    MergeStatus::Unmerged => {
                        reasons.push(format!(
                            "Branch {name} has unique commits not on {}",
                            branch.base_branch
                        ));
                        unmerged_targets.push(name.clone());
                    }
                    MergeStatus::Pending => {
                        reasons.push(format!(
                            "Branch {name} merge status is still being computed; Git will verify safe deletion"
                        ));
                    }
                    _ => {}
                }
            }

            if let Some(worktree) = self
                .worktrees
                .items()
                .iter()
                .find(|worktree| worktree.branch.as_deref() == Some(name.as_str()))
            {
                let path_display = abbreviate_path(&worktree.path, target_width);
                reasons.push(format!(
                    "Branch {name} is checked out in {path_display}"
                ));
                if !worktree.is_main {
                    worktree_targets.push(name.clone());
                    let is_dirty = !worktree.wt_status.is_clean();
                    force_cascade |= is_dirty;
                    if is_dirty {
                        reasons.push(format!(
                            "Worktree {path_display} has uncommitted changes"
                        ));
                    }
                }
            }
        }

        let mut extra_keys: Vec<ConfirmExtraKey> = Vec::new();
        if !unmerged_targets.is_empty() {
            extra_keys.push(ConfirmExtraKey {
                key: '!',
                label: "force-delete".into(),
                action: BranchAction::DeleteLocalForce,
                targets: unmerged_targets.clone(),
            });
        }
        if !worktree_targets.is_empty() {
            let force = force_cascade
                || worktree_targets
                    .iter()
                    .any(|name| unmerged_targets.contains(name));
            extra_keys.push(ConfirmExtraKey {
                key: 'r',
                label: if force {
                    "force-remove worktree + delete".into()
                } else {
                    "remove worktree + delete".into()
                },
                action: if force {
                    BranchAction::DeleteBranchAndRemoveWorktreeForce
                } else {
                    BranchAction::DeleteBranchAndRemoveWorktree
                },
                targets: worktree_targets,
            });
        }

        let reason = if reasons.is_empty() {
            None
        } else {
            Some(reasons.join(", OR\n  "))
        };
        (reason, extra_keys)
    }

    fn push_selected_branches(&mut self) {
        let targets = self.get_selected_branch_names();
        self.open_confirm(BranchAction::Push, ViewId::Branches, targets);
    }

    fn delete_selected_remote_branches(&mut self) {
        let targets = list_state::collect_targets(&self.remotes, |b| {
            (!b.is_pinned()).then(|| b.short_name.clone())
        });
        self.open_confirm(BranchAction::DeleteRemoteBranch, ViewId::Remotes, targets);
    }

    fn delete_selected_tags(&mut self, include_remote: bool) {
        let targets = list_state::collect_targets(&self.tags, |t| Some(t.name.clone()));
        let action = if include_remote {
            BranchAction::DeleteTagAndRemote
        } else {
            BranchAction::DeleteTag
        };
        self.open_confirm(action, ViewId::Tags, targets);
    }

    fn push_selected_tags(&mut self) {
        let targets = list_state::collect_targets(&self.tags, |t| Some(t.name.clone()));
        self.open_confirm(BranchAction::PushTag, ViewId::Tags, targets);
    }

    fn remove_selected_worktrees(&mut self, force: bool) {
        self.worktree_enrich_rx = None;
        let targets = list_state::collect_targets(&self.worktrees, |wt| {
            (!wt.is_main).then(|| wt.path.to_string_lossy().to_string())
        });
        let action = if force {
            BranchAction::WorktreeForceRemove
        } else {
            BranchAction::WorktreeRemove
        };
        self.open_confirm(action, ViewId::Worktrees, targets);
    }

    fn get_selected_branch_names(&self) -> Vec<String> {
        list_state::collect_targets(&self.branches, |b| (!b.is_pinned()).then(|| b.name.clone()))
    }

    /// Open a confirm overlay for `action` over `targets`, returning to
    /// `return_view` when it closes. No-op when `targets` is empty.
    fn open_confirm(&mut self, action: BranchAction, return_view: ViewId, targets: Vec<String>) {
        self.open_confirm_with_remote(action, return_view, targets, None);
    }

    fn open_confirm_with_remote(
        &mut self,
        action: BranchAction,
        return_view: ViewId,
        targets: Vec<String>,
        remote: Option<String>,
    ) {
        self.open_confirm_with_reason(
            action,
            return_view,
            targets,
            remote,
            None,
            Vec::new(),
        );
    }

    /// Full pre-flight entry point: open the Confirm overlay with a
    /// pre-built reason block and alternate-action keys (plan P005 §6).
    /// Used by `delete_selected_branches` after walking the targets for
    /// unmerged commits and worktree-checked-out branches.
    #[allow(clippy::too_many_arguments)]
    fn open_confirm_with_reason(
        &mut self,
        action: BranchAction,
        return_view: ViewId,
        targets: Vec<String>,
        remote: Option<String>,
        reason: Option<String>,
        extra_keys: Vec<ConfirmExtraKey>,
    ) {
        if targets.is_empty() {
            return;
        }
        self.return_view = return_view;
        self.overlay = Some(Overlay::Confirm {
            action,
            targets,
            remote,
            reason,
            extra_keys,
        });
    }

    // ---- Sorting ----

    fn cycle_sort(&mut self) {
        match self.active_view {
            ViewId::Graph => {}
            ViewId::Branches => {
                list_state::cycle_sort_and_apply(&mut self.branches, &self.branch_columns)
            }
            ViewId::Remotes => {
                list_state::cycle_sort_and_apply(&mut self.remotes, &self.remote_columns)
            }
            ViewId::Tags => list_state::cycle_sort_and_apply(&mut self.tags, &self.tag_columns),
            ViewId::Worktrees => {
                list_state::cycle_sort_and_apply(&mut self.worktrees, &self.worktree_columns)
            }
        }
        self.save_sort_config();
    }

    fn toggle_sort_direction(&mut self) {
        match self.active_view {
            ViewId::Graph => {}
            ViewId::Branches => list_state::toggle_sort_direction_and_apply(
                &mut self.branches,
                &self.branch_columns,
            ),
            ViewId::Remotes => {
                list_state::toggle_sort_direction_and_apply(&mut self.remotes, &self.remote_columns)
            }
            ViewId::Tags => {
                list_state::toggle_sort_direction_and_apply(&mut self.tags, &self.tag_columns)
            }
            ViewId::Worktrees => list_state::toggle_sort_direction_and_apply(
                &mut self.worktrees,
                &self.worktree_columns,
            ),
        }
        self.save_sort_config();
    }

    // ---- View Loading ----

    fn ensure_view_loaded(&mut self) {
        match self.active_view {
            ViewId::Graph if self.graph.snapshot().is_none() && !self.graph.is_loading() => {
                self.spawn_graph_load(self.graph.max_count(), self.graph.includes_remotes());
            }
            ViewId::Branches
                if self.worktrees.items().is_empty()
                    && !self.worktrees.loading
                    && self.config.load_worktrees_on_launch != Some(false) =>
            {
                // Branch-delete pre-flight needs worktree ownership and
                // working-tree status. Load it lazily when Branches becomes
                // visible, while preserving the opt-out for large repos.
                self.spawn_worktree_load();
            }
            ViewId::Tags if self.tags.items().is_empty() && !self.tags.loading => {
                self.spawn_tag_load();
            }
            ViewId::Remotes if self.remotes.items().is_empty() && !self.remotes.loading => {
                self.spawn_remote_load();
                // Trigger fetch if not yet done this session
                if !self.remote_fetched && self.config.auto_fetch == Some(true) {
                    self.start_remote_fetch();
                }
            }
            ViewId::Worktrees if self.worktrees.items().is_empty() && !self.worktrees.loading => {
                self.spawn_worktree_load();
            }
            _ => {}
        }
    }

    /// Preload the authoritative rows needed to adapt Graph refs into the
    /// existing Remote and Tag action menus. Branch rows already stream at
    /// launch through `phase1_rx`.
    pub fn preload_graph_action_metadata(&mut self) {
        if self.remotes.items().is_empty() && !self.remotes.loading {
            self.spawn_remote_load();
        }
        if self.tags.items().is_empty() && !self.tags.loading {
            self.spawn_tag_load();
        }
    }

    pub fn spawn_graph_load(&mut self, max_count: usize, include_remotes: bool) {
        self.graph_generation = self.graph_generation.saturating_add(1);
        // Drop any in-flight enrichment from a previous load. The
        // background thread will see the dropped receiver and exit on its
        // next `tx.send`; no stale update can be applied to the new
        // snapshot via the generation check.
        self.graph_enrich_rx = None;
        self.graph.begin_load(max_count, include_remotes);
        self.graph_rx = Some(graph::spawn_graph_loader(
            self.repo_path.clone(),
            graph::GraphLoadOptions {
                max_count,
                include_remotes,
                line_style: graph::GraphLineStyle::from_symbol_name(self.symbols.name),
                base_branch: Some(self.base_branch.clone()),
            },
        ));
        self.toast = Some(Toast::new("Loading graph...".into(), 300));
    }

    fn reload_graph_for_symbol_change(&mut self) {
        if self.graph.snapshot().is_some() || self.graph.is_loading() {
            self.spawn_graph_load(self.graph.max_count(), self.graph.includes_remotes());
        }
    }

    fn reload_graph(&mut self) {
        self.spawn_graph_load(self.graph.max_count(), self.graph.includes_remotes());
    }

    fn reload_graph_with_remotes(&mut self, include_remotes: bool) {
        self.spawn_graph_load(self.graph.max_count(), include_remotes);
    }

    fn apply_graph_options(&mut self, cursor: usize, include_remotes: bool) {
        if cursor == 0 {
            self.reload_graph_with_remotes(include_remotes);
        } else {
            self.graph.set_include_remotes(include_remotes);
            self.load_older_graph();
        }
    }

    fn load_older_graph(&mut self) {
        let max_count = self.graph.load_older_history();
        self.spawn_graph_load(max_count, self.graph.includes_remotes());
    }

    fn open_graph_options(&mut self) {
        self.return_view = ViewId::Graph;
        self.overlay = Some(Overlay::GraphOptions {
            cursor: 0,
            include_remotes: self.graph.includes_remotes(),
        });
    }

    fn spawn_tag_load(&mut self) {
        self.tags.loading = true;
        let repo_path = self.repo_path.clone();
        let (tx, rx) = mpsc::channel();
        self.tag_load_rx = Some(rx);
        self.toast = Some(Toast::new("Loading tags...".into(), 300));
        std::thread::spawn(move || {
            if let Ok(repo) = git2::Repository::open(&repo_path) {
                let tag_list = tags::list_tags(&repo);
                let _ = tx.send(tag_list);
            }
        });
    }

    fn spawn_remote_load(&mut self) {
        self.remotes.loading = true;
        let repo_path = self.repo_path.clone();
        let base_branch = self.base_branch.clone();
        let (tx, rx) = mpsc::channel();
        self.remote_load_rx = Some(rx);
        self.toast = Some(Toast::new("Loading remote branches...".into(), 300));
        std::thread::spawn(move || {
            let Ok(repo) = git2::Repository::open(&repo_path) else {
                return;
            };
            let Ok(remote_branches) = branch::list_remote_branches_phase1(&repo, &base_branch)
            else {
                return;
            };

            let branch_cache = cache::BranchCache::load(&repo_path);
            // Remote branches don't precompute a merge base, so the merge-base slot is
            // None and is_squash_merged falls back to `git merge-base` for them.
            let candidates: Vec<(String, String, Option<String>)> = remote_branches
                .iter()
                .filter(|b| b.merge_status == MergeStatus::Pending && !b.is_base)
                .filter_map(|b| {
                    let refname = format!("refs/remotes/{}", b.full_ref);
                    repo.find_reference(&refname)
                        .ok()
                        .and_then(|r| r.peel_to_commit().ok())
                        .map(|c| (b.full_ref.clone(), c.id().to_string(), None))
                })
                .collect();

            let _ = tx.send((remote_branches, candidates, branch_cache));
        });
    }

    /// Re-correlate each loaded worktree's merge status from the current branch
    /// list. Cheap linear scan, no I/O. Called whenever either the worktree set
    /// or branch merge statuses change, since the two load on separate channels
    /// and can arrive in any order.
    fn refresh_worktree_merge_status(&mut self) {
        worktree::apply_branch_merge_status(self.worktrees.items_mut(), self.branches.items());
    }

    fn spawn_worktree_load(&mut self) {
        self.worktrees.loading = true;
        let repo_path = self.repo_path.clone();
        let (tx, rx) = mpsc::channel();
        self.worktree_load_rx = Some(rx);
        self.toast = Some(Toast::new("Loading worktrees...".into(), 300));
        std::thread::spawn(move || {
            let wts = worktree::list_worktrees(&repo_path);
            let _ = tx.send(wts);
        });
    }

    fn start_remote_fetch(&mut self) {
        let repo_path = self.repo_path.clone();
        let (tx, rx) = mpsc::channel();
        self.remote_fetch_rx = Some(rx);
        self.toast = Some(Toast::new("Fetching remote branches...".into(), 300));
        std::thread::spawn(move || {
            let ok = operations::fetch_sync(&repo_path);
            let _ = tx.send(ok);
        });
    }

    pub fn refresh_branches(&mut self, trigger: &str) {
        let repo_path = self.repo_path.clone();
        let base_branch = self.base_branch.clone();

        let Ok(repo) = git2::Repository::open(&repo_path) else {
            return;
        };

        self.has_configured_remote = repo.remotes().map(|r| !r.is_empty()).unwrap_or(false);

        let fingerprint = branch_input_fingerprint(&repo, &base_branch);
        let inputs_changed = self.last_branch_fingerprint != Some(fingerprint);
        self.last_branch_fingerprint = Some(fingerprint);
        let _span = tracing::info_span!(
            "branch_load",
            trigger = %trigger,
            path = "sync_full",
            inputs_changed = inputs_changed,
        )
        .entered();

        let Ok(branches) = branch::list_branches_phase1(&repo, &base_branch) else {
            return;
        };

        let new_cache = cache::BranchCache::load(&repo_path);
        let cache_for_cherry = cache::BranchCache::load(&repo_path);

        // list_branches_phase1 already filled merge bases; skip disjoint branches
        // (no merge base) and carry the precomputed merge base into the squash check.
        let candidates: Vec<(String, String, Option<String>)> = branches
            .iter()
            .filter(|b| {
                b.merge_status == MergeStatus::Pending
                    && !b.is_base
                    && !b.is_current
                    && b.merge_base_commit.is_some()
            })
            .filter_map(|b| {
                branch::get_commit_hash(&repo, &b.name)
                    .map(|hash| (b.name.clone(), hash, b.merge_base_commit.clone()))
            })
            .collect();

        self.branches.set_items(branches);

        // Restore sort
        list_state::apply_sort(&mut self.branches, &self.branch_columns);

        // Spawn squash checker
        self.squash_total = candidates.len();
        self.squash_checked = 0;
        let cherry_candidates = candidates.clone();
        if !candidates.is_empty() {
            self.squash_rx = Some(squash_loader::spawn_squash_checker(
                repo_path.clone(),
                base_branch.clone(),
                candidates,
                new_cache,
            ));
        }

        // Spawn cherry-pick checker
        self.cherry_total = cherry_candidates.len();
        self.cherry_checked = 0;
        if !cherry_candidates.is_empty() {
            self.cherry_rx = Some(cherry_loader::spawn_cherry_checker(
                repo_path.clone(),
                base_branch.clone(),
                cherry_candidates,
                cache_for_cherry,
            ));
        }

        // Re-spawn PR loader
        self.pr_rx = Some(pr_loader::spawn_pr_loader(repo_path));
    }

    fn refresh_after_operation(&mut self) {
        match self.return_view {
            ViewId::Graph => {
                self.reload_graph();
                self.active_view = ViewId::Graph;
            }
            ViewId::Branches => {
                self.refresh_branches("post_operation");
                self.active_view = ViewId::Branches;
            }
            ViewId::Remotes => {
                // Reload remote branches (spawn_remote_load sets loading=true;
                // do not reset ListState here or sort_column/sort_ascending is lost)
                self.spawn_remote_load();
                self.active_view = ViewId::Remotes;
            }
            ViewId::Tags => {
                self.spawn_tag_load();
                self.active_view = ViewId::Tags;
            }
            ViewId::Worktrees => {
                self.spawn_worktree_load();
                self.active_view = ViewId::Worktrees;
            }
        }
    }

    /// Like `refresh_after_operation`, but does not snap `active_view` back to
    /// the completed job's originating view. Since job-queue execution is
    /// non-modal, the user may have navigated elsewhere while it ran, and
    /// forcing them back to `view` when it finishes would be a jarring
    /// regression (this is safe for `refresh_after_operation`'s other callers
    /// only because those flows are still fully modal).
    fn refresh_view_data(&mut self, view: ViewId) {
        match view {
            ViewId::Graph => self.reload_graph(),
            ViewId::Branches => self.refresh_branches("post_operation"),
            ViewId::Remotes => self.spawn_remote_load(),
            ViewId::Tags => self.spawn_tag_load(),
            ViewId::Worktrees => self.spawn_worktree_load(),
        }
    }

    fn refresh_after_job(&mut self, action: BranchAction, origin: ViewId) {
        for view in graph_affected_views(action) {
            self.refresh_view_data(*view);
        }
        if origin != ViewId::Graph && !graph_affected_views(action).contains(&origin) {
            self.refresh_view_data(origin);
        }
        if origin == ViewId::Graph {
            self.refresh_view_data(ViewId::Graph);
        }
    }

    fn start_fetch(&mut self, prune: bool) {
        let repo_path = self.repo_path.clone();
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_clone = Arc::clone(&cancel);
        self.op_rx = Some(rx);
        self.cancel_flag = Some(cancel);
        self.return_view = self.active_view;
        let label = if prune {
            "Fetching with prune..."
        } else {
            "Fetching..."
        };
        self.overlay = Some(Overlay::Executing {
            label: label.into(),
            progress: None,
        });
        std::thread::spawn(move || {
            let result = if prune {
                operations::fetch_prune(&repo_path, &cancel_clone)
            } else {
                operations::fetch(&repo_path, &cancel_clone)
            };
            let _ = tx.send(vec![result]);
        });
    }

    fn clear_cache_and_refresh(&mut self) {
        let mut bc = cache::BranchCache::load(&self.repo_path);
        bc.clear();
        self.refresh_branches("manual_refresh_R");
        self.toast = Some(Toast::new("Cache cleared".into(), 3));
    }

    // ---- Diagnostics ----

    /// Run the cache-accuracy audit on a background thread, recomputing every
    /// cached value fresh and diffing it against the on-disk cache. Progress is
    /// streamed to the Executing overlay; the result lands in `diag_rx`.
    fn run_cache_audit(&mut self) {
        let repo_path = self.repo_path.clone();
        let base_branch = self.base_branch.clone();

        let (diag_tx, diag_rx) = mpsc::channel();
        let (prog_tx, prog_rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_clone = Arc::clone(&cancel);

        self.overlay = Some(Overlay::Executing {
            label: "Verifying cache accuracy...".into(),
            progress: None,
        });
        self.diag_rx = Some(diag_rx);
        self.progress_rx = Some(prog_rx);
        self.cancel_flag = Some(cancel);

        std::thread::spawn(move || {
            let Ok(repo) = git2::Repository::open(&repo_path) else {
                return;
            };
            let branch_cache = cache::BranchCache::load(&repo_path);
            let audit = diagnostics::audit_cache(
                &repo,
                &repo_path,
                &base_branch,
                &branch_cache,
                &cancel_clone,
                |completed, total, item| {
                    let _ = prog_tx.send(ProgressUpdate {
                        completed,
                        total,
                        current_item: item.to_string(),
                    });
                },
            );
            let _ = diag_tx.send(audit);
        });
    }

    /// Write the audit's freshly-computed corrections back to the cache and
    /// reload the view so the screen reflects the corrected data.
    fn apply_cache_fix(&mut self, audit: CacheAudit) {
        let mut branch_cache = cache::BranchCache::load(&self.repo_path);
        diagnostics::apply_fix(&mut branch_cache, &audit);
        self.toast = Some(Toast::new("Cache corrected".into(), 3));
        self.refresh_after_operation();
    }

    // ---- Config ----

    fn save_config(&mut self) {
        self.config.theme = Some(self.theme.name.to_string());
        self.config.symbols = Some(self.symbols.name.to_string());
        self.config.include_remotes = Some(self.graph.includes_remotes());
        self.save_sort_config_only();
        self.config.save();
    }

    fn save_sort_config(&mut self) {
        self.save_sort_config_only();
        self.config.save();
    }

    fn save_sort_config_only(&mut self) {
        self.config.sort_column_branches = self
            .branches
            .sort_column()
            .and_then(|i| sort_keys::key_for_index(&self.branch_columns, i))
            .map(str::to_string);
        self.config.sort_asc_branches = Some(self.branches.sort_ascending());
        self.config.sort_column_remotes = self
            .remotes
            .sort_column()
            .and_then(|i| sort_keys::key_for_index(&self.remote_columns, i))
            .map(str::to_string);
        self.config.sort_asc_remotes = Some(self.remotes.sort_ascending());
        self.config.sort_column_tags = self
            .tags
            .sort_column()
            .and_then(|i| sort_keys::key_for_index(&self.tag_columns, i))
            .map(str::to_string);
        self.config.sort_asc_tags = Some(self.tags.sort_ascending());
        self.config.sort_column_worktrees = self
            .worktrees
            .sort_column()
            .and_then(|i| sort_keys::key_for_index(&self.worktree_columns, i))
            .map(str::to_string);
        self.config.sort_asc_worktrees = Some(self.worktrees.sort_ascending());
    }

    /// Cycle the given view's sort through its cycle of states. `forward` selects
    /// direction of the cycle (Right/Left in settings).
    fn cycle_view_sort(&mut self, view: ViewId, forward: bool) {
        match view {
            ViewId::Graph => {}
            ViewId::Branches => {
                let pairs = sort_keys::sort_state_cycle(&self.branch_columns);
                let current = (self.branches.sort_column(), self.branches.sort_ascending());
                let cur_pos = pairs.iter().position(|&p| p == current).unwrap_or(0);
                let len = pairs.len();
                let next_pos = if forward {
                    (cur_pos + 1) % len
                } else {
                    (cur_pos + len - 1) % len
                };
                let (col, asc) = pairs[next_pos];
                self.branches.set_sort(col, asc);
                list_state::apply_sort(&mut self.branches, &self.branch_columns);
            }
            ViewId::Remotes => {
                let pairs = sort_keys::sort_state_cycle(&self.remote_columns);
                let current = (self.remotes.sort_column(), self.remotes.sort_ascending());
                let cur_pos = pairs.iter().position(|&p| p == current).unwrap_or(0);
                let len = pairs.len();
                let next_pos = if forward {
                    (cur_pos + 1) % len
                } else {
                    (cur_pos + len - 1) % len
                };
                let (col, asc) = pairs[next_pos];
                self.remotes.set_sort(col, asc);
                list_state::apply_sort(&mut self.remotes, &self.remote_columns);
            }
            ViewId::Tags => {
                let pairs = sort_keys::sort_state_cycle(&self.tag_columns);
                let current = (self.tags.sort_column(), self.tags.sort_ascending());
                let cur_pos = pairs.iter().position(|&p| p == current).unwrap_or(0);
                let len = pairs.len();
                let next_pos = if forward {
                    (cur_pos + 1) % len
                } else {
                    (cur_pos + len - 1) % len
                };
                let (col, asc) = pairs[next_pos];
                self.tags.set_sort(col, asc);
                list_state::apply_sort(&mut self.tags, &self.tag_columns);
            }
            ViewId::Worktrees => {
                let pairs = sort_keys::sort_state_cycle(&self.worktree_columns);
                let current = (
                    self.worktrees.sort_column(),
                    self.worktrees.sort_ascending(),
                );
                let cur_pos = pairs.iter().position(|&p| p == current).unwrap_or(0);
                let len = pairs.len();
                let next_pos = if forward {
                    (cur_pos + 1) % len
                } else {
                    (cur_pos + len - 1) % len
                };
                let (col, asc) = pairs[next_pos];
                self.worktrees.set_sort(col, asc);
                list_state::apply_sort(&mut self.worktrees, &self.worktree_columns);
            }
        }
    }

    // ---- Filter helpers ----

    fn active_filter_query(&self) -> String {
        match self.active_view {
            ViewId::Graph => String::new(),
            ViewId::Branches => self.branches.filter_query().to_string(),
            ViewId::Remotes => self.remotes.filter_query().to_string(),
            ViewId::Tags => self.tags.filter_query().to_string(),
            ViewId::Worktrees => self.worktrees.filter_query().to_string(),
        }
    }

    fn set_active_filter(&mut self, query: String) {
        match self.active_view {
            ViewId::Graph => {}
            ViewId::Branches => self.branches.set_filter_query(query),
            ViewId::Remotes => self.remotes.set_filter_query(query),
            ViewId::Tags => self.tags.set_filter_query(query),
            ViewId::Worktrees => self.worktrees.set_filter_query(query),
        }
    }

    fn clear_toast(&mut self) {
        self.toast = None;
    }
}

fn graph_affected_views(action: BranchAction) -> &'static [ViewId] {
    match action {
        BranchAction::DeleteLocal => &[ViewId::Branches, ViewId::Remotes],
        BranchAction::DeleteLocalAndRemote => &[ViewId::Branches, ViewId::Remotes],
        BranchAction::Checkout
        | BranchAction::FastForward
        | BranchAction::Merge
        | BranchAction::SquashMerge
        | BranchAction::Rebase => &[ViewId::Branches, ViewId::Worktrees],
        BranchAction::Fetch | BranchAction::FetchPrune => {
            &[ViewId::Branches, ViewId::Remotes, ViewId::Tags]
        }
        BranchAction::Push | BranchAction::ForcePush => &[ViewId::Branches, ViewId::Remotes],
        BranchAction::Pull => &[ViewId::Branches, ViewId::Remotes, ViewId::Worktrees],
        BranchAction::Worktree => &[ViewId::Worktrees],
        BranchAction::DeleteTag | BranchAction::DeleteTagAndRemote | BranchAction::PushTag => {
            &[ViewId::Tags]
        }
        BranchAction::DeleteRemoteBranch => &[ViewId::Branches, ViewId::Remotes],
        BranchAction::DeleteRemoteAndLocal => &[ViewId::Branches, ViewId::Remotes],
        BranchAction::CheckoutRemote => &[ViewId::Branches, ViewId::Remotes, ViewId::Worktrees],
        BranchAction::FetchRemote => &[ViewId::Branches, ViewId::Remotes, ViewId::Tags],
        BranchAction::PullRemote
        | BranchAction::MergeRemoteIntoCurrent
        | BranchAction::CherryPickRemote => &[ViewId::Branches, ViewId::Remotes, ViewId::Worktrees],
        BranchAction::WorktreeRemove | BranchAction::WorktreeForceRemove => &[ViewId::Worktrees],
        BranchAction::WorktreeRemoveAndDeleteBranch => {
            &[ViewId::Branches, ViewId::Remotes, ViewId::Worktrees]
        }
        BranchAction::WorktreeRemoveAndDeleteBranchRemote => {
            &[ViewId::Branches, ViewId::Remotes, ViewId::Worktrees]
        }
        // P005 #067: new cascade-by-name variants. They are reached from the
        // Branches view's Confirm/Results overlay (`!`/`r` recovery keys and
        // the menu), and touch worktrees; the Remote variant also needs the
        // Remotes view so menu discovery stays consistent with the
        // path-based siblings above.
        BranchAction::DeleteLocalForce => &[ViewId::Branches, ViewId::Remotes],
        BranchAction::DeleteBranchAndRemoveWorktree
        | BranchAction::DeleteBranchAndRemoveWorktreeForce => {
            &[ViewId::Branches, ViewId::Remotes, ViewId::Worktrees]
        }
        BranchAction::DeleteBranchAndRemoveWorktreeRemote => {
            &[ViewId::Branches, ViewId::Remotes, ViewId::Worktrees]
        }
        BranchAction::ViewRemotePR => &[],
    }
}

/// Get branch prefix style: extract prefix before first '/' and look up color.
fn branch_prefix_style(name: &str, theme: &Theme) -> Style {
    let prefix = name.split('/').next().unwrap_or(name);
    prefix_style(prefix, theme).unwrap_or_default()
}

fn visible_data_col_width(
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

fn age_text_for_column(
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
            1 => {
                let (text, style) = match &item.tracking {
                    TrackingStatus::Tracked { remote_ref, gone } => {
                        if *gone {
                            ("gone".to_string(), theme.secondary_text)
                        } else {
                            (remote_ref.clone(), theme.secondary_text)
                        }
                    }
                    TrackingStatus::Local => ("local".to_string(), theme.secondary_text),
                };
                lines.push(Line::from(Span::styled(text, style)));
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
            1 => {
                // Local indicator: checkmark symbol when local branch exists
                let text = if item.has_local {
                    symbols.status_merged.to_string()
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone, Utc};
    use std::process::Command;

    fn worktree(branch: &str) -> WorktreeInfo {
        WorktreeInfo {
            path: PathBuf::from("/repo/.worktrees/example"),
            branch: Some(branch.to_string()),
            is_main: false,
            is_base: false,
            commit_hash: "abc1234".into(),
            wt_status: WorkingTreeStatus::clean(),
            age_date: Utc::now(),
            merge_status: MergeStatus::Unmerged,
            ahead: None,
            behind: None,
            pr: None,
        }
    }

    fn remote_branch() -> RemoteBranchInfo {
        RemoteBranchInfo {
            full_ref: "origin/feature/remote-age".into(),
            remote: "origin".into(),
            short_name: "feature/remote-age".into(),
            has_local: false,
            is_base: false,
            last_commit_date: Utc::now() - Duration::minutes(5),
            merge_status: MergeStatus::Unmerged,
            ahead: None,
            behind: None,
            disjoint: false,
            pr: None,
        }
    }

    fn branch(name: &str, tracking: TrackingStatus) -> BranchInfo {
        BranchInfo {
            name: name.into(),
            is_current: false,
            is_base: false,
            tracking,
            ahead: None,
            behind: None,
            last_commit_date: Utc::now(),
            merge_status: MergeStatus::Unmerged,
            base_branch: "main".into(),
            merge_base_commit: None,
            pr: None,
        }
    }

    fn remote(full_ref: &str, short_name: &str) -> RemoteBranchInfo {
        RemoteBranchInfo {
            full_ref: full_ref.into(),
            remote: full_ref.split_once('/').unwrap().0.into(),
            short_name: short_name.into(),
            has_local: false,
            is_base: false,
            last_commit_date: Utc::now(),
            merge_status: MergeStatus::Unmerged,
            ahead: None,
            behind: None,
            disjoint: false,
            pr: None,
        }
    }

    fn tag(name: &str) -> TagInfo {
        TagInfo {
            name: name.into(),
            commit_hash: "1111111".into(),
            date: Utc::now(),
            message: None,
            is_annotated: false,
        }
    }

    fn graph_snapshot(refs: Vec<graph::GraphRef>) -> graph::GraphSnapshot {
        graph::GraphSnapshot {
            source: graph::GraphSource::Gleisbau,
            commits: vec![graph::GraphCommit {
                oid: "1111111111111111111111111111111111111111".into(),
                summary: "selected commit".into(),
                parents: vec!["0000000000000000000000000000000000000000".into()],
                lane: Some(0),
                branch: None,
                refs,
                is_possible_squash_merge: false,
                fuzzy_squash_match: None,
                ..graph::GraphCommit::default()
            }],
            lines: vec![graph::GraphLine {
                graph: "*".into(),
                commit_index: Some(0),
            }],
            ref_counts: Default::default(),
            max_count: 500,
            includes_remotes: true,
            generation: None,
        }
    }

    fn graph_ref(name: &str, kind: graph::GraphRefKind) -> graph::GraphRef {
        graph::GraphRef {
            name: name.into(),
            kind,
            has_linked_worktree: false,
            tracking: None,
        }
    }

    fn graph_app(refs: Vec<graph::GraphRef>) -> App {
        let tmpdir = tempfile::tempdir().expect("temp repo");
        let mut app = App::new(
            tmpdir.path().to_path_buf(),
            "main".into(),
            Config::default(),
        );
        app.active_view = ViewId::Graph;
        app.graph.apply_result(Ok(graph_snapshot(refs)));
        app
    }

    fn info_modal_items(app: &App) -> &[MenuItem] {
        match app.overlay.as_ref() {
            Some(Overlay::InfoModal { items, .. }) => items,
            other => panic!("expected info modal, got {other:?}"),
        }
    }

    fn cell_text(line: &Line<'static>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn branch_row_renders_base_info_and_full_remote_ref() {
        let theme = Theme::dark();
        let symbols = SymbolSet::ascii();
        let ctx = CellContext {
            theme: &theme,
            symbols: &symbols,
            area_width: 120,
            compact: false,
            data_col_widths: vec![40, 28],
            first_col_width: 40,
        };
        let item = BranchInfo {
            name: "feature/test".into(),
            is_current: false,
            is_base: false,
            tracking: TrackingStatus::Tracked {
                remote_ref: "origin/feature/test".into(),
                gone: false,
            },
            ahead: None,
            behind: None,
            last_commit_date: Utc::now(),
            merge_status: MergeStatus::Unmerged,
            base_branch: "main".into(),
            merge_base_commit: Some("ac13ef04".into()),
            pr: None,
        };

        let rows = render_branch_row(&item, 0, false, false, &[0, 1], &ctx);

        assert_eq!(cell_text(&rows[0]), "feature/test (main - ac13ef04)");
        assert_eq!(cell_text(&rows[1]), "origin/feature/test");
    }

    #[test]
    fn graph_options_overlay_applies_remote_refs() {
        let tmpdir = tempfile::tempdir().expect("temp repo");
        let mut app = App::new(
            tmpdir.path().to_path_buf(),
            "main".into(),
            Config::default(),
        );
        app.active_view = ViewId::Graph;

        app.handle_key(KeyEvent::new(
            KeyCode::Char('o'),
            crossterm::event::KeyModifiers::NONE,
        ));
        assert!(matches!(
            app.overlay,
            Some(Overlay::GraphOptions {
                cursor: 0,
                include_remotes: false
            })
        ));

        app.handle_overlay_key(KeyEvent::new(
            KeyCode::Char(' '),
            crossterm::event::KeyModifiers::NONE,
        ));
        app.apply_graph_options(0, true);
        assert!(app.graph.includes_remotes());
        assert!(app.graph.is_loading());
        assert!(app.graph_rx.is_some());
    }

    #[test]
    fn app_starts_on_graph_tab() {
        let tmpdir = tempfile::tempdir().expect("temp repo");
        let app = App::new(
            tmpdir.path().to_path_buf(),
            "main".into(),
            Config::default(),
        );

        assert_eq!(app.active_view, ViewId::Graph);
    }

    #[test]
    fn graph_load_older_control_reloads_with_next_page() {
        let tmpdir = tempfile::tempdir().expect("temp repo");
        let mut app = App::new(
            tmpdir.path().to_path_buf(),
            "main".into(),
            Config::default(),
        );
        app.active_view = ViewId::Graph;
        assert_eq!(app.graph.max_count(), 500);

        app.handle_graph_key(KeyEvent::new(
            KeyCode::Char('L'),
            crossterm::event::KeyModifiers::NONE,
        ));
        assert_eq!(app.graph.max_count(), 1000);
        assert!(app.graph.is_loading());
        assert!(app.graph_rx.is_some());
    }

    #[test]
    fn app_restores_graph_remote_ref_preference_from_config() {
        let tmpdir = tempfile::tempdir().expect("temp repo");
        let config: Config = toml::from_str("include_remotes = true\n").unwrap();

        let app = App::new(tmpdir.path().to_path_buf(), "main".into(), config);

        assert!(app.graph.includes_remotes());
    }

    #[test]
    fn graph_result_clears_loading_toast() {
        let tmpdir = tempfile::tempdir().expect("temp repo");
        let mut app = App::new(
            tmpdir.path().to_path_buf(),
            "main".into(),
            Config::default(),
        );
        app.graph.begin_load(500, false);
        app.toast = Some(Toast::new("Loading graph...".into(), 300));

        let (tx, rx) = mpsc::channel();
        tx.send(Ok(graph::GraphSnapshot {
            source: graph::GraphSource::Gleisbau,
            commits: vec![],
            lines: vec![],
            ref_counts: Default::default(),
            max_count: 500,
            includes_remotes: false,
            generation: None,
        }))
        .expect("send graph result");
        app.graph_rx = Some(rx);

        app.drain_channels();

        assert!(!app.graph.is_loading());
        assert!(app.toast.is_none());
    }

    #[test]
    fn graph_navigation_and_horizontal_scroll_keys_bypass_generic_table_handler() {
        let tmpdir = tempfile::tempdir().expect("temp repo");
        let mut app = App::new(
            tmpdir.path().to_path_buf(),
            "main".into(),
            Config::default(),
        );
        app.active_view = ViewId::Graph;
        app.graph.apply_result(Ok(graph::GraphSnapshot {
            source: graph::GraphSource::Gleisbau,
            commits: vec![
                graph::GraphCommit {
                    oid: "1111111111111111111111111111111111111111".into(),
                    summary: "first".into(),
                    parents: vec![],
                    lane: Some(0),
                    branch: None,
                    refs: vec![],
                    is_possible_squash_merge: false,
                    fuzzy_squash_match: None,
                    ..graph::GraphCommit::default()
                },
                graph::GraphCommit {
                    oid: "2222222222222222222222222222222222222222".into(),
                    summary: "second".into(),
                    parents: vec![],
                    lane: Some(0),
                    branch: None,
                    refs: vec![],
                    is_possible_squash_merge: false,
                    fuzzy_squash_match: None,
                    ..graph::GraphCommit::default()
                },
            ],
            lines: vec![
                graph::GraphLine {
                    graph: "*".into(),
                    commit_index: Some(0),
                },
                graph::GraphLine {
                    graph: "*".into(),
                    commit_index: Some(1),
                },
            ],
            ref_counts: Default::default(),
            max_count: 500,
            includes_remotes: false,
            generation: None,
        }));

        app.handle_key(KeyEvent::new(
            KeyCode::Char('j'),
            crossterm::event::KeyModifiers::NONE,
        ));
        assert_eq!(app.graph.commit_cursor(), 1);
        app.handle_key(KeyEvent::new(
            KeyCode::Char('l'),
            crossterm::event::KeyModifiers::NONE,
        ));
        assert_eq!(app.graph.commit_cursor(), 1);
        assert_eq!(app.graph.horizontal_offset(), 1);
        app.handle_key(KeyEvent::new(
            KeyCode::Left,
            crossterm::event::KeyModifiers::NONE,
        ));
        assert_eq!(app.graph.horizontal_offset(), 0);
        app.handle_key(KeyEvent::new(
            KeyCode::Right,
            crossterm::event::KeyModifiers::NONE,
        ));
        assert_eq!(app.graph.horizontal_offset(), 1);
        assert_eq!(app.graph.commit_cursor(), 1);
        app.handle_key(KeyEvent::new(
            KeyCode::Tab,
            crossterm::event::KeyModifiers::NONE,
        ));
        assert_eq!(app.active_view, ViewId::Branches);
    }

    #[test]
    fn tab_cycles_through_all_five_views_forward_and_reverse() {
        let tmpdir = tempfile::tempdir().expect("temp repo");
        let mut app = App::new(
            tmpdir.path().to_path_buf(),
            "main".into(),
            Config::default(),
        );
        assert_eq!(app.active_view, ViewId::Graph);

        // Forward: Graph -> Branches -> Remotes -> Tags -> Worktrees -> Graph
        let forward = [
            ViewId::Branches,
            ViewId::Remotes,
            ViewId::Tags,
            ViewId::Worktrees,
            ViewId::Graph,
        ];
        for expected in forward {
            app.handle_key(KeyEvent::new(
                KeyCode::Tab,
                crossterm::event::KeyModifiers::NONE,
            ));
            assert_eq!(app.active_view, expected);
        }

        // Reverse: Shift-Tab (Worktrees -> Tags -> Remotes -> Branches -> Graph)
        app.active_view = ViewId::Graph;
        let reverse_shift = [
            ViewId::Worktrees,
            ViewId::Tags,
            ViewId::Remotes,
            ViewId::Branches,
            ViewId::Graph,
        ];
        for expected in reverse_shift {
            app.handle_key(KeyEvent::new(
                KeyCode::Tab,
                crossterm::event::KeyModifiers::SHIFT,
            ));
            assert_eq!(app.active_view, expected);
        }

        // Reverse: BackTab (same direction as Shift-Tab)
        app.active_view = ViewId::Graph;
        let reverse_backtab = [
            ViewId::Worktrees,
            ViewId::Tags,
            ViewId::Remotes,
            ViewId::Branches,
            ViewId::Graph,
        ];
        for expected in reverse_backtab {
            app.handle_key(KeyEvent::new(
                KeyCode::BackTab,
                crossterm::event::KeyModifiers::NONE,
            ));
            assert_eq!(app.active_view, expected);
        }
    }

    #[test]
    fn existing_four_tab_navigation_still_works_after_graph_tab_added() {
        let tmpdir = tempfile::tempdir().expect("temp repo");
        let mut app = App::new(
            tmpdir.path().to_path_buf(),
            "main".into(),
            Config::default(),
        );

        // Branches: cursor moves with j/k; Enter opens a context menu
        app.active_view = ViewId::Branches;
        app.branches.set_items(vec![branch("feature/a", TrackingStatus::Local)]);
        app.handle_key(KeyEvent::new(
            KeyCode::Char('j'),
            crossterm::event::KeyModifiers::NONE,
        ));
        app.handle_key(KeyEvent::new(
            KeyCode::Char('k'),
            crossterm::event::KeyModifiers::NONE,
        ));
        app.handle_key(KeyEvent::new(
            KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        ));
        assert!(matches!(app.overlay, Some(Overlay::InfoModal { .. })));
        app.overlay = None;

        // Remotes: cursor moves with j/k; Enter opens a context menu
        app.active_view = ViewId::Remotes;
        app.remotes.set_items(vec![remote("origin/feature/b", "feature/b")]);
        app.handle_key(KeyEvent::new(
            KeyCode::Char('j'),
            crossterm::event::KeyModifiers::NONE,
        ));
        app.handle_key(KeyEvent::new(
            KeyCode::Char('k'),
            crossterm::event::KeyModifiers::NONE,
        ));
        app.handle_key(KeyEvent::new(
            KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        ));
        assert!(matches!(app.overlay, Some(Overlay::InfoModal { .. })));
        app.overlay = None;

        // Tags: cursor moves with j/k; Enter opens a context menu
        app.active_view = ViewId::Tags;
        app.tags.set_items(vec![tag("v1.0.0")]);
        app.handle_key(KeyEvent::new(
            KeyCode::Char('j'),
            crossterm::event::KeyModifiers::NONE,
        ));
        app.handle_key(KeyEvent::new(
            KeyCode::Char('k'),
            crossterm::event::KeyModifiers::NONE,
        ));
        app.handle_key(KeyEvent::new(
            KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        ));
        assert!(matches!(app.overlay, Some(Overlay::InfoModal { .. })));
        app.overlay = None;

        // Worktrees: cursor moves with j/k; Enter opens a context menu
        app.active_view = ViewId::Worktrees;
        app.worktrees.set_items(vec![worktree("feature/c")]);
        app.handle_key(KeyEvent::new(
            KeyCode::Char('j'),
            crossterm::event::KeyModifiers::NONE,
        ));
        app.handle_key(KeyEvent::new(
            KeyCode::Char('k'),
            crossterm::event::KeyModifiers::NONE,
        ));
        app.handle_key(KeyEvent::new(
            KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        ));
        assert!(matches!(app.overlay, Some(Overlay::InfoModal { .. })));
    }

    #[test]
    fn graph_enter_uses_local_branch_metadata_for_actions() {
        let mut app = graph_app(vec![graph_ref(
            "feature/local",
            graph::GraphRefKind::LocalBranch,
        )]);
        let mut local = branch(
            "feature/local",
            TrackingStatus::Tracked {
                remote_ref: "origin/feature/local".into(),
                gone: false,
            },
        );
        local.ahead = Some(0);
        app.branches.set_items(vec![local]);
        app.has_configured_remote = true;
        app.pr_map.insert(
            "feature/local".into(),
            PrInfo {
                number: 42,
                status: PrStatus::Open,
            },
        );

        app.handle_key(KeyEvent::new(
            KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        ));

        let items = info_modal_items(&app);
        let push = items
            .iter()
            .find(|item| item.action == BranchAction::Push)
            .expect("local Push action");
        assert!(!push.enabled);
        assert_eq!(push.reason.as_deref(), Some("not ahead"));
        assert!(items
            .iter()
            .any(|item| { item.action == BranchAction::ViewRemotePR && item.enabled }));
    }

    #[test]
    fn graph_enter_uses_remote_branch_metadata_for_actions() {
        let mut app = graph_app(vec![graph_ref(
            "upstream/feature/remote",
            graph::GraphRefKind::RemoteBranch,
        )]);
        app.remotes
            .set_items(vec![remote("upstream/feature/remote", "feature/remote")]);

        app.handle_key(KeyEvent::new(
            KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        ));

        let items = info_modal_items(&app);
        assert!(items
            .iter()
            .any(|item| { item.action == BranchAction::CheckoutRemote && item.enabled }));
        let delete = items
            .iter()
            .find(|item| item.action == BranchAction::DeleteRemoteBranch)
            .expect("remote Delete action");
        assert!(delete.enabled);
        assert_eq!(delete.target, "feature/remote");
        assert_eq!(delete.remote.as_deref(), Some("upstream"));
    }

    #[test]
    fn graph_enter_uses_tag_metadata_and_keeps_ref_free_commits_informational() {
        let mut tag_app = graph_app(vec![graph_ref("v1.2.3", graph::GraphRefKind::Tag)]);
        tag_app.tags.set_items(vec![tag("v1.2.3")]);

        tag_app.handle_key(KeyEvent::new(
            KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        ));
        assert!(info_modal_items(&tag_app)
            .iter()
            .any(|item| item.action == BranchAction::PushTag));

        let mut ref_free_app = graph_app(vec![]);
        ref_free_app.handle_key(KeyEvent::new(
            KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        ));
        assert!(matches!(
            ref_free_app.overlay,
            Some(Overlay::InfoModal {
                ref items,
                focus: InfoModalFocus::Info,
                ..
            }) if items.is_empty()
        ));
    }

    #[test]
    fn graph_info_modal_picks_up_authoritative_metadata_loaded_in_background() {
        let mut app = graph_app(vec![graph_ref("v1.2.3", graph::GraphRefKind::Tag)]);
        app.handle_key(KeyEvent::new(
            KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        ));
        assert!(info_modal_items(&app).is_empty());

        let (tx, rx) = mpsc::channel();
        tx.send(vec![tag("v1.2.3")]).unwrap();
        app.tag_load_rx = Some(rx);
        app.drain_channels();

        assert!(info_modal_items(&app)
            .iter()
            .any(|item| item.action == BranchAction::PushTag));
    }

    #[test]
    fn graph_info_modal_refreshes_when_selected_commit_is_reloaded() {
        let mut app = graph_app(vec![graph_ref(
            "feature/local",
            graph::GraphRefKind::LocalBranch,
        )]);
        app.handle_key(KeyEvent::new(
            KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        ));

        let mut snapshot = graph_snapshot(vec![graph_ref(
            "feature/local",
            graph::GraphRefKind::LocalBranch,
        )]);
        snapshot.commits[0].summary = "reloaded commit".into();
        app.graph.apply_result(Ok(snapshot));
        app.refresh_open_graph_menu();

        assert!(matches!(
            app.overlay,
            Some(Overlay::InfoModal {
                row: InfoModalRow::GraphCommit(ref commit),
                ..
            }) if commit.summary == "reloaded commit"
        ));
    }

    #[test]
    fn graph_multiple_refs_confirm_the_selected_authoritative_target() {
        let mut app = graph_app(vec![
            graph_ref("feature/local", graph::GraphRefKind::LocalBranch),
            graph_ref("origin/feature/remote", graph::GraphRefKind::RemoteBranch),
            graph_ref("v1.2.3", graph::GraphRefKind::Tag),
        ]);
        let mut local = branch("feature/local", TrackingStatus::Local);
        local.is_current = true;
        app.branches.set_items(vec![local]);
        app.remotes
            .set_items(vec![remote("origin/feature/remote", "feature/remote")]);
        app.tags.set_items(vec![tag("v1.2.3")]);

        app.handle_key(KeyEvent::new(
            KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        ));

        let items = info_modal_items(&app);
        assert!(items
            .iter()
            .all(|item| item.shortcut.is_none() && item.label.contains(":")));
        let disabled_local_delete = items
            .iter()
            .find(|item| item.action == BranchAction::DeleteLocal)
            .expect("local Delete action");
        assert!(!disabled_local_delete.enabled);
        assert_eq!(disabled_local_delete.reason.as_deref(), Some("current"));

        let remote_delete_cursor = items
            .iter()
            .position(|item| item.action == BranchAction::DeleteRemoteBranch)
            .expect("remote Delete action");
        let overlay = app.overlay.take().unwrap();
        let Overlay::InfoModal {
            items,
            info_cursor,
            focus,
            row,
            ..
        } = overlay
        else {
            unreachable!();
        };
        app.overlay = Some(Overlay::InfoModal {
            items,
            cursor: remote_delete_cursor,
            info_cursor,
            focus,
            row,
        });

        app.handle_overlay_key(KeyEvent::new(
            KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        ));

        assert!(matches!(
            app.overlay,
            Some(Overlay::Confirm {
                action: BranchAction::DeleteRemoteBranch,
                ref targets,
                ..
            }) if targets == &["feature/remote"]
        ));
        assert_eq!(app.return_view, ViewId::Graph);
    }

    #[test]
    fn graph_origin_job_refreshes_authoritative_list_and_graph() {
        let tmpdir = tempfile::tempdir().expect("temp repo");
        let dir = tmpdir.path();
        run_git(dir, &["init"]);
        run_git(dir, &["config", "user.email", "test@example.com"]);
        run_git(dir, &["config", "user.name", "Test User"]);
        std::fs::write(dir.join("README.md"), "base\n").unwrap();
        run_git(dir, &["add", "README.md"]);
        run_git(dir, &["commit", "-m", "initial"]);
        run_git(dir, &["branch", "-M", "main"]);
        run_git(dir, &["branch", "feature/refresh"]);

        let mut app = App::new(dir.to_path_buf(), "main".into(), Config::default());
        app.active_view = ViewId::Tags;
        app.branches.set_items(vec![]);
        app.graph.apply_result(Ok(graph_snapshot(vec![graph_ref(
            "feature/refresh",
            graph::GraphRefKind::LocalBranch,
        )])));

        let (op_tx, op_rx) = mpsc::channel();
        let (_prog_tx, prog_rx) = mpsc::channel();
        app.job_queue.inject_running_for_test(
            git_branch_manager::job_queue::ActionJob {
                action: BranchAction::Push,
                targets: vec!["feature/refresh".into()],
                remote: None,
                return_view: ViewId::Graph,
            },
            op_rx,
            prog_rx,
            Arc::new(AtomicBool::new(false)),
        );
        op_tx
            .send(vec![OperationResult::success(
                "feature/refresh",
                BranchAction::Push,
                "Pushed feature/refresh",
            )])
            .unwrap();

        app.drain_channels();

        assert!(app
            .branches
            .items()
            .iter()
            .any(|branch| branch.name == "feature/refresh"));
        assert!(app.remotes.loading);
        assert!(app.graph.is_loading());
        assert_eq!(app.active_view, ViewId::Tags);
    }

    #[test]
    fn graph_origin_remote_checkout_refreshes_branches_remotes_and_graph() {
        let tmpdir = tempfile::tempdir().expect("temp repo");
        let dir = tmpdir.path();
        run_git(dir, &["init"]);
        run_git(dir, &["config", "user.email", "test@example.com"]);
        run_git(dir, &["config", "user.name", "Test User"]);
        std::fs::write(dir.join("README.md"), "base\n").unwrap();
        run_git(dir, &["add", "README.md"]);
        run_git(dir, &["commit", "-m", "initial"]);
        run_git(dir, &["branch", "-M", "main"]);
        run_git(dir, &["branch", "feature/remote"]);

        let mut app = App::new(dir.to_path_buf(), "main".into(), Config::default());
        app.active_view = ViewId::Tags;
        app.branches.set_items(vec![]);
        app.graph.apply_result(Ok(graph_snapshot(vec![graph_ref(
            "origin/feature/remote",
            graph::GraphRefKind::RemoteBranch,
        )])));

        let (op_tx, op_rx) = mpsc::channel();
        let (_prog_tx, prog_rx) = mpsc::channel();
        app.job_queue.inject_running_for_test(
            git_branch_manager::job_queue::ActionJob {
                action: BranchAction::CheckoutRemote,
                targets: vec!["feature/remote".into()],
                remote: None,
                return_view: ViewId::Graph,
            },
            op_rx,
            prog_rx,
            Arc::new(AtomicBool::new(false)),
        );
        op_tx
            .send(vec![OperationResult::success(
                "feature/remote",
                BranchAction::CheckoutRemote,
                "Checked out feature/remote",
            )])
            .unwrap();

        app.drain_channels();

        assert!(app
            .branches
            .items()
            .iter()
            .any(|branch| branch.name == "feature/remote"));
        assert!(app.remotes.loading);
        assert!(app.worktrees.loading);
        assert!(app.graph.is_loading());
        assert_eq!(app.active_view, ViewId::Tags);
    }

    #[test]
    fn graph_origin_remote_delete_and_local_refreshes_branches_remotes_and_graph() {
        let tmpdir = tempfile::tempdir().expect("temp repo");
        let dir = tmpdir.path();
        run_git(dir, &["init"]);
        run_git(dir, &["config", "user.email", "test@example.com"]);
        run_git(dir, &["config", "user.name", "Test User"]);
        std::fs::write(dir.join("README.md"), "base\n").unwrap();
        run_git(dir, &["add", "README.md"]);
        run_git(dir, &["commit", "-m", "initial"]);
        run_git(dir, &["branch", "-M", "main"]);
        run_git(dir, &["branch", "feature/remote"]);

        let mut app = App::new(dir.to_path_buf(), "main".into(), Config::default());
        app.active_view = ViewId::Tags;
        app.branches.set_items(vec![]);
        app.graph.apply_result(Ok(graph_snapshot(vec![graph_ref(
            "origin/feature/remote",
            graph::GraphRefKind::RemoteBranch,
        )])));

        let (op_tx, op_rx) = mpsc::channel();
        let (_prog_tx, prog_rx) = mpsc::channel();
        app.job_queue.inject_running_for_test(
            git_branch_manager::job_queue::ActionJob {
                action: BranchAction::DeleteRemoteAndLocal,
                targets: vec!["feature/remote".into()],
                remote: None,
                return_view: ViewId::Graph,
            },
            op_rx,
            prog_rx,
            Arc::new(AtomicBool::new(false)),
        );
        op_tx
            .send(vec![OperationResult::success(
                "feature/remote",
                BranchAction::DeleteRemoteAndLocal,
                "Deleted feature/remote",
            )])
            .unwrap();

        app.drain_channels();

        assert!(app
            .branches
            .items()
            .iter()
            .any(|branch| branch.name == "feature/remote"));
        assert!(app.remotes.loading);
        assert!(app.graph.is_loading());
        assert_eq!(app.active_view, ViewId::Tags);
    }

    #[test]
    fn graph_action_refreshes_dependent_ref_views() {
        assert_eq!(
            graph_affected_views(BranchAction::DeleteLocal),
            &[ViewId::Branches, ViewId::Remotes]
        );
        assert_eq!(
            graph_affected_views(BranchAction::DeleteRemoteBranch),
            &[ViewId::Branches, ViewId::Remotes]
        );
        assert_eq!(
            graph_affected_views(BranchAction::FetchRemote),
            &[ViewId::Branches, ViewId::Remotes, ViewId::Tags]
        );
    }

    #[test]
    fn branch_origin_delete_refreshes_remote_and_worktree_views() {
        let tmpdir = tempfile::tempdir().expect("temp repo");
        let dir = tmpdir.path();
        run_git(dir, &["init", "-b", "main"]);
        run_git(dir, &["config", "user.email", "test@example.com"]);
        run_git(dir, &["config", "user.name", "Test User"]);
        std::fs::write(dir.join("README.md"), "base\n").unwrap();
        run_git(dir, &["add", "README.md"]);
        run_git(dir, &["commit", "-m", "initial"]);

        let mut app = App::new(dir.to_path_buf(), "main".into(), Config::default());
        app.remotes.loading = false;
        app.worktrees.loading = false;

        let (op_tx, op_rx) = mpsc::channel();
        let (_prog_tx, prog_rx) = mpsc::channel();
        app.job_queue.inject_running_for_test(
            git_branch_manager::job_queue::ActionJob {
                action: BranchAction::DeleteBranchAndRemoveWorktreeForce,
                targets: vec!["feature/delete".into()],
                remote: None,
                return_view: ViewId::Branches,
            },
            op_rx,
            prog_rx,
            Arc::new(AtomicBool::new(false)),
        );
        op_tx
            .send(vec![OperationResult::success(
                "feature/delete",
                BranchAction::DeleteBranchAndRemoveWorktreeForce,
                "Deleted feature/delete",
            )])
            .unwrap();

        app.drain_channels();

        assert!(app.remotes.loading);
        assert!(app.worktrees.loading);
    }

    #[test]
    fn branches_view_lazily_loads_worktrees_for_delete_preflight() {
        let tmpdir = tempfile::tempdir().expect("temp repo");
        let mut app = App::new(
            tmpdir.path().to_path_buf(),
            "main".into(),
            Config::default(),
        );
        app.active_view = ViewId::Branches;
        app.worktrees.loading = false;

        app.ensure_view_loaded();

        assert!(app.worktrees.loading);
    }

    #[test]
    fn branches_view_respects_worktree_load_opt_out() {
        let tmpdir = tempfile::tempdir().expect("temp repo");
        let config = Config {
            load_worktrees_on_launch: Some(false),
            ..Config::default()
        };
        let mut app = App::new(tmpdir.path().to_path_buf(), "main".into(), config);
        app.active_view = ViewId::Branches;
        app.worktrees.loading = false;

        app.ensure_view_loaded();

        assert!(!app.worktrees.loading);
    }

    #[test]
    fn local_remote_delete_keeps_remote_when_remote_deletion_fails() {
        let mut app = graph_app(vec![graph_ref(
            "feature/local",
            graph::GraphRefKind::LocalBranch,
        )]);
        app.remotes
            .set_items(vec![remote("origin/feature/local", "feature/local")]);

        let (op_tx, op_rx) = mpsc::channel();
        let (_prog_tx, prog_rx) = mpsc::channel();
        app.job_queue.inject_running_for_test(
            git_branch_manager::job_queue::ActionJob {
                action: BranchAction::DeleteLocalAndRemote,
                targets: vec!["feature/local".into()],
                remote: None,
                return_view: ViewId::Graph,
            },
            op_rx,
            prog_rx,
            Arc::new(AtomicBool::new(false)),
        );
        op_tx
            .send(vec![
                OperationResult::success(
                    "feature/local",
                    BranchAction::DeleteLocal,
                    "Deleted local feature/local",
                ),
                OperationResult {
                    branch_name: "feature/local".into(),
                    action: BranchAction::DeleteRemoteBranch,
                    success: false,
                    message: "Remote deletion failed".into(),
                    failure: None,
                },
            ])
            .unwrap();

        app.drain_channels();

        assert!(app.remotes.loading);
        assert!(app
            .remotes
            .items()
            .iter()
            .any(|branch| branch.short_name == "feature/local"));
    }

    #[test]
    fn local_remote_delete_filters_only_the_selected_remote() {
        let mut app = graph_app(vec![graph_ref(
            "feature/local",
            graph::GraphRefKind::LocalBranch,
        )]);
        app.remotes.set_items(vec![
            remote("origin/feature/local", "feature/local"),
            remote("upstream/feature/local", "feature/local"),
        ]);

        let (op_tx, op_rx) = mpsc::channel();
        let (_prog_tx, prog_rx) = mpsc::channel();
        app.job_queue.inject_running_for_test(
            git_branch_manager::job_queue::ActionJob {
                action: BranchAction::DeleteLocalAndRemote,
                targets: vec!["feature/local".into()],
                remote: Some("upstream".into()),
                return_view: ViewId::Graph,
            },
            op_rx,
            prog_rx,
            Arc::new(AtomicBool::new(false)),
        );
        op_tx
            .send(vec![
                OperationResult::success(
                    "feature/local",
                    BranchAction::DeleteLocal,
                    "Deleted local feature/local",
                ),
                OperationResult::success(
                    "feature/local",
                    BranchAction::DeleteRemoteBranch,
                    "Deleted upstream/feature/local",
                ),
            ])
            .unwrap();

        app.drain_channels();

        assert!(app
            .remotes
            .items()
            .iter()
            .any(|branch| branch.full_ref == "origin/feature/local"));
        assert!(!app
            .remotes
            .items()
            .iter()
            .any(|branch| branch.full_ref == "upstream/feature/local"));
    }

    fn run_git(dir: &std::path::Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap_or_else(|e| panic!("failed to run git {:?}: {}", args, e));
        if !output.status.success() {
            panic!(
                "git {:?} failed in {}: {}",
                args,
                dir.display(),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    fn run_git_with_env(dir: &std::path::Path, args: &[&str], env: &[(&str, &str)]) {
        let output = Command::new("git")
            .args(args)
            .envs(env.iter().copied())
            .current_dir(dir)
            .output()
            .unwrap_or_else(|e| panic!("failed to run git {:?}: {}", args, e));
        if !output.status.success() {
            panic!(
                "git {:?} failed in {}: {}",
                args,
                dir.display(),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    #[test]
    fn branch_operation_result_refreshes_branch_metadata_immediately() {
        let tmpdir = tempfile::tempdir().expect("temp repo");
        let dir = tmpdir.path();
        run_git(dir, &["init"]);
        run_git(dir, &["config", "user.email", "test@example.com"]);
        run_git(dir, &["config", "user.name", "Test User"]);

        std::fs::write(dir.join("README.md"), "base\n").unwrap();
        run_git(dir, &["add", "README.md"]);
        run_git_with_env(
            dir,
            &["commit", "-m", "initial"],
            &[
                ("GIT_AUTHOR_DATE", "2001-01-01T00:00:00Z"),
                ("GIT_COMMITTER_DATE", "2001-01-01T00:00:00Z"),
            ],
        );
        run_git(dir, &["branch", "-M", "main"]);

        let branch_name = "feature/rebase-refresh";
        run_git(dir, &["checkout", "-b", branch_name]);
        std::fs::write(dir.join("feature.txt"), "old\n").unwrap();
        run_git(dir, &["add", "feature.txt"]);
        run_git_with_env(
            dir,
            &["commit", "-m", "feature old"],
            &[
                ("GIT_AUTHOR_DATE", "2001-01-02T00:00:00Z"),
                ("GIT_COMMITTER_DATE", "2001-01-02T00:00:00Z"),
            ],
        );

        let mut app = App::new(dir.to_path_buf(), "main".into(), Config::default());
        app.return_view = ViewId::Branches;
        let stale_date = Utc.with_ymd_and_hms(1999, 1, 1, 0, 0, 0).unwrap();
        app.branches.set_items(vec![BranchInfo {
            name: branch_name.into(),
            is_current: false,
            is_base: false,
            tracking: TrackingStatus::Local,
            ahead: Some(99),
            behind: Some(99),
            last_commit_date: stale_date,
            merge_status: MergeStatus::Merged,
            base_branch: "main".into(),
            merge_base_commit: Some("stale".into()),
            pr: Some(PrInfo {
                number: 123,
                status: PrStatus::Open,
            }),
        }]);

        std::fs::write(dir.join("feature.txt"), "new\n").unwrap();
        run_git(dir, &["add", "feature.txt"]);
        run_git_with_env(
            dir,
            &["commit", "-m", "feature new"],
            &[
                ("GIT_AUTHOR_DATE", "2001-01-03T00:00:00Z"),
                ("GIT_COMMITTER_DATE", "2001-01-03T00:00:00Z"),
            ],
        );

        // Simulate having navigated to a different view while the job ran in
        // the background -- completing it must not snap the user back.
        app.active_view = ViewId::Worktrees;

        let (op_tx, op_rx) = mpsc::channel();
        let (_prog_tx, prog_rx) = mpsc::channel();
        app.job_queue.inject_running_for_test(
            git_branch_manager::job_queue::ActionJob {
                action: BranchAction::Rebase,
                targets: vec![branch_name.to_string()],
                remote: None,
                return_view: ViewId::Branches,
            },
            op_rx,
            prog_rx,
            Arc::new(AtomicBool::new(false)),
        );
        op_tx
            .send(vec![OperationResult::success(
                branch_name,
                BranchAction::Rebase,
                "Rebased feature/rebase-refresh onto main",
            )])
            .unwrap();

        app.drain_channels();

        let refreshed = app
            .branches
            .items()
            .iter()
            .find(|branch| branch.name == branch_name)
            .expect("refreshed feature branch");
        assert_eq!(
            refreshed.last_commit_date,
            Utc.with_ymd_and_hms(2001, 1, 3, 0, 0, 0).unwrap(),
            "branch metadata should refresh as soon as operation results arrive"
        );
        assert_eq!(
            app.active_view,
            ViewId::Worktrees,
            "completing a background job must not snap the user back to its view"
        );
        assert!(
            app.overlay.is_none(),
            "confirmed-action completion is non-modal"
        );
    }

    /// Companion to `confirmed_action_completion_is_non_modal`: when the
    /// background job returns a typed failure, the Results overlay must
    /// auto-open so the user sees the cause and the recovery keys
    /// (`!`/`r` once those are wired in #072). Plan P005 §5 + §9.
    #[test]
    fn failed_confirmed_action_opens_results_overlay() {
        let tmpdir = tempfile::tempdir().expect("temp repo");
        let dir = tmpdir.path();
        run_git(dir, &["init"]);
        run_git(dir, &["config", "user.email", "test@example.com"]);
        run_git(dir, &["config", "user.name", "Test User"]);
        std::fs::write(dir.join("README.md"), "base\n").unwrap();
        run_git(dir, &["add", "README.md"]);
        run_git(dir, &["commit", "-m", "initial"]);
        run_git(dir, &["branch", "-M", "main"]);

        let mut app = App::new(dir.to_path_buf(), "main".into(), Config::default());

        let branch_name = "feature/delete-fail";
        let (op_tx, op_rx) = mpsc::channel();
        let (_prog_tx, prog_rx) = mpsc::channel();
        app.job_queue.inject_running_for_test(
            git_branch_manager::job_queue::ActionJob {
                action: BranchAction::DeleteLocal,
                targets: vec![branch_name.to_string()],
                remote: None,
                return_view: ViewId::Branches,
            },
            op_rx,
            prog_rx,
            Arc::new(AtomicBool::new(false)),
        );
        op_tx
            .send(vec![OperationResult::failure(
                branch_name,
                BranchAction::DeleteLocal,
                FailureCause::Other {
                    raw_message: "could not delete because reasons".into(),
                },
                "Failed to delete feature/delete-fail: could not delete because reasons",
            )])
            .unwrap();

        app.drain_channels();

        match &app.overlay {
            Some(Overlay::Results { results }) => {
                assert_eq!(results.len(), 1);
                assert_eq!(results[0].branch_name, branch_name);
                assert!(matches!(
                    results[0].failure,
                    Some(FailureCause::Other { .. })
                ));
            }
            other => panic!("expected Overlay::Results after a failed job, got {other:?}"),
        }
    }

    /// `BranchNotFound` is treated as success for overlay purposes (plan
    /// P005 §9): the row will vanish on the next refresh, so we suppress
    /// the modal so the user isn't interrupted by something already
    /// resolved on disk.
    #[test]
    fn branch_not_found_failure_does_not_open_overlay() {
        let tmpdir = tempfile::tempdir().expect("temp repo");
        let dir = tmpdir.path();
        run_git(dir, &["init"]);
        run_git(dir, &["config", "user.email", "test@example.com"]);
        run_git(dir, &["config", "user.name", "Test User"]);
        std::fs::write(dir.join("README.md"), "base\n").unwrap();
        run_git(dir, &["add", "README.md"]);
        run_git(dir, &["commit", "-m", "initial"]);
        run_git(dir, &["branch", "-M", "main"]);

        let mut app = App::new(dir.to_path_buf(), "main".into(), Config::default());

        let branch_name = "feature/already-gone";
        let (op_tx, op_rx) = mpsc::channel();
        let (_prog_tx, prog_rx) = mpsc::channel();
        app.job_queue.inject_running_for_test(
            git_branch_manager::job_queue::ActionJob {
                action: BranchAction::DeleteLocal,
                targets: vec![branch_name.to_string()],
                remote: None,
                return_view: ViewId::Branches,
            },
            op_rx,
            prog_rx,
            Arc::new(AtomicBool::new(false)),
        );
        op_tx
            .send(vec![OperationResult::failure(
                branch_name,
                BranchAction::DeleteLocal,
                FailureCause::BranchNotFound,
                format!("Branch not found: {branch_name}"),
            )])
            .unwrap();

        app.drain_channels();

        assert!(
            app.overlay.is_none(),
            "BranchNotFound must not auto-open the Results overlay"
        );
    }

    #[test]
    fn results_force_key_starts_force_delete_for_only_unmerged_failures() {
        let tmpdir = tempfile::tempdir().expect("temp repo");
        let mut app = App::new(
            tmpdir.path().to_path_buf(),
            "main".into(),
            Config::default(),
        );
        app.return_view = ViewId::Branches;
        app.overlay = Some(Overlay::Results {
            results: vec![
                OperationResult::failure(
                    "feature/unmerged",
                    BranchAction::DeleteLocal,
                    FailureCause::NotMerged,
                    "not merged",
                ),
                OperationResult::failure(
                    "feature/other",
                    BranchAction::DeleteLocal,
                    FailureCause::Other {
                        raw_message: "other".into(),
                    },
                    "other",
                ),
            ],
        });

        app.handle_overlay_key(KeyEvent::new(
            KeyCode::Char('!'),
            crossterm::event::KeyModifiers::NONE,
        ));

        assert!(app.overlay.is_none());
        assert_eq!(
            app.job_queue.current_action_for_test(),
            Some(BranchAction::DeleteLocalForce)
        );
    }

    #[test]
    fn results_worktree_key_starts_safe_cascade_for_linked_worktree_failures() {
        let tmpdir = tempfile::tempdir().expect("temp repo");
        let mut app = App::new(
            tmpdir.path().to_path_buf(),
            "main".into(),
            Config::default(),
        );
        app.return_view = ViewId::Branches;
        app.overlay = Some(Overlay::Results {
            results: vec![OperationResult::failure(
                "feature/worktree",
                BranchAction::DeleteLocal,
                FailureCause::CheckedOutInWorktree {
                    worktree_path: PathBuf::from("/repo/.worktrees/feature-worktree"),
                    is_main: false,
                },
                "checked out elsewhere",
            )],
        });

        app.handle_overlay_key(KeyEvent::new(
            KeyCode::Char('r'),
            crossterm::event::KeyModifiers::NONE,
        ));

        assert!(app.overlay.is_none());
        assert_eq!(
            app.job_queue.current_action_for_test(),
            Some(BranchAction::DeleteBranchAndRemoveWorktree)
        );
    }

    #[test]
    fn delete_confirm_shows_force_key_for_unmerged_branch() {
        let tmpdir = tempfile::tempdir().expect("temp repo");
        let mut app = App::new(
            tmpdir.path().to_path_buf(),
            "main".into(),
            Config::default(),
        );
        let mut item = branch("feature/x", TrackingStatus::Local);
        item.merge_status = MergeStatus::Unmerged;
        app.branches.set_items(vec![item]);

        app.delete_selected_branches(false);

        let Some(Overlay::Confirm {
            action, extra_keys, ..
        }) = app.overlay.as_ref()
        else {
            panic!("expected confirm overlay, got {:?}", app.overlay);
        };
        assert_eq!(*action, BranchAction::DeleteLocal);
        let force = extra_keys
            .iter()
            .find(|key| key.key == '!')
            .expect("expected a `!` force-delete extra key");
        assert_eq!(force.action, BranchAction::DeleteLocalForce);
        assert_eq!(force.targets, vec!["feature/x".to_string()]);
    }

    #[test]
    fn delete_confirm_shows_worktree_key_for_linked_branch() {
        let tmpdir = tempfile::tempdir().expect("temp repo");
        let mut app = App::new(
            tmpdir.path().to_path_buf(),
            "main".into(),
            Config::default(),
        );
        let mut item = branch("feature/y", TrackingStatus::Local);
        item.merge_status = MergeStatus::Merged;
        app.branches.set_items(vec![item]);
        app.worktrees.set_items(vec![worktree("feature/y")]);

        app.delete_selected_branches(false);

        let Some(Overlay::Confirm { extra_keys, .. }) = app.overlay.as_ref() else {
            panic!("expected confirm overlay, got {:?}", app.overlay);
        };
        let cascade = extra_keys
            .iter()
            .find(|key| key.key == 'r')
            .expect("expected an `r` remove-worktree extra key");
        assert_eq!(cascade.action, BranchAction::DeleteBranchAndRemoveWorktree);
        assert_eq!(cascade.targets, vec!["feature/y".to_string()]);
    }

    #[test]
    fn delete_confirm_force_delete_menu_opens_confirm() {
        let tmpdir = tempfile::tempdir().expect("temp repo");
        let mut app = App::new(
            tmpdir.path().to_path_buf(),
            "main".into(),
            Config::default(),
        );
        app.branches
            .set_items(vec![branch("feature/z", TrackingStatus::Local)]);

        let item = app
            .build_branch_menu()
            .into_iter()
            .find(|item| item.label == "Force-delete local")
            .expect("expected a `Force-delete local` menu entry");
        assert_eq!(item.shortcut, Some('!'));
        assert!(item.enabled);

        app.execute_menu_action(item);

        let Some(Overlay::Confirm {
            action,
            targets,
            reason,
            extra_keys,
            ..
        }) = app.overlay.as_ref()
        else {
            panic!("expected confirm overlay, got {:?}", app.overlay);
        };
        assert_eq!(*action, BranchAction::DeleteLocalForce);
        assert_eq!(*targets, vec!["feature/z".to_string()]);
        assert!(reason.is_none());
        assert!(extra_keys.is_empty());
    }

    #[test]
    fn build_delete_preflight_abbreviates_long_worktree_path_to_target_width() {
        let tmpdir = tempfile::tempdir().expect("temp repo");
        let mut app = App::new(
            tmpdir.path().to_path_buf(),
            "main".into(),
            Config::default(),
        );

        // Mirror build_delete_preflight's own target-width derivation so the
        // assertion holds regardless of whether stdout is a TTY in this run.
        let term_width = crossterm::terminal::size()
            .map(|(w, _)| w)
            .unwrap_or(80);
        let target_width = ((term_width as usize * 60 / 100).saturating_sub(4)).max(20);

        let mut wt = worktree("feature/y");
        let long_path =
            "/Users/chris/dev/git-branch-manager/.claude/worktrees/feat";
        assert!(
            long_path.chars().count() > target_width,
            "fixture path must exceed the computed budget for this test to be meaningful"
        );
        wt.path = PathBuf::from(long_path);
        app.worktrees.set_items(vec![wt]);

        let (reason, _extra_keys) = app.build_delete_preflight(&["feature/y".to_string()]);

        let reason = reason.expect("expected a preflight reason for a checked-out worktree");
        assert!(
            reason.contains("is checked out in"),
            "got: {reason:?}"
        );
        // The raw, unabbreviated path must not appear verbatim in the reason.
        assert!(
            !reason.contains(long_path),
            "path should have been abbreviated, got: {reason:?}"
        );
        // The final path component ("feat") must stay fully visible per
        // abbreviate_path's "keep the tail" contract.
        assert!(reason.contains("feat"), "got: {reason:?}");
    }

    #[test]
    fn worktree_branch_cell_left_truncates_to_column_width() {
        let theme = Theme::dark();
        let symbols = SymbolSet::ascii();
        let ctx = CellContext {
            theme: &theme,
            symbols: &symbols,
            area_width: 100,
            compact: false,
            data_col_widths: vec![20, 12],
            first_col_width: 20,
        };
        let rows = render_worktree_row(
            &worktree("feature/very-long-branch-name"),
            0,
            false,
            false,
            &[0, 1],
            &ctx,
        );

        assert_eq!(cell_text(&rows[1]), "\u{2026}branch-name");
        assert_eq!(rows[1].alignment, Some(Alignment::Right));
    }

    #[test]
    fn worktree_base_merge_cell_is_blank() {
        let theme = Theme::dark();
        let symbols = SymbolSet::ascii();
        let ctx = CellContext {
            theme: &theme,
            symbols: &symbols,
            area_width: 120,
            compact: false,
            data_col_widths: vec![30, 20, 12, 8, 12],
            first_col_width: 30,
        };

        // Base-branch worktree: a branch can't be merged into itself, so the
        // Merge cell must be blank (mirrors the Branches view's base row).
        let mut base_wt = worktree("main");
        base_wt.is_base = true;
        base_wt.merge_status = MergeStatus::Unmerged;
        let rows = render_worktree_row(&base_wt, 0, false, false, &[0, 1, 2, 3, 4], &ctx);
        assert_eq!(
            cell_text(&rows[4]),
            "",
            "base worktree Merge cell should be blank, not 'unmerged'"
        );

        // Non-base worktree: the Merge cell shows the real status.
        let mut feat_wt = worktree("feature/x");
        feat_wt.merge_status = MergeStatus::Merged;
        let rows = render_worktree_row(&feat_wt, 0, false, false, &[0, 1, 2, 3, 4], &ctx);
        assert_ne!(
            cell_text(&rows[4]),
            "",
            "non-base worktree should display its merge status"
        );
    }

    #[test]
    fn remote_age_cell_uses_short_text_when_column_is_too_narrow() {
        let theme = Theme::dark();
        let symbols = SymbolSet::ascii();
        let ctx = CellContext {
            theme: &theme,
            symbols: &symbols,
            area_width: 120,
            compact: false,
            data_col_widths: vec![30, 6, 8, 5, 12],
            first_col_width: 30,
        };
        let rows = render_remote_row(&remote_branch(), 0, false, false, &[0, 1, 2, 3, 4], &ctx);

        assert_eq!(cell_text(&rows[4]), "5m");
        assert_eq!(rows[4].alignment, Some(Alignment::Right));
    }

    #[test]
    fn remote_age_cell_uses_long_text_when_column_fits() {
        let theme = Theme::dark();
        let symbols = SymbolSet::ascii();
        let ctx = CellContext {
            theme: &theme,
            symbols: &symbols,
            area_width: 120,
            compact: false,
            data_col_widths: vec![30, 6, 8, 5, 14],
            first_col_width: 30,
        };
        let rows = render_remote_row(&remote_branch(), 0, false, false, &[0, 1, 2, 3, 4], &ctx);

        assert_eq!(cell_text(&rows[4]), "5 minutes ago");
        assert_eq!(rows[4].alignment, Some(Alignment::Right));
    }

    #[test]
    fn worktree_branch_cell_does_not_truncate_when_width_unknown() {
        let theme = Theme::dark();
        let symbols = SymbolSet::ascii();
        let ctx = CellContext {
            theme: &theme,
            symbols: &symbols,
            area_width: 100,
            compact: false,
            data_col_widths: Vec::new(),
            first_col_width: 20,
        };
        let rows = render_worktree_row(
            &worktree("feature/very-long-branch-name"),
            0,
            false,
            false,
            &[1],
            &ctx,
        );

        assert_eq!(cell_text(&rows[0]), "feature/very-long-branch-name");
        assert_eq!(rows[0].alignment, None);
    }

    #[test]
    fn info_modal_focus_and_cursors_are_independent() {
        let tmpdir = tempfile::tempdir().expect("temp repo");
        let mut app = App::new(
            tmpdir.path().to_path_buf(),
            "main".into(),
            Config::default(),
        );
        app.active_view = ViewId::Branches;
        app.branches.set_items(vec![BranchInfo {
            name: "feature/test".into(),
            is_current: false,
            is_base: false,
            tracking: TrackingStatus::Local,
            ahead: None,
            behind: None,
            last_commit_date: Utc::now(),
            merge_status: MergeStatus::Unmerged,
            base_branch: "main".into(),
            merge_base_commit: None,
            pr: None,
        }]);
        app.open_context_menu();

        assert!(matches!(
            app.overlay,
            Some(Overlay::InfoModal {
                focus: InfoModalFocus::Actions,
                cursor: 0,
                info_cursor: 0,
                ..
            })
        ));
        app.handle_overlay_key(KeyEvent::new(
            KeyCode::Tab,
            crossterm::event::KeyModifiers::NONE,
        ));
        app.handle_overlay_key(KeyEvent::new(
            KeyCode::Down,
            crossterm::event::KeyModifiers::NONE,
        ));
        assert!(matches!(
            app.overlay,
            Some(Overlay::InfoModal {
                focus: InfoModalFocus::Info,
                cursor: 0,
                info_cursor: 1,
                ..
            })
        ));
        app.handle_overlay_key(KeyEvent::new(
            KeyCode::Tab,
            crossterm::event::KeyModifiers::NONE,
        ));
        app.handle_overlay_key(KeyEvent::new(
            KeyCode::Down,
            crossterm::event::KeyModifiers::NONE,
        ));
        assert!(matches!(
            app.overlay,
            Some(Overlay::InfoModal {
                focus: InfoModalFocus::Actions,
                cursor: 1,
                info_cursor: 1,
                ..
            })
        ));
    }

    #[test]
    fn worktree_menu_delete_branch_enabled_when_clean() {
        let tmpdir = tempfile::tempdir().expect("temp repo");
        let mut app = App::new(
            tmpdir.path().to_path_buf(),
            "main".into(),
            Config::default(),
        );
        app.worktrees.set_items(vec![worktree("feature/clean")]);
        app.branches.set_items(vec![BranchInfo {
            name: "feature/clean".into(),
            is_current: false,
            is_base: false,
            tracking: TrackingStatus::Tracked {
                remote_ref: "origin/feature/clean".into(),
                gone: false,
            },
            ahead: None,
            behind: None,
            last_commit_date: Utc::now(),
            merge_status: MergeStatus::Unmerged,
            base_branch: "main".into(),
            merge_base_commit: None,
            pr: None,
        }]);

        let items = app.build_worktree_menu();
        let solo = items
            .iter()
            .find(|mi| mi.label == "Remove worktree + branch")
            .unwrap();
        let combo = items
            .iter()
            .find(|mi| mi.label == "Remove worktree + branch (local + remote)")
            .unwrap();
        assert!(solo.enabled && solo.reason.is_none());
        assert!(combo.enabled && combo.reason.is_none());
    }

    #[test]
    fn worktree_menu_delete_branch_remote_disabled_when_no_remote() {
        let tmpdir = tempfile::tempdir().expect("temp repo");
        let mut app = App::new(
            tmpdir.path().to_path_buf(),
            "main".into(),
            Config::default(),
        );
        app.worktrees.set_items(vec![worktree("feature/no-remote")]);
        app.branches.set_items(vec![BranchInfo {
            name: "feature/no-remote".into(),
            is_current: false,
            is_base: false,
            tracking: TrackingStatus::Local,
            ahead: None,
            behind: None,
            last_commit_date: Utc::now(),
            merge_status: MergeStatus::Unmerged,
            base_branch: "main".into(),
            merge_base_commit: None,
            pr: None,
        }]);

        let items = app.build_worktree_menu();
        let solo = items
            .iter()
            .find(|mi| mi.label == "Remove worktree + branch")
            .unwrap();
        let combo = items
            .iter()
            .find(|mi| mi.label == "Remove worktree + branch (local + remote)")
            .unwrap();
        assert!(solo.enabled && solo.reason.is_none());
        assert!(!combo.enabled);
        assert_eq!(combo.reason.as_deref(), Some("no remote"));
    }

    #[test]
    fn worktree_menu_delete_branch_disabled_when_dirty() {
        let tmpdir = tempfile::tempdir().expect("temp repo");
        let mut app = App::new(
            tmpdir.path().to_path_buf(),
            "main".into(),
            Config::default(),
        );
        let mut wt = worktree("feature/dirty");
        wt.wt_status = WorkingTreeStatus {
            has_staged: false,
            has_modified: true,
            has_untracked: false,
            changed_files: Vec::new(),
        };
        app.worktrees.set_items(vec![wt]);

        let solo = app
            .build_worktree_menu()
            .into_iter()
            .find(|mi| mi.label == "Remove worktree + branch")
            .unwrap();
        assert!(!solo.enabled);
        assert_eq!(solo.reason.as_deref(), Some("dirty"));
    }

    #[test]
    fn worktree_menu_delete_branch_disabled_when_detached() {
        let tmpdir = tempfile::tempdir().expect("temp repo");
        let mut app = App::new(
            tmpdir.path().to_path_buf(),
            "main".into(),
            Config::default(),
        );
        let mut wt = worktree("feature/detached");
        wt.branch = None;
        app.worktrees.set_items(vec![wt]);

        let solo = app
            .build_worktree_menu()
            .into_iter()
            .find(|mi| mi.label == "Remove worktree + branch")
            .unwrap();
        assert!(!solo.enabled);
        assert_eq!(solo.reason.as_deref(), Some("detached HEAD"));
    }

    #[test]
    fn worktree_menu_delete_branch_disabled_when_base() {
        let tmpdir = tempfile::tempdir().expect("temp repo");
        let mut app = App::new(
            tmpdir.path().to_path_buf(),
            "main".into(),
            Config::default(),
        );
        let mut wt = worktree("main");
        wt.is_base = true;
        app.worktrees.set_items(vec![wt]);

        let combo = app
            .build_worktree_menu()
            .into_iter()
            .find(|mi| mi.label == "Remove worktree + branch (local + remote)")
            .unwrap();
        assert!(!combo.enabled);
        assert_eq!(combo.reason.as_deref(), Some("base branch"));
    }

    // `execute_action`'s worktree-remove-and-delete-branch coverage moved to
    // `job_queue.rs`'s own test module, since that's where the (now private)
    // function lives.

    #[test]
    fn graph_enrichment_with_stale_generation_is_dropped_by_drain() {
        // Drives the App's drain_channels directly: a stale enrichment
        // message (generation 1) must NOT mark a snapshot whose generation
        // is 2. This is the App-level half of the "stale enrichment cannot
        // overwrite a newer snapshot" requirement.
        let tmpdir = tempfile::tempdir().expect("temp repo");
        let mut app = App::new(
            tmpdir.path().to_path_buf(),
            "main".into(),
            Config::default(),
        );
        app.active_view = ViewId::Graph;
        // Force a known starting generation.
        app.spawn_graph_load(500, false);
        let first_generation = app.graph_generation;

        // Inject the structural snapshot for the first load directly.
        let (tx, rx) = mpsc::channel();
        tx.send(Ok(graph::GraphSnapshot {
            source: graph::GraphSource::Gleisbau,
            commits: vec![graph::GraphCommit {
                oid: "1111111111111111111111111111111111111111".into(),
                summary: "first".into(),
                parents: vec![],
                lane: Some(0),
                branch: None,
                refs: vec![graph::GraphRef {
                    name: "main".into(),
                    kind: graph::GraphRefKind::LocalBranch,
                    has_linked_worktree: false,
                    tracking: None,
                }],
                is_possible_squash_merge: false,
                fuzzy_squash_match: None,
                ..graph::GraphCommit::default()
            }],
            lines: vec![graph::GraphLine {
                graph: "*".into(),
                commit_index: Some(0),
            }],
            ref_counts: Default::default(),
            max_count: 500,
            includes_remotes: false,
            generation: None,
        }))
        .expect("send first snapshot");
        app.graph_rx = Some(rx);
        app.drain_channels();
        assert!(!app.graph.is_loading());

        // Trigger a second load — bumps graph_generation.
        app.spawn_graph_load(500, false);
        let second_generation = app.graph_generation;
        assert!(second_generation > first_generation);

        // Inject a structural snapshot for the second load.
        let (tx2, rx2) = mpsc::channel();
        tx2.send(Ok(graph::GraphSnapshot {
            source: graph::GraphSource::Gleisbau,
            commits: vec![graph::GraphCommit {
                oid: "2222222222222222222222222222222222222222".into(),
                summary: "second".into(),
                parents: vec![],
                lane: Some(0),
                branch: None,
                refs: vec![],
                is_possible_squash_merge: false,
                fuzzy_squash_match: None,
                ..graph::GraphCommit::default()
            }],
            lines: vec![graph::GraphLine {
                graph: "*".into(),
                commit_index: Some(0),
            }],
            ref_counts: Default::default(),
            max_count: 500,
            includes_remotes: false,
            generation: None,
        }))
        .expect("send second snapshot");
        app.graph_rx = Some(rx2);
        app.drain_channels();

        // Now a STALE enrichment arrives (tagged with the first generation).
        // The App's drain must drop it because msg.generation != current.
        let (etx, erx) = mpsc::channel();
        etx.send(graph::GraphEnrichmentMsg {
            generation: first_generation,
            updates: vec![graph::GraphEnrichmentUpdate {
                oid: "2222222222222222222222222222222222222222".into(),
                is_possible_squash_merge: true,
                fuzzy_squash_match: None,
                is_cherry_picked_commit: false,
            }],
        })
        .expect("send stale enrichment");
        app.graph_enrich_rx = Some(erx);
        app.drain_channels();

        let current = app
            .graph
            .snapshot()
            .expect("snapshot present after second load");
        assert!(
            !current.commits[0].is_possible_squash_merge,
            "stale enrichment must not overwrite the newer snapshot's marker"
        );
    }

    #[test]
    fn graph_enrichment_with_matching_generation_is_applied_by_drain() {
        // Companion to the stale-enrichment test: a matching-generation
        // enrichment message must apply, confirming the App's gate is the
        // only thing filtering enrichment.
        let tmpdir = tempfile::tempdir().expect("temp repo");
        let mut app = App::new(
            tmpdir.path().to_path_buf(),
            "main".into(),
            Config::default(),
        );
        app.active_view = ViewId::Graph;
        app.spawn_graph_load(500, false);
        let generation = app.graph_generation;

        let (tx, rx) = mpsc::channel();
        tx.send(Ok(graph::GraphSnapshot {
            source: graph::GraphSource::Gleisbau,
            commits: vec![graph::GraphCommit {
                oid: "3333333333333333333333333333333333333333".into(),
                summary: "third".into(),
                parents: vec![],
                lane: Some(0),
                branch: None,
                refs: vec![],
                is_possible_squash_merge: false,
                fuzzy_squash_match: None,
                ..graph::GraphCommit::default()
            }],
            lines: vec![graph::GraphLine {
                graph: "*".into(),
                commit_index: Some(0),
            }],
            ref_counts: Default::default(),
            max_count: 500,
            includes_remotes: false,
            generation: None,
        }))
        .expect("send snapshot");
        app.graph_rx = Some(rx);
        app.drain_channels();

        // Enrichment tagged with the same generation as the snapshot.
        let (etx, erx) = mpsc::channel();
        etx.send(graph::GraphEnrichmentMsg {
            generation,
            updates: vec![graph::GraphEnrichmentUpdate {
                oid: "3333333333333333333333333333333333333333".into(),
                is_possible_squash_merge: true,
                fuzzy_squash_match: None,
                is_cherry_picked_commit: false,
            }],
        })
        .expect("send matching enrichment");
        app.graph_enrich_rx = Some(erx);
        app.drain_channels();

        let current = app.graph.snapshot().expect("snapshot present");
        assert!(
            current.commits[0].is_possible_squash_merge,
            "matching-generation enrichment must be applied by the drain"
        );
    }
}
