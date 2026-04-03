# Phase 4: App Shell & Integration

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire everything together into a running application. Build the App struct, event loop, key/mouse dispatch, background channel management, and main.rs startup sequence. After this phase, the application is fully functional.

**Architecture:** The App struct is much smaller than the current ~4000 lines because view state lives in `ListState<T>` instances, rendering lives in `ui/`, and git operations live in `git/`. App owns the 4 list states, an overlay stack, background channels, and config. The event loop drains channels generically and dispatches events to the appropriate handler.

**Tech Stack:** Rust, ratatui 0.30, crossterm 0.29

**Prerequisites:** Phases 1, 2, and 3 must all be complete.

**Reference:** Current `src/app.rs` (~3900 lines). The rewrite target is ~500-800 lines.

---

### Task 1: App Struct & Overlay Enum

**Files:**
- Create: `src/app.rs`

- [ ] **Step 1: Define Overlay enum**

```rust
use crate::types::*;
use crate::view::ViewId;

/// Overlay views rendered on top of the active primary view.
/// At most one overlay is active at a time.
#[derive(Debug)]
pub enum Overlay {
    Help,
    Menu { cursor: usize, items: Vec<crate::ui::menu::MenuItem> },
    Confirm { action: BranchAction, item_names: Vec<String> },
    Executing { label: String },
    Results { results: Vec<OperationResult>, return_view: ViewId },
    Settings { cursor: usize },
    Filter,
}
```

- [ ] **Step 2: Define App struct**

```rust
use crate::config::Config;
use crate::git::cache::BranchCache;
use crate::symbols::SymbolSet;
use crate::theme::Theme;
use crate::view::list_state::ListState;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub struct App {
    // Core
    pub repo_path: PathBuf,
    pub base_branch: String,
    pub config: Config,
    pub theme: Theme,
    pub symbols: SymbolSet,
    pub should_exit: bool,

    // View state — 4 peers
    pub active_view: ViewId,
    pub branches: ListState<BranchInfo>,
    pub remotes: ListState<RemoteBranchInfo>,
    pub tags: ListState<TagInfo>,
    pub worktrees: ListState<WorktreeInfo>,

    // Overlay
    pub overlay: Option<Overlay>,

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
    pub remote_fetch_rx: Option<Receiver<bool>>,
    pub tag_load_rx: Option<Receiver<Vec<TagInfo>>>,
    pub worktree_load_rx: Option<Receiver<Vec<WorktreeInfo>>>,
    pub remote_load_rx: Option<Receiver<Vec<RemoteBranchInfo>>>,

    // Working tree status (for the main repo)
    pub working_tree_status: WorkingTreeStatus,

    // Cache
    pub cache: BranchCache,

    // Toast
    pub toast: Option<crate::ui::toast::Toast>,

    // Operation cancellation
    pub cancel_flag: Option<Arc<AtomicBool>>,

    // Terminal dimensions (for mouse handling)
    pub terminal_rows: u16,
}
```

- [ ] **Step 3: Implement App constructor**

```rust
impl App {
    pub fn new(
        repo_path: PathBuf,
        base_branch: String,
        branches: Vec<BranchInfo>,
        working_tree_status: WorkingTreeStatus,
        cache: BranchCache,
        config: Config,
    ) -> Self {
        let theme = Theme::from_name(config.theme.as_deref().unwrap_or("dark"));
        let symbols = SymbolSet::from_name(
            config.symbols.as_deref().unwrap_or("auto"),
        );

        Self {
            repo_path,
            base_branch,
            config,
            theme,
            symbols,
            should_exit: false,
            active_view: ViewId::Branches,
            branches: ListState::new(branches),
            remotes: ListState::empty(),
            tags: ListState::empty(),
            worktrees: ListState::empty(),
            overlay: None,
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
            remote_fetch_rx: None,
            tag_load_rx: None,
            worktree_load_rx: None,
            remote_load_rx: None,
            working_tree_status,
            cache,
            toast: None,
            cancel_flag: None,
            terminal_rows: 0,
        }
    }
}
```

- [ ] **Step 4: Verify it compiles, commit**

```bash
git add src/app.rs src/lib.rs
git commit -m "feat: add App struct and Overlay enum"
```

---

### Task 2: Background Channel Draining

**Files:**
- Modify: `src/app.rs`

- [ ] **Step 1: Implement generic channel drain helper**

```rust
/// Try to receive from an optional channel. Returns items received.
/// Sets the channel to None when disconnected.
fn drain_channel<T>(rx: &mut Option<Receiver<T>>, max_per_tick: usize) -> Vec<T> {
    let Some(receiver) = rx.as_ref() else { return vec![]; };
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
```

- [ ] **Step 2: Implement drain_all_channels method on App**

```rust
impl App {
    pub fn drain_channels(&mut self) {
        // Squash-merge results (cap 32 per tick)
        for result in drain_channel(&mut self.squash_rx, 32) {
            self.squash_checked += 1;
            if let Some(branch) = self.branches.items_mut()
                .iter_mut()
                .find(|b| b.name == result.branch_name)
            {
                branch.merge_status = if result.is_squash_merged {
                    MergeStatus::SquashMerged
                } else {
                    MergeStatus::Unmerged
                };
            }
        }

        // Remote enrichment
        for result in drain_channel(&mut self.remote_enrich_rx, 32) {
            if let Some(branch) = self.remotes.items_mut()
                .iter_mut()
                .find(|b| b.full_ref == result.full_ref)
            {
                branch.merge_status = result.merge_status;
                branch.ahead = result.ahead;
                branch.behind = result.behind;
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
        }

        // Tag loading (one-shot)
        for tags in drain_channel(&mut self.tag_load_rx, 1) {
            self.tags.set_items(tags);
            self.tags.loading = false;
        }

        // Remote loading (one-shot)
        for remotes in drain_channel(&mut self.remote_load_rx, 1) {
            self.remotes.set_items(remotes);
            self.remotes.loading = false;
        }

        // Worktree loading (one-shot)
        for worktrees in drain_channel(&mut self.worktree_load_rx, 1) {
            self.worktrees.set_items(worktrees);
            self.worktrees.loading = false;
        }

        // Operation results (one-shot)
        for results in drain_channel(&mut self.op_rx, 1) {
            self.overlay = Some(Overlay::Results {
                results,
                return_view: self.active_view,
            });
        }

        // Progress updates
        for update in drain_channel(&mut self.progress_rx, 32) {
            // Update executing view progress display
        }

        // Expire toast
        if let Some(ref toast) = self.toast {
            if toast.is_expired() {
                self.toast = None;
            }
        }
    }
}
```

- [ ] **Step 3: Verify it compiles, commit**

```bash
git add src/app.rs
git commit -m "feat: add generic channel draining for all background tasks"
```

---

### Task 3: Event Loop

**Files:**
- Modify: `src/app.rs`

- [ ] **Step 1: Implement main run loop**

```rust
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io;
use std::time::Duration;

impl App {
    pub fn run(&mut self, terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
        loop {
            // Drain background channels
            self.drain_channels();

            // Render
            terminal.draw(|frame| {
                self.terminal_rows = frame.area().height;
                crate::ui::render::draw(frame, self);
            })?;

            if self.should_exit {
                return Ok(());
            }

            // Poll for events (50ms timeout)
            if event::poll(Duration::from_millis(50))? {
                let ev = event::read()?;
                self.handle_event(ev);
            }
        }
    }

    fn handle_event(&mut self, event: Event) {
        match event {
            Event::Key(key) => self.handle_key(key),
            Event::Mouse(mouse) => self.handle_mouse(mouse),
            Event::Resize(_, _) => {} // ratatui handles this
            _ => {}
        }
    }
}
```

- [ ] **Step 2: Verify it compiles, commit**

```bash
git add src/app.rs
git commit -m "feat: add main event loop with 50ms poll interval"
```

---

### Task 4: Key Dispatch

**Files:**
- Modify: `src/app.rs`

- [ ] **Step 1: Implement key dispatch**

```rust
impl App {
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

        // Global keys (work in every view)
        match key.code {
            KeyCode::Char('q') => { self.should_exit = true; return; }
            KeyCode::Char('?') => { self.overlay = Some(Overlay::Help); return; }
            KeyCode::Char(',') => { self.overlay = Some(Overlay::Settings { cursor: 0 }); return; }
            KeyCode::Char('T') => { self.theme = self.theme.next(); self.save_config(); return; }
            KeyCode::Char('Y') => { self.symbols = self.symbols.next(); self.save_config(); return; }
            KeyCode::Char('f') => { self.start_fetch(false); return; }
            KeyCode::Char('F') => { self.start_fetch(true); return; }
            KeyCode::Tab => { self.active_view = self.active_view.next(); self.ensure_view_loaded(); return; }
            KeyCode::BackTab => { self.active_view = self.active_view.prev(); self.ensure_view_loaded(); return; }
            KeyCode::Char('/') => { self.toggle_search(); return; }
            KeyCode::Char('\\') => { self.overlay = Some(Overlay::Filter); return; }
            _ => {}
        }

        // Common navigation/selection keys (work in every view)
        self.handle_common_list_key(key);

        // View-specific keys
        match self.active_view {
            ViewId::Branches => self.handle_branches_key(key),
            ViewId::Remotes => self.handle_remotes_key(key),
            ViewId::Tags => self.handle_tags_key(key),
            ViewId::Worktrees => self.handle_worktrees_key(key),
        }
    }

    /// Keys shared by all 4 views: navigation, selection, sorting
    fn handle_common_list_key(&mut self, key: KeyEvent) {
        use crate::view::list_state::*;

        // Dispatch to the active view's ListState
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
            KeyCode::Char('j') | KeyCode::Down => with_state!(nav_down),
            KeyCode::Char('k') | KeyCode::Up => with_state!(nav_up),
            KeyCode::PageDown => with_state!(|s| nav_page_down(s, 20)),
            KeyCode::PageUp => with_state!(|s| nav_page_up(s, 20)),
            KeyCode::Home => with_state!(nav_home),
            KeyCode::End => with_state!(nav_end),
            KeyCode::Char(' ') => with_state!(select_toggle),
            KeyCode::Char('a') => with_state!(select_all),
            KeyCode::Char('n') => with_state!(deselect_all),
            KeyCode::Char('i') => with_state!(invert_selection),
            KeyCode::Char('m') => with_state!(select_merged),
            KeyCode::Char('s') => { /* cycle sort column per view */ }
            KeyCode::Char('S') => { /* toggle sort direction per view */ }
            KeyCode::Enter => self.open_context_menu(),
            _ => {}
        }
    }
}
```

- [ ] **Step 2: Implement view-specific key handlers**

```rust
impl App {
    fn handle_branches_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('d') => self.delete_selected_branches(false),
            KeyCode::Char('D') => self.delete_selected_branches(true),
            KeyCode::Char('p') => self.push_selected_branches(),
            KeyCode::Char('R') => self.clear_cache_and_refresh(),
            // Direct view toggle keys
            KeyCode::Char('r') => { self.active_view = ViewId::Remotes; self.ensure_view_loaded(); }
            KeyCode::Char('t') => { self.active_view = ViewId::Tags; self.ensure_view_loaded(); }
            KeyCode::Char('w') => { self.active_view = ViewId::Worktrees; self.ensure_view_loaded(); }
            _ => {}
        }
    }

    fn handle_remotes_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('d') => self.delete_selected_remote_branches(),
            KeyCode::Char('r') | KeyCode::Esc => { self.active_view = ViewId::Branches; }
            KeyCode::Char('t') => { self.active_view = ViewId::Tags; self.ensure_view_loaded(); }
            KeyCode::Char('w') => { self.active_view = ViewId::Worktrees; self.ensure_view_loaded(); }
            _ => {}
        }
    }

    fn handle_tags_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('d') => self.delete_selected_tags(false),
            KeyCode::Char('D') => self.delete_selected_tags(true),
            KeyCode::Char('p') => self.push_selected_tags(),
            KeyCode::Char('t') | KeyCode::Esc => { self.active_view = ViewId::Branches; }
            KeyCode::Char('r') => { self.active_view = ViewId::Remotes; self.ensure_view_loaded(); }
            KeyCode::Char('w') => { self.active_view = ViewId::Worktrees; self.ensure_view_loaded(); }
            _ => {}
        }
    }

    fn handle_worktrees_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('d') => self.remove_selected_worktrees(false),
            KeyCode::Char('D') => self.remove_selected_worktrees(true),
            KeyCode::Char('w') | KeyCode::Esc => { self.active_view = ViewId::Branches; }
            KeyCode::Char('r') => { self.active_view = ViewId::Remotes; self.ensure_view_loaded(); }
            KeyCode::Char('t') => { self.active_view = ViewId::Tags; self.ensure_view_loaded(); }
            _ => {}
        }
    }
}
```

- [ ] **Step 3: Verify it compiles, commit**

```bash
git add src/app.rs
git commit -m "feat: add key dispatch with global, common, and view-specific handlers"
```

---

### Task 5: Mouse Handling

**Files:**
- Modify: `src/app.rs`

- [ ] **Step 1: Implement mouse handler**

```rust
use crossterm::event::{MouseEvent, MouseEventKind, MouseButton};

impl App {
    fn handle_mouse(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::ScrollDown => {
                // nav_down on active view
            }
            MouseEventKind::ScrollUp => {
                // nav_up on active view
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
        // 1. Check if click on header row (y == 1) → sort by column
        // 2. Check if click on status bar (y == terminal_rows - 1) → trigger action
        // 3. Check if click on checkbox column (x < 4) → toggle selection
        // 4. Otherwise → move cursor to clicked row
        //
        // Use header_columns and status_bar_items from active ListState
        // This logic is generic — same for all 4 views
    }

    fn handle_right_click(&mut self, x: u16, y: u16) {
        // Move cursor to clicked row, then open context menu
    }
}
```

- [ ] **Step 2: Verify it compiles, commit**

```bash
git add src/app.rs
git commit -m "feat: add mouse handling (scroll, click, right-click context menu)"
```

---

### Task 6: Action Execution

**Files:**
- Modify: `src/app.rs`

- [ ] **Step 1: Implement action spawning**

Methods that spawn background operation threads and set up the Executing overlay:

```rust
impl App {
    fn delete_selected_branches(&mut self, include_remote: bool) {
        let indices = self.branches.selected_indices();
        let names: Vec<String> = indices.iter()
            .map(|&i| self.branches.items()[i].name.clone())
            .filter(|name| {
                let b = self.branches.items().iter().find(|b| b.name == *name).unwrap();
                !b.is_pinned()
            })
            .collect();

        if names.is_empty() { return; }

        let action = if include_remote {
            BranchAction::DeleteLocalAndRemote
        } else {
            BranchAction::DeleteLocal
        };

        self.overlay = Some(Overlay::Confirm {
            action,
            item_names: names,
        });
    }

    pub fn execute_confirmed_action(&mut self, action: BranchAction, item_names: Vec<String>) {
        let repo_path = self.repo_path.clone();
        let base_branch = self.base_branch.clone();
        let (tx, rx) = std::sync::mpsc::channel();

        self.overlay = Some(Overlay::Executing {
            label: action.label().to_string(),
        });
        self.op_rx = Some(rx);

        std::thread::spawn(move || {
            let repo = git2::Repository::open(&repo_path).ok();
            let mut results = Vec::new();

            for name in &item_names {
                let result = match action {
                    BranchAction::DeleteLocal => {
                        if let Some(ref repo) = repo {
                            crate::git::operations::delete_local(repo, name)
                        } else {
                            continue;
                        }
                    }
                    // ... handle each action type
                    _ => continue,
                };
                results.push(result);
            }

            let _ = tx.send(results);
        });
    }

    fn ensure_view_loaded(&mut self) {
        match self.active_view {
            ViewId::Tags if self.tags.items().is_empty() && !self.tags.loading => {
                self.tags.loading = true;
                let repo_path = self.repo_path.clone();
                let (tx, rx) = std::sync::mpsc::channel();
                self.tag_load_rx = Some(rx);
                std::thread::spawn(move || {
                    if let Ok(repo) = git2::Repository::open(&repo_path) {
                        let tags = crate::git::tags::list_tags(&repo);
                        let _ = tx.send(tags);
                    }
                });
            }
            ViewId::Remotes if self.remotes.items().is_empty() && !self.remotes.loading => {
                self.remotes.loading = true;
                // Spawn remote loading thread
            }
            ViewId::Worktrees if self.worktrees.items().is_empty() && !self.worktrees.loading => {
                self.worktrees.loading = true;
                // Spawn worktree loading thread
            }
            _ => {}
        }
    }

    fn refresh_branches(&mut self) {
        // Re-run phase 1, spawn new squash checker
    }

    fn start_fetch(&mut self, prune: bool) {
        // Spawn fetch in background, set toast
    }

    fn save_config(&mut self) {
        self.config.theme = Some(self.theme.name.clone());
        self.config.symbols = Some(self.symbols.name().to_string());
        self.config.save();
    }
}
```

- [ ] **Step 2: Verify it compiles, commit**

```bash
git add src/app.rs
git commit -m "feat: add action execution with background threading"
```

---

### Task 7: Overlay Key Handling

**Files:**
- Modify: `src/app.rs`

- [ ] **Step 1: Implement overlay key handlers**

```rust
impl App {
    fn handle_overlay_key(&mut self, key: KeyEvent) {
        let overlay = self.overlay.take();
        match overlay {
            Some(Overlay::Help) => {
                // Any key closes help
                // (don't put overlay back)
            }
            Some(Overlay::Confirm { action, item_names }) => {
                match key.code {
                    KeyCode::Char('y') | KeyCode::Enter => {
                        self.execute_confirmed_action(action, item_names);
                    }
                    KeyCode::Char('n') | KeyCode::Esc => {
                        // Cancel — don't put overlay back
                    }
                    _ => {
                        self.overlay = Some(Overlay::Confirm { action, item_names });
                    }
                }
            }
            Some(Overlay::Menu { cursor, items }) => {
                match key.code {
                    KeyCode::Char('j') | KeyCode::Down => {
                        let new_cursor = (cursor + 1).min(items.len().saturating_sub(1));
                        self.overlay = Some(Overlay::Menu { cursor: new_cursor, items });
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        let new_cursor = cursor.saturating_sub(1);
                        self.overlay = Some(Overlay::Menu { cursor: new_cursor, items });
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
                    _ => {
                        // Check shortcut keys
                        if let KeyCode::Char(c) = key.code {
                            if let Some((_, item)) = items.iter().enumerate()
                                .find(|(_, mi)| mi.shortcut == Some(c) && mi.enabled)
                            {
                                self.execute_menu_action(item.action);
                                return;
                            }
                        }
                        self.overlay = Some(Overlay::Menu { cursor, items });
                    }
                }
            }
            Some(Overlay::Results { return_view, .. }) => {
                // Any key returns to the view and refreshes
                self.active_view = return_view;
                self.refresh_branches();
            }
            Some(Overlay::Executing { label }) => {
                if key.code == KeyCode::Esc {
                    if let Some(flag) = &self.cancel_flag {
                        flag.store(true, Ordering::Relaxed);
                    }
                }
                self.overlay = Some(Overlay::Executing { label });
            }
            Some(Overlay::Settings { cursor }) => {
                // Handle settings navigation and toggling
                self.handle_settings_key(key, cursor);
            }
            Some(Overlay::Filter) => {
                self.handle_filter_key(key);
            }
            None => {}
        }
    }
}
```

- [ ] **Step 2: Verify it compiles, commit**

```bash
git add src/app.rs
git commit -m "feat: add overlay key handling (help, confirm, menu, results, settings, filter)"
```

---

### Task 8: main.rs Startup Sequence

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Implement main.rs**

```rust
use anyhow::Result;
use clap::Parser;
use crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    event::{EnableMouseCapture, DisableMouseCapture},
};
use git2::Repository;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io;

mod app;
mod cli;
mod config;
mod git;
mod symbols;
mod theme;
mod types;
mod ui;
mod view;

use cli::Cli;
use config::Config;

fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = Config::load();

    // Open repo
    let repo = Repository::discover(".")?;
    let repo_path = repo.workdir()
        .ok_or_else(|| anyhow::anyhow!("Not a git working directory"))?
        .to_path_buf();

    // Detect base branch
    let base_branch = git::branch::detect_base_branch(&repo, cli.base.as_deref())?;

    // Non-interactive mode
    if cli.list {
        let branches = git::branch::list_branches_phase1(&repo, &base_branch)?;
        for b in &branches {
            println!("{}", b.name);
        }
        return Ok(());
    }

    // Phase 1: synchronous branch load
    let mut branches = git::branch::list_branches_phase1(&repo, &base_branch)?;
    let working_tree_status = git::status::detect_working_tree_status(&repo);
    let cache = git::cache::BranchCache::load(&repo_path);

    // Collect squash-merge candidates
    let candidates: Vec<(String, String)> = branches
        .iter()
        .filter(|b| b.merge_status == types::MergeStatus::Pending)
        .filter_map(|b| {
            git::operations::get_commit_hash_str(&repo, &b.name)
                .map(|hash| (b.name.clone(), hash))
        })
        .collect();

    // Create app
    let mut app = app::App::new(
        repo_path.clone(),
        base_branch.clone(),
        branches,
        working_tree_status,
        cache.clone(),
        config,
    );

    // Apply CLI symbol override
    if let Some(ref sym) = cli.symbols {
        app.symbols = symbols::SymbolSet::from_name(sym);
    }

    // Spawn background enrichment
    app.squash_total = candidates.len();
    if !candidates.is_empty() {
        app.squash_rx = Some(git::squash_loader::spawn_squash_checker(
            repo_path.clone(),
            base_branch.clone(),
            candidates,
            cache,
        ));
    }

    // Spawn PR loader
    app.pr_rx = Some(git::pr_loader::spawn_pr_loader(repo_path.clone()));

    // Auto-fetch if configured
    if app.config.auto_fetch.unwrap_or(false) {
        let path = repo_path.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        app.remote_fetch_rx = Some(rx);
        std::thread::spawn(move || {
            let success = git::operations::fetch_sync(&path);
            let _ = tx.send(success);
        });
    }

    // Preload worktrees if configured
    if app.config.load_worktrees_on_launch.unwrap_or(false) {
        app.worktrees.loading = true;
        let path = repo_path.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        app.worktree_load_rx = Some(rx);
        std::thread::spawn(move || {
            let wts = git::worktree::list_worktrees(&path);
            let _ = tx.send(wts);
        });
    }

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Run app
    let result = app.run(&mut terminal);

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result.map_err(Into::into)
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build`
Expected: Compiles (there will be unresolved references to fill in from rendering — stub them).

- [ ] **Step 3: Commit**

```bash
git add src/main.rs
git commit -m "feat: add main.rs startup sequence with phase-1 load and background enrichment"
```

---

### Task 9: --list Non-Interactive Mode

**Files:**
- Already handled in Task 8 (the `if cli.list` branch)

- [ ] **Step 1: Test non-interactive mode**

Run from inside a git repo: `cargo run -- --list`
Expected: Prints branch names to stdout and exits.

- [ ] **Step 2: Commit if any fixes needed**

---

### Task 10: End-to-End Integration

- [ ] **Step 1: Run the full test suite**

Run: `cargo test`
Expected: All tests pass.

- [ ] **Step 2: Run clippy**

Run: `cargo clippy -- -D warnings`

- [ ] **Step 3: Test the TUI manually**

Run: `cargo run` from inside a git repository.
Expected:
- Branch list renders with correct columns
- Tab/Shift+Tab cycles through all 4 views
- Navigation (j/k) works
- Selection (space/a/n/i/m) works
- Sorting (s/S) works
- Search (/) works
- Filter (\) works
- Help (?) works
- Settings (,) works
- Context menu (Enter) works
- Mouse scroll/click works
- Right-click opens context menu
- q quits

- [ ] **Step 4: Final commit**

```bash
git add -A
git commit -m "feat: Phase 4 complete — fully functional git-branch-manager rewrite"
```
