# Phase 1: Core Types & Git Layer

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Establish all data types, configuration, CLI parsing, and git operations for the rewrite. This phase produces a fully testable git layer with no UI dependencies.

**Architecture:** Port the existing git layer with minor cleanups. Types are in a single `types.rs`. Git modules mirror the current structure. Config and CLI are standalone. All integration tests must pass.

**Tech Stack:** Rust (edition 2021), git2 0.20, clap 4, chrono 0.4, serde/serde_json, toml, dirs 6, thiserror 2, anyhow 1, tempfile 3 (dev)

**Prerequisites:** None — this is the first phase.

**Reference:** Current implementation in `src/types.rs`, `src/git/`, `src/config.rs`, `src/cli.rs`

---

### Task 1: Project Skeleton

**Files:**
- Create: `src/lib.rs`
- Create: `src/types.rs`
- Create: `src/config.rs`
- Create: `src/cli.rs`
- Create: `src/git/mod.rs`
- Modify: `Cargo.toml`

This task sets up the module structure. We create empty files with module declarations so subsequent tasks can fill them in without worrying about `mod` wiring.

- [ ] **Step 1: Update Cargo.toml**

The existing Cargo.toml has all needed dependencies. Verify it compiles with empty modules. The key dependencies are:

```toml
[package]
name = "git-branch-manager"
edition = "2021"

[dependencies]
ratatui = "0.30"
crossterm = "0.29"
git2 = "0.20"
clap = { version = "4", features = ["derive"] }
anyhow = "1"
thiserror = "2"
chrono = { version = "0.4", features = ["clock"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "1.0"
dirs = "6"

[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 2: Create `src/lib.rs`**

```rust
pub mod config;
pub mod git;
pub mod types;
```

- [ ] **Step 3: Create `src/git/mod.rs`**

```rust
pub mod branch;
pub mod cache;
pub mod github;
pub mod merge_detection;
pub mod operations;
pub mod pr_loader;
pub mod squash_loader;
pub mod status;
pub mod tags;
pub mod worktree;
```

- [ ] **Step 4: Create stub files for all git modules**

Create these files, each containing just enough to compile:

`src/types.rs`:
```rust
// Core data types — filled in Task 2-4
```

`src/config.rs`:
```rust
// Config system — filled in Task 5
```

`src/cli.rs`:
```rust
// CLI parsing — filled in Task 5
```

`src/git/branch.rs`, `src/git/cache.rs`, `src/git/github.rs`, `src/git/merge_detection.rs`, `src/git/operations.rs`, `src/git/pr_loader.rs`, `src/git/squash_loader.rs`, `src/git/status.rs`, `src/git/tags.rs`, `src/git/worktree.rs`:
```rust
// Stub — filled in later tasks
```

- [ ] **Step 5: Create stub `src/main.rs`**

```rust
fn main() {
    println!("git-branch-manager rewrite — Phase 1");
}
```

- [ ] **Step 6: Verify it compiles**

Run: `cargo build`
Expected: Compiles with no errors (warnings about unused modules are OK).

- [ ] **Step 7: Commit**

```bash
git add src/lib.rs src/types.rs src/config.rs src/cli.rs src/git/ src/main.rs Cargo.toml
git commit -m "feat: scaffold project skeleton for rewrite"
```

---

### Task 2: Core Enums & Utility Functions

**Files:**
- Modify: `src/types.rs`
- Test: `src/types.rs` (inline `#[cfg(test)]`)

- [ ] **Step 1: Write tests for format_age utilities**

Add to `src/types.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn format_age_just_now() {
        let now = Utc::now();
        assert_eq!(format_age(&now), "just now");
    }

    #[test]
    fn format_age_minutes() {
        let date = Utc::now() - chrono::Duration::minutes(5);
        assert_eq!(format_age(&date), "5 minutes ago");
    }

    #[test]
    fn format_age_singular_day() {
        let date = Utc::now() - chrono::Duration::days(1);
        assert_eq!(format_age(&date), "1 day ago");
    }

    #[test]
    fn format_age_plural_days() {
        let date = Utc::now() - chrono::Duration::days(3);
        assert_eq!(format_age(&date), "3 days ago");
    }

    #[test]
    fn format_age_short_days() {
        let date = Utc::now() - chrono::Duration::days(3);
        assert_eq!(format_age_short(&date), "3d");
    }

    #[test]
    fn format_age_short_weeks() {
        let date = Utc::now() - chrono::Duration::weeks(2);
        assert_eq!(format_age_short(&date), "2w");
    }

    #[test]
    fn format_age_short_months() {
        let date = Utc::now() - chrono::Duration::days(60);
        assert_eq!(format_age_short(&date), "2mo");
    }

    #[test]
    fn format_age_short_years() {
        let date = Utc::now() - chrono::Duration::days(400);
        assert_eq!(format_age_short(&date), "1y");
    }

    #[test]
    fn working_tree_status_clean() {
        let s = WorkingTreeStatus::clean();
        assert!(s.is_clean());
        assert_eq!(s.summary(), "");
    }

    #[test]
    fn working_tree_status_staged_only() {
        let s = WorkingTreeStatus { has_staged: true, has_unstaged: false, has_untracked: false };
        assert!(!s.is_clean());
        assert_eq!(s.summary(), "staged");
    }

    #[test]
    fn working_tree_status_all_three() {
        let s = WorkingTreeStatus { has_staged: true, has_unstaged: true, has_untracked: true };
        assert_eq!(s.summary(), "staged+unstaged+untracked");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib types`
Expected: FAIL — types not defined yet.

- [ ] **Step 3: Implement enums and utility types**

Write into `src/types.rs`:

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;

// --- Enums ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MergeStatus {
    Merged,
    SquashMerged,
    Unmerged,
    Pending,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrackingStatus {
    Tracked { remote_ref: String, gone: bool },
    Local,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrStatus {
    Draft,
    Open,
    Merged,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrInfo {
    pub number: u32,
    pub status: PrStatus,
}

pub type PrMap = HashMap<String, PrInfo>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchAction {
    // Local branch actions
    DeleteLocal,
    DeleteLocalAndRemote,
    Checkout,
    Fetch,
    FetchPrune,
    FastForward,
    Merge,
    SquashMerge,
    Rebase,
    Worktree,
    Push,
    ForcePush,
    Pull,
    // Tag actions
    DeleteTag,
    DeleteTagAndRemote,
    PushTag,
    // Remote branch actions
    DeleteRemoteBranch,
    DeleteRemoteAndLocal,
    CheckoutRemote,
    FetchRemote,
    PullRemote,
    MergeRemoteIntoCurrent,
    CherryPickRemote,
    ViewRemotePR,
    // Worktree actions
    WorktreeRemove,
    WorktreeForceRemove,
}

impl BranchAction {
    pub fn label(&self) -> &'static str {
        match self {
            Self::DeleteLocal => "Delete local",
            Self::DeleteLocalAndRemote => "Delete local + remote",
            Self::Checkout => "Checkout",
            Self::Fetch => "Fetch all",
            Self::FetchPrune => "Fetch + prune",
            Self::FastForward => "Fast-forward",
            Self::Merge => "Merge into base",
            Self::SquashMerge => "Squash merge into base",
            Self::Rebase => "Rebase onto base",
            Self::Worktree => "Create worktree",
            Self::Push => "Push",
            Self::ForcePush => "Force push",
            Self::Pull => "Pull",
            Self::DeleteTag => "Delete tag",
            Self::DeleteTagAndRemote => "Delete tag (local + remote)",
            Self::PushTag => "Push tag",
            Self::DeleteRemoteBranch => "Delete remote branch",
            Self::DeleteRemoteAndLocal => "Delete remote + local",
            Self::CheckoutRemote => "Checkout remote",
            Self::FetchRemote => "Fetch remote",
            Self::PullRemote => "Pull remote",
            Self::MergeRemoteIntoCurrent => "Merge into current",
            Self::CherryPickRemote => "Cherry-pick latest",
            Self::ViewRemotePR => "Open PR in browser",
            Self::WorktreeRemove => "Remove worktree",
            Self::WorktreeForceRemove => "Force remove worktree",
        }
    }
}

// --- Working Tree Status ---

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkingTreeStatus {
    pub has_staged: bool,
    pub has_unstaged: bool,
    pub has_untracked: bool,
}

impl WorkingTreeStatus {
    pub fn clean() -> Self {
        Self { has_staged: false, has_unstaged: false, has_untracked: false }
    }

    pub fn is_clean(&self) -> bool {
        !self.has_staged && !self.has_unstaged && !self.has_untracked
    }

    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if self.has_staged { parts.push("staged"); }
        if self.has_unstaged { parts.push("unstaged"); }
        if self.has_untracked { parts.push("untracked"); }
        parts.join("+")
    }
}

// --- Format Helpers ---

pub fn format_age(date: &DateTime<Utc>) -> String {
    let secs = Utc::now().signed_duration_since(*date).num_seconds().max(0);
    if secs < 60 { return "just now".to_string(); }
    let mins = secs / 60;
    if mins < 60 { return plural(mins, "minute"); }
    let hours = mins / 60;
    if hours < 24 { return plural(hours, "hour"); }
    let days = hours / 24;
    if days < 7 { return plural(days, "day"); }
    let weeks = days / 7;
    if weeks < 5 { return plural(weeks, "week"); }
    let months = days / 30;
    if months < 12 { return plural(months, "month"); }
    let years = days / 365;
    plural(years, "year")
}

pub fn format_age_short(date: &DateTime<Utc>) -> String {
    let secs = Utc::now().signed_duration_since(*date).num_seconds().max(0);
    if secs < 60 { return "now".to_string(); }
    let mins = secs / 60;
    if mins < 60 { return format!("{mins}m"); }
    let hours = mins / 60;
    if hours < 24 { return format!("{hours}h"); }
    let days = hours / 24;
    if days < 7 { return format!("{days}d"); }
    let weeks = days / 7;
    if weeks < 5 { return format!("{weeks}w"); }
    let months = days / 30;
    if months < 12 { return format!("{months}mo"); }
    let years = days / 365;
    format!("{years}y")
}

fn plural(n: i64, unit: &str) -> String {
    if n == 1 { format!("{n} {unit} ago") } else { format!("{n} {unit}s ago") }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib types`
Expected: All tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/types.rs
git commit -m "feat: add core enums, WorkingTreeStatus, and format_age utilities"
```

---

### Task 3: Data Model Structs

**Files:**
- Modify: `src/types.rs`
- Test: `src/types.rs` (inline tests)

- [ ] **Step 1: Write tests for data model structs**

Add to the `tests` module in `src/types.rs`:

```rust
    #[test]
    fn branch_info_is_pinned() {
        let b = BranchInfo {
            name: "main".into(),
            is_current: false,
            is_base: true,
            tracking: TrackingStatus::Local,
            ahead: None,
            behind: None,
            last_commit_date: Utc::now(),
            merge_status: MergeStatus::Unmerged,
        };
        assert!(b.is_pinned());
    }

    #[test]
    fn branch_info_not_pinned() {
        let b = BranchInfo {
            name: "feature/x".into(),
            is_current: false,
            is_base: false,
            tracking: TrackingStatus::Local,
            ahead: None,
            behind: None,
            last_commit_date: Utc::now(),
            merge_status: MergeStatus::Unmerged,
        };
        assert!(!b.is_pinned());
    }

    #[test]
    fn tag_info_age_short() {
        let t = TagInfo {
            name: "v1.0".into(),
            commit_hash: "abc1234".into(),
            date: Utc::now() - chrono::Duration::days(5),
            message: Some("Release 1.0".into()),
            is_annotated: true,
        };
        assert_eq!(t.age_short(), "5d");
    }

    #[test]
    fn worktree_info_main_is_pinned() {
        let w = WorktreeInfo {
            path: PathBuf::from("/repo"),
            branch: Some("main".into()),
            is_main: true,
            commit_hash: "abc1234".into(),
            wt_status: WorkingTreeStatus::clean(),
            age_date: Utc::now(),
            merge_status: MergeStatus::Unmerged,
            ahead: None,
            behind: None,
            pr: None,
        };
        assert!(w.is_pinned());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib types`
Expected: FAIL — BranchInfo, TagInfo, WorktreeInfo not defined.

- [ ] **Step 3: Implement data model structs**

Add to `src/types.rs` (below the enums, above the tests module):

```rust
// --- Data Model Structs ---

#[derive(Debug, Clone)]
pub struct BranchInfo {
    pub name: String,
    pub is_current: bool,
    pub is_base: bool,
    pub tracking: TrackingStatus,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
    pub last_commit_date: DateTime<Utc>,
    pub merge_status: MergeStatus,
}

impl BranchInfo {
    pub fn is_pinned(&self) -> bool {
        self.is_base || self.is_current
    }

    pub fn age_display(&self) -> String {
        format_age(&self.last_commit_date)
    }

    pub fn age_short(&self) -> String {
        format_age_short(&self.last_commit_date)
    }
}

#[derive(Debug, Clone)]
pub struct RemoteBranchInfo {
    pub full_ref: String,
    pub remote: String,
    pub short_name: String,
    pub has_local: bool,
    pub is_base: bool,
    pub last_commit_date: DateTime<Utc>,
    pub merge_status: MergeStatus,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
}

impl RemoteBranchInfo {
    pub fn is_pinned(&self) -> bool {
        self.is_base
    }

    pub fn age_display(&self) -> String {
        format_age(&self.last_commit_date)
    }

    pub fn age_short(&self) -> String {
        format_age_short(&self.last_commit_date)
    }
}

#[derive(Debug, Clone)]
pub struct WorktreeInfo {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub is_main: bool,
    pub commit_hash: String,
    pub wt_status: WorkingTreeStatus,
    pub age_date: DateTime<Utc>,
    pub merge_status: MergeStatus,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
    pub pr: Option<PrStatus>,
}

impl WorktreeInfo {
    pub fn is_pinned(&self) -> bool {
        self.is_main
    }

    pub fn age_display(&self) -> String {
        format_age(&self.age_date)
    }

    pub fn age_short(&self) -> String {
        format_age_short(&self.age_date)
    }
}

#[derive(Debug, Clone)]
pub struct TagInfo {
    pub name: String,
    pub commit_hash: String,
    pub date: DateTime<Utc>,
    pub message: Option<String>,
    pub is_annotated: bool,
}

impl TagInfo {
    pub fn age_display(&self) -> String {
        format_age(&self.date)
    }

    pub fn age_short(&self) -> String {
        format_age_short(&self.date)
    }
}

// --- Operation/Channel Types ---

#[derive(Debug, Clone)]
pub struct OperationResult {
    pub branch_name: String,
    pub action: BranchAction,
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct ProgressUpdate {
    pub completed: usize,
    pub total: usize,
    pub current_item: String,
}

#[derive(Debug, Clone)]
pub struct SquashResult {
    pub branch_name: String,
    pub is_squash_merged: bool,
}

#[derive(Debug, Clone)]
pub struct RemoteEnrichResult {
    pub full_ref: String,
    pub merge_status: MergeStatus,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct WorktreeEnrichResult {
    pub index: usize,
    pub wt_status: WorkingTreeStatus,
    pub age_date: DateTime<Utc>,
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib types`
Expected: All tests pass.

- [ ] **Step 5: Verify full build**

Run: `cargo build`
Expected: Compiles.

- [ ] **Step 6: Commit**

```bash
git add src/types.rs
git commit -m "feat: add BranchInfo, RemoteBranchInfo, WorktreeInfo, TagInfo data models"
```

---

### Task 4: Config System

**Files:**
- Modify: `src/config.rs`
- Test: `src/config.rs` (inline tests)

- [ ] **Step 1: Write tests for config**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults() {
        let c = Config::default();
        assert_eq!(c.symbols, None);
        assert_eq!(c.theme, None);
        assert_eq!(c.auto_fetch, None);
    }

    #[test]
    fn config_roundtrip_toml() {
        let mut c = Config::default();
        c.theme = Some("dracula".into());
        c.auto_fetch = Some(true);
        let toml_str = toml::to_string(&c).unwrap();
        let parsed: Config = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.theme, Some("dracula".into()));
        assert_eq!(parsed.auto_fetch, Some(true));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib config`
Expected: FAIL — Config not defined.

- [ ] **Step 3: Implement Config**

Write into `src/config.rs`:

```rust
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    pub symbols: Option<String>,
    pub theme: Option<String>,
    pub sort_column: Option<String>,
    pub sort_asc: Option<bool>,
    pub auto_fetch: Option<bool>,
    pub load_worktrees_on_launch: Option<bool>,
}

impl Config {
    pub fn load() -> Self {
        let path = Self::config_path();

        // Try new path first, then legacy path
        let content = fs::read_to_string(&path)
            .or_else(|_| fs::read_to_string(Self::legacy_config_path()))
            .unwrap_or_default();

        if content.is_empty() {
            return Self::default();
        }

        toml::from_str(&content).unwrap_or_default()
    }

    pub fn save(&self) {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(content) = toml::to_string(self) {
            let _ = fs::write(&path, content);
        }
    }

    fn config_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("git-branch-manager")
            .join("config.toml")
    }

    fn legacy_config_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("git-bm")
            .join("config.toml")
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib config`
Expected: All tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/config.rs
git commit -m "feat: add Config system with TOML persistence and legacy migration"
```

---

### Task 5: CLI Parsing

**Files:**
- Modify: `src/cli.rs`

- [ ] **Step 1: Implement CLI struct**

Write into `src/cli.rs`:

```rust
use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "git-branch-manager", about = "TUI for managing git branches")]
pub struct Cli {
    /// Override the auto-detected base branch
    #[arg(long)]
    pub base: Option<String>,

    /// Non-interactive: print branch list to stdout
    #[arg(long)]
    pub list: bool,

    /// Override symbol set (ascii, unicode, powerline)
    #[arg(long)]
    pub symbols: Option<String>,
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build`
Expected: Compiles.

- [ ] **Step 3: Commit**

```bash
git add src/cli.rs
git commit -m "feat: add CLI parsing with --base, --list, --symbols flags"
```

---

### Task 6: git/status.rs — Working Tree Status Detection

**Files:**
- Modify: `src/git/status.rs`
- Test: `tests/integration.rs`

- [ ] **Step 1: Write integration tests for status detection**

Create `tests/integration.rs`:

```rust
use git2::Repository;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn run_git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git command failed");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn setup_test_repo() -> (TempDir, Repository) {
    let dir = TempDir::new().unwrap();
    let path = dir.path();
    run_git(path, &["init", "-b", "main"]);
    run_git(path, &["config", "user.email", "test@test.com"]);
    run_git(path, &["config", "user.name", "Test"]);
    std::fs::write(path.join("README.md"), "init").unwrap();
    run_git(path, &["add", "."]);
    run_git(path, &["commit", "-m", "initial"]);
    let repo = Repository::open(path).unwrap();
    (dir, repo)
}

#[test]
fn test_wt_status_clean() {
    let (dir, repo) = setup_test_repo();
    let status = git_branch_manager::git::status::detect_working_tree_status(&repo);
    assert!(status.is_clean());
}

#[test]
fn test_wt_status_staged_only() {
    let (dir, repo) = setup_test_repo();
    std::fs::write(dir.path().join("new.txt"), "content").unwrap();
    run_git(dir.path(), &["add", "new.txt"]);
    let status = git_branch_manager::git::status::detect_working_tree_status(&repo);
    assert!(status.has_staged);
    assert!(!status.has_unstaged);
    assert!(!status.has_untracked);
}

#[test]
fn test_wt_status_unstaged_only() {
    let (dir, repo) = setup_test_repo();
    std::fs::write(dir.path().join("README.md"), "modified").unwrap();
    let status = git_branch_manager::git::status::detect_working_tree_status(&repo);
    assert!(!status.has_staged);
    assert!(status.has_unstaged);
}

#[test]
fn test_wt_status_untracked_only() {
    let (dir, repo) = setup_test_repo();
    std::fs::write(dir.path().join("untracked.txt"), "x").unwrap();
    let status = git_branch_manager::git::status::detect_working_tree_status(&repo);
    assert!(!status.has_staged);
    assert!(!status.has_unstaged);
    assert!(status.has_untracked);
}

#[test]
fn test_wt_status_all_three() {
    let (dir, repo) = setup_test_repo();
    std::fs::write(dir.path().join("staged.txt"), "s").unwrap();
    run_git(dir.path(), &["add", "staged.txt"]);
    std::fs::write(dir.path().join("README.md"), "modified").unwrap();
    std::fs::write(dir.path().join("untracked.txt"), "u").unwrap();
    let status = git_branch_manager::git::status::detect_working_tree_status(&repo);
    assert!(status.has_staged);
    assert!(status.has_unstaged);
    assert!(status.has_untracked);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test test_wt_status`
Expected: FAIL — module `status` has no function `detect_working_tree_status`.

- [ ] **Step 3: Implement status detection**

Write into `src/git/status.rs`:

```rust
use crate::types::WorkingTreeStatus;
use git2::{Repository, StatusOptions};

pub fn detect_working_tree_status(repo: &Repository) -> WorkingTreeStatus {
    let mut opts = StatusOptions::new();
    opts.include_untracked(true);

    let statuses = match repo.statuses(Some(&mut opts)) {
        Ok(s) => s,
        Err(_) => return WorkingTreeStatus::clean(),
    };

    let mut has_staged = false;
    let mut has_unstaged = false;
    let mut has_untracked = false;

    for entry in statuses.iter() {
        let s = entry.status();
        if s.intersects(
            git2::Status::INDEX_NEW
                | git2::Status::INDEX_MODIFIED
                | git2::Status::INDEX_DELETED
                | git2::Status::INDEX_RENAMED
                | git2::Status::INDEX_TYPECHANGE,
        ) {
            has_staged = true;
        }
        if s.intersects(
            git2::Status::WT_MODIFIED
                | git2::Status::WT_DELETED
                | git2::Status::WT_RENAMED
                | git2::Status::WT_TYPECHANGE,
        ) {
            has_unstaged = true;
        }
        if s.contains(git2::Status::WT_NEW) {
            has_untracked = true;
        }
    }

    WorkingTreeStatus { has_staged, has_unstaged, has_untracked }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test test_wt_status`
Expected: All 5 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/git/status.rs tests/integration.rs
git commit -m "feat: add working tree status detection"
```

---

### Task 7: git/cache.rs — Squash-Merge Cache

**Files:**
- Modify: `src/git/cache.rs`
- Test: `src/git/cache.rs` (inline tests)

- [ ] **Step 1: Write tests for cache**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn cache_insert_and_lookup() {
        let dir = TempDir::new().unwrap();
        let mut cache = BranchCache::load(dir.path());
        cache.insert("feature/x", &MergeStatus::SquashMerged, "abc123");
        assert_eq!(cache.lookup("feature/x", "abc123"), Some(MergeStatus::SquashMerged));
    }

    #[test]
    fn cache_unmerged_invalidated_on_new_commit() {
        let dir = TempDir::new().unwrap();
        let mut cache = BranchCache::load(dir.path());
        cache.insert("feature/x", &MergeStatus::Unmerged, "abc123");
        assert_eq!(cache.lookup("feature/x", "abc123"), Some(MergeStatus::Unmerged));
        assert_eq!(cache.lookup("feature/x", "def456"), None);
    }

    #[test]
    fn cache_merged_permanent() {
        let dir = TempDir::new().unwrap();
        let mut cache = BranchCache::load(dir.path());
        cache.insert("feature/x", &MergeStatus::Merged, "abc123");
        // Merged is permanent regardless of commit hash
        assert_eq!(cache.lookup("feature/x", "def456"), Some(MergeStatus::Merged));
    }

    #[test]
    fn cache_clear_removes_entries() {
        let dir = TempDir::new().unwrap();
        let mut cache = BranchCache::load(dir.path());
        cache.insert("feature/x", &MergeStatus::Merged, "abc123");
        cache.clear();
        assert_eq!(cache.lookup("feature/x", "abc123"), None);
    }

    #[test]
    fn cache_save_and_reload() {
        let dir = TempDir::new().unwrap();
        let mut cache = BranchCache::load(dir.path());
        cache.insert("feature/x", &MergeStatus::SquashMerged, "abc123");
        cache.save();

        let reloaded = BranchCache::load(dir.path());
        assert_eq!(reloaded.lookup("feature/x", "abc123"), Some(MergeStatus::SquashMerged));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib cache`
Expected: FAIL — BranchCache not defined.

- [ ] **Step 3: Implement cache**

Write into `src/git/cache.rs`:

```rust
use crate::types::MergeStatus;
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize)]
struct CacheEntry {
    merge_status: String,
    commit_hash: String,
}

pub struct BranchCache {
    path: PathBuf,
    entries: HashMap<String, CacheEntry>,
}

impl BranchCache {
    pub fn load(repo_path: &Path) -> Self {
        let path = cache_path(repo_path);
        let entries = fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self { path, entries }
    }

    pub fn save(&self) {
        if let Ok(json) = serde_json::to_string(&self.entries) {
            let _ = fs::write(&self.path, json);
        }
    }

    pub fn lookup(&self, branch_name: &str, current_commit_hash: &str) -> Option<MergeStatus> {
        let entry = self.entries.get(branch_name)?;
        let status = match entry.merge_status.as_str() {
            "merged" => MergeStatus::Merged,
            "squash_merged" => MergeStatus::SquashMerged,
            "unmerged" => MergeStatus::Unmerged,
            _ => return None,
        };
        match status {
            // Merged and SquashMerged are permanent
            MergeStatus::Merged | MergeStatus::SquashMerged => Some(status),
            // Unmerged is only valid if commit hasn't changed
            MergeStatus::Unmerged => {
                if entry.commit_hash == current_commit_hash {
                    Some(status)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    pub fn insert(&mut self, branch_name: &str, status: &MergeStatus, commit_hash: &str) {
        let status_str = match status {
            MergeStatus::Merged => "merged",
            MergeStatus::SquashMerged => "squash_merged",
            MergeStatus::Unmerged => "unmerged",
            MergeStatus::Pending => return, // Never cache Pending
        };
        self.entries.insert(
            branch_name.to_string(),
            CacheEntry {
                merge_status: status_str.to_string(),
                commit_hash: commit_hash.to_string(),
            },
        );
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        let _ = fs::remove_file(&self.path);
    }
}

fn cache_path(repo_path: &Path) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    repo_path.hash(&mut hasher);
    let hash = hasher.finish();
    PathBuf::from(format!("/tmp/git-bm-cache-{hash:x}.json"))
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib cache`
Expected: All 5 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/git/cache.rs
git commit -m "feat: add squash-merge result cache with persistence"
```

---

### Task 8: git/merge_detection.rs — Merge & Squash Detection

**Files:**
- Modify: `src/git/merge_detection.rs`
- Test: `tests/integration.rs` (add tests)

- [ ] **Step 1: Write integration tests**

Add to `tests/integration.rs`:

```rust
#[test]
fn test_merged_branch_detection() {
    let (dir, repo) = setup_test_repo();
    let path = dir.path();

    // Create and merge a branch
    run_git(path, &["checkout", "-b", "feature/merged"]);
    std::fs::write(path.join("feature.txt"), "feature content").unwrap();
    run_git(path, &["add", "."]);
    run_git(path, &["commit", "-m", "feature commit"]);
    run_git(path, &["checkout", "main"]);
    run_git(path, &["merge", "feature/merged", "--no-ff", "-m", "merge feature"]);

    let mut branches = git_branch_manager::git::branch::list_branches_phase1(&repo, "main").unwrap();
    git_branch_manager::git::merge_detection::detect_merged_branches(&repo, "main", &mut branches).unwrap();

    let feature = branches.iter().find(|b| b.name == "feature/merged").unwrap();
    assert_eq!(feature.merge_status, git_branch_manager::types::MergeStatus::Merged);
}

#[test]
fn test_squash_merged_detection() {
    let (dir, repo) = setup_test_repo();
    let path = dir.path();

    // Create a branch with content
    run_git(path, &["checkout", "-b", "feature/squashed"]);
    std::fs::write(path.join("squash.txt"), "squash content").unwrap();
    run_git(path, &["add", "."]);
    run_git(path, &["commit", "-m", "squash commit"]);
    run_git(path, &["checkout", "main"]);

    // Squash merge (--squash + commit)
    run_git(path, &["merge", "--squash", "feature/squashed"]);
    run_git(path, &["commit", "-m", "squashed feature"]);

    let is_squash = git_branch_manager::git::merge_detection::is_squash_merged(
        path, "main", "feature/squashed", None,
    );
    assert!(is_squash);
}

#[test]
fn test_unmerged_detection() {
    let (dir, repo) = setup_test_repo();
    let path = dir.path();

    run_git(path, &["checkout", "-b", "feature/unmerged"]);
    std::fs::write(path.join("unmerged.txt"), "unmerged").unwrap();
    run_git(path, &["add", "."]);
    run_git(path, &["commit", "-m", "unmerged commit"]);
    run_git(path, &["checkout", "main"]);

    let is_squash = git_branch_manager::git::merge_detection::is_squash_merged(
        path, "main", "feature/unmerged", None,
    );
    assert!(!is_squash);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test test_merged_branch_detection test_squash_merged_detection test_unmerged_detection`
Expected: FAIL — functions not defined.

- [ ] **Step 3: Implement merge detection**

Write into `src/git/merge_detection.rs`:

```rust
use crate::types::{BranchInfo, MergeStatus};
use git2::Repository;
use std::path::Path;
use std::process::Command;

pub fn detect_merged_branches(
    repo: &Repository,
    base_branch: &str,
    branches: &mut [BranchInfo],
) -> anyhow::Result<()> {
    let base_ref = repo
        .find_branch(base_branch, git2::BranchType::Local)?
        .get()
        .target()
        .ok_or_else(|| anyhow::anyhow!("base branch has no target"))?;

    for branch in branches.iter_mut() {
        if branch.is_base || branch.is_current {
            continue;
        }
        let branch_oid = match repo
            .find_branch(&branch.name, git2::BranchType::Local)
            .and_then(|b| b.get().target().ok_or_else(|| git2::Error::from_str("no target")))
        {
            Ok(oid) => oid,
            Err(_) => continue,
        };

        if repo.graph_descendant_of(base_ref, branch_oid).unwrap_or(false) {
            branch.merge_status = MergeStatus::Merged;
        }
    }
    Ok(())
}

pub fn is_squash_merged(
    repo_path: &Path,
    base_branch: &str,
    branch_name: &str,
    commit_hash: Option<&str>,
) -> bool {
    let git = |args: &[&str]| -> Option<String> {
        let out = Command::new("git")
            .args(args)
            .current_dir(repo_path)
            .stdin(std::process::Stdio::null())
            .output()
            .ok()?;
        if out.status.success() {
            Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
        } else {
            None
        }
    };

    // Step 1: find merge-base
    let ancestor = match git(&["merge-base", base_branch, branch_name]) {
        Some(a) if !a.is_empty() => a,
        _ => return false,
    };

    // Step 2: create temp commit-tree
    let tree_spec = if let Some(hash) = commit_hash {
        format!("{hash}^{{tree}}")
    } else {
        format!("{branch_name}^{{tree}}")
    };
    let temp_commit = match git(&["commit-tree", &tree_spec, "-p", &ancestor, "-m", "_"]) {
        Some(c) if !c.is_empty() => c,
        _ => return false,
    };

    // Step 3: cherry check
    match git(&["cherry", base_branch, &temp_commit]) {
        Some(result) => result.starts_with('-'),
        None => false,
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test test_merged_branch_detection test_squash_merged_detection test_unmerged_detection`
Expected: All 3 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/git/merge_detection.rs tests/integration.rs
git commit -m "feat: add regular merge and squash-merge detection"
```

---

### Task 9: git/branch.rs — Branch Listing & Base Detection

**Files:**
- Modify: `src/git/branch.rs`
- Test: `tests/integration.rs` (add tests)

- [ ] **Step 1: Write integration tests**

Add to `tests/integration.rs`:

```rust
#[test]
fn test_detect_base_branch_main() {
    let (_dir, repo) = setup_test_repo();
    let base = git_branch_manager::git::branch::detect_base_branch(&repo, None).unwrap();
    assert_eq!(base, "main");
}

#[test]
fn test_detect_base_branch_override() {
    let (dir, repo) = setup_test_repo();
    run_git(dir.path(), &["checkout", "-b", "develop"]);
    run_git(dir.path(), &["checkout", "main"]);
    let base = git_branch_manager::git::branch::detect_base_branch(&repo, Some("develop")).unwrap();
    assert_eq!(base, "develop");
}

#[test]
fn test_list_branches_phase1() {
    let (dir, repo) = setup_test_repo();
    let path = dir.path();

    run_git(path, &["checkout", "-b", "feature/a"]);
    std::fs::write(path.join("a.txt"), "a").unwrap();
    run_git(path, &["add", "."]);
    run_git(path, &["commit", "-m", "feature a"]);
    run_git(path, &["checkout", "main"]);

    let branches = git_branch_manager::git::branch::list_branches_phase1(&repo, "main").unwrap();
    assert!(branches.len() >= 2);
    let main_branch = branches.iter().find(|b| b.name == "main").unwrap();
    assert!(main_branch.is_base);
    let feature = branches.iter().find(|b| b.name == "feature/a").unwrap();
    assert!(!feature.is_base);
}

#[test]
fn test_delete_local_branch() {
    let (dir, repo) = setup_test_repo();
    let path = dir.path();
    run_git(path, &["checkout", "-b", "to-delete"]);
    run_git(path, &["checkout", "main"]);

    let result = git_branch_manager::git::operations::delete_local(&repo, "to-delete");
    assert!(result.success);

    let branches = git_branch_manager::git::branch::list_branches_phase1(&repo, "main").unwrap();
    assert!(branches.iter().all(|b| b.name != "to-delete"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test test_detect_base test_list_branches_phase1 test_delete_local`
Expected: FAIL — functions not defined.

- [ ] **Step 3: Implement branch listing**

Write into `src/git/branch.rs`:

```rust
use crate::types::*;
use chrono::{DateTime, TimeZone, Utc};
use git2::{BranchType, Repository};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum GitError {
    #[error("not a git repository")]
    NotARepo,
    #[error("base branch not found: {0}")]
    BaseBranchNotFound(String),
    #[error("cannot auto-detect base branch")]
    CannotDetectBase,
    #[error("command failed: {command}: {stderr}")]
    CommandFailed { command: String, stderr: String },
    #[error("parse error: {0}")]
    ParseError(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Git2(#[from] git2::Error),
}

type Result<T> = std::result::Result<T, GitError>;

pub fn detect_base_branch(repo: &Repository, override_base: Option<&str>) -> Result<String> {
    if let Some(base) = override_base {
        // Validate that the branch exists
        if repo.find_branch(base, BranchType::Local).is_ok() {
            return Ok(base.to_string());
        }
        return Err(GitError::BaseBranchNotFound(base.to_string()));
    }

    // Try remote HEAD symref
    if let Ok(reference) = repo.find_reference("refs/remotes/origin/HEAD") {
        if let Some(target) = reference.symbolic_target() {
            if let Some(name) = target.strip_prefix("refs/remotes/origin/") {
                if repo.find_branch(name, BranchType::Local).is_ok() {
                    return Ok(name.to_string());
                }
            }
        }
    }

    // Fallback: try common names
    for name in &["main", "master", "develop"] {
        if repo.find_branch(name, BranchType::Local).is_ok() {
            return Ok(name.to_string());
        }
    }

    // Last resort: first branch
    let branches = repo.branches(Some(BranchType::Local))?;
    for branch_result in branches {
        if let Ok((branch, _)) = branch_result {
            if let Some(name) = branch.name()? {
                return Ok(name.to_string());
            }
        }
    }

    Err(GitError::CannotDetectBase)
}

pub fn list_branches_phase1(repo: &Repository, base_branch: &str) -> Result<Vec<BranchInfo>> {
    let mut branches = collect_branch_metadata(repo, base_branch)?;
    super::merge_detection::detect_merged_branches(repo, base_branch, &mut branches)?;

    // Mark unmerged non-pinned branches as Pending (for squash check)
    for b in &mut branches {
        if !b.is_pinned() && b.merge_status == MergeStatus::Unmerged {
            b.merge_status = MergeStatus::Pending;
        }
    }

    // Sort: pinned first, then by date descending
    branches.sort_by(|a, b| {
        b.is_pinned()
            .cmp(&a.is_pinned())
            .then(b.last_commit_date.cmp(&a.last_commit_date))
    });

    Ok(branches)
}

pub fn list_remote_branches_phase1(
    repo: &Repository,
    base_branch: &str,
) -> Result<Vec<RemoteBranchInfo>> {
    let mut remotes = Vec::new();
    let branches = repo.branches(Some(BranchType::Remote))?;

    for branch_result in branches {
        let (branch, _) = branch_result?;
        let name = match branch.name()? {
            Some(n) => n.to_string(),
            None => continue,
        };

        // Skip HEAD pseudo-refs
        if name.ends_with("/HEAD") {
            continue;
        }

        let (remote, short_name) = match name.split_once('/') {
            Some((r, s)) => (r.to_string(), s.to_string()),
            None => continue,
        };

        let commit = branch.get().peel_to_commit()?;
        let time = commit.committer().when();
        let date = Utc.timestamp_opt(time.seconds(), 0).single().unwrap_or_else(Utc::now);

        let has_local = repo.find_branch(&short_name, BranchType::Local).is_ok();
        let is_base = short_name == base_branch;

        remotes.push(RemoteBranchInfo {
            full_ref: name,
            remote,
            short_name,
            has_local,
            is_base,
            last_commit_date: date,
            merge_status: if is_base { MergeStatus::Merged } else { MergeStatus::Pending },
            ahead: None,
            behind: None,
        });
    }

    remotes.sort_by(|a, b| {
        b.is_pinned()
            .cmp(&a.is_pinned())
            .then(b.last_commit_date.cmp(&a.last_commit_date))
    });

    Ok(remotes)
}

pub fn spawn_remote_enricher(
    repo_path: std::path::PathBuf,
    base_branch: String,
    branches: Vec<RemoteBranchInfo>,
) -> std::sync::mpsc::Receiver<RemoteEnrichResult> {
    let (tx, rx) = std::sync::mpsc::channel();

    std::thread::spawn(move || {
        let repo = match Repository::open(&repo_path) {
            Ok(r) => r,
            Err(_) => return,
        };

        // Prefer local base ref, fall back to remote tracking
        let base_oid = repo
            .find_branch(&base_branch, BranchType::Local)
            .or_else(|_| {
                repo.find_branch(
                    &format!("origin/{base_branch}"),
                    BranchType::Remote,
                )
            })
            .and_then(|b| b.get().target().ok_or_else(|| git2::Error::from_str("no target")))
            .ok();

        let base_oid = match base_oid {
            Some(oid) => oid,
            None => return,
        };

        for branch in &branches {
            if branch.is_base {
                continue;
            }

            let branch_ref = format!("refs/remotes/{}", branch.full_ref);
            let branch_oid = match repo
                .find_reference(&branch_ref)
                .and_then(|r| r.target().ok_or_else(|| git2::Error::from_str("no target")))
            {
                Ok(oid) => oid,
                Err(_) => continue,
            };

            let merge_status =
                if repo.graph_descendant_of(base_oid, branch_oid).unwrap_or(false) {
                    MergeStatus::Merged
                } else {
                    MergeStatus::Unmerged
                };

            let (ahead, behind) = repo
                .graph_ahead_behind(branch_oid, base_oid)
                .map(|(a, b)| (Some(a as u32), Some(b as u32)))
                .unwrap_or((None, None));

            let result = RemoteEnrichResult {
                full_ref: branch.full_ref.clone(),
                merge_status,
                ahead,
                behind,
            };
            if tx.send(result).is_err() {
                return;
            }
        }
    });

    rx
}

fn collect_branch_metadata(repo: &Repository, base_branch: &str) -> Result<Vec<BranchInfo>> {
    let head = repo.head().ok();
    let current_branch = head
        .as_ref()
        .and_then(|h| h.shorthand().map(|s| s.to_string()));

    let mut branches = Vec::new();
    let branch_iter = repo.branches(Some(BranchType::Local))?;

    for branch_result in branch_iter {
        let (branch, _) = branch_result?;
        let name = match branch.name()? {
            Some(n) => n.to_string(),
            None => continue,
        };

        let is_current = current_branch.as_deref() == Some(&name);
        let is_base = name == base_branch;

        // Tracking status
        let tracking = match branch.upstream() {
            Ok(upstream) => {
                let remote_ref = upstream
                    .name()?
                    .unwrap_or_default()
                    .to_string();
                let gone = upstream.get().target().is_none()
                    || repo.find_reference(upstream.get().name().unwrap_or_default()).is_err();
                TrackingStatus::Tracked { remote_ref, gone }
            }
            Err(_) => TrackingStatus::Local,
        };

        // Commit date
        let commit = branch.get().peel_to_commit()?;
        let time = commit.committer().when();
        let date = Utc
            .timestamp_opt(time.seconds(), 0)
            .single()
            .unwrap_or_else(Utc::now);

        // Ahead/behind (only for tracked, non-gone branches)
        let (ahead, behind) = match &tracking {
            TrackingStatus::Tracked { gone: false, .. } => {
                let branch_oid = commit.id();
                match branch.upstream() {
                    Ok(upstream) => {
                        let upstream_oid = upstream.get().peel_to_commit()?.id();
                        let (a, b) = repo.graph_ahead_behind(branch_oid, upstream_oid)?;
                        (Some(a as u32), Some(b as u32))
                    }
                    Err(_) => (None, None),
                }
            }
            _ => (None, None),
        };

        branches.push(BranchInfo {
            name,
            is_current,
            is_base,
            tracking,
            ahead,
            behind,
            last_commit_date: date,
            merge_status: MergeStatus::Unmerged, // detect_merged_branches fills this in
        });
    }

    Ok(branches)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test test_detect_base test_list_branches_phase1 test_delete_local`
Expected: Passes (test_delete_local depends on Task 10 — skip it for now if needed).

Run: `cargo test test_detect_base test_list_branches_phase1`
Expected: These 3 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/git/branch.rs tests/integration.rs
git commit -m "feat: add branch listing, base detection, and remote enrichment"
```

---

### Task 10: git/operations.rs — Git Operations

**Files:**
- Modify: `src/git/operations.rs`
- Test: `tests/integration.rs` (add tests)

This is a large file — port the existing operations from the current codebase. The key structure: a `git_cmd()` helper that creates `Command` with `GIT_TERMINAL_PROMPT=0` and `stdin(Stdio::null())`, then individual operation functions that use it.

- [ ] **Step 1: Write integration tests for key operations**

Add to `tests/integration.rs`:

```rust
fn setup_remote_test_repo() -> (TempDir, TempDir, Repository) {
    // Create bare remote
    let remote_dir = TempDir::new().unwrap();
    run_git(remote_dir.path(), &["init", "--bare"]);

    // Clone it
    let local_dir = TempDir::new().unwrap();
    let remote_url = remote_dir.path().to_str().unwrap();
    Command::new("git")
        .args(["clone", remote_url, local_dir.path().to_str().unwrap()])
        .output()
        .unwrap();

    let path = local_dir.path();
    run_git(path, &["config", "user.email", "test@test.com"]);
    run_git(path, &["config", "user.name", "Test"]);
    std::fs::write(path.join("README.md"), "init").unwrap();
    run_git(path, &["add", "."]);
    run_git(path, &["commit", "-m", "initial"]);
    run_git(path, &["push", "-u", "origin", "main"]);

    let repo = Repository::open(path).unwrap();
    (local_dir, remote_dir, repo)
}

#[test]
fn test_checkout_branch() {
    let (dir, repo) = setup_test_repo();
    let path = dir.path();
    run_git(path, &["checkout", "-b", "feature/checkout-test"]);
    run_git(path, &["checkout", "main"]);

    let result = git_branch_manager::git::operations::checkout_branch(&repo, path, "feature/checkout-test", false);
    assert!(result.success);
    let head = repo.head().unwrap();
    assert_eq!(head.shorthand().unwrap(), "feature/checkout-test");
}

#[test]
fn test_push_branch() {
    let (local_dir, _remote_dir, _repo) = setup_remote_test_repo();
    let path = local_dir.path();
    run_git(path, &["checkout", "-b", "feature/push-test"]);
    std::fs::write(path.join("push.txt"), "push").unwrap();
    run_git(path, &["add", "."]);
    run_git(path, &["commit", "-m", "push commit"]);

    let result = git_branch_manager::git::operations::push_branch(path, "feature/push-test");
    assert!(result.success);
}

#[test]
fn test_fetch_sync() {
    let (local_dir, _remote_dir, _repo) = setup_remote_test_repo();
    let result = git_branch_manager::git::operations::fetch_sync(local_dir.path());
    assert!(result);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test test_checkout_branch test_push_branch test_fetch_sync`
Expected: FAIL — functions not defined.

- [ ] **Step 3: Implement operations**

Write into `src/git/operations.rs`. This is the largest single file — port from the current codebase with the same function signatures:

```rust
use crate::types::{BranchAction, OperationResult};
use git2::Repository;
use std::path::Path;
use std::process::{Command, Stdio};

fn git_cmd(repo_path: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.current_dir(repo_path)
        .stdin(Stdio::null())
        .env("GIT_TERMINAL_PROMPT", "0");
    cmd
}

pub fn delete_local(repo: &Repository, branch_name: &str) -> OperationResult {
    match repo.find_branch(branch_name, git2::BranchType::Local) {
        Ok(mut branch) => match branch.delete() {
            Ok(()) => OperationResult {
                branch_name: branch_name.to_string(),
                action: BranchAction::DeleteLocal,
                success: true,
                message: format!("Deleted {branch_name}"),
            },
            Err(e) => OperationResult {
                branch_name: branch_name.to_string(),
                action: BranchAction::DeleteLocal,
                success: false,
                message: format!("Failed to delete {branch_name}: {e}"),
            },
        },
        Err(e) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::DeleteLocal,
            success: false,
            message: format!("Branch not found: {e}"),
        },
    }
}

pub fn checkout_branch(
    repo: &Repository,
    repo_path: &Path,
    branch_name: &str,
    stash: bool,
) -> OperationResult {
    let action = BranchAction::Checkout;

    if stash {
        let _ = git_cmd(repo_path).args(["stash", "push", "-m", "gbm-auto-stash"]).output();
    }

    let result = git_cmd(repo_path)
        .args(["checkout", branch_name])
        .output();

    if stash {
        let _ = git_cmd(repo_path).args(["stash", "pop"]).output();
    }

    match result {
        Ok(out) if out.status.success() => OperationResult {
            branch_name: branch_name.to_string(),
            action,
            success: true,
            message: format!("Checked out {branch_name}"),
        },
        Ok(out) => OperationResult {
            branch_name: branch_name.to_string(),
            action,
            success: false,
            message: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        },
        Err(e) => OperationResult {
            branch_name: branch_name.to_string(),
            action,
            success: false,
            message: e.to_string(),
        },
    }
}

pub fn fetch(repo_path: &Path) -> OperationResult {
    run_fetch_cmd(repo_path, false)
}

pub fn fetch_prune(repo_path: &Path) -> OperationResult {
    run_fetch_cmd(repo_path, true)
}

pub fn fetch_sync(repo_path: &Path) -> bool {
    let out = git_cmd(repo_path)
        .args(["fetch", "--all"])
        .output();
    matches!(out, Ok(o) if o.status.success())
}

fn run_fetch_cmd(repo_path: &Path, prune: bool) -> OperationResult {
    let mut args = vec!["fetch", "--all"];
    if prune {
        args.push("--prune");
    }
    let action = if prune { BranchAction::FetchPrune } else { BranchAction::Fetch };

    match git_cmd(repo_path).args(&args).output() {
        Ok(out) if out.status.success() => OperationResult {
            branch_name: String::new(),
            action,
            success: true,
            message: "Fetched all remotes".to_string(),
        },
        Ok(out) => OperationResult {
            branch_name: String::new(),
            action,
            success: false,
            message: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        },
        Err(e) => OperationResult {
            branch_name: String::new(),
            action,
            success: false,
            message: e.to_string(),
        },
    }
}

pub fn fast_forward(repo_path: &Path, branch_name: &str) -> OperationResult {
    let out = git_cmd(repo_path)
        .args(["fetch", "origin", &format!("{branch_name}:{branch_name}")])
        .output();

    match out {
        Ok(o) if o.status.success() => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::FastForward,
            success: true,
            message: format!("Fast-forwarded {branch_name}"),
        },
        Ok(o) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::FastForward,
            success: false,
            message: String::from_utf8_lossy(&o.stderr).trim().to_string(),
        },
        Err(e) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::FastForward,
            success: false,
            message: e.to_string(),
        },
    }
}

pub fn pull_branch(repo_path: &Path, branch_name: &str, is_current: bool) -> OperationResult {
    if is_current {
        let out = git_cmd(repo_path).args(["pull", "--ff-only"]).output();
        match out {
            Ok(o) if o.status.success() => OperationResult {
                branch_name: branch_name.to_string(),
                action: BranchAction::Pull,
                success: true,
                message: format!("Pulled {branch_name}"),
            },
            Ok(o) => OperationResult {
                branch_name: branch_name.to_string(),
                action: BranchAction::Pull,
                success: false,
                message: String::from_utf8_lossy(&o.stderr).trim().to_string(),
            },
            Err(e) => OperationResult {
                branch_name: branch_name.to_string(),
                action: BranchAction::Pull,
                success: false,
                message: e.to_string(),
            },
        }
    } else {
        fast_forward(repo_path, branch_name)
    }
}

pub fn push_branch(repo_path: &Path, branch_name: &str) -> OperationResult {
    let out = git_cmd(repo_path)
        .args(["push", "origin", branch_name])
        .output();

    match out {
        Ok(o) if o.status.success() => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::Push,
            success: true,
            message: format!("Pushed {branch_name}"),
        },
        Ok(o) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::Push,
            success: false,
            message: String::from_utf8_lossy(&o.stderr).trim().to_string(),
        },
        Err(e) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::Push,
            success: false,
            message: e.to_string(),
        },
    }
}

pub fn force_push_branch(repo_path: &Path, branch_name: &str) -> OperationResult {
    let out = git_cmd(repo_path)
        .args(["push", "--force-with-lease", "origin", branch_name])
        .output();

    match out {
        Ok(o) if o.status.success() => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::ForcePush,
            success: true,
            message: format!("Force pushed {branch_name}"),
        },
        Ok(o) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::ForcePush,
            success: false,
            message: String::from_utf8_lossy(&o.stderr).trim().to_string(),
        },
        Err(e) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::ForcePush,
            success: false,
            message: e.to_string(),
        },
    }
}

pub fn merge_branch(
    repo_path: &Path,
    branch_name: &str,
    base: &str,
    squash: bool,
    stash: bool,
) -> Vec<OperationResult> {
    let action = if squash { BranchAction::SquashMerge } else { BranchAction::Merge };

    if stash {
        let _ = git_cmd(repo_path).args(["stash", "push", "-m", "gbm-auto-stash"]).output();
    }

    // Checkout base
    let co = git_cmd(repo_path).args(["checkout", base]).output();
    if !matches!(&co, Ok(o) if o.status.success()) {
        if stash { let _ = git_cmd(repo_path).args(["stash", "pop"]).output(); }
        return vec![OperationResult {
            branch_name: branch_name.to_string(),
            action,
            success: false,
            message: format!("Failed to checkout {base}"),
        }];
    }

    let mut merge_args = vec!["merge"];
    if squash { merge_args.push("--squash"); }
    merge_args.push(branch_name);

    let out = git_cmd(repo_path).args(&merge_args).output();

    let result = match out {
        Ok(o) if o.status.success() => {
            if squash {
                let _ = git_cmd(repo_path)
                    .args(["commit", "-m", &format!("Squash merge {branch_name}")])
                    .output();
            }
            OperationResult {
                branch_name: branch_name.to_string(),
                action,
                success: true,
                message: format!("Merged {branch_name} into {base}"),
            }
        }
        _ => {
            let _ = git_cmd(repo_path).args(["merge", "--abort"]).output();
            OperationResult {
                branch_name: branch_name.to_string(),
                action,
                success: false,
                message: format!("Merge conflict — aborted"),
            }
        }
    };

    if stash { let _ = git_cmd(repo_path).args(["stash", "pop"]).output(); }
    vec![result]
}

pub fn rebase_branch(
    repo_path: &Path,
    branch_name: &str,
    base: &str,
    stash: bool,
) -> Vec<OperationResult> {
    if stash {
        let _ = git_cmd(repo_path).args(["stash", "push", "-m", "gbm-auto-stash"]).output();
    }

    let co = git_cmd(repo_path).args(["checkout", branch_name]).output();
    if !matches!(&co, Ok(o) if o.status.success()) {
        if stash { let _ = git_cmd(repo_path).args(["stash", "pop"]).output(); }
        return vec![OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::Rebase,
            success: false,
            message: format!("Failed to checkout {branch_name}"),
        }];
    }

    let out = git_cmd(repo_path).args(["rebase", base]).output();
    let result = match out {
        Ok(o) if o.status.success() => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::Rebase,
            success: true,
            message: format!("Rebased {branch_name} onto {base}"),
        },
        _ => {
            let _ = git_cmd(repo_path).args(["rebase", "--abort"]).output();
            OperationResult {
                branch_name: branch_name.to_string(),
                action: BranchAction::Rebase,
                success: false,
                message: "Rebase conflict — aborted".to_string(),
            }
        }
    };

    if stash { let _ = git_cmd(repo_path).args(["stash", "pop"]).output(); }
    vec![result]
}

pub fn checkout_remote_branch(
    repo_path: &Path,
    remote: &str,
    short_name: &str,
) -> OperationResult {
    let out = git_cmd(repo_path)
        .args(["checkout", "-b", short_name, "--track", &format!("{remote}/{short_name}")])
        .output();

    match out {
        Ok(o) if o.status.success() => OperationResult {
            branch_name: short_name.to_string(),
            action: BranchAction::CheckoutRemote,
            success: true,
            message: format!("Checked out {short_name} tracking {remote}/{short_name}"),
        },
        Ok(o) => OperationResult {
            branch_name: short_name.to_string(),
            action: BranchAction::CheckoutRemote,
            success: false,
            message: String::from_utf8_lossy(&o.stderr).trim().to_string(),
        },
        Err(e) => OperationResult {
            branch_name: short_name.to_string(),
            action: BranchAction::CheckoutRemote,
            success: false,
            message: e.to_string(),
        },
    }
}

pub fn delete_remotes_batch(repo_path: &Path, branch_names: &[String]) -> Vec<OperationResult> {
    if branch_names.is_empty() {
        return vec![];
    }

    // Try batch delete first
    let mut args = vec!["push", "origin", "--delete"];
    let refs: Vec<&str> = branch_names.iter().map(|s| s.as_str()).collect();
    args.extend(&refs);

    let out = git_cmd(repo_path).args(&args).output();
    if matches!(&out, Ok(o) if o.status.success()) {
        return branch_names
            .iter()
            .map(|name| OperationResult {
                branch_name: name.clone(),
                action: BranchAction::DeleteRemoteBranch,
                success: true,
                message: format!("Deleted remote {name}"),
            })
            .collect();
    }

    // Fallback to individual deletes
    branch_names
        .iter()
        .map(|name| delete_remote(repo_path, name))
        .collect()
}

fn delete_remote(repo_path: &Path, branch_name: &str) -> OperationResult {
    let out = git_cmd(repo_path)
        .args(["push", "origin", "--delete", branch_name])
        .output();

    match out {
        Ok(o) if o.status.success() => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::DeleteRemoteBranch,
            success: true,
            message: format!("Deleted remote {branch_name}"),
        },
        Ok(o) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::DeleteRemoteBranch,
            success: false,
            message: String::from_utf8_lossy(&o.stderr).trim().to_string(),
        },
        Err(e) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::DeleteRemoteBranch,
            success: false,
            message: e.to_string(),
        },
    }
}

pub fn fetch_remote(repo_path: &Path, remote: &str) -> Vec<OperationResult> {
    let out = git_cmd(repo_path).args(["fetch", remote]).output();
    vec![match out {
        Ok(o) if o.status.success() => OperationResult {
            branch_name: remote.to_string(),
            action: BranchAction::FetchRemote,
            success: true,
            message: format!("Fetched {remote}"),
        },
        Ok(o) => OperationResult {
            branch_name: remote.to_string(),
            action: BranchAction::FetchRemote,
            success: false,
            message: String::from_utf8_lossy(&o.stderr).trim().to_string(),
        },
        Err(e) => OperationResult {
            branch_name: remote.to_string(),
            action: BranchAction::FetchRemote,
            success: false,
            message: e.to_string(),
        },
    }]
}

pub fn pull_remote(repo_path: &Path, remote: &str, short_name: &str) -> Vec<OperationResult> {
    let out = git_cmd(repo_path)
        .args(["fetch", remote, &format!("{short_name}:{short_name}")])
        .output();

    vec![match out {
        Ok(o) if o.status.success() => OperationResult {
            branch_name: short_name.to_string(),
            action: BranchAction::PullRemote,
            success: true,
            message: format!("Pulled {remote}/{short_name}"),
        },
        Ok(o) => OperationResult {
            branch_name: short_name.to_string(),
            action: BranchAction::PullRemote,
            success: false,
            message: String::from_utf8_lossy(&o.stderr).trim().to_string(),
        },
        Err(e) => OperationResult {
            branch_name: short_name.to_string(),
            action: BranchAction::PullRemote,
            success: false,
            message: e.to_string(),
        },
    }]
}

pub fn merge_remote_into_current(
    repo_path: &Path,
    full_ref: &str,
    short_name: &str,
) -> Vec<OperationResult> {
    let out = git_cmd(repo_path).args(["merge", full_ref]).output();
    vec![match out {
        Ok(o) if o.status.success() => OperationResult {
            branch_name: short_name.to_string(),
            action: BranchAction::MergeRemoteIntoCurrent,
            success: true,
            message: format!("Merged {full_ref} into current"),
        },
        _ => {
            let _ = git_cmd(repo_path).args(["merge", "--abort"]).output();
            OperationResult {
                branch_name: short_name.to_string(),
                action: BranchAction::MergeRemoteIntoCurrent,
                success: false,
                message: "Merge conflict — aborted".to_string(),
            }
        }
    }]
}

pub fn cherry_pick_remote(
    repo_path: &Path,
    full_ref: &str,
    short_name: &str,
) -> Vec<OperationResult> {
    let out = git_cmd(repo_path).args(["cherry-pick", full_ref]).output();
    vec![match out {
        Ok(o) if o.status.success() => OperationResult {
            branch_name: short_name.to_string(),
            action: BranchAction::CherryPickRemote,
            success: true,
            message: format!("Cherry-picked {full_ref}"),
        },
        _ => {
            let _ = git_cmd(repo_path).args(["cherry-pick", "--abort"]).output();
            OperationResult {
                branch_name: short_name.to_string(),
                action: BranchAction::CherryPickRemote,
                success: false,
                message: "Cherry-pick conflict — aborted".to_string(),
            }
        }
    }]
}

pub fn create_worktree(repo_path: &Path, branch_name: &str) -> OperationResult {
    let sanitized = branch_name.replace('/', "-");
    let wt_path = repo_path.join(".worktrees").join(&sanitized);
    let wt_str = wt_path.to_string_lossy();

    let out = git_cmd(repo_path)
        .args(["worktree", "add", &wt_str, branch_name])
        .output();

    match out {
        Ok(o) if o.status.success() => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::Worktree,
            success: true,
            message: format!("Created worktree at {wt_str}"),
        },
        Ok(o) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::Worktree,
            success: false,
            message: String::from_utf8_lossy(&o.stderr).trim().to_string(),
        },
        Err(e) => OperationResult {
            branch_name: branch_name.to_string(),
            action: BranchAction::Worktree,
            success: false,
            message: e.to_string(),
        },
    }
}

pub fn remove_worktree(repo_path: &Path, worktree_path: &Path) -> OperationResult {
    let wt_str = worktree_path.to_string_lossy();
    let out = git_cmd(repo_path).args(["worktree", "remove", &wt_str]).output();

    match out {
        Ok(o) if o.status.success() => OperationResult {
            branch_name: wt_str.to_string(),
            action: BranchAction::WorktreeRemove,
            success: true,
            message: format!("Removed worktree {wt_str}"),
        },
        Ok(o) => OperationResult {
            branch_name: wt_str.to_string(),
            action: BranchAction::WorktreeRemove,
            success: false,
            message: String::from_utf8_lossy(&o.stderr).trim().to_string(),
        },
        Err(e) => OperationResult {
            branch_name: wt_str.to_string(),
            action: BranchAction::WorktreeRemove,
            success: false,
            message: e.to_string(),
        },
    }
}

pub fn force_remove_worktree(repo_path: &Path, worktree_path: &Path) -> OperationResult {
    let wt_str = worktree_path.to_string_lossy();
    let out = git_cmd(repo_path)
        .args(["worktree", "remove", "--force", &wt_str])
        .output();

    match out {
        Ok(o) if o.status.success() => OperationResult {
            branch_name: wt_str.to_string(),
            action: BranchAction::WorktreeForceRemove,
            success: true,
            message: format!("Force removed worktree {wt_str}"),
        },
        Ok(o) => OperationResult {
            branch_name: wt_str.to_string(),
            action: BranchAction::WorktreeForceRemove,
            success: false,
            message: String::from_utf8_lossy(&o.stderr).trim().to_string(),
        },
        Err(e) => OperationResult {
            branch_name: wt_str.to_string(),
            action: BranchAction::WorktreeForceRemove,
            success: false,
            message: e.to_string(),
        },
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test test_checkout_branch test_push_branch test_fetch_sync test_delete_local`
Expected: All tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/git/operations.rs tests/integration.rs
git commit -m "feat: add all git operations (checkout, push, pull, merge, rebase, fetch, worktree)"
```

---

### Task 11: git/squash_loader.rs — Background Squash Checker

**Files:**
- Modify: `src/git/squash_loader.rs`

- [ ] **Step 1: Implement squash loader**

Write into `src/git/squash_loader.rs`:

```rust
use crate::git::cache::BranchCache;
use crate::git::merge_detection::is_squash_merged;
use crate::types::{MergeStatus, SquashResult};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};

pub fn spawn_squash_checker(
    repo_path: PathBuf,
    base_branch: String,
    candidates: Vec<(String, String)>, // (branch_name, commit_hash)
    mut cache: BranchCache,
) -> Receiver<SquashResult> {
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        for (branch_name, commit_hash) in &candidates {
            // Check cache first
            if let Some(status) = cache.lookup(branch_name, commit_hash) {
                let is_squash = matches!(status, MergeStatus::SquashMerged);
                if tx.send(SquashResult {
                    branch_name: branch_name.clone(),
                    is_squash_merged: is_squash,
                }).is_err() {
                    return; // Receiver dropped
                }
                continue;
            }

            let is_squash = is_squash_merged(&repo_path, &base_branch, branch_name, None);

            let status = if is_squash { MergeStatus::SquashMerged } else { MergeStatus::Unmerged };
            cache.insert(branch_name, &status, commit_hash);

            if tx.send(SquashResult {
                branch_name: branch_name.clone(),
                is_squash_merged: is_squash,
            }).is_err() {
                return;
            }
        }

        cache.save();
    });

    rx
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build`
Expected: Compiles.

- [ ] **Step 3: Commit**

```bash
git add src/git/squash_loader.rs
git commit -m "feat: add background squash-merge checker with cache integration"
```

---

### Task 12: git/worktree.rs — Worktree Listing & Enrichment

**Files:**
- Modify: `src/git/worktree.rs`
- Test: `tests/integration.rs` (add tests)

- [ ] **Step 1: Write integration tests**

Add to `tests/integration.rs`:

```rust
#[test]
fn test_list_worktrees_main_only() {
    let (dir, _repo) = setup_test_repo();
    let worktrees = git_branch_manager::git::worktree::list_worktrees(dir.path());
    assert_eq!(worktrees.len(), 1);
    assert!(worktrees[0].is_main);
}

#[test]
fn test_create_and_list_worktree() {
    let (dir, _repo) = setup_test_repo();
    let path = dir.path();
    run_git(path, &["checkout", "-b", "feature/wt-test"]);
    run_git(path, &["checkout", "main"]);

    let result = git_branch_manager::git::operations::create_worktree(path, "feature/wt-test");
    assert!(result.success);

    let worktrees = git_branch_manager::git::worktree::list_worktrees(path);
    assert_eq!(worktrees.len(), 2);
    let wt = worktrees.iter().find(|w| !w.is_main).unwrap();
    assert_eq!(wt.branch.as_deref(), Some("feature/wt-test"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test test_list_worktrees test_create_and_list_worktree`
Expected: FAIL — function not defined.

- [ ] **Step 3: Implement worktree listing**

Write into `src/git/worktree.rs`:

```rust
use crate::types::{MergeStatus, WorkingTreeStatus, WorktreeEnrichResult, WorktreeInfo};
use chrono::{DateTime, TimeZone, Utc};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver};

fn git_out(dir: &Path, args: &[&str]) -> String {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .stdin(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

pub fn list_worktrees(repo_path: &Path) -> Vec<WorktreeInfo> {
    let output = git_out(repo_path, &["worktree", "list", "--porcelain"]);
    if output.is_empty() {
        return vec![];
    }

    let mut worktrees = Vec::new();
    let mut current_path: Option<PathBuf> = None;
    let mut current_hash = String::new();
    let mut current_branch: Option<String> = None;
    let mut is_first = true;

    for line in output.lines() {
        if let Some(path_str) = line.strip_prefix("worktree ") {
            if let Some(path) = current_path.take() {
                let wt = build_worktree(
                    path,
                    std::mem::take(&mut current_hash),
                    current_branch.take(),
                    is_first,
                    head_commit_date_from_hash(repo_path, &current_hash),
                );
                worktrees.push(wt);
                is_first = false;
            }
            current_path = Some(PathBuf::from(path_str));
            current_hash.clear();
            current_branch = None;
        } else if let Some(hash) = line.strip_prefix("HEAD ") {
            current_hash = hash[..7.min(hash.len())].to_string();
        } else if let Some(branch_ref) = line.strip_prefix("branch ") {
            current_branch = branch_ref
                .strip_prefix("refs/heads/")
                .map(|s| s.to_string());
        } else if line == "detached" {
            current_branch = None;
        }
    }

    // Don't forget the last entry
    if let Some(path) = current_path {
        let wt = build_worktree(
            path,
            current_hash,
            current_branch,
            is_first,
            Utc::now(),
        );
        worktrees.push(wt);
    }

    worktrees
}

pub fn enrich_worktrees(worktrees: Vec<WorktreeInfo>) -> Receiver<WorktreeEnrichResult> {
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        for (index, wt) in worktrees.iter().enumerate() {
            let (wt_status, age_date) = status_and_age(&wt.path);
            if tx.send(WorktreeEnrichResult { index, wt_status, age_date }).is_err() {
                return;
            }
        }
    });

    rx
}

fn build_worktree(
    path: PathBuf,
    commit_hash: String,
    branch: Option<String>,
    is_main: bool,
    age_date: DateTime<Utc>,
) -> WorktreeInfo {
    WorktreeInfo {
        path,
        branch,
        is_main,
        commit_hash,
        wt_status: WorkingTreeStatus::clean(),
        age_date,
        merge_status: MergeStatus::Unmerged,
        ahead: None,
        behind: None,
        pr: None,
    }
}

fn status_and_age(dir: &Path) -> (WorkingTreeStatus, DateTime<Utc>) {
    let output = git_out(dir, &["status", "--porcelain"]);
    let mut has_staged = false;
    let mut has_unstaged = false;
    let mut has_untracked = false;

    for line in output.lines() {
        let bytes = line.as_bytes();
        if bytes.len() < 2 { continue; }
        let index = bytes[0];
        let work = bytes[1];

        if index == b'?' { has_untracked = true; continue; }
        if index != b' ' && index != b'?' { has_staged = true; }
        if work != b' ' && work != b'?' { has_unstaged = true; }
    }

    let status = WorkingTreeStatus { has_staged, has_unstaged, has_untracked };
    let age = head_commit_date(dir);
    (status, age)
}

fn head_commit_date(dir: &Path) -> DateTime<Utc> {
    let output = git_out(dir, &["log", "-1", "--format=%ct", "HEAD"]);
    output
        .parse::<i64>()
        .ok()
        .and_then(|ts| Utc.timestamp_opt(ts, 0).single())
        .unwrap_or_else(Utc::now)
}

fn head_commit_date_from_hash(repo_path: &Path, _hash: &str) -> DateTime<Utc> {
    // For phase-1 loading, we just use now; enrichment fills in the real date
    Utc::now()
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test test_list_worktrees test_create_and_list_worktree`
Expected: All tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/git/worktree.rs tests/integration.rs
git commit -m "feat: add worktree listing and enrichment"
```

---

### Task 13: git/tags.rs — Tag Operations

**Files:**
- Modify: `src/git/tags.rs`
- Test: `tests/integration.rs` (add tests)

- [ ] **Step 1: Write integration tests**

Add to `tests/integration.rs`:

```rust
#[test]
fn test_list_tags_empty() {
    let (_dir, repo) = setup_test_repo();
    let tags = git_branch_manager::git::tags::list_tags(&repo);
    assert!(tags.is_empty());
}

#[test]
fn test_list_tags_with_annotated() {
    let (dir, repo) = setup_test_repo();
    run_git(dir.path(), &["tag", "-a", "v1.0", "-m", "Release 1.0"]);
    let tags = git_branch_manager::git::tags::list_tags(&repo);
    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0].name, "v1.0");
    assert!(tags[0].is_annotated);
    assert_eq!(tags[0].message.as_deref(), Some("Release 1.0"));
}

#[test]
fn test_list_tags_lightweight() {
    let (dir, repo) = setup_test_repo();
    run_git(dir.path(), &["tag", "v0.1"]);
    let tags = git_branch_manager::git::tags::list_tags(&repo);
    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0].name, "v0.1");
    assert!(!tags[0].is_annotated);
    assert!(tags[0].message.is_none());
}

#[test]
fn test_delete_tag() {
    let (dir, repo) = setup_test_repo();
    run_git(dir.path(), &["tag", "v1.0"]);
    let result = git_branch_manager::git::tags::delete_tag(&repo, "v1.0");
    assert!(result.success);
    let tags = git_branch_manager::git::tags::list_tags(&repo);
    assert!(tags.is_empty());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test test_list_tags test_delete_tag`
Expected: FAIL.

- [ ] **Step 3: Implement tag operations**

Write into `src/git/tags.rs`:

```rust
use crate::types::{BranchAction, OperationResult, TagInfo};
use chrono::{TimeZone, Utc};
use git2::{ObjectType, Repository};
use std::path::Path;
use std::process::{Command, Stdio};

pub fn list_tags(repo: &Repository) -> Vec<TagInfo> {
    let tag_names = match repo.tag_names(None) {
        Ok(names) => names,
        Err(_) => return vec![],
    };

    let mut tags: Vec<TagInfo> = tag_names
        .iter()
        .flatten()
        .filter_map(|name| {
            let ref_name = format!("refs/tags/{name}");
            let reference = repo.find_reference(&ref_name).ok()?;
            let obj = reference.peel(ObjectType::Commit).ok()?;
            let commit = obj.as_commit()?;
            let time = commit.committer().when();
            let date = Utc.timestamp_opt(time.seconds(), 0).single()?;
            let hash = commit.id().to_string();

            // Check if annotated
            let (is_annotated, message) = match reference.peel(ObjectType::Tag) {
                Ok(tag_obj) => {
                    let tag = repo.find_tag(tag_obj.id()).ok();
                    let msg = tag
                        .as_ref()
                        .and_then(|t| t.message().map(|m| m.trim().to_string()));
                    (true, msg)
                }
                Err(_) => (false, None),
            };

            Some(TagInfo {
                name: name.to_string(),
                commit_hash: hash[..7.min(hash.len())].to_string(),
                date,
                message,
                is_annotated,
            })
        })
        .collect();

    tags.sort_by(|a, b| b.date.cmp(&a.date));
    tags
}

pub fn delete_tag(repo: &Repository, tag_name: &str) -> OperationResult {
    match repo.tag_delete(tag_name) {
        Ok(()) => OperationResult {
            branch_name: tag_name.to_string(),
            action: BranchAction::DeleteTag,
            success: true,
            message: format!("Deleted tag {tag_name}"),
        },
        Err(e) => OperationResult {
            branch_name: tag_name.to_string(),
            action: BranchAction::DeleteTag,
            success: false,
            message: format!("Failed: {e}"),
        },
    }
}

pub fn delete_tags_batch(repo: &Repository, tag_names: &[String]) -> Vec<OperationResult> {
    tag_names.iter().map(|name| delete_tag(repo, name)).collect()
}

pub fn delete_remote_tags_batch(repo_path: &Path, tag_names: &[String]) -> Vec<OperationResult> {
    if tag_names.is_empty() {
        return vec![];
    }

    let mut args = vec!["push", "origin", "--delete"];
    let refs: Vec<&str> = tag_names.iter().map(|s| s.as_str()).collect();
    args.extend(&refs);

    let out = Command::new("git")
        .args(&args)
        .current_dir(repo_path)
        .stdin(Stdio::null())
        .env("GIT_TERMINAL_PROMPT", "0")
        .output();

    if matches!(&out, Ok(o) if o.status.success()) {
        return tag_names
            .iter()
            .map(|name| OperationResult {
                branch_name: name.clone(),
                action: BranchAction::DeleteTagAndRemote,
                success: true,
                message: format!("Deleted remote tag {name}"),
            })
            .collect();
    }

    // Fallback to individual
    tag_names
        .iter()
        .map(|name| {
            let out = Command::new("git")
                .args(["push", "origin", "--delete", name])
                .current_dir(repo_path)
                .stdin(Stdio::null())
                .env("GIT_TERMINAL_PROMPT", "0")
                .output();

            match out {
                Ok(o) if o.status.success() => OperationResult {
                    branch_name: name.clone(),
                    action: BranchAction::DeleteTagAndRemote,
                    success: true,
                    message: format!("Deleted remote tag {name}"),
                },
                _ => OperationResult {
                    branch_name: name.clone(),
                    action: BranchAction::DeleteTagAndRemote,
                    success: false,
                    message: format!("Failed to delete remote tag {name}"),
                },
            }
        })
        .collect()
}

pub fn push_tag(repo_path: &Path, tag_name: &str) -> OperationResult {
    let out = Command::new("git")
        .args(["push", "origin", tag_name])
        .current_dir(repo_path)
        .stdin(Stdio::null())
        .env("GIT_TERMINAL_PROMPT", "0")
        .output();

    match out {
        Ok(o) if o.status.success() => OperationResult {
            branch_name: tag_name.to_string(),
            action: BranchAction::PushTag,
            success: true,
            message: format!("Pushed tag {tag_name}"),
        },
        Ok(o) => OperationResult {
            branch_name: tag_name.to_string(),
            action: BranchAction::PushTag,
            success: false,
            message: String::from_utf8_lossy(&o.stderr).trim().to_string(),
        },
        Err(e) => OperationResult {
            branch_name: tag_name.to_string(),
            action: BranchAction::PushTag,
            success: false,
            message: e.to_string(),
        },
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test test_list_tags test_delete_tag`
Expected: All 4 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/git/tags.rs tests/integration.rs
git commit -m "feat: add tag listing with annotated message support and tag operations"
```

---

### Task 14: git/github.rs & git/pr_loader.rs — PR Integration

**Files:**
- Modify: `src/git/github.rs`
- Modify: `src/git/pr_loader.rs`

- [ ] **Step 1: Implement GitHub PR fetching**

Write into `src/git/github.rs`:

```rust
use crate::types::{PrInfo, PrMap, PrStatus};
use serde::Deserialize;
use std::path::Path;
use std::process::{Command, Stdio};

#[derive(Deserialize)]
struct PrEntry {
    number: u32,
    #[serde(rename = "headRefName")]
    head_ref_name: String,
    #[serde(rename = "isDraft")]
    is_draft: bool,
    state: String,
}

pub fn fetch_open_prs(repo_path: &Path) -> PrMap {
    let out = Command::new("gh")
        .args([
            "pr", "list",
            "--json", "number,headRefName,isDraft,state",
            "--state", "all",
            "--limit", "200",
        ])
        .current_dir(repo_path)
        .stdin(Stdio::null())
        .output();

    let output = match out {
        Ok(o) if o.status.success() => o.stdout,
        _ => return PrMap::new(),
    };

    let entries: Vec<PrEntry> = match serde_json::from_slice(&output) {
        Ok(e) => e,
        Err(_) => return PrMap::new(),
    };

    entries
        .into_iter()
        .map(|e| {
            let status = if e.is_draft {
                PrStatus::Draft
            } else if e.state == "MERGED" {
                PrStatus::Merged
            } else if e.state == "CLOSED" {
                PrStatus::Closed
            } else {
                PrStatus::Open
            };
            (e.head_ref_name, PrInfo { number: e.number, status })
        })
        .collect()
}
```

- [ ] **Step 2: Implement background PR loader**

Write into `src/git/pr_loader.rs`:

```rust
use crate::types::PrMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};

pub fn spawn_pr_loader(repo_path: PathBuf) -> Receiver<PrMap> {
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        let prs = super::github::fetch_open_prs(&repo_path);
        let _ = tx.send(prs);
    });

    rx
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo build`
Expected: Compiles.

- [ ] **Step 4: Commit**

```bash
git add src/git/github.rs src/git/pr_loader.rs
git commit -m "feat: add GitHub PR integration and background PR loader"
```

---

### Task 15: Run Full Test Suite & Final Cleanup

**Files:**
- Possibly modify: any file with compilation issues

- [ ] **Step 1: Run the full test suite**

Run: `cargo test`
Expected: All tests pass.

- [ ] **Step 2: Run clippy**

Run: `cargo clippy -- -D warnings`
Expected: No warnings (fix any that appear).

- [ ] **Step 3: Verify build**

Run: `cargo build`
Expected: Clean build.

- [ ] **Step 4: Commit any cleanup**

```bash
git add -A
git commit -m "chore: Phase 1 complete — all types and git operations tested"
```
