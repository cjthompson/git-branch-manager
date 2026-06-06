use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, MouseButton, MouseEventKind};
use ratatui::backend::CrosstermBackend;
use ratatui::prelude::*;
use ratatui::widgets::Cell;
use ratatui::Terminal;

use git_branch_manager::config::Config;
use git_branch_manager::git::{
    branch, cache, operations, pr_loader, squash_loader, tags, worktree,
};
use git_branch_manager::symbols::SymbolSet;
use git_branch_manager::theme::Theme;
use git_branch_manager::types::*;
use git_branch_manager::ui::cells::{
    age_cell, ahead_behind_cell, merge_status_cell, merge_status_cell_for_branch, pr_cell,
};
use git_branch_manager::ui::list_render::CellContext;
use git_branch_manager::ui::menu::MenuItem;
use git_branch_manager::ui::render::{Overlay, RenderContext};
use git_branch_manager::ui::shared::{prefix_style, truncate};
use git_branch_manager::ui::toast::Toast;
use git_branch_manager::view::branches::BranchesViewDef;
use git_branch_manager::view::column::ColumnDef;
use git_branch_manager::view::filter::{FilterSet, FilterTokenDef};
use git_branch_manager::view::list_state::{self, ListState};
use git_branch_manager::view::remotes::RemotesViewDef;
use git_branch_manager::view::tags::TagsViewDef;
use git_branch_manager::view::worktrees::WorktreesViewDef;
use git_branch_manager::view::ViewId;

/// Messages sent by the background phase-1 thread.
pub enum Phase1Msg {
    /// Fast path: branch list + caches. Sent before merge detection.
    Fast(Vec<BranchInfo>, cache::BranchCache, cache::BranchCache),
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

    // View state -- 4 peers
    pub active_view: ViewId,
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
    pub remote_enrich_rx: Option<Receiver<RemoteEnrichResult>>,
    pub worktree_enrich_rx: Option<Receiver<WorktreeEnrichResult>>,
    pub pr_rx: Option<Receiver<PrMap>>,
    pub pr_map: PrMap,
    pub op_rx: Option<Receiver<Vec<OperationResult>>>,
    pub progress_rx: Option<Receiver<ProgressUpdate>>,
    pub progress: Option<ProgressUpdate>,
    pub remote_fetch_rx: Option<Receiver<bool>>,
    pub tag_load_rx: Option<Receiver<Vec<TagInfo>>>,
    pub worktree_load_rx: Option<Receiver<Vec<WorktreeInfo>>>,
    #[allow(clippy::type_complexity)]
    pub remote_load_rx: Option<
        Receiver<(
            Vec<RemoteBranchInfo>,
            Vec<(String, String)>,
            cache::BranchCache,
        )>,
    >,
    pub phase1_rx: Option<Receiver<Phase1Msg>>,

    // Cache (used for R-key cache clearing)
    #[allow(dead_code)]
    pub cache: cache::BranchCache,

    // Toast
    pub toast: Option<Toast>,

    // Operation cancellation
    pub cancel_flag: Option<Arc<AtomicBool>>,

    // Terminal dimensions (for mouse handling)
    pub terminal_rows: u16,

    // Whether remote fetch has been done this session
    pub remote_fetched: bool,
}

// ---- Watchdog helpers ----

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ---- Generic channel drain helper ----

fn drain_channel<T>(rx: &mut Option<Receiver<T>>, max_per_tick: usize) -> Vec<T> {
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

    results
}

impl App {
    pub fn new(repo_path: PathBuf, base_branch: String, config: Config) -> Self {
        let theme = Theme::from_name(config.theme.as_deref().unwrap_or("dark"));
        let symbols = SymbolSet::from_name(config.symbols.as_deref().unwrap_or("auto"));

        // Init sort from config
        let sort_col: Option<usize> = config.sort_column.as_deref().and_then(|s| match s {
            "name" => Some(0),
            "remote" => Some(1),
            "ahead" => Some(2),
            "pr" => Some(3),
            "age" => Some(4),
            "status" => Some(5),
            _ => None,
        });
        let sort_asc = config.sort_asc.unwrap_or(true);

        let branch_def = BranchesViewDef;
        let remote_def = RemotesViewDef;
        let tag_def = TagsViewDef;
        let worktree_def = WorktreesViewDef;

        let mut branch_state = ListState::empty();
        branch_state.loading = true;
        branch_state.set_sort(sort_col, sort_asc);

        let remote_fetched = config.auto_fetch == Some(true);
        let cache = cache::BranchCache::load(&repo_path);

        Self {
            repo_path,
            base_branch,
            config,
            theme,
            symbols,
            should_exit: false,
            active_view: ViewId::Branches,
            branches: branch_state,
            remotes: ListState::empty(),
            tags: ListState::empty(),
            worktrees: ListState::empty(),
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
            remote_enrich_rx: None,
            worktree_enrich_rx: None,
            pr_rx: None,
            pr_map: PrMap::new(),
            op_rx: None,
            progress_rx: None,
            progress: None,
            remote_fetch_rx: None,
            tag_load_rx: None,
            worktree_load_rx: None,
            remote_load_rx: None,
            phase1_rx: None,
            cache,
            toast: None,
            cancel_flag: None,
            terminal_rows: 0,
            remote_fetched,
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

        loop {
            tick_ms.store(now_ms(), Ordering::Relaxed);
            self.drain_channels();

            terminal.draw(|frame| {
                self.terminal_rows = frame.area().height;
                let mut ctx = self.build_render_context();
                git_branch_manager::ui::render::draw(frame, &mut ctx);
            })?;

            if self.should_exit {
                return Ok(());
            }

            if event::poll(Duration::from_millis(50))? {
                let ev = event::read()?;
                self.handle_event(ev);
            }
        }
    }

    fn build_render_context(&mut self) -> RenderContext<'_> {
        let active_filter_tokens: &[FilterTokenDef] = match self.active_view {
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

    fn drain_channels(&mut self) {
        // Phase-1 messages (fast metadata first, merge statuses second)
        for msg in drain_channel(&mut self.phase1_rx, 2) {
            match msg {
                Phase1Msg::Fast(branches, cache_for_app, _cache_for_squash) => {
                    self.cache = cache_for_app;
                    self.branches.set_items(branches);
                    self.branches.loading = false;
                    list_state::apply_sort(&mut self.branches, &self.branch_columns);
                    // Squash checker and PR loader are spawned after merge statuses arrive.
                }
                Phase1Msg::MergeStatuses(updates) => {
                    let update_map: std::collections::HashMap<String, MergeStatus> =
                        updates.into_iter().collect();
                    for b in self.branches.items_mut() {
                        if let Some(&new_status) = update_map.get(&b.name) {
                            b.merge_status = new_status;
                        }
                    }
                    self.branches.rebuild_display_indices();

                    // Now spawn squash checker on the updated (merged-filtered) set.
                    let repo_path = self.repo_path.clone();
                    let base_branch = self.base_branch.clone();
                    let cache_for_squash = cache::BranchCache::load(&repo_path);
                    if let Ok(repo) = git2::Repository::open(&repo_path) {
                        let candidates: Vec<(String, String)> = self
                            .branches
                            .items()
                            .iter()
                            .filter(|b| {
                                b.merge_status == MergeStatus::Pending
                                    && !b.is_base
                                    && !b.is_current
                            })
                            .filter_map(|b| {
                                branch::get_commit_hash(&repo, &b.name)
                                    .map(|hash| (b.name.clone(), hash))
                            })
                            .collect();

                        self.squash_total = candidates.len();
                        self.squash_checked = 0;
                        if !candidates.is_empty() {
                            self.squash_rx = Some(squash_loader::spawn_squash_checker(
                                repo_path.clone(),
                                base_branch,
                                candidates,
                                cache_for_squash,
                            ));
                        }
                    }
                    self.pr_rx = Some(pr_loader::spawn_pr_loader(repo_path));
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
        for result in drain_channel(&mut self.squash_rx, 32) {
            self.squash_checked += 1;
            if let Some(b) = self
                .branches
                .items_mut()
                .iter_mut()
                .find(|b| b.name == result.branch_name)
            {
                b.merge_status = if result.is_squash_merged {
                    MergeStatus::SquashMerged
                } else {
                    MergeStatus::Unmerged
                };
            }
        }

        // Remote squash-merge results
        for result in drain_channel(&mut self.remote_squash_rx, 32) {
            if let Some(b) = self
                .remotes
                .items_mut()
                .iter_mut()
                .find(|b| b.full_ref == result.branch_name)
            {
                b.merge_status = if result.is_squash_merged {
                    MergeStatus::SquashMerged
                } else {
                    MergeStatus::Unmerged
                };
            }
        }

        // Remote enrichment
        for result in drain_channel(&mut self.remote_enrich_rx, 32) {
            if let Some(b) = self
                .remotes
                .items_mut()
                .iter_mut()
                .find(|b| b.full_ref == result.full_ref)
            {
                b.merge_status = result.merge_status;
                b.ahead = result.ahead;
                b.behind = result.behind;
            }
        }

        // Worktree enrichment
        for result in drain_channel(&mut self.worktree_enrich_rx, 32) {
            if let Some(wt) = self.worktrees.items_mut().get_mut(result.index) {
                wt.wt_status = result.wt_status;
                wt.age_date = result.age_date;
            }
        }

        // PR map (one-shot)
        for map in drain_channel(&mut self.pr_rx, 1) {
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
        for items in drain_channel(&mut self.tag_load_rx, 1) {
            self.tags.set_items(items);
            self.tags.loading = false;
            self.clear_toast();
        }

        // Remote loading (one-shot)
        for (remotes, candidates, remote_cache) in drain_channel(&mut self.remote_load_rx, 1) {
            self.remotes.set_items(remotes);
            self.remotes.loading = false;
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
        for items in drain_channel(&mut self.worktree_load_rx, 1) {
            self.worktrees.set_items(items);
            self.worktrees.loading = false;
            self.clear_toast();

            // Spawn worktree enrichment
            let rx = worktree::enrich_worktrees(self.worktrees.items().to_vec());
            self.worktree_enrich_rx = Some(rx);
        }

        // Operation results (one-shot)
        for results in drain_channel(&mut self.op_rx, 1) {
            self.cancel_flag = None;
            self.progress_rx = None;
            self.progress = None;
            self.overlay = Some(Overlay::Results { results });
        }

        // Progress updates
        for update in drain_channel(&mut self.progress_rx, 32) {
            self.progress = Some(update.clone());
            if let Some(Overlay::Executing { progress, .. }) = &mut self.overlay {
                *progress = Some(update);
            }
        }

        // Remote fetch completion
        for success in drain_channel(&mut self.remote_fetch_rx, 1) {
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
            }
        }
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
            _ => {}
        }

        // Common navigation/selection keys (work in every view)
        if self.handle_common_list_key(key) {
            return;
        }

        // View-specific keys
        match self.active_view {
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
            Some(Overlay::Confirm { action, targets }) => match key.code {
                KeyCode::Char('y') | KeyCode::Enter => {
                    self.execute_confirmed_action(action, targets);
                }
                KeyCode::Char('n') | KeyCode::Esc => {
                    // Cancel -- don't put overlay back
                }
                _ => {
                    self.overlay = Some(Overlay::Confirm { action, targets });
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
                            self.execute_menu_action(item.action);
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
                        self.execute_menu_action(item.action);
                    } else {
                        self.overlay = Some(Overlay::Menu { cursor, items });
                    }
                }
                _ => {
                    self.overlay = Some(Overlay::Menu { cursor, items });
                }
            },
            Some(Overlay::Results { results }) => match key.code {
                KeyCode::Enter | KeyCode::Esc => {
                    self.refresh_after_operation();
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
            None => {}
        }
    }

    fn handle_settings_key(&mut self, key: KeyEvent, cursor: usize) {
        const SORT_CYCLE: [Option<usize>; 7] =
            [None, Some(0), Some(1), Some(2), Some(3), Some(4), Some(5)];
        const NUM_ROWS: usize = 6;

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
                    }
                    1 => {
                        self.theme = self.theme.next();
                    }
                    2 => {
                        let sort_col = self.branches.sort_column();
                        let pos = SORT_CYCLE.iter().position(|&c| c == sort_col).unwrap_or(0);
                        let next_pos = (pos + 1) % SORT_CYCLE.len();
                        self.branches
                            .set_sort(SORT_CYCLE[next_pos], self.branches.sort_ascending());
                        list_state::apply_sort(&mut self.branches, &self.branch_columns);
                    }
                    3 => {
                        let asc = !self.branches.sort_ascending();
                        self.branches.set_sort(self.branches.sort_column(), asc);
                        list_state::apply_sort(&mut self.branches, &self.branch_columns);
                    }
                    4 => {
                        self.config.auto_fetch = Some(self.config.auto_fetch != Some(true));
                    }
                    5 => {
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
                    }
                    1 => {
                        // backward = next() 3 times (4-cycle)
                        self.theme = self.theme.next();
                        self.theme = self.theme.next();
                        self.theme = self.theme.next();
                    }
                    2 => {
                        let sort_col = self.branches.sort_column();
                        let pos = SORT_CYCLE.iter().position(|&c| c == sort_col).unwrap_or(0);
                        let next_pos = (pos + SORT_CYCLE.len() - 1) % SORT_CYCLE.len();
                        self.branches
                            .set_sort(SORT_CYCLE[next_pos], self.branches.sort_ascending());
                        list_state::apply_sort(&mut self.branches, &self.branch_columns);
                    }
                    3 => {
                        let asc = !self.branches.sort_ascending();
                        self.branches.set_sort(self.branches.sort_column(), asc);
                        list_state::apply_sort(&mut self.branches, &self.branch_columns);
                    }
                    4 => {
                        self.config.auto_fetch = Some(self.config.auto_fetch != Some(true));
                    }
                    5 => {
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
            ViewId::Branches => &self.branch_filter_tokens,
            ViewId::Remotes => &self.remote_filter_tokens,
            ViewId::Tags => &self.tag_filter_tokens,
            ViewId::Worktrees => &self.worktree_filter_tokens,
        };

        match key.code {
            KeyCode::Esc | KeyCode::Char('\\') => {
                // Close filter (overlay already taken)
            }
            KeyCode::Char('c') => {
                // Clear all filters
                self.set_active_filter(String::new());
            }
            KeyCode::Char(ch) => {
                // Check if this matches a filter token
                if let Some(token_def) = active_tokens.iter().find(|t| t.key == ch) {
                    let current = self.active_filter_query();
                    let new = FilterSet::toggle_token(&current, token_def.token);
                    self.set_active_filter(new);
                } else {
                    self.overlay = Some(Overlay::Filter);
                }
            }
            _ => {
                self.overlay = Some(Overlay::Filter);
            }
        }
    }

    // ---- Search ----

    fn is_search_active(&self) -> bool {
        match self.active_view {
            ViewId::Branches => self.branches.search_active(),
            ViewId::Remotes => self.remotes.search_active(),
            ViewId::Tags => self.tags.search_active(),
            ViewId::Worktrees => self.worktrees.search_active(),
        }
    }

    fn toggle_search(&mut self) {
        match self.active_view {
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
            ViewId::Branches => with_state!(self.branches, key.code),
            ViewId::Remotes => with_state!(self.remotes, key.code),
            ViewId::Tags => with_state!(self.tags, key.code),
            ViewId::Worktrees => with_state!(self.worktrees, key.code),
        }
    }

    // ---- Mouse handling ----

    fn handle_mouse(&mut self, mouse: crossterm::event::MouseEvent) {
        // Don't handle mouse in overlays
        if self.overlay.is_some() {
            return;
        }

        match mouse.kind {
            MouseEventKind::ScrollDown => {
                macro_rules! with_state {
                    ($op:expr) => {
                        match self.active_view {
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

    fn handle_left_click(&mut self, x: u16, y: u16) {
        // Header row click (y == 1): sort by column
        if y == 1 {
            let clicked_col = self.find_header_click(x);
            if let Some(col) = clicked_col {
                match self.active_view {
                    ViewId::Branches => {
                        if self.branches.sort_column() == Some(col) {
                            list_state::toggle_sort_direction(&mut self.branches);
                        } else {
                            self.branches.set_sort(Some(col), true);
                        }
                        list_state::apply_sort(&mut self.branches, &self.branch_columns);
                    }
                    ViewId::Remotes => {
                        if self.remotes.sort_column() == Some(col) {
                            list_state::toggle_sort_direction(&mut self.remotes);
                        } else {
                            self.remotes.set_sort(Some(col), true);
                        }
                        list_state::apply_sort(&mut self.remotes, &self.remote_columns);
                    }
                    ViewId::Tags => {
                        if self.tags.sort_column() == Some(col) {
                            list_state::toggle_sort_direction(&mut self.tags);
                        } else {
                            self.tags.set_sort(Some(col), true);
                        }
                        list_state::apply_sort(&mut self.tags, &self.tag_columns);
                    }
                    ViewId::Worktrees => {
                        if self.worktrees.sort_column() == Some(col) {
                            list_state::toggle_sort_direction(&mut self.worktrees);
                        } else {
                            self.worktrees.set_sort(Some(col), true);
                        }
                        list_state::apply_sort(&mut self.worktrees, &self.worktree_columns);
                    }
                }
            }
        } else if self.terminal_rows > 0 && y == self.terminal_rows - 1 {
            // Status bar click
            let items = match self.active_view {
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
                ViewId::Branches => click_row!(self.branches),
                ViewId::Remotes => click_row!(self.remotes),
                ViewId::Tags => click_row!(self.tags),
                ViewId::Worktrees => click_row!(self.worktrees),
            }
        }
    }

    fn find_header_click(&self, x: u16) -> Option<usize> {
        let header_columns = match self.active_view {
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
        if items.is_empty() {
            return;
        }
        self.return_view = self.active_view;
        self.overlay = Some(Overlay::Menu { items, cursor: 0 });
    }

    fn build_menu_items(&self) -> Vec<MenuItem> {
        match self.active_view {
            ViewId::Branches => self.build_branch_menu(),
            ViewId::Remotes => self.build_remote_menu(),
            ViewId::Tags => self.build_tag_menu(),
            ViewId::Worktrees => self.build_worktree_menu(),
        }
    }

    fn build_branch_menu(&self) -> Vec<MenuItem> {
        let Some(branch) = self.branches.cursor_item() else {
            return vec![];
        };
        let has_remote = matches!(
            &branch.tracking,
            TrackingStatus::Tracked { gone: false, .. }
        );
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
            },
            MenuItem {
                label: "Push".into(),
                enabled: is_ahead,
                reason: if !has_remote {
                    Some("no remote".into())
                } else if !is_ahead {
                    Some("not ahead".into())
                } else {
                    None
                },
                shortcut: Some('p'),
                action: BranchAction::Push,
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
            },
            MenuItem {
                label: "Open PR in browser".into(),
                enabled: has_pr,
                reason: if !has_pr { Some("no PR".into()) } else { None },
                shortcut: Some('o'),
                action: BranchAction::ViewRemotePR,
            },
        ]
    }

    fn build_remote_menu(&self) -> Vec<MenuItem> {
        let Some(branch) = self.remotes.cursor_item() else {
            return vec![];
        };
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
            },
            MenuItem {
                label: "Delete remote branch".into(),
                enabled: !pinned,
                reason: if pinned { Some("base".into()) } else { None },
                shortcut: Some('d'),
                action: BranchAction::DeleteRemoteBranch,
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
            },
            MenuItem {
                label: "Fetch remote".into(),
                enabled: !pinned,
                reason: if pinned { Some("base".into()) } else { None },
                shortcut: Some('f'),
                action: BranchAction::FetchRemote,
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
            },
            MenuItem {
                label: "Merge into current".into(),
                enabled: !pinned,
                reason: if pinned { Some("base".into()) } else { None },
                shortcut: Some('m'),
                action: BranchAction::MergeRemoteIntoCurrent,
            },
            MenuItem {
                label: "Cherry-pick latest".into(),
                enabled: !pinned,
                reason: if pinned { Some("base".into()) } else { None },
                shortcut: Some('p'),
                action: BranchAction::CherryPickRemote,
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
            },
        ]
    }

    fn build_tag_menu(&self) -> Vec<MenuItem> {
        let Some(_tag) = self.tags.cursor_item() else {
            return vec![];
        };
        vec![
            MenuItem {
                label: "Delete tag".into(),
                enabled: true,
                reason: None,
                shortcut: Some('d'),
                action: BranchAction::DeleteTag,
            },
            MenuItem {
                label: "Delete tag (local + remote)".into(),
                enabled: true,
                reason: None,
                shortcut: Some('D'),
                action: BranchAction::DeleteTagAndRemote,
            },
            MenuItem {
                label: "Push tag to remote".into(),
                enabled: true,
                reason: None,
                shortcut: Some('p'),
                action: BranchAction::PushTag,
            },
        ]
    }

    fn build_worktree_menu(&self) -> Vec<MenuItem> {
        let Some(wt) = self.worktrees.cursor_item() else {
            return vec![];
        };
        let is_main = wt.is_main;
        let is_dirty = !wt.wt_status.is_clean();

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
            },
        ]
    }

    fn execute_menu_action(&mut self, action: BranchAction) {
        // View PR -- fire and forget, no confirm
        if action == BranchAction::ViewRemotePR {
            let name = match self.active_view {
                ViewId::Branches => self
                    .branches
                    .cursor_item()
                    .map(|b| b.name.clone())
                    .unwrap_or_default(),
                ViewId::Remotes => self
                    .remotes
                    .cursor_item()
                    .map(|b| b.short_name.clone())
                    .unwrap_or_default(),
                _ => return,
            };
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

        // Get target names for confirm dialog
        let targets = self.get_cursor_targets(action);
        if targets.is_empty() {
            return;
        }

        self.overlay = Some(Overlay::Confirm { action, targets });
    }

    /// Get target name(s) for a single-item action from the context menu
    fn get_cursor_targets(&self, _action: BranchAction) -> Vec<String> {
        match self.active_view {
            ViewId::Branches => self
                .branches
                .cursor_item()
                .map(|b| vec![b.name.clone()])
                .unwrap_or_default(),
            ViewId::Remotes => self
                .remotes
                .cursor_item()
                .map(|b| vec![b.short_name.clone()])
                .unwrap_or_default(),
            ViewId::Tags => self
                .tags
                .cursor_item()
                .map(|t| vec![t.name.clone()])
                .unwrap_or_default(),
            ViewId::Worktrees => self
                .worktrees
                .cursor_item()
                .map(|w| vec![w.path.to_string_lossy().to_string()])
                .unwrap_or_default(),
        }
    }

    // ---- Action Execution ----

    fn execute_confirmed_action(&mut self, action: BranchAction, item_names: Vec<String>) {
        let label = format!("{}...", action.label());
        let repo_path = self.repo_path.clone();
        let base_branch = self.base_branch.clone();

        let (op_tx, op_rx) = mpsc::channel();
        let (prog_tx, prog_rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_clone = cancel.clone();

        self.overlay = Some(Overlay::Executing {
            label,
            progress: None,
        });
        self.op_rx = Some(op_rx);
        self.progress_rx = Some(prog_rx);
        self.cancel_flag = Some(cancel);

        std::thread::spawn(move || {
            let needs_stash = git2::Repository::open(&repo_path)
                .map(|r| !crate::git::status::detect_working_tree_status(&r).is_clean())
                .unwrap_or(false);
            let results = execute_action(
                action,
                &item_names,
                &repo_path,
                &base_branch,
                needs_stash,
                &prog_tx,
                &cancel_clone,
            );
            let _ = op_tx.send(results);
        });
    }

    // ---- View-level action helpers ----

    fn delete_selected_branches(&mut self, include_remote: bool) {
        let names = self.get_selected_branch_names();
        if names.is_empty() {
            return;
        }
        let action = if include_remote {
            BranchAction::DeleteLocalAndRemote
        } else {
            BranchAction::DeleteLocal
        };
        self.return_view = ViewId::Branches;
        self.overlay = Some(Overlay::Confirm {
            action,
            targets: names,
        });
    }

    fn push_selected_branches(&mut self) {
        let names = self.get_selected_branch_names();
        if names.is_empty() {
            return;
        }
        self.return_view = ViewId::Branches;
        self.overlay = Some(Overlay::Confirm {
            action: BranchAction::Push,
            targets: names,
        });
    }

    fn delete_selected_remote_branches(&mut self) {
        let indices = self.remotes.selected_indices();
        let names: Vec<String> = indices
            .iter()
            .filter_map(|&i| {
                let b = &self.remotes.items()[i];
                if !b.is_pinned() {
                    Some(b.short_name.clone())
                } else {
                    None
                }
            })
            .collect();
        if names.is_empty() {
            return;
        }
        self.return_view = ViewId::Remotes;
        self.overlay = Some(Overlay::Confirm {
            action: BranchAction::DeleteRemoteBranch,
            targets: names,
        });
    }

    fn delete_selected_tags(&mut self, include_remote: bool) {
        let indices = self.tags.selected_indices();
        let names: Vec<String> = indices
            .iter()
            .map(|&i| self.tags.items()[i].name.clone())
            .collect();
        if names.is_empty() {
            return;
        }
        let action = if include_remote {
            BranchAction::DeleteTagAndRemote
        } else {
            BranchAction::DeleteTag
        };
        self.return_view = ViewId::Tags;
        self.overlay = Some(Overlay::Confirm {
            action,
            targets: names,
        });
    }

    fn push_selected_tags(&mut self) {
        let indices = self.tags.selected_indices();
        let names: Vec<String> = indices
            .iter()
            .map(|&i| self.tags.items()[i].name.clone())
            .collect();
        if names.is_empty() {
            return;
        }
        self.return_view = ViewId::Tags;
        self.overlay = Some(Overlay::Confirm {
            action: BranchAction::PushTag,
            targets: names,
        });
    }

    fn remove_selected_worktrees(&mut self, force: bool) {
        self.worktree_enrich_rx = None;
        let indices = self.worktrees.selected_indices();
        let names: Vec<String> = indices
            .iter()
            .filter_map(|&i| {
                let wt = &self.worktrees.items()[i];
                if !wt.is_main {
                    Some(wt.path.to_string_lossy().to_string())
                } else {
                    None
                }
            })
            .collect();
        if names.is_empty() {
            return;
        }
        let action = if force {
            BranchAction::WorktreeForceRemove
        } else {
            BranchAction::WorktreeRemove
        };
        self.return_view = ViewId::Worktrees;
        self.overlay = Some(Overlay::Confirm {
            action,
            targets: names,
        });
    }

    fn get_selected_branch_names(&self) -> Vec<String> {
        let indices = self.branches.selected_indices();
        indices
            .iter()
            .filter_map(|&i| {
                let b = &self.branches.items()[i];
                if !b.is_pinned() {
                    Some(b.name.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    // ---- Sorting ----

    fn cycle_sort(&mut self) {
        match self.active_view {
            ViewId::Branches => {
                list_state::cycle_sort_column(&mut self.branches, &self.branch_columns);
                list_state::apply_sort(&mut self.branches, &self.branch_columns);
            }
            ViewId::Remotes => {
                list_state::cycle_sort_column(&mut self.remotes, &self.remote_columns);
                list_state::apply_sort(&mut self.remotes, &self.remote_columns);
            }
            ViewId::Tags => {
                list_state::cycle_sort_column(&mut self.tags, &self.tag_columns);
                list_state::apply_sort(&mut self.tags, &self.tag_columns);
            }
            ViewId::Worktrees => {
                list_state::cycle_sort_column(&mut self.worktrees, &self.worktree_columns);
                list_state::apply_sort(&mut self.worktrees, &self.worktree_columns);
            }
        }
        self.save_sort_config();
    }

    fn toggle_sort_direction(&mut self) {
        match self.active_view {
            ViewId::Branches => {
                list_state::toggle_sort_direction(&mut self.branches);
                list_state::apply_sort(&mut self.branches, &self.branch_columns);
            }
            ViewId::Remotes => {
                list_state::toggle_sort_direction(&mut self.remotes);
                list_state::apply_sort(&mut self.remotes, &self.remote_columns);
            }
            ViewId::Tags => {
                list_state::toggle_sort_direction(&mut self.tags);
                list_state::apply_sort(&mut self.tags, &self.tag_columns);
            }
            ViewId::Worktrees => {
                list_state::toggle_sort_direction(&mut self.worktrees);
                list_state::apply_sort(&mut self.worktrees, &self.worktree_columns);
            }
        }
        self.save_sort_config();
    }

    // ---- View Loading ----

    fn ensure_view_loaded(&mut self) {
        match self.active_view {
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
            let candidates: Vec<(String, String)> = remote_branches
                .iter()
                .filter(|b| b.merge_status == MergeStatus::Pending && !b.is_base)
                .filter_map(|b| {
                    let refname = format!("refs/remotes/{}", b.full_ref);
                    repo.find_reference(&refname)
                        .ok()
                        .and_then(|r| r.peel_to_commit().ok())
                        .map(|c| (b.full_ref.clone(), c.id().to_string()))
                })
                .collect();

            let _ = tx.send((remote_branches, candidates, branch_cache));
        });
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

    pub fn refresh_branches(&mut self) {
        let repo_path = self.repo_path.clone();
        let base_branch = self.base_branch.clone();

        let Ok(repo) = git2::Repository::open(&repo_path) else {
            return;
        };
        let Ok(branches) = branch::list_branches_phase1(&repo, &base_branch) else {
            return;
        };

        let new_cache = cache::BranchCache::load(&repo_path);

        let candidates: Vec<(String, String)> = branches
            .iter()
            .filter(|b| b.merge_status == MergeStatus::Pending && !b.is_base && !b.is_current)
            .filter_map(|b| {
                branch::get_commit_hash(&repo, &b.name).map(|hash| (b.name.clone(), hash))
            })
            .collect();

        self.branches.set_items(branches);

        // Restore sort
        list_state::apply_sort(&mut self.branches, &self.branch_columns);

        // Spawn squash checker
        self.squash_total = candidates.len();
        self.squash_checked = 0;
        if !candidates.is_empty() {
            self.squash_rx = Some(squash_loader::spawn_squash_checker(
                repo_path.clone(),
                base_branch,
                candidates,
                new_cache,
            ));
        }

        // Re-spawn PR loader
        self.pr_rx = Some(pr_loader::spawn_pr_loader(repo_path));
    }

    fn refresh_after_operation(&mut self) {
        match self.return_view {
            ViewId::Branches => {
                self.refresh_branches();
                self.active_view = ViewId::Branches;
            }
            ViewId::Remotes => {
                // Reload remote branches
                self.remotes = ListState::empty();
                self.spawn_remote_load();
                self.active_view = ViewId::Remotes;
            }
            ViewId::Tags => {
                self.tags = ListState::empty();
                self.spawn_tag_load();
                self.active_view = ViewId::Tags;
            }
            ViewId::Worktrees => {
                self.worktrees = ListState::empty();
                self.spawn_worktree_load();
                self.active_view = ViewId::Worktrees;
            }
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
        self.refresh_branches();
        self.toast = Some(Toast::new("Cache cleared".into(), 3));
    }

    // ---- Config ----

    fn save_config(&mut self) {
        self.config.theme = Some(self.theme.name.to_string());
        self.config.symbols = Some(self.symbols.name.to_string());
        self.save_sort_config_only();
        self.config.save();
    }

    fn save_sort_config(&mut self) {
        self.save_sort_config_only();
        self.config.save();
    }

    fn save_sort_config_only(&mut self) {
        let sort_col = self.branches.sort_column();
        self.config.sort_column = sort_col.map(|c| {
            match c {
                0 => "name",
                1 => "remote",
                2 => "ahead",
                3 => "pr",
                4 => "age",
                5 => "status",
                _ => "name",
            }
            .to_string()
        });
        self.config.sort_asc = Some(self.branches.sort_ascending());
    }

    // ---- Filter helpers ----

    fn active_filter_query(&self) -> String {
        match self.active_view {
            ViewId::Branches => self.branches.filter_query().to_string(),
            ViewId::Remotes => self.remotes.filter_query().to_string(),
            ViewId::Tags => self.tags.filter_query().to_string(),
            ViewId::Worktrees => self.worktrees.filter_query().to_string(),
        }
    }

    fn set_active_filter(&mut self, query: String) {
        match self.active_view {
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

// ---- Action Execution (runs on background thread) ----

fn execute_action(
    action: BranchAction,
    item_names: &[String],
    repo_path: &Path,
    base_branch: &str,
    needs_stash: bool,
    prog_tx: &Sender<ProgressUpdate>,
    cancel_flag: &Arc<AtomicBool>,
) -> Vec<OperationResult> {
    let total = item_names.len();
    let mut results = Vec::new();

    match action {
        BranchAction::DeleteLocal | BranchAction::DeleteLocalAndRemote => {
            let repo = match git2::Repository::open(repo_path) {
                Ok(r) => r,
                Err(e) => {
                    return vec![OperationResult {
                        branch_name: String::new(),
                        action,
                        success: false,
                        message: format!("Failed to open repo: {e}"),
                    }];
                }
            };
            let mut locally_deleted = Vec::new();
            for (i, name) in item_names.iter().enumerate() {
                if cancel_flag.load(Ordering::Relaxed) {
                    results.push(OperationResult {
                        branch_name: String::new(),
                        action,
                        success: false,
                        message: "Cancelled by user".into(),
                    });
                    break;
                }
                let _ = prog_tx.send(ProgressUpdate {
                    completed: i,
                    total,
                    current_item: name.clone(),
                });
                let result = operations::delete_local(&repo, name);
                if result.success {
                    locally_deleted.push(name.clone());
                }
                results.push(result);
            }
            if action == BranchAction::DeleteLocalAndRemote && !locally_deleted.is_empty() {
                let _ = prog_tx.send(ProgressUpdate {
                    completed: locally_deleted.len(),
                    total,
                    current_item: "Deleting remote branches...".into(),
                });
                results.extend(operations::delete_remotes_batch(
                    repo_path,
                    &locally_deleted,
                    cancel_flag,
                ));
            }
        }
        BranchAction::Checkout => {
            if let Some(name) = item_names.first() {
                let repo = match git2::Repository::open(repo_path) {
                    Ok(r) => r,
                    Err(e) => {
                        return vec![OperationResult {
                            branch_name: name.clone(),
                            action,
                            success: false,
                            message: format!("Failed to open repo: {e}"),
                        }];
                    }
                };
                results.push(operations::checkout_branch(
                    &repo,
                    repo_path,
                    name,
                    needs_stash,
                ));
            }
        }
        BranchAction::FastForward => {
            if let Some(name) = item_names.first() {
                results.push(operations::fast_forward(repo_path, name, cancel_flag));
            }
        }
        BranchAction::Push => {
            for (i, name) in item_names.iter().enumerate() {
                if cancel_flag.load(Ordering::Relaxed) {
                    break;
                }
                let _ = prog_tx.send(ProgressUpdate {
                    completed: i,
                    total,
                    current_item: name.clone(),
                });
                results.push(operations::push_branch(repo_path, name, cancel_flag));
            }
        }
        BranchAction::ForcePush => {
            if let Some(name) = item_names.first() {
                results.push(operations::force_push_branch(repo_path, name, cancel_flag));
            }
        }
        BranchAction::Pull => {
            if let Some(name) = item_names.first() {
                // Assume not current for context menu; pull_branch handles it
                results.push(operations::pull_branch(repo_path, name, false, cancel_flag));
            }
        }
        BranchAction::Merge | BranchAction::SquashMerge => {
            if let Some(name) = item_names.first() {
                let squash = action == BranchAction::SquashMerge;
                results.extend(operations::merge_branch(
                    repo_path,
                    name,
                    base_branch,
                    squash,
                    needs_stash,
                ));
            }
        }
        BranchAction::Rebase => {
            if let Some(name) = item_names.first() {
                results.extend(operations::rebase_branch(
                    repo_path,
                    name,
                    base_branch,
                    needs_stash,
                ));
            }
        }
        BranchAction::Worktree => {
            if let Some(name) = item_names.first() {
                results.push(operations::create_worktree(repo_path, name));
            }
        }
        BranchAction::DeleteTag | BranchAction::DeleteTagAndRemote => {
            let repo = match git2::Repository::open(repo_path) {
                Ok(r) => r,
                Err(e) => {
                    return vec![OperationResult {
                        branch_name: String::new(),
                        action,
                        success: false,
                        message: format!("Failed to open repo: {e}"),
                    }];
                }
            };
            let tag_names: Vec<String> = item_names.to_vec();
            results.extend(tags::delete_tags_batch(&repo, &tag_names));
            if action == BranchAction::DeleteTagAndRemote {
                let successfully_deleted: Vec<String> = results
                    .iter()
                    .filter(|r| r.success)
                    .map(|r| r.branch_name.clone())
                    .collect();
                if !successfully_deleted.is_empty() {
                    results.extend(tags::delete_remote_tags_batch(
                        repo_path,
                        &successfully_deleted,
                    ));
                }
            }
        }
        BranchAction::PushTag => {
            for name in item_names {
                results.push(tags::push_tag(repo_path, name));
            }
        }
        BranchAction::DeleteRemoteBranch => {
            let short_names: Vec<String> = item_names.to_vec();
            results.extend(
                operations::delete_remotes_batch(repo_path, &short_names, cancel_flag)
                    .into_iter()
                    .map(|mut r| {
                        r.action = BranchAction::DeleteRemoteBranch;
                        r
                    }),
            );
        }
        BranchAction::DeleteRemoteAndLocal => {
            if let Some(name) = item_names.first() {
                let remote_results = operations::delete_remotes_batch(
                    repo_path,
                    std::slice::from_ref(name),
                    cancel_flag,
                );
                results.extend(remote_results.into_iter().map(|mut r| {
                    r.action = BranchAction::DeleteRemoteAndLocal;
                    r
                }));
                if let Ok(repo) = git2::Repository::open(repo_path) {
                    let local_result = operations::delete_local(&repo, name);
                    results.push(OperationResult {
                        action: BranchAction::DeleteRemoteAndLocal,
                        ..local_result
                    });
                }
            }
        }
        BranchAction::CheckoutRemote => {
            if let Some(name) = item_names.first() {
                results.push(operations::checkout_remote_branch(
                    repo_path, "origin", name,
                ));
            }
        }
        BranchAction::FetchRemote => {
            if let Some(name) = item_names.first() {
                results.extend(operations::fetch_remote(repo_path, name, cancel_flag));
            }
        }
        BranchAction::PullRemote => {
            if let Some(name) = item_names.first() {
                results.extend(operations::pull_remote(
                    repo_path,
                    "origin",
                    name,
                    cancel_flag,
                ));
            }
        }
        BranchAction::MergeRemoteIntoCurrent => {
            if let Some(name) = item_names.first() {
                let full_ref = format!("origin/{name}");
                results.extend(operations::merge_remote_into_current(
                    repo_path, &full_ref, name,
                ));
            }
        }
        BranchAction::CherryPickRemote => {
            if let Some(name) = item_names.first() {
                let full_ref = format!("origin/{name}");
                results.extend(operations::cherry_pick_remote(repo_path, &full_ref, name));
            }
        }
        BranchAction::WorktreeRemove | BranchAction::WorktreeForceRemove => {
            let force = action == BranchAction::WorktreeForceRemove;
            for (i, path_str) in item_names.iter().enumerate() {
                if cancel_flag.load(Ordering::Relaxed) {
                    break;
                }
                let _ = prog_tx.send(ProgressUpdate {
                    completed: i,
                    total,
                    current_item: path_str.clone(),
                });
                let wt_path = PathBuf::from(path_str);
                let result = if force {
                    operations::force_remove_worktree(repo_path, &wt_path)
                } else {
                    operations::remove_worktree(repo_path, &wt_path)
                };
                results.push(result);
            }
        }
        BranchAction::Fetch | BranchAction::FetchPrune => {
            let result = if action == BranchAction::FetchPrune {
                operations::fetch_prune(repo_path, cancel_flag)
            } else {
                operations::fetch(repo_path, cancel_flag)
            };
            results.push(result);
        }
        BranchAction::ViewRemotePR => {
            // Handled in execute_menu_action, shouldn't reach here
        }
    }

    // Send final progress
    let _ = prog_tx.send(ProgressUpdate {
        completed: results.iter().filter(|r| r.success).count().min(total),
        total,
        current_item: "Done".to_string(),
    });

    results
}

/// Get branch prefix style: extract prefix before first '/' and look up color.
fn branch_prefix_style(name: &str, theme: &Theme) -> Style {
    let prefix = name.split('/').next().unwrap_or(name);
    prefix_style(prefix, theme).unwrap_or_default()
}

// ---- Row Renderers ----

fn render_branch_row(
    item: &BranchInfo,
    _raw_idx: usize,
    _is_selected: bool,
    _is_cursor: bool,
    visible_cols: &[usize],
    ctx: &CellContext,
) -> Vec<Cell<'static>> {
    let mut cells = Vec::new();
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
                } else if !item.is_current {
                    match &item.merge_base_commit {
                        Some(hash) => format!(" ({} - {})", item.base_branch, hash),
                        None => String::new(),
                    }
                } else {
                    String::new()
                };
                let name = format!("{prefix}{}{suffix}", item.name);
                cells.push(Cell::from(Span::styled(name, style)));
            }
            1 => {
                // Remote tracking
                let text = match &item.tracking {
                    TrackingStatus::Tracked { remote_ref, gone } => {
                        if *gone {
                            "gone".to_string()
                        } else {
                            remote_ref.clone()
                        }
                    }
                    TrackingStatus::Local => "local".to_string(),
                };
                cells.push(Cell::from(Span::styled(text, theme.secondary_text)));
            }
            2 => {
                cells.push(ahead_behind_cell(item.ahead, item.behind, ctx));
            }
            3 => {
                cells.push(pr_cell(item.pr.as_ref(), ctx));
            }
            4 => {
                let age = if ctx.compact {
                    item.age_short()
                } else {
                    item.age_display()
                };
                cells.push(age_cell(age, &item.last_commit_date, ctx));
            }
            5 => {
                cells.push(merge_status_cell_for_branch(
                    &item.merge_status,
                    item.is_base,
                    ctx,
                ));
            }
            _ => cells.push(Cell::from("")),
        }
    }
    cells
}

fn render_remote_row(
    item: &RemoteBranchInfo,
    _raw_idx: usize,
    _is_selected: bool,
    _is_cursor: bool,
    visible_cols: &[usize],
    ctx: &CellContext,
) -> Vec<Cell<'static>> {
    let mut cells = Vec::new();
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
                cells.push(Cell::from(Span::styled(name, style)));
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
                cells.push(Cell::from(Span::styled(text, style)));
            }
            2 => {
                cells.push(ahead_behind_cell(item.ahead, item.behind, ctx));
            }
            3 => {
                cells.push(pr_cell(item.pr.as_ref(), ctx));
            }
            4 => {
                let age = if ctx.compact {
                    item.age_short()
                } else {
                    item.age_display()
                };
                cells.push(age_cell(age, &item.last_commit_date, ctx));
            }
            5 => {
                cells.push(merge_status_cell(&item.merge_status, ctx));
            }
            _ => cells.push(Cell::from("")),
        }
    }
    cells
}

fn render_tag_row(
    item: &TagInfo,
    _raw_idx: usize,
    _is_selected: bool,
    _is_cursor: bool,
    visible_cols: &[usize],
    ctx: &CellContext,
) -> Vec<Cell<'static>> {
    let mut cells = Vec::new();
    let theme = ctx.theme;

    for &col_idx in visible_cols {
        match col_idx {
            0 => {
                // Tag name
                let style = branch_prefix_style(&item.name, theme);
                cells.push(Cell::from(Span::styled(item.name.clone(), style)));
            }
            1 => {
                // Commit hash (short)
                let hash = if item.commit_hash.len() > 8 {
                    &item.commit_hash[..8]
                } else {
                    &item.commit_hash
                };
                cells.push(Cell::from(Span::styled(
                    hash.to_string(),
                    theme.secondary_text,
                )));
            }
            2 => {
                let age = if ctx.compact {
                    item.age_short()
                } else {
                    item.age_display()
                };
                cells.push(age_cell(age, &item.date, ctx));
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
                cells.push(Cell::from(Span::styled(text, theme.secondary_text)));
            }
            _ => cells.push(Cell::from("")),
        }
    }
    cells
}

fn render_worktree_row(
    item: &WorktreeInfo,
    _raw_idx: usize,
    _is_selected: bool,
    _is_cursor: bool,
    visible_cols: &[usize],
    ctx: &CellContext,
) -> Vec<Cell<'static>> {
    let mut cells = Vec::new();
    let theme = ctx.theme;

    for &col_idx in visible_cols {
        match col_idx {
            0 => {
                // Path
                let path_str = item.path.to_string_lossy().to_string();
                let style = if item.is_main {
                    theme.current_branch
                } else {
                    Style::default()
                };
                cells.push(Cell::from(Span::styled(path_str, style)));
            }
            1 => {
                // Branch name
                let name = item.branch.as_deref().unwrap_or("[detached]");
                let style = prefix_style(name, theme).unwrap_or(theme.primary_text);
                cells.push(Cell::from(Span::styled(name.to_string(), style)));
            }
            2 => {
                // Working tree status
                let text = if item.wt_status.is_clean() {
                    "clean".to_string()
                } else {
                    item.wt_status.summary()
                };
                let style = if item.wt_status.is_clean() {
                    theme.merged
                } else {
                    theme.unmerged
                };
                cells.push(Cell::from(Span::styled(text, style)));
            }
            3 => {
                let age = if ctx.compact {
                    item.age_short()
                } else {
                    item.age_display()
                };
                cells.push(age_cell(age, &item.age_date, ctx));
            }
            4 => {
                cells.push(merge_status_cell(&item.merge_status, ctx));
            }
            _ => cells.push(Cell::from("")),
        }
    }
    cells
}
