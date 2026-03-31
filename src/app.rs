use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, MouseButton, MouseEventKind};
use ratatui::DefaultTerminal;
use ratatui::widgets::TableState;

use git_branch_manager::git::{branch, cache, operations, pr_loader, squash_loader, status, tags, worktree};
use git_branch_manager::git::github::PrMap;
use git_branch_manager::git::tags::TagInfo;
use git_branch_manager::types::{BranchAction, BranchInfo, MergeStatus, OperationResult, ProgressUpdate, RemoteBranchInfo, SquashResult, TrackingStatus, WorkingTreeStatus, WorktreeInfo};
use crate::ui;
use crate::ui::symbols::SymbolSet;
use crate::ui::theme::Theme;

/// Trait for shared list navigation, selection, and mouse click logic.
trait ListNav {
    /// Display-ordered filtered indices (pinned first, then non-pinned).
    fn display_indices(&self) -> Vec<usize>;
    /// Current cursor position (raw index into the data array).
    fn cursor(&self) -> usize;
    /// Set cursor to a raw index and update table_state to the display position.
    fn set_cursor(&mut self, raw_idx: usize, display_pos: usize);
    /// The selection boolean vec.
    fn selection(&self) -> &[bool];
    /// Mutable access to selection vec.
    fn selection_mut(&mut self) -> &mut Vec<bool>;
    /// Whether the item at raw_idx can be selected (i.e. not pinned).
    fn is_selectable(&self, raw_idx: usize) -> bool;
    /// Merge status of item at raw_idx.
    fn merge_status(&self, raw_idx: usize) -> &MergeStatus;
}

/// Adapter for local branch list navigation.
struct BranchListNav<'a> {
    app: &'a mut App,
}

impl ListNav for BranchListNav<'_> {
    fn display_indices(&self) -> Vec<usize> {
        self.app.filtered_branch_indices()
    }
    fn cursor(&self) -> usize {
        self.app.cursor
    }
    fn set_cursor(&mut self, raw_idx: usize, display_pos: usize) {
        self.app.cursor = raw_idx;
        self.app.table_state.select(Some(display_pos));
    }
    fn selection(&self) -> &[bool] {
        &self.app.selected
    }
    fn selection_mut(&mut self) -> &mut Vec<bool> {
        &mut self.app.selected
    }
    fn is_selectable(&self, raw_idx: usize) -> bool {
        let b = &self.app.branches[raw_idx];
        !b.is_base && !b.is_current
    }
    fn merge_status(&self, raw_idx: usize) -> &MergeStatus {
        &self.app.branches[raw_idx].merge_status
    }
}

/// Adapter for remote branch list navigation.
struct RemoteListNav<'a> {
    app: &'a mut App,
}

impl ListNav for RemoteListNav<'_> {
    fn display_indices(&self) -> Vec<usize> {
        self.app.filtered_remote_indices()
    }
    #[allow(clippy::misnamed_getters)]
    fn cursor(&self) -> usize {
        self.app.remote_cursor
    }
    fn set_cursor(&mut self, raw_idx: usize, display_pos: usize) {
        self.app.remote_cursor = raw_idx;
        self.app.remote_table_state.select(Some(display_pos));
    }
    fn selection(&self) -> &[bool] {
        &self.app.remote_selected
    }
    fn selection_mut(&mut self) -> &mut Vec<bool> {
        &mut self.app.remote_selected
    }
    fn is_selectable(&self, raw_idx: usize) -> bool {
        !self.app.remote_branches[raw_idx].is_pinned()
    }
    fn merge_status(&self, raw_idx: usize) -> &MergeStatus {
        &self.app.remote_branches[raw_idx].merge_status
    }
}

/// Adapter for worktree list navigation.
struct WorktreeListNav<'a> {
    app: &'a mut App,
}

impl ListNav for WorktreeListNav<'_> {
    fn display_indices(&self) -> Vec<usize> {
        let mut pinned: Vec<usize> = Vec::new();
        let mut rest: Vec<usize> = Vec::new();
        for (i, wt) in self.app.worktrees.iter().enumerate() {
            if wt.is_pinned() {
                pinned.push(i);
            } else {
                rest.push(i);
            }
        }
        pinned.extend(rest);
        pinned
    }
    fn cursor(&self) -> usize {
        self.app.worktree_cursor
    }
    fn set_cursor(&mut self, raw_idx: usize, display_pos: usize) {
        self.app.worktree_cursor = raw_idx;
        self.app.worktree_table_state.select(Some(display_pos));
    }
    fn selection(&self) -> &[bool] {
        &self.app.worktree_selected
    }
    fn selection_mut(&mut self) -> &mut Vec<bool> {
        &mut self.app.worktree_selected
    }
    fn is_selectable(&self, raw_idx: usize) -> bool {
        !self.app.worktrees[raw_idx].is_pinned()
    }
    fn merge_status(&self, raw_idx: usize) -> &MergeStatus {
        &self.app.worktrees[raw_idx].merge_status
    }
}

/// Payload sent from the background initial-load thread.
pub struct InitialLoad {
    pub branches: Vec<BranchInfo>,
    pub working_tree_status: WorkingTreeStatus,
    pub candidates: Vec<(String, String)>,
    pub cache: cache::BranchCache,
    /// True if a `git fetch` was performed as part of this load (auto_fetch on startup).
    pub did_fetch: bool,
}

/// Progress message sent from the background load thread.
pub struct LoadProgress {
    pub message: String,
}

/// Payload sent from the background remote-branch-load thread.
/// Payload sent from the background tag-load thread.
pub(crate) struct TagLoad {
    pub tags: Vec<TagInfo>,
}

pub(crate) struct RemoteLoad {
    remote_branches: Vec<RemoteBranchInfo>,
    candidates: Vec<(String, String)>,
    cache: cache::BranchCache,
}

pub(crate) struct WorktreeLoad {
    pub worktrees: Vec<WorktreeInfo>,
}

pub(crate) struct WorktreeEnrich {
    pub worktrees: Vec<WorktreeInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum View {
    BranchList,
    Confirm { action: BranchAction },
    Executing,
    Results,
    Help,
    Menu { cursor: usize },
    Tags,
    Settings { cursor: usize },
    Filter,
    TagFilter,
    RemoteBranches,
    RemoteFilter,
    Worktrees,
}

pub struct App {
    pub base_branch: String,
    pub repo_path: PathBuf,
    pub branches: Vec<BranchInfo>,
    pub view: View,
    pub cursor: usize,
    pub selected: Vec<bool>,
    pub list_scroll_offset: usize,
    pub results: Vec<OperationResult>,
    pub should_exit: bool,
    pub squash_rx: Option<Receiver<SquashResult>>,
    pub squash_checked: usize,
    pub squash_total: usize,
    pub working_tree_status: WorkingTreeStatus,
    pub table_state: TableState,
    pub symbols: &'static SymbolSet,
    pub sort_column: Option<usize>,  // 0=name, 1=age, 2=ahead, 3=status
    pub sort_ascending: bool,
    pub search_query: String,
    pub search_active: bool,
    pub tags: Vec<TagInfo>,
    pub tag_cursor: usize,
    pub tag_table_state: TableState,
    pub tag_selected: Vec<bool>,
    pub tag_search_query: String,
    pub tag_search_active: bool,
    pub tag_sort_by_name: bool,
    /// True while background tag loading is in progress.
    pub tag_loading: bool,
    /// Receiver for background tag loading.
    pub tag_load_rx: Option<Receiver<TagLoad>>,
    /// Which view to return to after the Results screen (BranchList or Tags).
    pub results_return_view: ResultsReturnView,
    /// Column header x-ranges for mouse click sorting: (x_start, sort_column_index).
    /// Populated during branch_list rendering. The last entry extends to the end of the row.
    pub header_columns: Vec<(u16, usize)>,
    /// Status bar clickable items: (x_start, x_end_exclusive, key_to_simulate).
    /// Populated during branch_list rendering.
    pub status_bar_items: Vec<(u16, u16, KeyCode)>,
    /// Terminal height in rows, updated each frame. Used to detect status bar row clicks.
    pub terminal_rows: u16,
    /// GitHub PR numbers keyed by branch name.
    pub pr_map: PrMap,
    /// Receiver for background PR data fetch. Receives exactly one PrMap, then closes.
    pub pr_rx: Option<Receiver<PrMap>>,
    /// Active color theme.
    pub theme: Theme,
    /// Persisted configuration (used for saving sort state and other settings).
    pub config: git_branch_manager::config::Config,
    /// Receiver for background git operation results.
    pub op_rx: Option<Receiver<Vec<OperationResult>>>,
    /// Description of the currently executing operation (shown in the Executing view).
    pub executing_label: String,
    /// Receiver for per-item progress updates from background operations.
    pub progress_rx: Option<Receiver<ProgressUpdate>>,
    /// Current progress state (updated each tick from progress_rx).
    pub progress: Option<ProgressUpdate>,
    /// Shared cancellation flag: set to true when user presses Esc during Executing.
    pub cancel_flag: Option<Arc<AtomicBool>>,
    /// True while the initial branch load is in progress.
    pub loading: bool,
    /// Receiver for the initial background load (branches + working tree status).
    pub load_rx: Option<Receiver<InitialLoad>>,
    /// Receiver for loading progress messages (e.g. "Fetching from remote...").
    pub load_progress_rx: Option<Receiver<LoadProgress>>,
    /// Current loading status message shown on the loading screen.
    pub loading_message: String,
    // ── Remote branches state ──
    pub remote_branches: Vec<RemoteBranchInfo>,
    pub remote_cursor: usize,
    pub remote_selected: Vec<bool>,
    pub remote_table_state: TableState,
    pub remote_search_query: String,
    pub remote_search_active: bool,
    pub remote_sort_column: Option<usize>,
    pub remote_sort_ascending: bool,
    pub remote_squash_rx: Option<Receiver<SquashResult>>,
    pub remote_squash_checked: usize,
    pub remote_squash_total: usize,
    /// Whether `git fetch` has been run this session (lazy fetch on first open).
    pub remote_fetched: bool,
    /// Whether the remote branches view is currently loading (fetch in progress).
    pub remote_loading: bool,
    /// Receiver for background `git fetch` completion. When present, a fetch is in progress.
    pub remote_fetch_rx: Option<Receiver<bool>>,
    /// Column header x-ranges for mouse click sorting in remote branches view.
    pub remote_header_columns: Vec<(u16, usize)>,
    /// Status bar clickable items for remote branches view.
    pub remote_status_bar_items: Vec<(u16, u16, KeyCode)>,
    /// Receiver for background remote branch loading (phase 1 enumeration).
    pub remote_load_rx: Option<Receiver<RemoteLoad>>,
    // ── Worktrees state ──
    pub worktrees: Vec<WorktreeInfo>,
    pub worktree_cursor: usize,
    pub worktree_table_state: TableState,
    pub worktree_selected: Vec<bool>,
    pub worktree_load_rx: Option<Receiver<WorktreeLoad>>,
    pub worktree_enrich_rx: Option<Receiver<WorktreeEnrich>>,
    pub worktree_loading: bool,
    /// Status bar clickable items for worktrees view.
    pub worktree_status_bar_items: Vec<(u16, u16, KeyCode)>,
    pub worktree_sort_column: Option<usize>,  // 0=branch, 1=path, 2=age, 3=status
    pub worktree_sort_ascending: bool,
    /// The view that was active before entering View::Menu — used to dispatch the right menu.
    pub prev_view: View,
    // ── Timing instrumentation (GBM_TIMING=1) ──
    timing_enabled: bool,
    timing_file: Option<std::fs::File>,
    timing_start: Option<Instant>,
    timing_key_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResultsReturnView {
    BranchList,
    Tags,
    RemoteBranches,
    Worktrees,
}

impl App {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        base_branch: String,
        repo_path: PathBuf,
        symbols: &'static SymbolSet,
        theme: Theme,
        config: git_branch_manager::config::Config,
        load_rx: Receiver<InitialLoad>,
        load_progress_rx: Receiver<LoadProgress>,
    ) -> Self {
        let init_sort_col: Option<usize> = config.sort_column.as_deref().and_then(|s| match s {
            "name" => Some(0),
            "age" => Some(1),
            "ahead" => Some(2),
            "behind" => Some(3),
            "status" => Some(4),
            _ => None,
        });
        let init_sort_asc: bool = config.sort_asc.unwrap_or(true);
        // If auto_fetch is enabled, a fetch is already in progress on startup — mark as fetched
        // immediately so switching to Remote Branches view doesn't trigger a redundant second fetch.
        let remote_fetched = config.auto_fetch == Some(true);

        Self {
            base_branch,
            repo_path,
            branches: Vec::new(),
            view: View::BranchList,
            cursor: 0,
            selected: Vec::new(),
            list_scroll_offset: 0,
            results: Vec::new(),
            should_exit: false,
            squash_rx: None,
            squash_checked: 0,
            squash_total: 0,
            working_tree_status: WorkingTreeStatus::clean(),
            table_state: TableState::default().with_selected(Some(0)),
            symbols,
            sort_column: init_sort_col,
            sort_ascending: init_sort_asc,
            search_query: String::new(),
            search_active: false,
            tags: Vec::new(),
            tag_cursor: 0,
            tag_table_state: TableState::default(),
            tag_selected: Vec::new(),
            tag_search_query: String::new(),
            tag_search_active: false,
            tag_sort_by_name: false,
            tag_loading: false,
            tag_load_rx: None,
            results_return_view: ResultsReturnView::BranchList,
            header_columns: Vec::new(),
            status_bar_items: Vec::new(),
            terminal_rows: 0,
            pr_map: HashMap::new(),
            pr_rx: None,
            theme,
            config,
            op_rx: None,
            executing_label: String::new(),
            progress_rx: None,
            progress: None,
            cancel_flag: None,
            loading: true,
            load_rx: Some(load_rx),
            load_progress_rx: Some(load_progress_rx),
            loading_message: "Loading...".into(),
            remote_branches: Vec::new(),
            remote_cursor: 0,
            remote_selected: Vec::new(),
            remote_table_state: TableState::default(),
            remote_search_query: String::new(),
            remote_search_active: false,
            remote_sort_column: None,
            remote_sort_ascending: true,
            remote_squash_rx: None,
            remote_squash_checked: 0,
            remote_squash_total: 0,
            remote_fetched,
            remote_loading: false,
            remote_fetch_rx: None,
            remote_header_columns: Vec::new(),
            remote_status_bar_items: Vec::new(),
            remote_load_rx: None,
            worktrees: Vec::new(),
            worktree_cursor: 0,
            worktree_table_state: TableState::default(),
            worktree_selected: Vec::new(),
            worktree_load_rx: None,
            worktree_enrich_rx: None,
            worktree_loading: false,
            worktree_status_bar_items: Vec::new(),
            worktree_sort_column: None,
            worktree_sort_ascending: true,
            prev_view: View::BranchList,
            timing_enabled: std::env::var("GBM_TIMING").map_or(false, |v| v == "1"),
            timing_file: std::env::var("GBM_TIMING")
                .ok()
                .filter(|v| v == "1")
                .and_then(|_| std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open("key_timing.log")
                    .ok()),
            timing_start: None,
            timing_key_name: None,
        }
    }

    fn sort_col_name(col: usize) -> &'static str {
        match col {
            0 => "name",
            1 => "age",
            2 => "ahead",
            3 => "behind",
            4 => "status",
            _ => "name",
        }
    }

    /// Returns true if any background loading is currently in progress.
    fn is_any_loading(&self) -> bool {
        self.loading
            || self.tag_loading
            || self.remote_loading
            || self.worktree_loading
            || self.squash_rx.is_some()
            || self.remote_squash_rx.is_some()
            || self.pr_rx.is_some()
            || self.remote_fetch_rx.is_some()
            || self.load_rx.is_some()
            || self.remote_load_rx.is_some()
            || self.worktree_load_rx.is_some()
            || self.worktree_enrich_rx.is_some()
            || self.tag_load_rx.is_some()
            || self.op_rx.is_some()
    }

    /// Start a timing measurement for the given key name.
    fn timing_start(&mut self, key: KeyCode) {
        if !self.timing_enabled { return; }
        self.timing_start = Some(Instant::now());
        self.timing_key_name = Some(Self::key_name(key));
    }

    /// If a timing measurement is pending and all loading is finished, log it.
    fn timing_check_and_log(&mut self) {
        if !self.timing_enabled { return; }
        let (Some(start), Some(name)) = (self.timing_start, &self.timing_key_name) else { return };
        if self.is_any_loading() { return; }
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        let name = name.clone();
        if let Some(ref mut f) = self.timing_file {
            let _ = writeln!(f, "{}\t{:.3}", name, elapsed_ms);
        }
        self.timing_start = None;
        self.timing_key_name = None;
    }

    fn key_name(key: KeyCode) -> String {
        match key {
            KeyCode::Char(c) => format!("{}", c),
            KeyCode::Enter => "Enter".into(),
            KeyCode::Esc => "Esc".into(),
            KeyCode::Tab => "Tab".into(),
            KeyCode::BackTab => "BackTab".into(),
            KeyCode::Backspace => "Backspace".into(),
            KeyCode::Up => "Up".into(),
            KeyCode::Down => "Down".into(),
            KeyCode::Left => "Left".into(),
            KeyCode::Right => "Right".into(),
            KeyCode::Home => "Home".into(),
            KeyCode::End => "End".into(),
            KeyCode::PageUp => "PageUp".into(),
            KeyCode::PageDown => "PageDown".into(),
            KeyCode::Delete => "Delete".into(),
            KeyCode::F(n) => format!("F{}", n),
            _ => format!("{:?}", key),
        }
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        crossterm::execute!(std::io::stdout(), crossterm::event::EnableMouseCapture)?;
        while !self.should_exit {
            self.drain_load_rx();
            self.drain_tag_load_rx();
            self.drain_remote_load_rx();
            self.drain_worktree_load_rx();
            self.drain_worktree_enrich_rx();
            self.drain_squash_rx();
            self.drain_remote_squash_rx();
            self.drain_pr_rx();
            self.drain_progress_rx();
            self.drain_op_rx();
            terminal.draw(|frame| ui::render::draw(frame, self))?;
            self.timing_check_and_log();

            // Check if background remote fetch completed
            if let Some(rx) = &self.remote_fetch_rx
                && let Ok(success) = rx.try_recv()
            {
                self.remote_fetch_rx = None;
                if success {
                    self.remote_fetched = true;
                    // Reload branches with updated remote refs
                    if self.view == View::RemoteBranches {
                        self.populate_remote_branches();
                    } else {
                        self.remote_loading = false;
                    }
                } else {
                    // Fetch failed or timed out — clear loading state
                    self.remote_loading = false;
                }
            }

            if event::poll(Duration::from_millis(50))? {
                let ev = event::read()?;
                self.handle_event(ev);
            }
        }
        crossterm::execute!(std::io::stdout(), crossterm::event::DisableMouseCapture)?;
        Ok(())
    }

    fn handle_event(&mut self, event: Event) {
        match event {
            Event::Key(key) => {
                if key.kind != KeyEventKind::Press {
                    return;
                }

                self.timing_start(key.code);

                // Search input takes priority over all other key handlers
                if self.tag_search_active && matches!(self.view, View::Tags) {
                    self.handle_tag_search_key(key.code);
                    return;
                }
                if self.remote_search_active && matches!(self.view, View::RemoteBranches) {
                    self.handle_remote_search_key(key.code);
                    return;
                }
                if self.search_active {
                    self.handle_search_key(key.code);
                    return;
                }

                match &self.view {
                    View::BranchList => self.handle_branch_list_key(key.code),
                    View::Confirm { .. } => self.handle_confirm_key(key.code),
                    View::Executing => self.handle_executing_key(key.code),
                    View::Results => self.handle_results_key(key.code),
                    View::Help => self.handle_help_key(key.code),
                    View::Menu { .. } => self.handle_menu_key(key.code),
                    View::Tags => self.handle_tags_key(key.code),
                    View::Settings { .. } => self.handle_settings_key(key.code),
                    View::Filter => self.handle_filter_key(key.code),
                    View::TagFilter => self.handle_tag_filter_key(key.code),
                    View::RemoteBranches => self.handle_remote_branches_key(key.code),
                    View::RemoteFilter => self.handle_remote_filter_key(key.code),
                    View::Worktrees => self.handle_worktrees_key(key.code),
                }
            }
            Event::Mouse(mouse) => {
                match mouse.kind {
                    MouseEventKind::Down(MouseButton::Left) => {
                        self.handle_mouse_click(mouse.column, mouse.row);
                    }
                    MouseEventKind::Down(MouseButton::Right) => {
                        self.handle_mouse_right_click(mouse.column, mouse.row);
                    }
                    MouseEventKind::ScrollDown => {
                        if self.view == View::BranchList {
                            self.handle_branch_list_key(KeyCode::Down);
                        } else if self.view == View::Tags {
                            self.handle_tags_key(KeyCode::Down);
                        } else if self.view == View::RemoteBranches {
                            self.handle_remote_branches_key(KeyCode::Down);
                        } else if self.view == View::Worktrees {
                            self.handle_worktrees_key(KeyCode::Down);
                        }
                    }
                    MouseEventKind::ScrollUp => {
                        if self.view == View::BranchList {
                            self.handle_branch_list_key(KeyCode::Up);
                        } else if self.view == View::Tags {
                            self.handle_tags_key(KeyCode::Up);
                        } else if self.view == View::RemoteBranches {
                            self.handle_remote_branches_key(KeyCode::Up);
                        } else if self.view == View::Worktrees {
                            self.handle_worktrees_key(KeyCode::Up);
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn handle_mouse_click(&mut self, x: u16, y: u16) {
        if self.view == View::BranchList {
            // Layout:
            //   y=0 — outer border top
            //   y=1 — header row (column titles)
            //   y=2+ — branch data rows (first data row at y=2)
            if y == 1 && !self.header_columns.is_empty() {
                // Header row click — determine which sort column was clicked
                let mut clicked_col: Option<usize> = None;
                for (i, &(col_x, sort_idx)) in self.header_columns.iter().enumerate() {
                    let next_x = if i + 1 < self.header_columns.len() {
                        self.header_columns[i + 1].0
                    } else {
                        u16::MAX
                    };
                    if x >= col_x && x < next_x {
                        clicked_col = Some(sort_idx);
                        break;
                    }
                }

                if let Some(col) = clicked_col {
                    if self.sort_column == Some(col) {
                        self.sort_ascending = !self.sort_ascending;
                    } else {
                        self.sort_column = Some(col);
                        self.sort_ascending = true;
                    }
                    self.apply_sort();
                    self.config.sort_column = self.sort_column.map(|c| Self::sort_col_name(c).to_string());
                    self.config.sort_asc = Some(self.sort_ascending);
                    self.config.save();
                }
            } else if self.terminal_rows > 0 && y == self.terminal_rows - 1 && !self.status_bar_items.is_empty() {
                // Status bar row click — look up which item was clicked and simulate its key
                for &(x_start, x_end, key) in &self.status_bar_items.clone() {
                    if x >= x_start && x < x_end {
                        self.handle_branch_list_key(key);
                        break;
                    }
                }
            } else if y >= 2 {
                let scroll_offset = self.table_state.offset();
                let clicked_display_row = (y - 2) as usize + scroll_offset;
                list_click_row(&mut BranchListNav { app: self }, clicked_display_row);
            }
        } else if self.view == View::RemoteBranches {
            if y == 1 && !self.remote_header_columns.is_empty() {
                let mut clicked_col: Option<usize> = None;
                for (i, &(col_x, sort_idx)) in self.remote_header_columns.iter().enumerate() {
                    let next_x = if i + 1 < self.remote_header_columns.len() {
                        self.remote_header_columns[i + 1].0
                    } else {
                        u16::MAX
                    };
                    if x >= col_x && x < next_x {
                        clicked_col = Some(sort_idx);
                        break;
                    }
                }

                if let Some(col) = clicked_col {
                    if self.remote_sort_column == Some(col) {
                        self.remote_sort_ascending = !self.remote_sort_ascending;
                    } else {
                        self.remote_sort_column = Some(col);
                        self.remote_sort_ascending = true;
                    }
                    self.apply_remote_sort();
                }
            } else if self.terminal_rows > 0 && y == self.terminal_rows - 1 && !self.remote_status_bar_items.is_empty() {
                for &(x_start, x_end, key) in &self.remote_status_bar_items.clone() {
                    if x >= x_start && x < x_end {
                        self.handle_remote_branches_key(key);
                        break;
                    }
                }
            } else if y >= 2 {
                let scroll_offset = self.remote_table_state.offset();
                let clicked_display_row = (y - 2) as usize + scroll_offset;
                list_click_row(&mut RemoteListNav { app: self }, clicked_display_row);
            }
        } else if self.view == View::Worktrees {
            if self.terminal_rows > 0 && y == self.terminal_rows - 1 && !self.worktree_status_bar_items.is_empty() {
                for &(x_start, x_end, key) in &self.worktree_status_bar_items.clone() {
                    if x >= x_start && x < x_end {
                        self.handle_worktrees_key(key);
                        break;
                    }
                }
            } else if y >= 2 {
                let scroll_offset = self.worktree_table_state.offset();
                let clicked_display_row = (y - 2) as usize + scroll_offset;
                list_click_row(&mut WorktreeListNav { app: self }, clicked_display_row);
            }
        }
    }

    fn handle_mouse_right_click(&mut self, _x: u16, y: u16) {
        if self.view == View::BranchList {
            if y >= 2 {
                let scroll_offset = self.table_state.offset();
                let clicked_display_row = (y - 2) as usize + scroll_offset;
                if list_right_click_row(&mut BranchListNav { app: self }, clicked_display_row) {
                    self.view = View::Menu { cursor: 0 };
                }
            }
        } else if self.view == View::RemoteBranches && y >= 2 {
            let scroll_offset = self.remote_table_state.offset();
            let clicked_display_row = (y - 2) as usize + scroll_offset;
            list_right_click_row(&mut RemoteListNav { app: self }, clicked_display_row);
        }
    }

    fn handle_branch_list_key(&mut self, code: KeyCode) {
        let filtered = self.filtered_branch_indices();
        if filtered.is_empty() {
            match code {
                KeyCode::Char('q') => self.should_exit = true,
                KeyCode::Tab => self.next_tab(),
                KeyCode::BackTab => self.prev_tab(),
                _ => {}
            }
            return;
        }

        match code {
            KeyCode::Char('j') | KeyCode::Down => nav_down(&mut BranchListNav { app: self }),
            KeyCode::Char('k') | KeyCode::Up => nav_up(&mut BranchListNav { app: self }),
            KeyCode::Char(' ') => select_toggle(&mut BranchListNav { app: self }),
            KeyCode::Char('a') => select_all(&mut BranchListNav { app: self }),
            KeyCode::Char('n') => deselect_all(&mut BranchListNav { app: self }),
            KeyCode::Char('m') => select_merged(&mut BranchListNav { app: self }),
            KeyCode::Char('i') => invert_selection(&mut BranchListNav { app: self }),
            KeyCode::Char('d') => {
                if self.has_selection() {
                    self.view = View::Confirm {
                        action: BranchAction::DeleteLocal,
                    };
                }
            }
            KeyCode::Char('D') => {
                if self.has_selection() {
                    self.view = View::Confirm {
                        action: BranchAction::DeleteLocalAndRemote,
                    };
                }
            }
            KeyCode::Char('R') => {
                let mut branch_cache = cache::BranchCache::load(&self.repo_path);
                branch_cache.clear();
                self.refresh_branches();
            }
            KeyCode::Char('x') => {
                let branch = &self.branches[self.cursor];
                if !branch.is_base && !branch.is_current {
                    self.view = View::Confirm {
                        action: BranchAction::DeleteLocal,
                    };
                }
            }
            KeyCode::Char('c') => {
                let branch = &self.branches[self.cursor];
                if !branch.is_current && !branch.is_base {
                    self.view = View::Confirm {
                        action: BranchAction::Checkout,
                    };
                }
            }
            KeyCode::Char('f') => {
                self.spawn_op("Fetching...".into(), {
                    let repo_path = self.repo_path.clone();
                    move || vec![operations::fetch(&repo_path)]
                });
            }
            KeyCode::Char('F') => {
                self.spawn_op("Fetching with prune...".into(), {
                    let repo_path = self.repo_path.clone();
                    move || vec![operations::fetch_prune(&repo_path)]
                });
            }
            KeyCode::Enter => {
                self.prev_view = View::BranchList;
                self.view = View::Menu { cursor: 0 };
            }
            KeyCode::Char('s') => {
                self.sort_column = Some(match self.sort_column {
                    Some(c) => (c + 1) % 5,
                    None => 0,
                });
                self.sort_ascending = true;
                self.apply_sort();
                self.config.sort_column = self.sort_column.map(|c| Self::sort_col_name(c).to_string());
                self.config.sort_asc = Some(self.sort_ascending);
                self.config.save();
            }
            KeyCode::Char('S') => {
                self.sort_ascending = !self.sort_ascending;
                self.apply_sort();
                self.config.sort_column = self.sort_column.map(|c| Self::sort_col_name(c).to_string());
                self.config.sort_asc = Some(self.sort_ascending);
                self.config.save();
            }
            KeyCode::Char('t') => {
                self.tag_cursor = 0;
                self.tag_search_query.clear();
                self.tag_search_active = false;
                self.tag_sort_by_name = false;
                self.load_tags();
            }
            KeyCode::Char('r') => {
                self.open_remote_branches_view();
            }
            KeyCode::Char('w') => {
                self.open_worktrees_view();
            }
            KeyCode::Char('/') => {
                self.search_active = true;
            }
            KeyCode::Char('\\') => {
                self.view = View::Filter;
            }
            KeyCode::PageDown => nav_page_down(&mut BranchListNav { app: self }),
            KeyCode::PageUp => nav_page_up(&mut BranchListNav { app: self }),
            KeyCode::Home => nav_home(&mut BranchListNav { app: self }),
            KeyCode::End => nav_end(&mut BranchListNav { app: self }),
            KeyCode::Char('T') => {
                self.theme = self.theme.next();
                let mut config = git_branch_manager::config::Config::load();
                config.theme = Some(self.theme.name.to_string());
                config.save();
            }
            KeyCode::Char('Y') => {
                self.symbols = crate::ui::symbols::next(self.symbols);
                let mut config = git_branch_manager::config::Config::load();
                config.symbols = Some(crate::ui::symbols::name(self.symbols).to_string());
                config.save();
            }
            KeyCode::Char('?') => {
                self.view = View::Help;
            }
            KeyCode::Char(',') => {
                self.view = View::Settings { cursor: 0 };
            }
            KeyCode::Char('q') => {
                self.should_exit = true;
            }
            KeyCode::Tab => self.next_tab(),
            KeyCode::BackTab => self.prev_tab(),
            _ => {}
        }
    }

    fn handle_confirm_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('y') => {
                let is_tag_action = matches!(
                    &self.view,
                    View::Confirm { action } if matches!(action, BranchAction::DeleteTag | BranchAction::DeleteTagAndRemote | BranchAction::PushTag)
                );
                let is_remote_action = matches!(
                    &self.view,
                    View::Confirm { action } if matches!(action, BranchAction::DeleteRemoteBranch | BranchAction::CheckoutRemote)
                );
                let is_worktree_action = matches!(
                    &self.view,
                    View::Confirm { action } if matches!(action,
                        BranchAction::WorktreeRemove | BranchAction::WorktreeForceRemove
                    )
                );
                if is_tag_action {
                    self.results_return_view = ResultsReturnView::Tags;
                } else if is_remote_action {
                    self.results_return_view = ResultsReturnView::RemoteBranches;
                } else if is_worktree_action {
                    self.results_return_view = ResultsReturnView::Worktrees;
                }
                self.execute_action_async();
            }
            KeyCode::Char('n') | KeyCode::Esc => {
                // Return to the appropriate view
                let is_tag_action = matches!(
                    &self.view,
                    View::Confirm { action } if matches!(action, BranchAction::DeleteTag | BranchAction::DeleteTagAndRemote | BranchAction::PushTag)
                );
                let is_remote_action = matches!(
                    &self.view,
                    View::Confirm { action } if matches!(action, BranchAction::DeleteRemoteBranch | BranchAction::CheckoutRemote)
                );
                let is_worktree_action = matches!(
                    &self.view,
                    View::Confirm { action } if matches!(action,
                        BranchAction::WorktreeRemove | BranchAction::WorktreeForceRemove
                    )
                );
                if is_tag_action {
                    self.view = View::Tags;
                } else if is_remote_action {
                    self.view = View::RemoteBranches;
                } else if is_worktree_action {
                    self.view = View::Worktrees;
                } else {
                    self.view = View::BranchList;
                }
            }
            _ => {}
        }
    }

    fn handle_executing_key(&mut self, code: KeyCode) {
        if let KeyCode::Esc = code
            && let Some(flag) = &self.cancel_flag
        {
            flag.store(true, Ordering::Relaxed);
        }
    }

    fn handle_results_key(&mut self, _code: KeyCode) {
        match self.results_return_view {
            ResultsReturnView::Tags => {
                self.results.clear();
                self.results_return_view = ResultsReturnView::BranchList;
                self.load_tags();
            }
            ResultsReturnView::BranchList => {
                self.refresh_branches();
                self.view = View::BranchList;
            }
            ResultsReturnView::RemoteBranches => {
                self.open_remote_branches_view();
            }
            ResultsReturnView::Worktrees => {
                for result in &self.results {
                    if result.success
                        && matches!(
                            result.action,
                            BranchAction::WorktreeRemove | BranchAction::WorktreeForceRemove
                        )
                    {
                        let removed_path = result.branch_name.clone();
                        self.worktrees.retain(|wt| wt.path.to_string_lossy() != removed_path);
                    }
                }
                self.results.clear();
                self.results_return_view = ResultsReturnView::BranchList;
                self.worktree_cursor = self.worktree_cursor.min(self.worktrees.len().saturating_sub(1));
                self.worktree_table_state.select(
                    if self.worktrees.is_empty() { None } else { Some(self.worktree_cursor) },
                );
                self.view = View::Worktrees;
            }
        }
    }

    fn handle_help_key(&mut self, _code: KeyCode) {
        self.view = View::BranchList;
    }

    fn handle_filter_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc | KeyCode::Char('\\') => {
                self.view = View::BranchList;
            }
            // Status toggles
            KeyCode::Char('m') => {
                self.search_query =
                    FilterSet::toggle_token(&self.search_query, "status:merged");
                self.reset_cursor_to_first_match();
                self.view = View::BranchList;
            }
            KeyCode::Char('s') => {
                self.search_query =
                    FilterSet::toggle_token(&self.search_query, "status:squash");
                self.reset_cursor_to_first_match();
                self.view = View::BranchList;
            }
            KeyCode::Char('u') => {
                self.search_query =
                    FilterSet::toggle_token(&self.search_query, "status:unmerged");
                self.reset_cursor_to_first_match();
                self.view = View::BranchList;
            }
            // PR toggles
            KeyCode::Char('p') => {
                self.search_query =
                    FilterSet::toggle_token(&self.search_query, "pr:yes");
                self.reset_cursor_to_first_match();
                self.view = View::BranchList;
            }
            KeyCode::Char('P') => {
                self.search_query =
                    FilterSet::toggle_token(&self.search_query, "pr:no");
                self.reset_cursor_to_first_match();
                self.view = View::BranchList;
            }
            // Sync toggles
            KeyCode::Char('a') => {
                self.search_query =
                    FilterSet::toggle_token(&self.search_query, "sync:ahead");
                self.reset_cursor_to_first_match();
                self.view = View::BranchList;
            }
            KeyCode::Char('b') => {
                self.search_query =
                    FilterSet::toggle_token(&self.search_query, "sync:behind");
                self.reset_cursor_to_first_match();
                self.view = View::BranchList;
            }
            // Age presets
            KeyCode::Char('1') => {
                self.search_query =
                    FilterSet::toggle_token(&self.search_query, "age:<7d");
                self.reset_cursor_to_first_match();
                self.view = View::BranchList;
            }
            KeyCode::Char('2') => {
                self.search_query =
                    FilterSet::toggle_token(&self.search_query, "age:<30d");
                self.reset_cursor_to_first_match();
                self.view = View::BranchList;
            }
            KeyCode::Char('3') => {
                self.search_query =
                    FilterSet::toggle_token(&self.search_query, "age:>30d");
                self.reset_cursor_to_first_match();
                self.view = View::BranchList;
            }
            KeyCode::Char('4') => {
                self.search_query =
                    FilterSet::toggle_token(&self.search_query, "age:>90d");
                self.reset_cursor_to_first_match();
                self.view = View::BranchList;
            }
            // Custom age — drop into search bar
            KeyCode::Char('n') => {
                let trimmed = self.search_query.trim().to_string();
                self.search_query = if trimmed.is_empty() {
                    "age:<".into()
                } else {
                    format!("{} age:<", trimmed)
                };
                self.search_active = true;
                self.view = View::BranchList;
            }
            KeyCode::Char('o') => {
                let trimmed = self.search_query.trim().to_string();
                self.search_query = if trimmed.is_empty() {
                    "age:>".into()
                } else {
                    format!("{} age:>", trimmed)
                };
                self.search_active = true;
                self.view = View::BranchList;
            }
            // Clear all
            KeyCode::Char('c') => {
                self.search_query.clear();
                self.search_active = false;
                self.reset_cursor_to_first_match();
                self.view = View::BranchList;
            }
            _ => {}
        }
    }

    fn handle_tags_key(&mut self, code: KeyCode) {
        let filtered: Vec<usize> = self.filtered_tag_indices();
        let len = filtered.len();
        if len == 0 {
            match code {
                KeyCode::Char('q') => {
                    self.should_exit = true;
                }
                KeyCode::Esc | KeyCode::Char('t') => {
                    self.view = View::BranchList;
                }
                KeyCode::Char('l') => {
                    self.view = View::BranchList;
                }
                KeyCode::Char('r') => {
                    self.open_remote_branches_view();
                }
                KeyCode::Char('/') => {
                    self.tag_search_active = true;
                }
                KeyCode::Char('\\') => {
                    self.view = View::TagFilter;
                }
                KeyCode::Char('c') if !self.tag_search_query.is_empty() => {
                    self.tag_search_query.clear();
                }
                KeyCode::Tab => self.next_tab(),
                KeyCode::BackTab => self.prev_tab(),
                _ => {}
            }
            return;
        }

        // Find current cursor position in filtered list
        let cursor_pos = filtered.iter().position(|&i| i == self.tag_cursor).unwrap_or(0);

        match code {
            KeyCode::Char('w') => {
                self.open_worktrees_view();
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if cursor_pos + 1 < len {
                    self.tag_cursor = filtered[cursor_pos + 1];
                    self.tag_table_state.select(Some(cursor_pos + 1));
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if cursor_pos > 0 {
                    self.tag_cursor = filtered[cursor_pos - 1];
                    self.tag_table_state.select(Some(cursor_pos - 1));
                }
            }
            KeyCode::PageDown => {
                let page_size = 20;
                let new_pos = (cursor_pos + page_size).min(len - 1);
                self.tag_cursor = filtered[new_pos];
                self.tag_table_state.select(Some(new_pos));
            }
            KeyCode::PageUp => {
                let page_size = 20;
                let new_pos = cursor_pos.saturating_sub(page_size);
                self.tag_cursor = filtered[new_pos];
                self.tag_table_state.select(Some(new_pos));
            }
            KeyCode::Char(' ') => {
                self.tag_selected[self.tag_cursor] = !self.tag_selected[self.tag_cursor];
            }
            KeyCode::Char('a') => {
                for &i in &filtered {
                    self.tag_selected[i] = true;
                }
            }
            KeyCode::Char('n') => {
                self.tag_selected.fill(false);
            }
            KeyCode::Char('i') => {
                for &i in &filtered {
                    self.tag_selected[i] = !self.tag_selected[i];
                }
            }
            KeyCode::Char('d') => {
                if self.has_tag_selection() || !self.tags.is_empty() {
                    self.results_return_view = ResultsReturnView::Tags;
                    self.view = View::Confirm {
                        action: BranchAction::DeleteTag,
                    };
                }
            }
            KeyCode::Char('D') => {
                if self.has_tag_selection() || !self.tags.is_empty() {
                    self.results_return_view = ResultsReturnView::Tags;
                    self.view = View::Confirm {
                        action: BranchAction::DeleteTagAndRemote,
                    };
                }
            }
            KeyCode::Char('p') => {
                if !self.tags.is_empty() {
                    let tag_name = self.tags[self.tag_cursor].name.clone();
                    self.results_return_view = ResultsReturnView::Tags;
                    self.spawn_op(format!("Pushing tag {}...", tag_name), {
                        let repo_path = self.repo_path.clone();
                        move || vec![tags::push_tag(&repo_path, &tag_name)]
                    });
                }
            }
            KeyCode::Char('/') => {
                self.tag_search_active = true;
            }
            KeyCode::Char('\\') => {
                self.view = View::TagFilter;
            }
            KeyCode::Char('s') => {
                self.tag_sort_by_name = !self.tag_sort_by_name;
                self.apply_tag_sort();
            }
            KeyCode::Char('q') => {
                self.should_exit = true;
            }
            KeyCode::Esc | KeyCode::Char('t') => {
                self.view = View::BranchList;
            }
            KeyCode::Char('l') => {
                self.view = View::BranchList;
            }
            KeyCode::Char('r') => {
                self.open_remote_branches_view();
            }
            KeyCode::Char('?') => {
                self.view = View::Help;
            }
            KeyCode::Tab => self.next_tab(),
            KeyCode::BackTab => self.prev_tab(),
            _ => {}
        }
    }

    fn handle_tag_search_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => {
                self.tag_search_query.clear();
                self.tag_search_active = false;
                self.reset_tag_cursor();
            }
            KeyCode::Enter => {
                self.tag_search_active = false;
            }
            KeyCode::Backspace => {
                self.tag_search_query.pop();
                self.reset_tag_cursor();
            }
            KeyCode::Char(c) => {
                self.tag_search_query.push(c);
                self.reset_tag_cursor();
            }
            _ => {}
        }
    }

    fn handle_remote_search_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => {
                self.remote_search_query.clear();
                self.remote_search_active = false;
                self.reset_remote_cursor();
            }
            KeyCode::Enter => {
                self.remote_search_active = false;
            }
            KeyCode::Backspace => {
                self.remote_search_query.pop();
                self.reset_remote_cursor();
            }
            KeyCode::Char(c) => {
                self.remote_search_query.push(c);
                self.reset_remote_cursor();
            }
            _ => {}
        }
    }

    fn handle_tag_filter_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc | KeyCode::Char('\\') => {
                self.view = View::Tags;
            }
            KeyCode::Char('1') => {
                self.tag_search_query =
                    FilterSet::toggle_token(&self.tag_search_query, "age:<7d");
                self.reset_tag_cursor();
                self.view = View::Tags;
            }
            KeyCode::Char('2') => {
                self.tag_search_query =
                    FilterSet::toggle_token(&self.tag_search_query, "age:<30d");
                self.reset_tag_cursor();
                self.view = View::Tags;
            }
            KeyCode::Char('3') => {
                self.tag_search_query =
                    FilterSet::toggle_token(&self.tag_search_query, "age:>30d");
                self.reset_tag_cursor();
                self.view = View::Tags;
            }
            KeyCode::Char('4') => {
                self.tag_search_query =
                    FilterSet::toggle_token(&self.tag_search_query, "age:>90d");
                self.reset_tag_cursor();
                self.view = View::Tags;
            }
            KeyCode::Char('n') => {
                let trimmed = self.tag_search_query.trim().to_string();
                self.tag_search_query = if trimmed.is_empty() {
                    "age:<".into()
                } else {
                    format!("{} age:<", trimmed)
                };
                self.tag_search_active = true;
                self.view = View::Tags;
            }
            KeyCode::Char('o') => {
                let trimmed = self.tag_search_query.trim().to_string();
                self.tag_search_query = if trimmed.is_empty() {
                    "age:>".into()
                } else {
                    format!("{} age:>", trimmed)
                };
                self.tag_search_active = true;
                self.view = View::Tags;
            }
            KeyCode::Char('c') => {
                self.tag_search_query.clear();
                self.tag_search_active = false;
                self.reset_tag_cursor();
                self.view = View::Tags;
            }
            _ => {}
        }
    }

    fn handle_remote_filter_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc | KeyCode::Char('\\') => {
                self.view = View::RemoteBranches;
            }
            KeyCode::Char('m') => {
                self.remote_search_query =
                    FilterSet::toggle_token(&self.remote_search_query, "status:merged");
                self.reset_remote_cursor();
                self.view = View::RemoteBranches;
            }
            KeyCode::Char('s') => {
                self.remote_search_query =
                    FilterSet::toggle_token(&self.remote_search_query, "status:squash");
                self.reset_remote_cursor();
                self.view = View::RemoteBranches;
            }
            KeyCode::Char('u') => {
                self.remote_search_query =
                    FilterSet::toggle_token(&self.remote_search_query, "status:unmerged");
                self.reset_remote_cursor();
                self.view = View::RemoteBranches;
            }
            KeyCode::Char('c') => {
                self.remote_search_query.clear();
                self.remote_search_active = false;
                self.reset_remote_cursor();
                self.view = View::RemoteBranches;
            }
            _ => {}
        }
    }

    fn handle_remote_branches_key(&mut self, code: KeyCode) {
        if self.filtered_remote_indices().is_empty() {
            match code {
                KeyCode::Char('q') => {
                    self.should_exit = true;
                }
                KeyCode::Esc | KeyCode::Char('r') => {
                    self.view = View::BranchList;
                }
                KeyCode::Char('l') => {
                    self.view = View::BranchList;
                }
                KeyCode::Char('t') => {
                    self.tag_cursor = 0;
                    self.tag_search_query.clear();
                    self.tag_search_active = false;
                    self.tag_sort_by_name = false;
                    self.load_tags();
                }
                KeyCode::Char('/') => {
                    self.remote_search_active = true;
                }
                KeyCode::Char('T') => {
                    self.theme = self.theme.next();
                    let mut config = git_branch_manager::config::Config::load();
                    config.theme = Some(self.theme.name.to_string());
                    config.save();
                }
                KeyCode::Char('Y') => {
                    self.symbols = crate::ui::symbols::next(self.symbols);
                    let mut config = git_branch_manager::config::Config::load();
                    config.symbols = Some(crate::ui::symbols::name(self.symbols).to_string());
                    config.save();
                }
                KeyCode::Char('?') => {
                    self.view = View::Help;
                }
                KeyCode::Tab => self.next_tab(),
                KeyCode::BackTab => self.prev_tab(),
                _ => {}
            }
            return;
        }

        match code {
            KeyCode::Char('j') | KeyCode::Down => nav_down(&mut RemoteListNav { app: self }),
            KeyCode::Char('k') | KeyCode::Up => nav_up(&mut RemoteListNav { app: self }),
            KeyCode::PageDown => nav_page_down(&mut RemoteListNav { app: self }),
            KeyCode::PageUp => nav_page_up(&mut RemoteListNav { app: self }),
            KeyCode::Home => nav_home(&mut RemoteListNav { app: self }),
            KeyCode::End => nav_end(&mut RemoteListNav { app: self }),
            KeyCode::Char(' ') => select_toggle(&mut RemoteListNav { app: self }),
            KeyCode::Char('a') => select_all(&mut RemoteListNav { app: self }),
            KeyCode::Char('n') => deselect_all(&mut RemoteListNav { app: self }),
            KeyCode::Char('i') => invert_selection(&mut RemoteListNav { app: self }),
            KeyCode::Char('m') => select_merged(&mut RemoteListNav { app: self }),
            KeyCode::Char('d') => {
                let has_selection = self.remote_selected.iter().any(|&s| s);
                if has_selection || !self.remote_branches.is_empty() {
                    self.results_return_view = ResultsReturnView::RemoteBranches;
                    self.view = View::Confirm {
                        action: BranchAction::DeleteRemoteBranch,
                    };
                }
            }
            KeyCode::Char('c') => {
                let branch = &self.remote_branches[self.remote_cursor];
                if !branch.is_pinned() {
                    self.results_return_view = ResultsReturnView::RemoteBranches;
                    self.view = View::Confirm {
                        action: BranchAction::CheckoutRemote,
                    };
                }
            }
            KeyCode::Char('s') => {
                // Cycle sort column: None -> 0=name -> 1=age -> 2=status -> 0...
                self.remote_sort_column = Some(match self.remote_sort_column {
                    Some(c) => (c + 1) % 3,
                    None => 0,
                });
                self.remote_sort_ascending = true;
                self.apply_remote_sort();
            }
            KeyCode::Char('S') => {
                self.remote_sort_ascending = !self.remote_sort_ascending;
                self.apply_remote_sort();
            }
            KeyCode::Char('/') => {
                self.remote_search_active = true;
            }
            KeyCode::Char('\\') => {
                self.view = View::RemoteFilter;
            }
            KeyCode::Enter => {
                self.prev_view = View::RemoteBranches;
                self.view = View::Menu { cursor: 0 };
            }
            KeyCode::Char('?') => {
                self.view = View::Help;
            }
            KeyCode::Char('T') => {
                self.theme = self.theme.next();
                let mut config = git_branch_manager::config::Config::load();
                config.theme = Some(self.theme.name.to_string());
                config.save();
            }
            KeyCode::Char('Y') => {
                self.symbols = crate::ui::symbols::next(self.symbols);
                let mut config = git_branch_manager::config::Config::load();
                config.symbols = Some(crate::ui::symbols::name(self.symbols).to_string());
                config.save();
            }
            KeyCode::Char('t') => {
                self.tag_cursor = 0;
                self.tag_search_query.clear();
                self.tag_search_active = false;
                self.tag_sort_by_name = false;
                self.load_tags();
            }
            KeyCode::Char('w') => {
                self.open_worktrees_view();
            }
            KeyCode::Tab => self.next_tab(),
            KeyCode::BackTab => self.prev_tab(),
            KeyCode::Char('l') => {
                self.view = View::BranchList;
            }
            KeyCode::Char('q') => {
                self.should_exit = true;
            }
            KeyCode::Esc | KeyCode::Char('r') => {
                self.view = View::BranchList;
            }
            _ => {}
        }
    }

    fn handle_worktrees_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('j') | KeyCode::Down => nav_down(&mut WorktreeListNav { app: self }),
            KeyCode::Char('k') | KeyCode::Up => nav_up(&mut WorktreeListNav { app: self }),
            KeyCode::PageDown => nav_page_down(&mut WorktreeListNav { app: self }),
            KeyCode::PageUp => nav_page_up(&mut WorktreeListNav { app: self }),
            KeyCode::Home => nav_home(&mut WorktreeListNav { app: self }),
            KeyCode::End => nav_end(&mut WorktreeListNav { app: self }),
            KeyCode::Char(' ') => select_toggle(&mut WorktreeListNav { app: self }),
            KeyCode::Char('a') => select_all(&mut WorktreeListNav { app: self }),
            KeyCode::Char('n') => deselect_all(&mut WorktreeListNav { app: self }),
            KeyCode::Char('m') => select_merged(&mut WorktreeListNav { app: self }),
            KeyCode::Char('i') => invert_selection(&mut WorktreeListNav { app: self }),
            KeyCode::Enter => {
                self.prev_view = View::Worktrees;
                self.view = View::Menu { cursor: 0 };
            }
            KeyCode::Char('d') => {
                if !self.worktrees.is_empty() {
                    let wt = &self.worktrees[self.worktree_cursor];
                    if !wt.is_main && wt.wt_status.is_clean() {
                        self.results_return_view = ResultsReturnView::Worktrees;
                        self.view = View::Confirm { action: BranchAction::WorktreeRemove };
                    }
                }
            }
            KeyCode::Char('D') => {
                if !self.worktrees.is_empty() && !self.worktrees[self.worktree_cursor].is_main {
                    self.results_return_view = ResultsReturnView::Worktrees;
                    self.view = View::Confirm { action: BranchAction::WorktreeForceRemove };
                }
            }
            KeyCode::Char('w') => {
                self.view = View::BranchList;
            }
            KeyCode::Tab => self.next_tab(),
            KeyCode::BackTab => self.prev_tab(),
            KeyCode::Char('r') => {
                self.open_remote_branches_view();
            }
            KeyCode::Char('t') => {
                self.tag_cursor = 0;
                self.tag_search_query.clear();
                self.tag_search_active = false;
                self.tag_sort_by_name = false;
                self.load_tags();
            }
            KeyCode::Char('?') => {
                self.view = View::Help;
            }
            KeyCode::Char('T') => {
                self.theme = self.theme.next();
                let mut config = git_branch_manager::config::Config::load();
                config.theme = Some(self.theme.name.to_string());
                config.save();
            }
            KeyCode::Char('Y') => {
                self.symbols = crate::ui::symbols::next(self.symbols);
                let mut config = git_branch_manager::config::Config::load();
                config.symbols = Some(crate::ui::symbols::name(self.symbols).to_string());
                config.save();
            }
            KeyCode::Char('l') => {
                self.view = View::BranchList;
            }
            KeyCode::Char('s') => {
                self.worktree_sort_column = Some(
                    self.worktree_sort_column.map(|c| (c + 1) % 4).unwrap_or(0)
                );
                self.apply_worktree_sort();
            }
            KeyCode::Char('S') => {
                self.worktree_sort_ascending = !self.worktree_sort_ascending;
                self.apply_worktree_sort();
            }
            KeyCode::Char('q') => {
                self.should_exit = true;
            }
            KeyCode::Esc => {
                self.view = View::BranchList;
            }
            _ => {}
        }
    }

    /// Spawn a closure on a background thread and transition to the Executing view.
    /// The closure should return a `Vec<OperationResult>`.
    fn spawn_op<F>(&mut self, label: String, f: F)
    where
        F: FnOnce() -> Vec<OperationResult> + Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let results = f();
            let _ = tx.send(results);
        });
        self.executing_label = label;
        self.op_rx = Some(rx);
        self.progress_rx = None;
        self.progress = None;
        self.cancel_flag = None;
        self.view = View::Executing;
    }

    /// Spawn a closure that receives a progress sender and cancellation flag.
    /// Used for bulk operations that process multiple items.
    fn spawn_op_with_progress<F>(&mut self, label: String, f: F)
    where
        F: FnOnce(Sender<ProgressUpdate>, Arc<AtomicBool>) -> Vec<OperationResult> + Send + 'static,
    {
        let (op_tx, op_rx) = mpsc::channel();
        let (prog_tx, prog_rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_clone = cancel.clone();
        std::thread::spawn(move || {
            let results = f(prog_tx, cancel_clone);
            let _ = op_tx.send(results);
        });
        self.executing_label = label;
        self.op_rx = Some(op_rx);
        self.progress_rx = Some(prog_rx);
        self.progress = None;
        self.cancel_flag = Some(cancel);
        self.view = View::Executing;
    }

    fn execute_action_async(&mut self) {
        let action = match &self.view {
            View::Confirm { action } => action.clone(),
            _ => return,
        };

        let label = format!("{}...", action.label());

        // Tag operations
        if action == BranchAction::DeleteTag || action == BranchAction::DeleteTagAndRemote {
            let target_names = self.target_tag_names();
            if target_names.is_empty() {
                return;
            }
            let repo_path = self.repo_path.clone();
            let delete_remote = action == BranchAction::DeleteTagAndRemote;
            self.spawn_op(label, move || {
                let repo = match git2::Repository::open(&repo_path) {
                    Ok(r) => r,
                    Err(e) => {
                        return vec![OperationResult {
                            branch_name: String::new(),
                            action: BranchAction::DeleteTag,
                            success: false,
                            message: format!("Failed to open repo: {}", e),
                        }];
                    }
                };
                let mut results = tags::delete_tags_batch(&repo, &target_names);
                if delete_remote {
                    let successfully_deleted: Vec<String> = results
                        .iter()
                        .filter(|r| r.success)
                        .map(|r| r.branch_name.clone())
                        .collect();
                    if !successfully_deleted.is_empty() {
                        results.extend(tags::delete_remote_tags_batch(&repo_path, &successfully_deleted));
                    }
                }
                results
            });
            return;
        }

        if action == BranchAction::PushTag {
            if !self.tags.is_empty() {
                let tag_name = self.tags[self.tag_cursor].name.clone();
                let repo_path = self.repo_path.clone();
                self.spawn_op(label, move || {
                    vec![tags::push_tag(&repo_path, &tag_name)]
                });
            }
            return;
        }

        // Checkout operates on the cursor branch, not the selection
        if action == BranchAction::Checkout {
            let branch_name = self.branches[self.cursor].name.clone();
            let needs_stash = !self.working_tree_status.is_clean();
            let repo_path = self.repo_path.clone();
            self.spawn_op(label, move || {
                let repo = match git2::Repository::open(&repo_path) {
                    Ok(r) => r,
                    Err(e) => {
                        return vec![OperationResult {
                            branch_name: branch_name.clone(),
                            action: BranchAction::Checkout,
                            success: false,
                            message: format!("Failed to open repo: {}", e),
                        }];
                    }
                };
                vec![operations::checkout_branch(&repo, &repo_path, &branch_name, needs_stash)]
            });
            return;
        }

        // Fast-forward operates on the cursor branch
        if action == BranchAction::FastForward {
            let branch_name = self.branches[self.cursor].name.clone();
            let repo_path = self.repo_path.clone();
            self.spawn_op(label, move || {
                vec![operations::fast_forward(&repo_path, &branch_name)]
            });
            return;
        }

        // Push operates on the cursor branch
        if action == BranchAction::Push {
            let branch_name = self.branches[self.cursor].name.clone();
            let repo_path = self.repo_path.clone();
            self.spawn_op(label, move || {
                vec![operations::push_branch(&repo_path, &branch_name)]
            });
            return;
        }

        // Force push operates on the cursor branch
        if action == BranchAction::ForcePush {
            let branch_name = self.branches[self.cursor].name.clone();
            let repo_path = self.repo_path.clone();
            self.spawn_op(label, move || {
                vec![operations::force_push_branch(&repo_path, &branch_name)]
            });
            return;
        }

        // Pull operates on the cursor branch
        if action == BranchAction::Pull {
            let branch_name = self.branches[self.cursor].name.clone();
            let is_current = self.branches[self.cursor].is_current;
            let repo_path = self.repo_path.clone();
            self.spawn_op(label, move || {
                vec![operations::pull_branch(&repo_path, &branch_name, is_current)]
            });
            return;
        }

        // Merge / squash merge operates on the cursor branch into base
        if action == BranchAction::Merge || action == BranchAction::SquashMerge {
            let branch_name = self.branches[self.cursor].name.clone();
            let needs_stash = !self.working_tree_status.is_clean();
            let squash = action == BranchAction::SquashMerge;
            let repo_path = self.repo_path.clone();
            let base_branch = self.base_branch.clone();
            self.spawn_op(label, move || {
                operations::merge_branch(&repo_path, &branch_name, &base_branch, squash, needs_stash)
            });
            return;
        }

        // Rebase operates on the cursor branch onto base
        if action == BranchAction::Rebase {
            let branch_name = self.branches[self.cursor].name.clone();
            let needs_stash = !self.working_tree_status.is_clean();
            let repo_path = self.repo_path.clone();
            let base_branch = self.base_branch.clone();
            self.spawn_op(label, move || {
                operations::rebase_branch(&repo_path, &branch_name, &base_branch, needs_stash)
            });
            return;
        }

        // Create worktree operates on the cursor branch
        if action == BranchAction::Worktree {
            let branch_name = self.branches[self.cursor].name.clone();
            let repo_path = self.repo_path.clone();
            self.spawn_op(label, move || {
                vec![operations::create_worktree(&repo_path, &branch_name)]
            });
            return;
        }

        // Delete remote branches: operates on remote selection (or cursor if nothing selected)
        if action == BranchAction::DeleteRemoteBranch {
            let selected: Vec<(String, String)> = self
                .remote_branches
                .iter()
                .zip(self.remote_selected.iter())
                .filter(|&(b, &sel)| sel && !b.is_pinned())
                .map(|(b, _)| (b.remote.clone(), b.short_name.clone()))
                .collect();
            let target: Vec<(String, String)> = if selected.is_empty() {
                let b = &self.remote_branches[self.remote_cursor];
                vec![(b.remote.clone(), b.short_name.clone())]
            } else {
                selected
            };
            // delete_remotes_batch takes short names; group by remote if needed.
            // Current implementation always uses "origin"; pass short_names only.
            let short_names: Vec<String> = target.into_iter().map(|(_, s)| s).collect();
            let repo_path = self.repo_path.clone();
            self.spawn_op(label, move || {
                operations::delete_remotes_batch(&repo_path, &short_names)
                    .into_iter()
                    .map(|mut r| {
                        r.action = BranchAction::DeleteRemoteBranch;
                        r
                    })
                    .collect()
            });
            return;
        }

        // CheckoutRemote: create a local tracking branch from the cursor remote branch
        if action == BranchAction::CheckoutRemote {
            if self.remote_branches.is_empty() {
                return;
            }
            let b = &self.remote_branches[self.remote_cursor];
            let remote = b.remote.clone();
            let short_name = b.short_name.clone();
            let repo_path = self.repo_path.clone();
            self.spawn_op(label, move || {
                vec![operations::checkout_remote_branch(&repo_path, &remote, &short_name)]
            });
            return;
        }

        // DeleteRemoteAndLocal: delete the remote branch AND the matching local branch
        if action == BranchAction::DeleteRemoteAndLocal {
            if self.remote_branches.is_empty() {
                return;
            }
            let b = &self.remote_branches[self.remote_cursor];
            let short_name = b.short_name.clone();
            let repo_path = self.repo_path.clone();
            self.spawn_op(label, move || {
                let mut results = Vec::new();
                // Delete remote first
                let remote_result = operations::delete_remotes_batch(&repo_path, &[short_name.clone()]);
                results.extend(remote_result.into_iter().map(|mut r| {
                    r.action = BranchAction::DeleteRemoteAndLocal;
                    r
                }));
                // Delete local
                let repo = match git2::Repository::open(&repo_path) {
                    Ok(r) => r,
                    Err(e) => {
                        results.push(OperationResult {
                            branch_name: short_name,
                            action: BranchAction::DeleteRemoteAndLocal,
                            success: false,
                            message: format!("Failed to open repo: {}", e),
                        });
                        return results;
                    }
                };
                let local_result = operations::delete_local(&repo, &short_name);
                results.push(OperationResult {
                    action: BranchAction::DeleteRemoteAndLocal,
                    ..local_result
                });
                results
            });
            return;
        }

        // FetchRemote: fetch from the specific remote
        if action == BranchAction::FetchRemote {
            if self.remote_branches.is_empty() {
                return;
            }
            let b = &self.remote_branches[self.remote_cursor];
            let remote = b.remote.clone();
            let repo_path = self.repo_path.clone();
            self.spawn_op(label, move || {
                operations::fetch_remote(&repo_path, &remote)
            });
            return;
        }

        // PullRemote: pull into the local tracking branch
        if action == BranchAction::PullRemote {
            if self.remote_branches.is_empty() {
                return;
            }
            let b = &self.remote_branches[self.remote_cursor];
            let remote = b.remote.clone();
            let short_name = b.short_name.clone();
            let repo_path = self.repo_path.clone();
            self.spawn_op(label, move || {
                operations::pull_remote(&repo_path, &remote, &short_name)
            });
            return;
        }

        // MergeRemoteIntoCurrent: merge remote ref into the current branch
        if action == BranchAction::MergeRemoteIntoCurrent {
            if self.remote_branches.is_empty() {
                return;
            }
            let b = &self.remote_branches[self.remote_cursor];
            let full_ref = b.full_ref.clone();
            let short_name = b.short_name.clone();
            let repo_path = self.repo_path.clone();
            self.spawn_op(label, move || {
                operations::merge_remote_into_current(&repo_path, &full_ref, &short_name)
            });
            return;
        }

        // CherryPickRemote: cherry-pick the tip commit of the remote branch
        if action == BranchAction::CherryPickRemote {
            if self.remote_branches.is_empty() {
                return;
            }
            let b = &self.remote_branches[self.remote_cursor];
            let full_ref = b.full_ref.clone();
            let short_name = b.short_name.clone();
            let repo_path = self.repo_path.clone();
            self.spawn_op(label, move || {
                operations::cherry_pick_remote(&repo_path, &full_ref, &short_name)
            });
            return;
        }

        if action == BranchAction::WorktreeRemove || action == BranchAction::WorktreeForceRemove {
            if self.worktrees.is_empty() {
                return;
            }
            let wt_path = self.worktrees[self.worktree_cursor].path.clone();
            let repo_path = self.repo_path.clone();
            let force = action == BranchAction::WorktreeForceRemove;
            self.spawn_op(label, move || {
                let result = if force {
                    operations::force_remove_worktree(&repo_path, &wt_path)
                } else {
                    operations::remove_worktree(&repo_path, &wt_path)
                };
                vec![result]
            });
            return;
        }

        // Bulk branch operations (delete local, delete local+remote)
        let target_branches: Vec<String> = {
            let selected: Vec<String> = self
                .branches
                .iter()
                .zip(self.selected.iter())
                .filter(|&(_, &sel)| sel)
                .map(|(b, _)| b.name.clone())
                .collect();
            if selected.is_empty() {
                vec![self.branches[self.cursor].name.clone()]
            } else {
                selected
            }
        };

        let repo_path = self.repo_path.clone();
        self.spawn_op_with_progress(label, move |prog_tx, cancel_flag| {
            let mut results = Vec::new();
            let total = target_branches.len();
            let repo = match git2::Repository::open(&repo_path) {
                Ok(r) => r,
                Err(e) => {
                    results.push(OperationResult {
                        branch_name: String::new(),
                        action: action.clone(),
                        success: false,
                        message: format!("Failed to open repo: {}", e),
                    });
                    return results;
                }
            };

            // Delete local branches (git2, fast local I/O)
            let mut locally_deleted = Vec::new();
            for (i, branch_name) in target_branches.iter().enumerate() {
                if cancel_flag.load(Ordering::Relaxed) {
                    results.push(OperationResult {
                        branch_name: String::new(),
                        action: action.clone(),
                        success: false,
                        message: "Cancelled by user".to_string(),
                    });
                    break;
                }

                let _ = prog_tx.send(ProgressUpdate {
                    completed: i,
                    total,
                    current_item: branch_name.clone(),
                });

                let result = operations::delete_local(&repo, branch_name);
                if result.success {
                    locally_deleted.push(branch_name.clone());
                }
                results.push(result);
            }

            // Batch-delete remote branches in a single git push (one network round-trip)
            if action == BranchAction::DeleteLocalAndRemote && !locally_deleted.is_empty() {
                let _ = prog_tx.send(ProgressUpdate {
                    completed: locally_deleted.len(),
                    total,
                    current_item: "Deleting remote branches...".to_string(),
                });

                let remote_results =
                    operations::delete_remotes_batch(&repo_path, &locally_deleted);
                results.extend(remote_results);
            }

            // Send final progress
            let _ = prog_tx.send(ProgressUpdate {
                completed: results.iter().filter(|r| r.success).count().min(total),
                total,
                current_item: "Done".to_string(),
            });

            results
        });
    }

    fn refresh_branches(&mut self) {
        // Spawn background thread — mirrors the initial load pattern in main.rs.
        // The event loop's drain_load_rx() will pick up the result.
        let repo_path = self.repo_path.clone();
        let base_branch = self.base_branch.clone();
        let (load_tx, load_rx) = mpsc::channel();
        let (prog_tx, prog_rx) = mpsc::channel();

        std::thread::spawn(move || {
            let _ = prog_tx.send(LoadProgress {
                message: "Refreshing branches...".into(),
            });

            let Ok(repo) = git2::Repository::open(&repo_path) else { return };
            let Ok(branches) = branch::list_branches_phase1(&repo, &base_branch) else {
                return;
            };

            let working_tree_status = status::detect_working_tree_status(&repo);
            let cache = cache::BranchCache::load(&repo_path);

            let candidates: Vec<(String, String)> = branches
                .iter()
                .filter(|b| {
                    b.merge_status == MergeStatus::Unmerged && !b.is_base && !b.is_current
                })
                .filter_map(|b| {
                    branch::get_commit_hash(&repo, &b.name)
                        .map(|hash| (b.name.clone(), hash))
                })
                .collect();

            let _ = load_tx.send(InitialLoad {
                branches,
                working_tree_status,
                candidates,
                cache,
                did_fetch: false,
            });
        });

        self.load_rx = Some(load_rx);
        self.load_progress_rx = Some(prog_rx);
        self.loading = true;
        self.loading_message = "Refreshing branches...".into();
    }

    /// Open the Remote Branches view.
    ///
    /// Loads currently known remote tracking refs in the background. If `auto_fetch`
    /// is enabled and a fetch hasn't been done this session, spawns a background
    /// `git fetch` and reloads branches when it completes.
    fn open_remote_branches_view(&mut self) {
        // Load currently known remote refs immediately (local-only, fast)
        // Always load what we already know from local refs
        self.populate_remote_branches();

        if !self.remote_fetched && self.config.auto_fetch == Some(true) {
            // Spawn background fetch; branches reload when it completes
            let repo_path = self.repo_path.clone();
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                let ok = operations::fetch_sync(&repo_path);
                let _ = tx.send(ok);
            });
            self.remote_fetch_rx = Some(rx);
            self.remote_loading = true;
        }

        self.view = View::RemoteBranches;
    }

    fn open_worktrees_view(&mut self) {
        self.spawn_worktree_load();
        self.view = View::Worktrees;
    }

    fn next_tab(&mut self) {
        match self.view {
            View::BranchList => self.open_remote_branches_view(),
            View::RemoteBranches => self.open_worktrees_view(),
            View::Worktrees => self.view = View::BranchList,
            _ => {}
        }
    }

    fn prev_tab(&mut self) {
        match self.view {
            View::BranchList => self.open_worktrees_view(),
            View::RemoteBranches => self.view = View::BranchList,
            View::Worktrees => self.open_remote_branches_view(),
            _ => {}
        }
    }

    fn spawn_worktree_load(&mut self) {
        let repo_path = self.repo_path.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let worktrees = worktree::list_worktrees(&repo_path);
            let _ = tx.send(WorktreeLoad { worktrees });
        });
        self.worktree_load_rx = Some(rx);
        self.worktree_loading = true;
    }

    fn spawn_worktree_enrich(&mut self) {
        let worktrees = self.worktrees.clone();
        let branches = self.branches.clone();
        let pr_map = self.pr_map.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let enriched: Vec<WorktreeInfo> = worktrees
                .into_iter()
                .map(|mut wt| {
                    if let Some(ref branch_name) = wt.branch {
                        if let Some(b) = branches.iter().find(|b| &b.name == branch_name) {
                            wt.merge_status = b.merge_status.clone();
                            wt.ahead = b.ahead;
                            wt.behind = b.behind;
                            wt.pr = pr_map.get(branch_name).map(|p| p.status.clone());
                        }
                    }
                    wt
                })
                .collect();
            let _ = tx.send(WorktreeEnrich { worktrees: enriched });
        });
        self.worktree_enrich_rx = Some(rx);
    }

    /// Spawn background thread to populate remote branches from local tracking refs.
    /// Results arrive via `remote_load_rx` and are applied in `drain_remote_load_rx()`.
    fn populate_remote_branches(&mut self) {
        let repo_path = self.repo_path.clone();
        let base_branch = self.base_branch.clone();
        let (tx, rx) = mpsc::channel();

        std::thread::spawn(move || {
            let Ok(repo) = git2::Repository::open(&repo_path) else { return };
            let Ok(remote_branches) =
                branch::list_remote_branches_phase1(&repo, &base_branch)
            else {
                return;
            };

            let branch_cache = cache::BranchCache::load(&repo_path);
            let candidates: Vec<(String, String)> = remote_branches
                .iter()
                .filter(|b| b.merge_status == MergeStatus::Unmerged && !b.is_base)
                .filter_map(|b| {
                    let refname = format!("refs/remotes/{}", b.full_ref);
                    repo.find_reference(&refname)
                        .ok()
                        .and_then(|r| r.peel_to_commit().ok())
                        .map(|c| (b.full_ref.clone(), c.id().to_string()))
                })
                .collect();

            let _ = tx.send(RemoteLoad {
                remote_branches,
                candidates,
                cache: branch_cache,
            });
        });

        self.remote_load_rx = Some(rx);
    }

    fn apply_sort(&mut self) {
        let Some(col) = self.sort_column else { return };
        let asc = self.sort_ascending;

        // Find where pinned rows end — pinned rows are never sorted
        let pin_count = self.branches.iter().take_while(|b| b.is_pinned()).count();
        let sortable = &mut self.branches[pin_count..];

        sortable.sort_by(|a, b| {
            let ord = match col {
                0 => a.name.cmp(&b.name),
                1 => a.last_commit_date.cmp(&b.last_commit_date),
                2 => a.ahead.unwrap_or(0).cmp(&b.ahead.unwrap_or(0)),
                3 => a.behind.unwrap_or(0).cmp(&b.behind.unwrap_or(0)),
                4 => {
                    let rank = |s: &MergeStatus| match s {
                        MergeStatus::Merged => 0,
                        MergeStatus::SquashMerged => 1,
                        MergeStatus::Unmerged => 2,
                    };
                    rank(&a.merge_status).cmp(&rank(&b.merge_status))
                }
                _ => std::cmp::Ordering::Equal,
            };
            if asc { ord } else { ord.reverse() }
        });

        // Reset selection and cursor after sort
        self.selected = vec![false; self.branches.len()];
        self.cursor = 0;
        self.table_state.select(Some(0));
    }

    fn drain_load_rx(&mut self) {
        use std::sync::mpsc::TryRecvError;

        // Drain progress messages
        if let Some(rx) = &self.load_progress_rx {
            loop {
                match rx.try_recv() {
                    Ok(p) => self.loading_message = p.message,
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        self.load_progress_rx = None;
                        break;
                    }
                }
            }
        }

        let Some(rx) = &self.load_rx else { return };

        match rx.try_recv() {
            Ok(load) => {
                self.load_rx = None;
                self.loading = false;

                let mut branches = load.branches;
                branches.sort_by(|a, b| {
                    let pin_a = if a.is_base { 0 } else if a.is_current { 1 } else { 2 };
                    let pin_b = if b.is_base { 0 } else if b.is_current { 1 } else { 2 };
                    pin_a.cmp(&pin_b).then(b.last_commit_date.cmp(&a.last_commit_date))
                });

                let len = branches.len();
                self.branches = branches;
                self.selected = vec![false; len];
                self.cursor = 0;
                self.list_scroll_offset = 0;
                self.table_state.select(Some(0));
                self.working_tree_status = load.working_tree_status;
                self.results.clear();
                self.search_query.clear();
                self.search_active = false;

                self.squash_total = load.candidates.len();
                self.squash_checked = 0;
                self.squash_rx = if load.candidates.is_empty() {
                    None
                } else {
                    Some(squash_loader::spawn_squash_checker(
                        self.repo_path.clone(),
                        self.base_branch.clone(),
                        load.candidates,
                        load.cache,
                    ))
                };

                // If the load thread already fetched (auto_fetch on startup),
                // mark remote as fetched so the Remote Branches view doesn't re-fetch.
                if load.did_fetch {
                    self.remote_fetched = true;
                }

                // Spawn PR loader now that branches are loaded
                self.pr_rx = Some(pr_loader::spawn_pr_loader(self.repo_path.clone()));

                if self.config.load_worktrees_on_launch == Some(true) {
                    self.spawn_worktree_load();
                }

                self.apply_sort();
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.load_rx = None;
                self.loading = false;
            }
        }
    }

    /// Spawn background thread to load tags. Results arrive via `tag_load_rx`.
    fn load_tags(&mut self) {
        let repo_path = self.repo_path.clone();
        let (tx, rx) = mpsc::channel();

        std::thread::spawn(move || {
            let Ok(repo) = git2::Repository::open(&repo_path) else { return };
            let tag_list = tags::list_tags(&repo);
            let _ = tx.send(TagLoad { tags: tag_list });
        });

        self.tag_load_rx = Some(rx);
        self.tag_loading = true;
        self.view = View::Tags;
    }

    fn drain_tag_load_rx(&mut self) {
        use std::sync::mpsc::TryRecvError;

        let Some(rx) = &self.tag_load_rx else { return };

        match rx.try_recv() {
            Ok(load) => {
                self.tag_load_rx = None;
                self.tag_loading = false;
                self.tags = load.tags;
                if self.tag_sort_by_name {
                    self.tags.sort_by(|a, b| a.name.cmp(&b.name));
                }
                self.tag_selected = vec![false; self.tags.len()];
                if self.tag_cursor >= self.tags.len() {
                    self.tag_cursor = self.tags.len().saturating_sub(1);
                }
                self.tag_table_state = TableState::default().with_selected(
                    if self.tags.is_empty() { None } else { Some(self.tag_cursor) },
                );
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.tag_load_rx = None;
                self.tag_loading = false;
            }
        }
    }

    fn drain_remote_load_rx(&mut self) {
        use std::sync::mpsc::TryRecvError;

        let Some(rx) = &self.remote_load_rx else { return };

        match rx.try_recv() {
            Ok(load) => {
                self.remote_load_rx = None;
                self.remote_loading = false;

                let len = load.remote_branches.len();
                self.remote_branches = load.remote_branches;
                self.remote_selected = vec![false; len];
                self.remote_cursor = 0;
                self.remote_search_query.clear();
                self.remote_search_active = false;
                self.remote_sort_column = None;
                self.remote_sort_ascending = true;
                self.remote_table_state = TableState::default().with_selected(
                    if len == 0 { None } else { Some(0) },
                );

                self.remote_squash_checked = 0;
                self.remote_squash_total = load.candidates.len();
                self.remote_squash_rx = if load.candidates.is_empty() {
                    None
                } else {
                    Some(squash_loader::spawn_squash_checker(
                        self.repo_path.clone(),
                        self.base_branch.clone(),
                        load.candidates,
                        load.cache,
                    ))
                };
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.remote_load_rx = None;
                self.remote_loading = false;
            }
        }
    }

    fn drain_worktree_load_rx(&mut self) {
        use std::sync::mpsc::TryRecvError;
        let Some(rx) = &self.worktree_load_rx else { return };
        match rx.try_recv() {
            Ok(load) => {
                self.worktree_load_rx = None;
                self.worktree_loading = false;
                let len = load.worktrees.len();
                self.worktrees = load.worktrees;
                self.worktree_selected = vec![false; len];
                self.worktree_cursor = 0;
                self.worktree_table_state = TableState::default().with_selected(
                    if len == 0 { None } else { Some(0) },
                );
                self.spawn_worktree_enrich();
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.worktree_load_rx = None;
                self.worktree_loading = false;
            }
        }
    }

    fn drain_worktree_enrich_rx(&mut self) {
        use std::sync::mpsc::TryRecvError;
        let Some(rx) = &self.worktree_enrich_rx else { return };
        match rx.try_recv() {
            Ok(enrich) => {
                self.worktree_enrich_rx = None;
                self.worktrees = enrich.worktrees;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.worktree_enrich_rx = None;
            }
        }
    }

    fn drain_squash_rx(&mut self) {
        use std::sync::mpsc::TryRecvError;

        let Some(rx) = &self.squash_rx else { return };

        // Build name→index map for O(1) lookup (only once per drain, not per result)
        let index_map: HashMap<String, usize> = self
            .branches
            .iter()
            .enumerate()
            .map(|(i, b)| (b.name.clone(), i))
            .collect();

        let mut drained = 0;
        let done = loop {
            if drained >= 32 {
                break false;
            }
            match rx.try_recv() {
                Ok(result) => {
                    drained += 1;
                    self.squash_checked += 1;
                    if result.is_squash_merged
                        && let Some(&idx) = index_map.get(result.branch_name.as_str())
                    {
                        self.branches[idx].merge_status = MergeStatus::SquashMerged;
                    }
                }
                Err(TryRecvError::Empty) => break false,
                Err(TryRecvError::Disconnected) => break true,
            }
        };

        if done {
            self.squash_rx = None;
        }
    }

    fn drain_remote_squash_rx(&mut self) {
        use std::sync::mpsc::TryRecvError;

        let Some(rx) = &self.remote_squash_rx else { return };

        // Build ref→index map for O(1) lookup
        let index_map: HashMap<String, usize> = self
            .remote_branches
            .iter()
            .enumerate()
            .map(|(i, b)| (b.full_ref.clone(), i))
            .collect();

        let mut drained = 0;
        let done = loop {
            if drained >= 32 {
                break false;
            }
            match rx.try_recv() {
                Ok(result) => {
                    drained += 1;
                    self.remote_squash_checked += 1;
                    if result.is_squash_merged
                        && let Some(&idx) = index_map.get(result.branch_name.as_str())
                    {
                        self.remote_branches[idx].merge_status = MergeStatus::SquashMerged;
                    }
                }
                Err(TryRecvError::Empty) => break false,
                Err(TryRecvError::Disconnected) => break true,
            }
        };

        if done {
            self.remote_squash_rx = None;
        }
    }

    fn drain_pr_rx(&mut self) {
        use std::sync::mpsc::TryRecvError;

        let Some(rx) = &self.pr_rx else { return };

        match rx.try_recv() {
            Ok(pr_map) => {
                self.pr_map = pr_map;
                self.pr_rx = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.pr_rx = None;
            }
        }
    }

    fn drain_progress_rx(&mut self) {
        use std::sync::mpsc::TryRecvError;

        let Some(rx) = &self.progress_rx else { return };

        let done = loop {
            match rx.try_recv() {
                Ok(update) => {
                    self.progress = Some(update);
                }
                Err(TryRecvError::Empty) => break false,
                Err(TryRecvError::Disconnected) => break true,
            }
        };

        if done {
            self.progress_rx = None;
        }
    }

    fn drain_op_rx(&mut self) {
        use std::sync::mpsc::TryRecvError;

        let Some(rx) = &self.op_rx else { return };

        match rx.try_recv() {
            Ok(op_results) => {
                self.results.extend(op_results);
                self.op_rx = None;
                self.progress_rx = None;
                self.progress = None;
                self.cancel_flag = None;
                self.view = View::Results;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                // Thread finished without sending (shouldn't happen, but handle gracefully)
                self.op_rx = None;
                self.progress_rx = None;
                self.progress = None;
                self.cancel_flag = None;
                self.view = View::Results;
            }
        }
    }

    pub fn build_menu_items(&self) -> Vec<ui::menu::MenuItem> {
        let branch = &self.branches[self.cursor];

        let mut items = Vec::new();

        // Checkout
        items.push(ui::menu::MenuItem {
            label: "Checkout".into(),
            enabled: !branch.is_current,
            reason: if branch.is_current {
                Some("current".into())
            } else {
                None
            },
            shortcut: Some('c'),
        });

        // Delete local
        items.push(ui::menu::MenuItem {
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
        });

        // Delete local + remote
        let has_remote = matches!(&branch.tracking, TrackingStatus::Tracked { gone: false, .. });
        items.push(ui::menu::MenuItem {
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
        });

        // Fast-forward: only for non-current branches with a tracked (non-gone) remote
        let has_live_remote = matches!(
            &branch.tracking,
            TrackingStatus::Tracked { gone: false, .. }
        );
        items.push(ui::menu::MenuItem {
            label: "Fast-forward".into(),
            enabled: !branch.is_current && has_live_remote,
            reason: if branch.is_current {
                Some("current".into())
            } else if !has_live_remote {
                Some("no remote".into())
            } else {
                None
            },
            shortcut: Some('f'),
        });

        // Push: only for branches that are ahead of their remote
        let is_ahead = branch.ahead.is_some_and(|a| a > 0);
        items.push(ui::menu::MenuItem {
            label: "Push".into(),
            enabled: is_ahead,
            reason: if !has_live_remote {
                Some("no remote".into())
            } else if !is_ahead {
                Some("not ahead".into())
            } else {
                None
            },
            shortcut: Some('p'),
        });

        // Force Push: only for branches that are both ahead AND behind their remote
        let is_ahead_and_behind = is_ahead && branch.behind.is_some_and(|b| b > 0);
        items.push(ui::menu::MenuItem {
            label: "Force push".into(),
            enabled: is_ahead_and_behind,
            reason: if !has_live_remote {
                Some("no remote".into())
            } else if !is_ahead {
                Some("not ahead".into())
            } else if branch.behind.is_none_or(|b| b == 0) {
                Some("not behind".into())
            } else {
                None
            },
            shortcut: Some('P'),
        });

        // Pull: only for branches that are behind their remote tracking branch
        let is_behind = branch.behind.is_some_and(|b| b > 0);
        items.push(ui::menu::MenuItem {
            label: "Pull".into(),
            enabled: is_behind && has_live_remote,
            reason: if !has_live_remote {
                Some("no remote".into())
            } else if !is_behind {
                Some("not behind".into())
            } else {
                None
            },
            shortcut: Some('l'),
        });

        // Merge into base
        items.push(ui::menu::MenuItem {
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
        });

        // Squash merge into base
        items.push(ui::menu::MenuItem {
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
        });

        // Rebase onto base
        items.push(ui::menu::MenuItem {
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
        });

        // Create worktree
        items.push(ui::menu::MenuItem {
            label: "Create worktree".into(),
            enabled: !branch.is_current,
            reason: if branch.is_current {
                Some("current".into())
            } else {
                None
            },
            shortcut: Some('w'),
        });

        // Open PR in browser
        let has_pr = self.pr_map.contains_key(&branch.name);
        items.push(ui::menu::MenuItem {
            label: "Open PR in browser".into(),
            enabled: has_pr,
            reason: if !has_pr { Some("no PR".into()) } else { None },
            shortcut: Some('o'),
        });

        items
    }

    pub fn build_worktree_menu_items(&self) -> Vec<ui::menu::MenuItem> {
        if self.worktrees.is_empty() {
            return vec![];
        }
        let wt = &self.worktrees[self.worktree_cursor];
        let is_main = wt.is_main;
        let is_dirty = !wt.wt_status.is_clean();

        vec![
            ui::menu::MenuItem {
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
            },
            ui::menu::MenuItem {
                label: "Force remove worktree".into(),
                enabled: !is_main,
                reason: if is_main { Some("main worktree".into()) } else { None },
                shortcut: Some('D'),
            },
        ]
    }

    pub fn build_remote_menu_items(&self) -> Vec<ui::menu::MenuItem> {
        if self.remote_branches.is_empty() {
            return vec![];
        }
        let branch = &self.remote_branches[self.remote_cursor];
        let pinned = branch.is_pinned();
        let has_local = branch.has_local;
        let has_pr = self.pr_map.contains_key(&branch.short_name);

        vec![
            // 0: Checkout
            ui::menu::MenuItem {
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
            },
            // 1: Delete remote branch
            ui::menu::MenuItem {
                label: "Delete remote branch".into(),
                enabled: !pinned,
                reason: if pinned { Some("base".into()) } else { None },
                shortcut: Some('d'),
            },
            // 2: Delete remote + local
            ui::menu::MenuItem {
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
            },
            // 3: Fetch
            ui::menu::MenuItem {
                label: "Fetch remote".into(),
                enabled: !pinned,
                reason: if pinned { Some("base".into()) } else { None },
                shortcut: Some('f'),
            },
            // 4: Pull
            ui::menu::MenuItem {
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
            },
            // 5: Merge into current
            ui::menu::MenuItem {
                label: "Merge into current".into(),
                enabled: !pinned,
                reason: if pinned { Some("base".into()) } else { None },
                shortcut: Some('m'),
            },
            // 6: Cherry-pick latest
            ui::menu::MenuItem {
                label: "Cherry-pick latest".into(),
                enabled: !pinned,
                reason: if pinned { Some("base".into()) } else { None },
                shortcut: Some('p'),
            },
            // 7: View PR in browser
            ui::menu::MenuItem {
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
            },
        ]
    }

    fn handle_menu_key(&mut self, code: KeyCode) {
        let items = if self.prev_view == View::RemoteBranches {
            self.build_remote_menu_items()
        } else if self.prev_view == View::Worktrees {
            self.build_worktree_menu_items()
        } else {
            self.build_menu_items()
        };
        match code {
            KeyCode::Char('j') | KeyCode::Down => {
                if let View::Menu { ref mut cursor } = self.view {
                    let mut next = *cursor + 1;
                    while next < items.len() && !items[next].enabled {
                        next += 1;
                    }
                    if next < items.len() {
                        *cursor = next;
                    }
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if let View::Menu { ref mut cursor } = self.view {
                    let mut prev = *cursor;
                    loop {
                        if prev == 0 {
                            break;
                        }
                        prev -= 1;
                        if items[prev].enabled {
                            *cursor = prev;
                            break;
                        }
                    }
                }
            }
            KeyCode::Enter => {
                let menu_cursor = if let View::Menu { cursor } = &self.view {
                    *cursor
                } else {
                    return;
                };
                let item = &items[menu_cursor];
                if item.enabled {
                    if self.prev_view == View::Worktrees {
                        let action = match menu_cursor {
                            0 => BranchAction::WorktreeRemove,
                            1 => BranchAction::WorktreeForceRemove,
                            _ => return,
                        };
                        self.results_return_view = ResultsReturnView::Worktrees;
                        self.view = View::Confirm { action };
                        return;
                    }
                    if self.prev_view == View::RemoteBranches {
                        // View PR in browser — no confirm needed, fire and forget
                        if menu_cursor == 7 {
                            let branch = &self.remote_branches[self.remote_cursor];
                            let short_name = branch.short_name.clone();
                            let repo_path = self.repo_path.clone();
                            std::thread::spawn(move || {
                                let _ = std::process::Command::new("gh")
                                    .args(["pr", "view", "--web", &short_name])
                                    .current_dir(&repo_path)
                                    .status();
                            });
                            self.view = View::RemoteBranches;
                            return;
                        }
                        let action = match menu_cursor {
                            0 => BranchAction::CheckoutRemote,
                            1 => BranchAction::DeleteRemoteBranch,
                            2 => BranchAction::DeleteRemoteAndLocal,
                            3 => BranchAction::FetchRemote,
                            4 => BranchAction::PullRemote,
                            5 => BranchAction::MergeRemoteIntoCurrent,
                            6 => BranchAction::CherryPickRemote,
                            _ => return,
                        };
                        self.results_return_view = ResultsReturnView::RemoteBranches;
                        self.view = View::Confirm { action };
                        return;
                    }
                    // Open PR in browser — no confirm needed, fire and forget
                    if menu_cursor == 11 {
                        let branch_name = self.branches[self.cursor].name.clone();
                        let repo_path = self.repo_path.clone();
                        std::thread::spawn(move || {
                            let _ = std::process::Command::new("gh")
                                .args(["pr", "view", "--web", &branch_name])
                                .current_dir(&repo_path)
                                .status();
                        });
                        self.view = View::BranchList;
                        return;
                    }
                    let action = match menu_cursor {
                        0 => BranchAction::Checkout,
                        1 => BranchAction::DeleteLocal,
                        2 => BranchAction::DeleteLocalAndRemote,
                        3 => BranchAction::FastForward,
                        4 => BranchAction::Push,
                        5 => BranchAction::ForcePush,
                        6 => BranchAction::Pull,
                        7 => BranchAction::Merge,
                        8 => BranchAction::SquashMerge,
                        9 => BranchAction::Rebase,
                        10 => BranchAction::Worktree,
                        _ => return,
                    };
                    self.view = View::Confirm { action };
                }
            }
            KeyCode::Char(ch) => {
                if self.prev_view == View::Worktrees {
                    if let Some((idx, _)) = items.iter().enumerate().find(|(_, item)| item.shortcut == Some(ch) && item.enabled) {
                        let action = match idx {
                            0 => BranchAction::WorktreeRemove,
                            1 => BranchAction::WorktreeForceRemove,
                            _ => return,
                        };
                        self.results_return_view = ResultsReturnView::Worktrees;
                        self.view = View::Confirm { action };
                    } else if ch == 'q' {
                        self.view = self.prev_view.clone();
                    }
                    return;
                }
                if self.prev_view == View::RemoteBranches {
                    if let Some((idx, _)) = items.iter().enumerate().find(|(_, item)| item.shortcut == Some(ch) && item.enabled) {
                        // View PR in browser (index 7) — no confirm needed, fire and forget
                        if idx == 7 {
                            let branch = &self.remote_branches[self.remote_cursor];
                            let short_name = branch.short_name.clone();
                            let repo_path = self.repo_path.clone();
                            std::thread::spawn(move || {
                                let _ = std::process::Command::new("gh")
                                    .args(["pr", "view", "--web", &short_name])
                                    .current_dir(&repo_path)
                                    .status();
                            });
                            self.view = View::RemoteBranches;
                            return;
                        }
                        let action = match idx {
                            0 => BranchAction::CheckoutRemote,
                            1 => BranchAction::DeleteRemoteBranch,
                            2 => BranchAction::DeleteRemoteAndLocal,
                            3 => BranchAction::FetchRemote,
                            4 => BranchAction::PullRemote,
                            5 => BranchAction::MergeRemoteIntoCurrent,
                            6 => BranchAction::CherryPickRemote,
                            _ => return,
                        };
                        self.results_return_view = ResultsReturnView::RemoteBranches;
                        self.view = View::Confirm { action };
                    } else if ch == 'q' {
                        self.view = self.prev_view.clone();
                    }
                    return;
                }
                if let Some((idx, _)) = items.iter().enumerate().find(|(_, item)| item.shortcut == Some(ch) && item.enabled) {
                    // Open PR in browser (index 11) — no confirm needed, fire and forget
                    if idx == 11 {
                        let branch_name = self.branches[self.cursor].name.clone();
                        let repo_path = self.repo_path.clone();
                        std::thread::spawn(move || {
                            let _ = std::process::Command::new("gh")
                                .args(["pr", "view", "--web", &branch_name])
                                .current_dir(&repo_path)
                                .status();
                        });
                        self.view = View::BranchList;
                        return;
                    }
                    let action = match idx {
                        0 => BranchAction::Checkout,
                        1 => BranchAction::DeleteLocal,
                        2 => BranchAction::DeleteLocalAndRemote,
                        3 => BranchAction::FastForward,
                        4 => BranchAction::Push,
                        5 => BranchAction::ForcePush,
                        6 => BranchAction::Pull,
                        7 => BranchAction::Merge,
                        8 => BranchAction::SquashMerge,
                        9 => BranchAction::Rebase,
                        10 => BranchAction::Worktree,
                        _ => return,
                    };
                    self.view = View::Confirm { action };
                } else if ch == 'q' {
                    self.view = View::BranchList;
                }
            }
            KeyCode::Esc => {
                self.view = self.prev_view.clone();
            }
            _ => {}
        }
    }

    fn handle_settings_key(&mut self, code: KeyCode) {
        // Sort column cycle order: None, 0=name, 1=age, 2=ahead, 3=behind, 4=status
        const SORT_CYCLE: [Option<usize>; 6] = [None, Some(0), Some(1), Some(2), Some(3), Some(4)];

        match code {
            KeyCode::Char('j') | KeyCode::Down => {
                if let View::Settings { ref mut cursor } = self.view {
                    *cursor = (*cursor + 1).min(5); // 6 rows (index 0..=5)
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if let View::Settings { ref mut cursor } = self.view {
                    *cursor = cursor.saturating_sub(1);
                }
            }
            KeyCode::Right | KeyCode::Char('l') => {
                let cursor = if let View::Settings { cursor } = self.view { cursor } else { return };
                if cursor == 0 {
                    self.symbols = crate::ui::symbols::next(self.symbols);
                    self.config.symbols = Some(crate::ui::symbols::name(self.symbols).to_string());
                    self.config.save();
                } else if cursor == 1 {
                    self.theme = self.theme.next();
                    self.config.theme = Some(self.theme.name.to_string());
                    self.config.save();
                } else if cursor == 2 {
                    // Advance sort column forward through cycle
                    let pos = SORT_CYCLE.iter().position(|&c| c == self.sort_column).unwrap_or(0);
                    let next_pos = (pos + 1) % SORT_CYCLE.len();
                    self.sort_column = SORT_CYCLE[next_pos];
                    self.apply_sort();
                    self.config.sort_column = self.sort_column.map(|c| Self::sort_col_name(c).to_string());
                    self.config.sort_asc = Some(self.sort_ascending);
                    self.config.save();
                } else if cursor == 3 {
                    // Toggle sort direction
                    self.sort_ascending = !self.sort_ascending;
                    self.apply_sort();
                    self.config.sort_asc = Some(self.sort_ascending);
                    self.config.save();
                } else if cursor == 4 {
                    // Toggle auto-fetch
                    self.config.auto_fetch = Some(self.config.auto_fetch != Some(true));
                    self.config.save();
                } else if cursor == 5 {
                    self.config.load_worktrees_on_launch = Some(self.config.load_worktrees_on_launch != Some(true));
                    self.config.save();
                }
            }
            KeyCode::Left | KeyCode::Char('h') => {
                let cursor = if let View::Settings { cursor } = self.view { cursor } else { return };
                if cursor == 0 {
                    // backward = next() twice (3-cycle)
                    self.symbols = crate::ui::symbols::next(self.symbols);
                    self.symbols = crate::ui::symbols::next(self.symbols);
                    self.config.symbols = Some(crate::ui::symbols::name(self.symbols).to_string());
                    self.config.save();
                } else if cursor == 1 {
                    // backward = next() 3 times (4-cycle: dark→light→solarized→dracula)
                    self.theme = self.theme.next();
                    self.theme = self.theme.next();
                    self.theme = self.theme.next();
                    self.config.theme = Some(self.theme.name.to_string());
                    self.config.save();
                } else if cursor == 2 {
                    // Advance sort column backward through cycle
                    let pos = SORT_CYCLE.iter().position(|&c| c == self.sort_column).unwrap_or(0);
                    let next_pos = (pos + SORT_CYCLE.len() - 1) % SORT_CYCLE.len();
                    self.sort_column = SORT_CYCLE[next_pos];
                    self.apply_sort();
                    self.config.sort_column = self.sort_column.map(|c| Self::sort_col_name(c).to_string());
                    self.config.sort_asc = Some(self.sort_ascending);
                    self.config.save();
                } else if cursor == 3 {
                    // Toggle sort direction (same as right)
                    self.sort_ascending = !self.sort_ascending;
                    self.apply_sort();
                    self.config.sort_asc = Some(self.sort_ascending);
                    self.config.save();
                } else if cursor == 4 {
                    // Toggle auto-fetch (same as right)
                    self.config.auto_fetch = Some(self.config.auto_fetch != Some(true));
                    self.config.save();
                } else if cursor == 5 {
                    self.config.load_worktrees_on_launch = Some(self.config.load_worktrees_on_launch != Some(true));
                    self.config.save();
                }
            }
            KeyCode::Char(' ') => {
                // Space toggles on cursor==3 (sort direction), cursor==4 (auto-fetch), or cursor==5 (load worktrees)
                let cursor = if let View::Settings { cursor } = self.view { cursor } else { return };
                if cursor == 3 {
                    self.sort_ascending = !self.sort_ascending;
                    self.apply_sort();
                    self.config.sort_asc = Some(self.sort_ascending);
                    self.config.save();
                } else if cursor == 4 {
                    self.config.auto_fetch = Some(self.config.auto_fetch != Some(true));
                    self.config.save();
                } else if cursor == 5 {
                    self.config.load_worktrees_on_launch = Some(self.config.load_worktrees_on_launch != Some(true));
                    self.config.save();
                }
            }
            KeyCode::Esc => {
                self.view = View::BranchList;
            }
            _ => {}
        }
    }

    fn handle_search_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => {
                self.search_query.clear();
                self.search_active = false;
                self.reset_cursor_to_first_match();
            }
            KeyCode::Enter => {
                self.search_active = false;
            }
            KeyCode::Backspace => {
                self.search_query.pop();
                self.reset_cursor_to_first_match();
            }
            KeyCode::Char(c) => {
                self.search_query.push(c);
                self.reset_cursor_to_first_match();
            }
            _ => {}
        }
    }

    /// Returns true if the branch matches the current search query and filters.
    /// Always matches if the query is empty or the branch is pinned.
    pub fn matches_search(&self, branch: &BranchInfo) -> bool {
        if self.search_query.is_empty() {
            return true;
        }
        if branch.is_pinned() {
            return true;
        }

        let fs = FilterSet::parse(&self.search_query);
        if fs.is_empty() {
            return true;
        }

        // Text filter (AND)
        if !fs.text.is_empty()
            && !branch
                .name
                .to_lowercase()
                .contains(&fs.text.to_lowercase())
        {
            return false;
        }

        // Status filter (OR within group)
        if !fs.statuses.is_empty() && !fs.statuses.contains(&branch.merge_status) {
            return false;
        }

        // PR filter — positive and negative
        let has_pr = self.pr_map.contains_key(&branch.name);
        if fs.pr_yes && !has_pr {
            return false;
        }
        if fs.pr_no && has_pr {
            return false;
        }

        // Sync filter (OR within group)
        if fs.sync_ahead || fs.sync_behind {
            let is_ahead = branch.ahead.unwrap_or(0) > 0;
            let is_behind = branch.behind.unwrap_or(0) > 0;
            let matches_sync = (fs.sync_ahead && is_ahead) || (fs.sync_behind && is_behind);
            if !matches_sync {
                return false;
            }
        }

        // Age filters
        let age_secs = (chrono::Utc::now() - branch.last_commit_date).num_seconds();
        if let Some(threshold) = fs.age_newer_secs
            && age_secs > threshold
        {
            return false;
        }
        if let Some(threshold) = fs.age_older_secs
            && age_secs < threshold
        {
            return false;
        }

        true
    }

    /// Reset cursor to the first branch that matches the search filter.
    fn reset_cursor_to_first_match(&mut self) {
        reset_list_cursor(&mut BranchListNav { app: self });
    }

    pub fn has_selection(&self) -> bool {
        self.selected.iter().any(|&s| s)
    }

    pub fn selection_count(&self) -> usize {
        self.selected.iter().filter(|&&s| s).count()
    }

    pub fn selected_branch_names(&self) -> Vec<&str> {
        self.branches
            .iter()
            .zip(self.selected.iter())
            .filter(|&(_, &sel)| sel)
            .map(|(b, _)| b.name.as_str())
            .collect()
    }

    /// Returns the branches that will be targeted by the current action.
    /// If branches are selected, returns those; otherwise returns the cursor branch.
    pub fn target_branch_names(&self) -> Vec<&str> {
        let selected = self.selected_branch_names();
        if selected.is_empty() {
            vec![self.branches[self.cursor].name.as_str()]
        } else {
            selected
        }
    }

    pub fn has_tag_selection(&self) -> bool {
        self.tag_selected.iter().any(|&s| s)
    }

    pub fn tag_selection_count(&self) -> usize {
        self.tag_selected.iter().filter(|&&s| s).count()
    }

    pub fn selected_tag_names(&self) -> Vec<String> {
        self.tags
            .iter()
            .zip(self.tag_selected.iter())
            .filter(|&(_, &sel)| sel)
            .map(|(t, _)| t.name.clone())
            .collect()
    }

    /// Returns tag names targeted by the current action.
    /// If tags are selected, returns those; otherwise returns the cursor tag.
    pub fn target_tag_names(&self) -> Vec<String> {
        let selected = self.selected_tag_names();
        if selected.is_empty() && !self.tags.is_empty() {
            vec![self.tags[self.tag_cursor].name.clone()]
        } else {
            selected
        }
    }

    pub fn filtered_tag_indices(&self) -> Vec<usize> {
        self.tags
            .iter()
            .enumerate()
            .filter(|(_, tag)| self.matches_tag_search(tag))
            .map(|(i, _)| i)
            .collect()
    }

    pub fn filtered_branch_indices(&self) -> Vec<usize> {
        let filtered: Vec<usize> = self.branches
            .iter()
            .enumerate()
            .filter(|(_, b)| self.matches_search(b))
            .map(|(i, _)| i)
            .collect();

        // Return in display order: pinned first, then non-pinned
        let mut pinned: Vec<usize> = filtered.iter().copied()
            .filter(|&i| self.branches[i].is_pinned())
            .collect();
        let non_pinned: Vec<usize> = filtered.iter().copied()
            .filter(|&i| !self.branches[i].is_pinned())
            .collect();
        pinned.extend(non_pinned);
        pinned
    }

    pub fn filtered_remote_indices(&self) -> Vec<usize> {
        let fs = FilterSet::parse(&self.remote_search_query);
        let filtered: Vec<usize> = self.remote_branches
            .iter()
            .enumerate()
            .filter(|(_, b)| {
                if self.remote_search_query.is_empty() {
                    return true;
                }
                if b.is_pinned() {
                    return true;
                }
                // Status filter tokens
                if !fs.statuses.is_empty() && !fs.statuses.contains(&b.merge_status) {
                    return false;
                }
                // Text filter on branch name / remote / full ref
                if !fs.text.is_empty() {
                    let text = fs.text.to_lowercase();
                    if !b.short_name.to_lowercase().contains(&text)
                        && !b.remote.to_lowercase().contains(&text)
                        && !b.full_ref.to_lowercase().contains(&text)
                    {
                        return false;
                    }
                }
                true
            })
            .map(|(i, _)| i)
            .collect();

        // Return in display order: pinned first, then non-pinned
        let mut pinned: Vec<usize> = filtered.iter().copied()
            .filter(|&i| self.remote_branches[i].is_pinned())
            .collect();
        let non_pinned: Vec<usize> = filtered.iter().copied()
            .filter(|&i| !self.remote_branches[i].is_pinned())
            .collect();
        pinned.extend(non_pinned);
        pinned
    }

    pub fn matches_tag_search(&self, tag: &TagInfo) -> bool {
        if self.tag_search_query.is_empty() {
            return true;
        }

        let fs = FilterSet::parse(&self.tag_search_query);

        // Text filter: match against tag name and message
        if !fs.text.is_empty() {
            let text_lower = fs.text.to_lowercase();
            let name_lower = tag.name.to_lowercase();
            let msg_lower = tag.message.as_deref().unwrap_or("").to_lowercase();
            if !name_lower.contains(&text_lower) && !msg_lower.contains(&text_lower) {
                return false;
            }
        }

        // Age newer filter
        if let Some(threshold) = fs.age_newer_secs {
            let age_secs = (chrono::Utc::now() - tag.date).num_seconds();
            if age_secs > threshold {
                return false;
            }
        }

        // Age older filter
        if let Some(threshold) = fs.age_older_secs {
            let age_secs = (chrono::Utc::now() - tag.date).num_seconds();
            if age_secs < threshold {
                return false;
            }
        }

        true
    }

    fn reset_tag_cursor(&mut self) {
        let filtered = self.filtered_tag_indices();
        if filtered.is_empty() {
            return;
        }
        // If current cursor is in filtered list, keep it
        if filtered.contains(&self.tag_cursor) {
            let pos = filtered.iter().position(|&i| i == self.tag_cursor).unwrap();
            self.tag_table_state.select(Some(pos));
        } else {
            self.tag_cursor = filtered[0];
            self.tag_table_state.select(Some(0));
        }
    }

    fn reset_remote_cursor(&mut self) {
        reset_list_cursor(&mut RemoteListNav { app: self });
    }

    fn apply_tag_sort(&mut self) {
        if self.tag_sort_by_name {
            self.tags.sort_by(|a, b| a.name.cmp(&b.name));
        } else {
            self.tags.sort_by(|a, b| b.date.cmp(&a.date));
        }
        self.tag_selected = vec![false; self.tags.len()];
        self.tag_cursor = 0;
        self.tag_table_state.select(if self.tags.is_empty() { None } else { Some(0) });
    }

    fn apply_remote_sort(&mut self) {
        let Some(col) = self.remote_sort_column else { return };
        let asc = self.remote_sort_ascending;

        // Pinned branches stay at the top — sort only non-pinned
        let pin_count = self.remote_branches.iter().take_while(|b| b.is_pinned()).count();
        let sortable = &mut self.remote_branches[pin_count..];

        sortable.sort_by(|a, b| {
            let ord = match col {
                0 => a.short_name.cmp(&b.short_name),
                1 => a.last_commit_date.cmp(&b.last_commit_date),
                2 => {
                    let rank = |s: &MergeStatus| match s {
                        MergeStatus::Merged => 0,
                        MergeStatus::SquashMerged => 1,
                        MergeStatus::Unmerged => 2,
                    };
                    rank(&a.merge_status).cmp(&rank(&b.merge_status))
                }
                _ => std::cmp::Ordering::Equal,
            };
            if asc { ord } else { ord.reverse() }
        });

        // Reset selection and cursor after sort
        self.remote_selected = vec![false; self.remote_branches.len()];
        self.remote_cursor = 0;
        self.remote_table_state.select(if self.remote_branches.is_empty() { None } else { Some(0) });
    }

    fn apply_worktree_sort(&mut self) {
        let Some(col) = self.worktree_sort_column else { return };
        let asc = self.worktree_sort_ascending;

        // Pinned worktrees (is_main) stay at the top — sort only non-pinned
        let pin_count = self.worktrees.iter().take_while(|w| w.is_pinned()).count();
        let sortable = &mut self.worktrees[pin_count..];

        sortable.sort_by(|a, b| {
            let ord = match col {
                0 => a.branch.as_deref().unwrap_or("").cmp(b.branch.as_deref().unwrap_or("")),
                1 => a.path.cmp(&b.path),
                2 => a.age_date.cmp(&b.age_date),
                3 => {
                    let rank = |s: &MergeStatus| match s {
                        MergeStatus::Merged => 0,
                        MergeStatus::SquashMerged => 1,
                        MergeStatus::Unmerged => 2,
                    };
                    rank(&a.merge_status).cmp(&rank(&b.merge_status))
                }
                _ => std::cmp::Ordering::Equal,
            };
            if asc { ord } else { ord.reverse() }
        });

        // Re-sync selection and cursor after sort
        self.worktree_selected = vec![false; self.worktrees.len()];
        self.worktree_cursor = 0;
        self.worktree_table_state.select(if self.worktrees.is_empty() { None } else { Some(0) });
    }
}

/// Parsed filter set from the search query.
#[derive(Debug, Default)]
pub struct FilterSet {
    pub statuses: Vec<MergeStatus>,
    pub pr_yes: bool,
    pub pr_no: bool,
    pub sync_ahead: bool,
    pub sync_behind: bool,
    pub age_newer_secs: Option<i64>,
    pub age_older_secs: Option<i64>,
    pub text: String,
}

impl FilterSet {
    pub fn parse(query: &str) -> Self {
        let mut fs = FilterSet::default();
        for token in query.split_whitespace() {
            if let Some(val) = token.strip_prefix("status:") {
                match val {
                    "merged" => fs.statuses.push(MergeStatus::Merged),
                    "squash" => fs.statuses.push(MergeStatus::SquashMerged),
                    "unmerged" => fs.statuses.push(MergeStatus::Unmerged),
                    _ => {}
                }
            } else if let Some(val) = token.strip_prefix("pr:") {
                match val {
                    "yes" => fs.pr_yes = true,
                    "no" => fs.pr_no = true,
                    _ => {}
                }
            } else if let Some(val) = token.strip_prefix("sync:") {
                match val {
                    "ahead" => fs.sync_ahead = true,
                    "behind" => fs.sync_behind = true,
                    _ => {}
                }
            } else if let Some(val) = token.strip_prefix("age:<") {
                if let Some(secs) = parse_age_duration(val) {
                    fs.age_newer_secs = Some(secs);
                }
            } else if let Some(val) = token.strip_prefix("age:>") {
                if let Some(secs) = parse_age_duration(val) {
                    fs.age_older_secs = Some(secs);
                }
            } else {
                // Plain text — append to text filter
                if !fs.text.is_empty() {
                    fs.text.push(' ');
                }
                fs.text.push_str(token);
            }
        }
        fs
    }

    pub fn is_empty(&self) -> bool {
        self.statuses.is_empty()
            && !self.pr_yes
            && !self.pr_no
            && !self.sync_ahead
            && !self.sync_behind
            && self.age_newer_secs.is_none()
            && self.age_older_secs.is_none()
            && self.text.is_empty()
    }

    /// Check if a token string is present in the query.
    pub fn has_token(query: &str, token: &str) -> bool {
        query.split_whitespace().any(|t| t == token)
    }

    /// Toggle a token in the query string. Returns the new query.
    pub fn toggle_token(query: &str, token: &str) -> String {
        if Self::has_token(query, token) {
            // Remove the token
            query
                .split_whitespace()
                .filter(|t| *t != token)
                .collect::<Vec<_>>()
                .join(" ")
        } else {
            let trimmed = query.trim();
            if trimmed.is_empty() {
                token.to_string()
            } else {
                format!("{} {}", trimmed, token)
            }
        }
    }
}

/// Parse age duration string like "7d", "30d", "6m", "1y" into seconds.
fn parse_age_duration(s: &str) -> Option<i64> {
    if s.is_empty() {
        return None;
    }
    let (num_str, suffix) = s.split_at(s.len() - 1);
    let num: i64 = num_str.parse().ok()?;
    match suffix {
        "d" => Some(num * 86400),
        "w" => Some(num * 604800),
        "m" => Some(num * 2_592_000), // 30 days
        "y" => Some(num * 31_536_000), // 365 days
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Shared ListNav free functions
// ---------------------------------------------------------------------------

fn nav_down(list: &mut impl ListNav) {
    let indices = list.display_indices();
    let len = indices.len();
    if len == 0 { return; }
    let pos = indices.iter().position(|&i| i == list.cursor()).unwrap_or(0);
    if pos + 1 < len {
        list.set_cursor(indices[pos + 1], pos + 1);
    }
}

fn nav_up(list: &mut impl ListNav) {
    let indices = list.display_indices();
    if indices.is_empty() { return; }
    let pos = indices.iter().position(|&i| i == list.cursor()).unwrap_or(0);
    if pos > 0 {
        list.set_cursor(indices[pos - 1], pos - 1);
    }
}

fn nav_page_down(list: &mut impl ListNav) {
    let indices = list.display_indices();
    let len = indices.len();
    if len == 0 { return; }
    let pos = indices.iter().position(|&i| i == list.cursor()).unwrap_or(0);
    let new_pos = (pos + 20).min(len - 1);
    list.set_cursor(indices[new_pos], new_pos);
}

fn nav_page_up(list: &mut impl ListNav) {
    let indices = list.display_indices();
    if indices.is_empty() { return; }
    let pos = indices.iter().position(|&i| i == list.cursor()).unwrap_or(0);
    let new_pos = pos.saturating_sub(20);
    list.set_cursor(indices[new_pos], new_pos);
}

fn nav_home(list: &mut impl ListNav) {
    let indices = list.display_indices();
    if let Some(&idx) = indices.first() {
        list.set_cursor(idx, 0);
    }
}

fn nav_end(list: &mut impl ListNav) {
    let indices = list.display_indices();
    if let Some(&idx) = indices.last() {
        list.set_cursor(idx, indices.len() - 1);
    }
}

fn select_toggle(list: &mut impl ListNav) {
    let cursor = list.cursor();
    if list.is_selectable(cursor) {
        let sel = list.selection_mut();
        sel[cursor] = !sel[cursor];
    }
}

fn select_all(list: &mut impl ListNav) {
    let flags: Vec<bool> = (0..list.selection().len())
        .map(|i| list.is_selectable(i))
        .collect();
    let sel = list.selection_mut();
    for (i, &can_select) in flags.iter().enumerate() {
        sel[i] = can_select;
    }
}

fn deselect_all(list: &mut impl ListNav) {
    list.selection_mut().fill(false);
}

fn select_merged(list: &mut impl ListNav) {
    let flags: Vec<bool> = (0..list.selection().len())
        .map(|i| {
            list.is_selectable(i)
                && matches!(
                    list.merge_status(i),
                    MergeStatus::Merged | MergeStatus::SquashMerged
                )
        })
        .collect();
    let sel = list.selection_mut();
    for (i, &flag) in flags.iter().enumerate() {
        sel[i] = flag;
    }
}

fn invert_selection(list: &mut impl ListNav) {
    let flags: Vec<bool> = (0..list.selection().len())
        .map(|i| list.is_selectable(i))
        .collect();
    let sel = list.selection_mut();
    for (i, &can_select) in flags.iter().enumerate() {
        if can_select {
            sel[i] = !sel[i];
        }
    }
}

fn list_click_row(list: &mut impl ListNav, display_row: usize) {
    let indices = list.display_indices();
    if let Some(&raw_idx) = indices.get(display_row) {
        list.set_cursor(raw_idx, display_row);
        if list.is_selectable(raw_idx) {
            let sel = list.selection_mut();
            sel[raw_idx] = !sel[raw_idx];
        }
    }
}

fn list_right_click_row(list: &mut impl ListNav, display_row: usize) -> bool {
    let indices = list.display_indices();
    if let Some(&raw_idx) = indices.get(display_row) {
        list.set_cursor(raw_idx, display_row);
        true
    } else {
        false
    }
}

fn reset_list_cursor(list: &mut impl ListNav) {
    let indices = list.display_indices();
    if indices.is_empty() {
        return;
    }
    let cursor = list.cursor();
    if let Some(pos) = indices.iter().position(|&i| i == cursor) {
        list.set_cursor(cursor, pos);
    } else {
        list.set_cursor(indices[0], 0);
    }
}

