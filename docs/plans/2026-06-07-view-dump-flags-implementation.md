# View Dump Flags Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `--branches`, `--remotes`, `--tags`, `--worktrees` CLI flags that print each TUI view's fully-enriched rows to stdout (faithful columns + colors), for use as a scriptable performance harness and a behavior-preserving correctness oracle.

**Architecture:** Switch the shared row representation from ratatui `Cell` (opaque) to `Line` (publicly introspectable) so one rendering source feeds both the TUI and a new headless text writer. A binary-side `dump` module runs each view's loaders synchronously to completion (draining the enricher channels inline), orders rows like the TUI, calls the existing per-view row renderers to get `Vec<Line>`, and a lib-side `ui::dump_render` serializes those `Line`s to a plain/ANSI table. JSON is deliberately deferred but the data/presentation seam makes it additive.

**Tech Stack:** Rust 2021, clap 4 (derive + `ValueEnum`/`ArgGroup`), ratatui 0.30 (`Line`/`Span`/`Style`), git2 0.20, `std::io::IsTerminal`. No new dependencies.

**Companion spec:** `docs/plans/2026-06-07-view-dump-flags-design.md`.

---

## Conventions for every command in this plan

- Worktree: `/Users/chris.thompson/dev/git-branch-manager/.claude/worktrees/view-dump-flags` (branch `view-dump-flags`).
- `cargo`/`rustc` are not on PATH; prefix once per shell:
  ```bash
  export PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH"
  cd /Users/chris.thompson/dev/git-branch-manager/.claude/worktrees/view-dump-flags
  ```
- Commit messages end with the repo's trailer:
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`

## File structure

| File | Change | Responsibility |
| --- | --- | --- |
| `src/ui/cells.rs` | modify | shared cell builders return `Line<'static>` (was `Cell`) |
| `src/ui/list_render.rs` | modify | `RowRenderer<T>` returns `Vec<Line>`; wrap `Line→Cell` in one place |
| `src/app.rs` | modify | 4 `render_*_row` fns return `Vec<Line>`, made `pub(crate)` |
| `src/cli.rs` | modify | 4 dump flags, `--color`, `ColorChoice` enum, mutual-exclusion group |
| `src/ui/dump_render.rs` | **new (lib)** | `render_table` + `Style`→ANSI (`sgr_prefix`) — pure, generic, testable |
| `src/dump.rs` | **new (binary)** | `DumpView`, `run()` — per-view synchronous orchestration |
| `src/main.rs` | modify | declare `mod dump;`; dispatch dump flags pre-TUI; `--list` alias |
| `src/ui/mod.rs` | modify | `pub mod dump_render;` |
| `tests/integration.rs` | modify | dump end-to-end + ordering tests |
| `CLAUDE.md` | modify | document flags; note `--list` deprecation |

---

## Task 1: Switch shared row rendering from `Cell` to `Line`

Pure refactor, **no behavior change** — verified by the existing test suite staying green. This unlocks the headless writer: ratatui `Cell` content is private and cannot be read back, but `Line` exposes `.spans` (each `Span` has public `.content` + `.style`) and `.alignment`.

**Files:**
- Modify: `src/ui/cells.rs`
- Modify: `src/ui/list_render.rs:25` (type alias) and the row-building loop (~line 150)
- Modify: `src/app.rs:2343-2616` (the four `render_*_row` fns + `branch_prefix_style`)

- [ ] **Step 1: Change the cell builders in `src/ui/cells.rs` to return `Line`**

Ensure `use ratatui::text::Line;` is present. Replace the five builders (currently returning `Cell<'static>`) with these (renamed `*_cell` → `*_line` for honesty):

```rust
pub fn ahead_behind_line(ahead: Option<u32>, behind: Option<u32>, ctx: &CellContext) -> Line<'static> {
    Line::from(ahead_behind_spans(ahead, behind, ctx))
}

pub fn pr_line(pr: Option<&PrInfo>, ctx: &CellContext) -> Line<'static> {
    let (text, style) = pr_parts(pr, ctx);
    Line::from(Span::styled(text, style))
}

pub fn age_line(age_text: String, date: &DateTime<Utc>, ctx: &CellContext) -> Line<'static> {
    let style = age_style(date, ctx.theme);
    Line::from(Span::styled(age_text, style)).alignment(Alignment::Right)
}

pub fn merge_status_line(status: &MergeStatus, ctx: &CellContext) -> Line<'static> {
    let (text, style) = merge_status_parts(status, ctx);
    Line::from(Span::styled(text, style)).alignment(Alignment::Right)
}

pub fn merge_status_line_for_branch(status: &MergeStatus, is_base: bool, ctx: &CellContext) -> Line<'static> {
    if is_base {
        Line::from("")
    } else {
        merge_status_line(status, ctx)
    }
}
```

Remove the now-unused `Cell` import from `cells.rs` if nothing else uses it.

- [ ] **Step 2: Change the `RowRenderer` type alias in `src/ui/list_render.rs:25`**

Add `use ratatui::text::Line;` if absent, then:

```rust
pub type RowRenderer<T> = fn(&T, usize, bool, bool, &[usize], &CellContext) -> Vec<Line<'static>>;
```

- [ ] **Step 3: Update the four `render_*_row` fns in `src/app.rs`**

For each of `render_branch_row`, `render_remote_row`, `render_tag_row`, `render_worktree_row`:
1. Change the return type `-> Vec<Cell<'static>>` to `-> Vec<Line<'static>>` and prefix `pub(crate) `.
2. Rename the local `let mut cells = Vec::new();` to `let mut lines = Vec::new();` and the final `cells` to `lines`.
3. Replace every `cells.push(Cell::from(Span::styled(X, style)))` with `lines.push(Line::from(Span::styled(X, style)))`.
4. Replace every `cells.push(Cell::from(""))` with `lines.push(Line::from(""))`.
5. Replace the renamed cell-builder calls: `ahead_behind_cell(` → `ahead_behind_line(`, `pr_cell(` → `pr_line(`, `age_cell(` → `age_line(`, `merge_status_cell(` → `merge_status_line(`, `merge_status_cell_for_branch(` → `merge_status_line_for_branch(`.

Add `use ratatui::text::Line;` to `app.rs` if absent. As the worked example, `render_branch_row` becomes:

```rust
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
                lines.push(Line::from(Span::styled(name, style)));
            }
            1 => {
                let text = match &item.tracking {
                    TrackingStatus::Tracked { remote_ref, gone } => {
                        if *gone { "gone".to_string() } else { remote_ref.clone() }
                    }
                    TrackingStatus::Local => "local".to_string(),
                };
                lines.push(Line::from(Span::styled(text, theme.secondary_text)));
            }
            2 => lines.push(ahead_behind_line(item.ahead, item.behind, ctx)),
            3 => lines.push(pr_line(item.pr.as_ref(), ctx)),
            4 => {
                let age = if ctx.compact { item.age_short() } else { item.age_display() };
                lines.push(age_line(age, &item.last_commit_date, ctx));
            }
            5 => lines.push(merge_status_line_for_branch(&item.merge_status, item.is_base, ctx)),
            _ => lines.push(Line::from("")),
        }
    }
    lines
}
```

Apply the identical mechanical transform to `render_remote_row`, `render_tag_row`, and `render_worktree_row` (same substitutions; their match arms are otherwise unchanged from the current code).

- [ ] **Step 4: Wrap `Line → Cell` in `render_list_view` (`src/ui/list_render.rs`)**

In the row-building closure, the current lines are:

```rust
let mut cells = vec![checkbox_cell];
cells.extend((params.render_row)(
    item, raw_idx, is_selected, is_cursor, &visible_col_indices, &ctx,
));
```

Replace with:

```rust
let mut cells = vec![checkbox_cell];
cells.extend(
    (params.render_row)(item, raw_idx, is_selected, is_cursor, &visible_col_indices, &ctx)
        .into_iter()
        .map(Cell::from),
);
```

(`Cell: From<Line>` exists in ratatui 0.30; `Cell::from(line)` preserves the `Line`'s spans and alignment, so rendering is identical.)

- [ ] **Step 5: Build and run the full suite — must be green (no behavior change)**

Run: `cargo build && cargo test`
Expected: builds clean; all existing tests pass. If anything fails, it is a mechanical miss in Steps 1–4 — fix before continuing.

- [ ] **Step 6: Commit**

```bash
git add src/ui/cells.rs src/ui/list_render.rs src/app.rs
git commit -m "refactor: row renderers return Line instead of Cell

Line exposes spans/style/alignment publicly; Cell does not. This lets a
headless text writer reuse the same row rendering as the TUI. No behavior
change — list_render wraps each Line into a Cell.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: Add `ColorChoice` and the `Style`→ANSI helper

**Files:**
- Modify: `src/cli.rs`
- Create: `src/ui/dump_render.rs`
- Modify: `src/ui/mod.rs`

- [ ] **Step 1: Add `ColorChoice` to `src/cli.rs`**

At the top ensure `use clap::{Parser, ValueEnum};`. Add:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Default)]
pub enum ColorChoice {
    #[default]
    Auto,
    Always,
    Never,
}
```

- [ ] **Step 2: Create `src/ui/dump_render.rs` with the SGR helper and a failing test**

```rust
//! Headless table rendering for the `--branches`/`--remotes`/`--tags`/`--worktrees`
//! dump flags. Serializes the same `Line` rows the TUI draws into plain or
//! ANSI-colored fixed-width text.

use ratatui::style::{Color, Modifier, Style};

const RESET: &str = "\x1b[0m";

/// Build the SGR escape prefix for a style (modifiers, then fg, then bg).
/// Returns an empty string when the style carries no fg/bg/modifier.
pub(crate) fn sgr_prefix(style: &Style) -> String {
    let mut codes: Vec<String> = Vec::new();
    let m = style.add_modifier;
    if m.contains(Modifier::BOLD) { codes.push("1".into()); }
    if m.contains(Modifier::DIM) { codes.push("2".into()); }
    if m.contains(Modifier::ITALIC) { codes.push("3".into()); }
    if m.contains(Modifier::UNDERLINED) { codes.push("4".into()); }
    if let Some(fg) = style.fg.and_then(|c| color_code(c, true)) { codes.push(fg); }
    if let Some(bg) = style.bg.and_then(|c| color_code(c, false)) { codes.push(bg); }
    if codes.is_empty() {
        String::new()
    } else {
        format!("\x1b[{}m", codes.join(";"))
    }
}

/// SGR numeric code for a color. `fg` selects foreground (else +10 for background).
fn color_code(c: Color, fg: bool) -> Option<String> {
    let base = |n: u8| -> String { if fg { n.to_string() } else { (n + 10).to_string() } };
    let code = match c {
        Color::Black => base(30),
        Color::Red => base(31),
        Color::Green => base(32),
        Color::Yellow => base(33),
        Color::Blue => base(34),
        Color::Magenta => base(35),
        Color::Cyan => base(36),
        Color::Gray => base(37),
        Color::DarkGray => base(90),
        Color::LightRed => base(91),
        Color::LightGreen => base(92),
        Color::LightYellow => base(93),
        Color::LightBlue => base(94),
        Color::LightMagenta => base(95),
        Color::LightCyan => base(96),
        Color::White => base(97),
        Color::Indexed(n) => format!("{};5;{}", if fg { 38 } else { 48 }, n),
        Color::Rgb(r, g, b) => format!("{};2;{};{};{}", if fg { 38 } else { 48 }, r, g, b),
        Color::Reset => return None,
    };
    Some(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sgr_empty_style_is_blank() {
        assert_eq!(sgr_prefix(&Style::default()), "");
    }

    #[test]
    fn sgr_named_fg() {
        assert_eq!(sgr_prefix(&Style::new().fg(Color::Green)), "\x1b[32m");
    }

    #[test]
    fn sgr_modifier_then_color() {
        assert_eq!(
            sgr_prefix(&Style::new().fg(Color::Red).add_modifier(Modifier::BOLD)),
            "\x1b[1;31m"
        );
    }

    #[test]
    fn sgr_indexed() {
        assert_eq!(sgr_prefix(&Style::new().fg(Color::Indexed(141))), "\x1b[38;5;141m");
    }
}
```

- [ ] **Step 3: Register the module in `src/ui/mod.rs`**

Add (alphabetically near `cells`): `pub mod dump_render;`

- [ ] **Step 4: Run the SGR tests**

Run: `cargo test --lib dump_render`
Expected: 4 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/cli.rs src/ui/dump_render.rs src/ui/mod.rs
git commit -m "feat: add ColorChoice and Style->ANSI SGR helper

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: Implement `render_table`

The generic, lib-side serializer. Takes the rows (already in display order), the view's `ColumnDef`s, the view's `RowRenderer`, a `CellContext`, and a `ColorChoice`; returns the full table string.

**Files:**
- Modify: `src/ui/dump_render.rs`

- [ ] **Step 1: Add `render_table` and `lay_out_cell` with failing tests**

Append to `src/ui/dump_render.rs`:

```rust
use std::io::IsTerminal;

use ratatui::text::Line;

use crate::cli::ColorChoice;
use crate::ui::list_render::{CellContext, RowRenderer};
use crate::view::column::ColumnDef;
use crate::view::ViewItem;

/// Width used for the synthetic terminal area when rendering a dump (wide enough
/// that no column hides and no compact short-forms trigger).
pub const DUMP_AREA_WIDTH: u16 = 200;

/// Render a view to a fixed-width table string.
///
/// `rows` must already be in the intended display order. `base` is printed as a
/// `base: <branch>` header line when present (branches view).
pub fn render_table<T: ViewItem>(
    base: Option<&str>,
    rows: &[T],
    columns: &[ColumnDef<T>],
    render_row: RowRenderer<T>,
    ctx: &CellContext,
    color: ColorChoice,
) -> String {
    let colorize = match color {
        ColorChoice::Always => true,
        ColorChoice::Never => false,
        ColorChoice::Auto => std::io::stdout().is_terminal(),
    };

    // All columns are shown (no width-based hiding) at their widest width.
    let all_cols: Vec<usize> = (0..columns.len()).collect();
    let widths: Vec<usize> = columns
        .iter()
        .map(|c| c.wide_width.unwrap_or(c.min_width) as usize)
        .collect();
    let right_align = |idx: usize| columns[idx].name == "Age" || idx == columns.len() - 1;

    let mut out = String::new();
    if let Some(b) = base {
        out.push_str(&format!("base: {b}\n\n"));
    }

    // Header (never colorized except plain text; keeps the oracle stable).
    let header_line: Line<'static> = Line::from("");
    let _ = header_line; // header cells are plain text, built below
    let header_fields: Vec<String> = all_cols
        .iter()
        .map(|&i| pad_plain(columns[i].name, widths[i], right_align(i)))
        .collect();
    out.push_str(header_fields.join("  ").trim_end());
    out.push('\n');

    // Data rows.
    for (idx, item) in rows.iter().enumerate() {
        let lines = render_row(item, idx, false, false, &all_cols, ctx);
        let fields: Vec<String> = lines
            .iter()
            .enumerate()
            .map(|(i, line)| lay_out_cell(line, widths[i], right_align(i), colorize))
            .collect();
        out.push_str(fields.join("  ").trim_end());
        out.push('\n');
    }

    out
}

/// Pad/truncate plain text to `width`, right- or left-aligned.
fn pad_plain(text: &str, width: usize, right: bool) -> String {
    let len = text.chars().count();
    if len >= width {
        text.chars().take(width).collect()
    } else if right {
        format!("{}{}", " ".repeat(width - len), text)
    } else {
        format!("{}{}", text, " ".repeat(width - len))
    }
}

/// Render one `Line`'s spans to a fixed-width field, optionally ANSI-colored.
/// Padding is computed from the visible (un-escaped) text width.
fn lay_out_cell(line: &Line<'static>, width: usize, right: bool, colorize: bool) -> String {
    let plain: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    let visible = plain.chars().count();

    // Overflow: fall back to plain truncation (drops color) — rare at dump widths.
    if visible >= width {
        return plain.chars().take(width).collect();
    }

    let body = if colorize {
        line.spans
            .iter()
            .map(|s| {
                let prefix = sgr_prefix(&s.style);
                if prefix.is_empty() {
                    s.content.to_string()
                } else {
                    format!("{prefix}{}{RESET}", s.content)
                }
            })
            .collect::<String>()
    } else {
        plain
    };

    let pad = " ".repeat(width - visible);
    if right { format!("{pad}{body}") } else { format!("{body}{pad}") }
}
```

> Note on `RESET`/`sgr_prefix`: they already exist from Task 2 in this file. The new `use` lines reference `crate::...` paths that resolve because `cli`, `ui`, and `view` are re-exported from `lib.rs`.

- [ ] **Step 2: Add `render_table` unit tests at the bottom `mod tests`**

A self-contained dummy type avoids depending on `app.rs` renderers:

```rust
    use crate::view::column::ColumnDef;
    use crate::ui::list_render::CellContext;
    use crate::symbols::SymbolSet;
    use crate::theme::Theme;
    use ratatui::style::Color;

    #[derive(Clone)]
    struct Dummy { name: String, pinned: bool }
    impl ViewItem for Dummy {
        fn is_pinned(&self) -> bool { self.pinned }
    }

    fn dummy_cols() -> Vec<ColumnDef<Dummy>> {
        vec![
            ColumnDef { name: "Name", min_width: 6, wide_width: None, hide_below_width: None, compare: None },
            ColumnDef { name: "Age", min_width: 5, wide_width: None, hide_below_width: None, compare: None },
        ]
    }

    fn dummy_row(item: &Dummy, _i: usize, _s: bool, _c: bool, cols: &[usize], _ctx: &CellContext) -> Vec<Line<'static>> {
        cols.iter().map(|&c| match c {
            0 => Line::from(Span::styled(item.name.clone(), Style::new().fg(Color::Green))),
            _ => Line::from("2d"),
        }).collect()
    }

    #[test]
    fn render_table_plain_no_ansi() {
        let theme = Theme::dark();
        let symbols = SymbolSet::ascii();
        let ctx = CellContext { theme: &theme, symbols: &symbols, area_width: DUMP_AREA_WIDTH, compact: false };
        let rows = vec![Dummy { name: "main".into(), pinned: true }];
        let cols = dummy_cols();
        let out = render_table(Some("main"), &rows, &cols, dummy_row, &ctx, ColorChoice::Never);
        assert!(out.starts_with("base: main\n\n"));
        assert!(out.contains("Name"));
        assert!(out.contains("main"));
        assert!(!out.contains('\x1b'), "Never must not emit ANSI: {out:?}");
    }

    #[test]
    fn render_table_always_emits_ansi() {
        let theme = Theme::dark();
        let symbols = SymbolSet::ascii();
        let ctx = CellContext { theme: &theme, symbols: &symbols, area_width: DUMP_AREA_WIDTH, compact: false };
        let rows = vec![Dummy { name: "main".into(), pinned: true }];
        let cols = dummy_cols();
        let out = render_table(None, &rows, &cols, dummy_row, &ctx, ColorChoice::Always);
        assert!(out.contains("\x1b[32m"), "Always must color the green name: {out:?}");
    }
```

Add `use ratatui::text::Span;` to the test module imports if not already pulled in.

- [ ] **Step 3: Run tests**

Run: `cargo test --lib dump_render`
Expected: 6 tests pass (4 from Task 2 + 2 here).

- [ ] **Step 4: Commit**

```bash
git add src/ui/dump_render.rs
git commit -m "feat: render_table serializes Line rows to plain/ANSI table

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: CLI flags, dump dispatch, and the branches view (first vertical slice)

**Files:**
- Modify: `src/cli.rs`
- Create: `src/dump.rs`
- Modify: `src/main.rs`
- Modify: `tests/integration.rs`

- [ ] **Step 1: Add the dump flags + mutual-exclusion group to `src/cli.rs`**

Ensure `use clap::ArgGroup;`. Add to the `#[command(...)]` attribute on `Cli` a group that makes the four dump flags and `--list` mutually exclusive:

```rust
#[command(
    name = "git-branch-manager",
    about = "TUI for managing git branches",
    group(ArgGroup::new("dump").args(["branches", "remotes", "tags", "worktrees", "list"]).multiple(false)),
)]
```

Add the fields to the `Cli` struct:

```rust
    /// Non-interactive: print the Branches view to stdout (fully enriched)
    #[arg(long)]
    pub branches: bool,

    /// Non-interactive: print the Remotes view to stdout (fully enriched)
    #[arg(long)]
    pub remotes: bool,

    /// Non-interactive: print the Tags view to stdout (fully enriched)
    #[arg(long)]
    pub tags: bool,

    /// Non-interactive: print the Worktrees view to stdout (fully enriched)
    #[arg(long)]
    pub worktrees: bool,

    /// When to colorize dump output
    #[arg(long, value_enum, default_value_t = ColorChoice::Auto)]
    pub color: ColorChoice,
```

- [ ] **Step 2: Create `src/dump.rs` with `DumpView` and `run` (branches only for now)**

```rust
//! Synchronous, non-interactive view dumps. Runs each view's loaders to
//! completion (draining background enrichers inline), orders rows like the TUI,
//! and renders via `ui::dump_render`.

use anyhow::Result;
use git2::Repository;
use std::path::Path;

use git_branch_manager::cli::ColorChoice;
use git_branch_manager::config::Config;
use git_branch_manager::git::{branch, github};
use git_branch_manager::symbols::SymbolSet;
use git_branch_manager::theme::Theme;
use git_branch_manager::ui::dump_render::{render_table, DUMP_AREA_WIDTH};
use git_branch_manager::ui::list_render::CellContext;
use git_branch_manager::view::branches::BranchesViewDef;
use git_branch_manager::view::ViewItem;

use crate::app::render_branch_row;

#[derive(Clone, Copy, Debug)]
pub enum DumpView {
    Branches,
    Remotes,
    Tags,
    Worktrees,
}

/// Stable pinned-first reorder, preserving each loader's within-group order
/// (which matches the TUI's `ListState` default display ordering).
fn pin_first<T: ViewItem>(rows: &mut [T]) {
    rows.sort_by(|a, b| b.is_pinned().cmp(&a.is_pinned()));
}

pub fn run(
    repo: &Repository,
    repo_path: &Path,
    base: &str,
    config: &Config,
    symbols_override: Option<&str>,
    view: DumpView,
    color: ColorChoice,
) -> Result<String> {
    let theme = Theme::from_name(config.theme.as_deref().unwrap_or("dark"));
    let symbols =
        SymbolSet::from_name(symbols_override.or(config.symbols.as_deref()).unwrap_or("auto"));
    let ctx = CellContext {
        theme: &theme,
        symbols: &symbols,
        area_width: DUMP_AREA_WIDTH,
        compact: false,
    };

    match view {
        DumpView::Branches => {
            let mut rows = branch::list_branches(repo, base)?;
            let pr_map = github::fetch_open_prs(repo_path);
            for b in &mut rows {
                b.pr = pr_map.get(&b.name).cloned();
            }
            pin_first(&mut rows);
            let cols = BranchesViewDef.columns();
            Ok(render_table(Some(base), &rows, &cols, render_branch_row, &ctx, color))
        }
        DumpView::Remotes | DumpView::Tags | DumpView::Worktrees => {
            // Implemented in Task 6.
            Ok(String::new())
        }
    }
}
```

- [ ] **Step 3: Wire dispatch into `src/main.rs`**

After `mod app;` add `mod dump;`. Add imports near the top:

```rust
use git_branch_manager::dump::{self, DumpView};
```

Replace the entire existing `if cli.list { ... }` block (and its `use ...TrackingStatus;`) with:

```rust
    // Non-interactive view dumps (also covers the deprecated `--list`).
    let dump_view = if cli.branches || cli.list {
        Some(DumpView::Branches)
    } else if cli.remotes {
        Some(DumpView::Remotes)
    } else if cli.tags {
        Some(DumpView::Tags)
    } else if cli.worktrees {
        Some(DumpView::Worktrees)
    } else {
        None
    };
    if let Some(view) = dump_view {
        if cli.list {
            eprintln!("note: --list is deprecated; use --branches");
        }
        let out = dump::run(
            &repo,
            &repo_path,
            &base_branch,
            &config,
            cli.symbols.as_deref(),
            view,
            cli.color,
        )?;
        print!("{out}");
        return Ok(());
    }
```

If `MergeStatus` is now unused in `main.rs`, remove its import to satisfy the build.

- [ ] **Step 4: Add end-to-end tests to `tests/integration.rs`**

Append:

```rust
#[test]
fn dump_branches_basic() {
    let (tmp, _repo) = setup_test_repo();
    let out = Command::new(env!("CARGO_BIN_EXE_git-branch-manager"))
        .args([
            "--repo",
            tmp.path().to_str().unwrap(),
            "--branches",
            "--color=never",
        ])
        .output()
        .expect("failed to run binary");
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let s = String::from_utf8(out.stdout).unwrap();
    assert!(s.starts_with("base: main"), "got: {s:?}");
    assert!(s.contains("Branch"), "header missing: {s:?}");
    assert!(s.contains("main"), "base branch row missing: {s:?}");
    assert!(!s.contains('\x1b'), "--color=never must be plain: {s:?}");
}

#[test]
fn dump_rejects_two_view_flags() {
    let (tmp, _repo) = setup_test_repo();
    let out = Command::new(env!("CARGO_BIN_EXE_git-branch-manager"))
        .args(["--repo", tmp.path().to_str().unwrap(), "--branches", "--tags"])
        .output()
        .expect("failed to run binary");
    assert!(!out.status.success(), "two view flags must be a usage error");
}

#[test]
fn dump_list_is_branches_alias() {
    let (tmp, _repo) = setup_test_repo();
    let out = Command::new(env!("CARGO_BIN_EXE_git-branch-manager"))
        .args(["--repo", tmp.path().to_str().unwrap(), "--list", "--color=never"])
        .output()
        .expect("failed to run binary");
    assert!(out.status.success());
    let s = String::from_utf8(out.stdout).unwrap();
    assert!(s.starts_with("base: main"), "got: {s:?}");
}
```

- [ ] **Step 5: Build and test**

Run: `cargo build && cargo test`
Expected: builds; the three new tests pass alongside the existing suite. (`cargo test` builds the binary first, so `CARGO_BIN_EXE_git-branch-manager` is populated.)

- [ ] **Step 6: Commit**

```bash
git add src/cli.rs src/dump.rs src/main.rs tests/integration.rs
git commit -m "feat: --branches dump flag + --list alias + dump dispatch

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: Remotes view

Remotes need ahead/behind + merge status, which the TUI fills via a background enricher thread. The dump drains that thread's channel to completion inline.

**Files:**
- Modify: `src/dump.rs`
- Modify: `tests/integration.rs`

- [ ] **Step 1: Implement the `DumpView::Remotes` arm in `src/dump.rs`**

Add imports: extend the `git` use to `use git_branch_manager::git::{branch, github};` already present — add `RemotesViewDef` and the remote renderer:

```rust
use git_branch_manager::view::remotes::RemotesViewDef;
use crate::app::render_remote_row;
```

Replace the combined `Remotes | Tags | Worktrees` arm so `Remotes` is handled:

```rust
        DumpView::Remotes => {
            let mut rows = branch::list_remote_branches_phase1(repo, base)?;
            // Drain the enricher to completion (runs on a worker thread; we block).
            let rx = branch::spawn_remote_enricher(
                repo_path.to_path_buf(),
                base.to_string(),
                rows.clone(),
            );
            for res in rx.iter() {
                if let Some(r) = rows.iter_mut().find(|r| r.full_ref == res.full_ref) {
                    r.merge_status = res.merge_status;
                    r.ahead = res.ahead;
                    r.behind = res.behind;
                }
            }
            let pr_map = github::fetch_open_prs(repo_path);
            for r in &mut rows {
                r.pr = pr_map.get(&r.short_name).cloned();
            }
            pin_first(&mut rows);
            let cols = RemotesViewDef.columns();
            Ok(render_table(None, &rows, &cols, render_remote_row, &ctx, color))
        }
        DumpView::Tags | DumpView::Worktrees => {
            // Implemented in Task 6.
            Ok(String::new())
        }
```

(`rx.iter()` blocks until the worker drops the sender, i.e. enrichment is complete.)

- [ ] **Step 2: Add a remotes test to `tests/integration.rs`**

Model it on the existing `test_list_remote_branches` setup (which creates `origin/*` refs). Minimal version:

```rust
#[test]
fn dump_remotes_basic() {
    let (tmp, _repo) = setup_test_repo();
    // A bare "remote" with one branch, fetched into the test repo.
    let remote = tempfile::tempdir().unwrap();
    run_git(remote.path(), &["init", "--bare", "-b", "main"]);
    run_git(tmp.path(), &["remote", "add", "origin", remote.path().to_str().unwrap()]);
    run_git(tmp.path(), &["push", "origin", "main"]);
    run_git(tmp.path(), &["fetch", "origin"]);

    let out = Command::new(env!("CARGO_BIN_EXE_git-branch-manager"))
        .args(["--repo", tmp.path().to_str().unwrap(), "--remotes", "--color=never"])
        .output()
        .expect("failed to run binary");
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let s = String::from_utf8(out.stdout).unwrap();
    assert!(s.contains("Name"), "header missing: {s:?}");
    assert!(s.contains("origin/main"), "remote row missing: {s:?}");
}
```

- [ ] **Step 3: Build and test**

Run: `cargo test dump_remotes_basic`
Expected: passes.

- [ ] **Step 4: Commit**

```bash
git add src/dump.rs tests/integration.rs
git commit -m "feat: --remotes dump (drains remote enricher synchronously)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 6: Tags and Worktrees views

**Files:**
- Modify: `src/dump.rs`
- Modify: `tests/integration.rs`

- [ ] **Step 1: Implement the `Tags` and `Worktrees` arms in `src/dump.rs`**

Add imports:

```rust
use git_branch_manager::git::{tags, worktree};
use git_branch_manager::view::tags::TagsViewDef;
use git_branch_manager::view::worktrees::WorktreesViewDef;
use crate::app::{render_tag_row, render_worktree_row};
```

Replace the remaining stub arm:

```rust
        DumpView::Tags => {
            let mut rows = tags::list_tags(repo);
            pin_first(&mut rows);
            let cols = TagsViewDef.columns();
            Ok(render_table(None, &rows, &cols, render_tag_row, &ctx, color))
        }
        DumpView::Worktrees => {
            let mut rows = worktree::list_worktrees(repo_path);
            // Drain the enricher to completion (runs on a worker thread; we block).
            let rx = worktree::enrich_worktrees(rows.clone());
            for res in rx.iter() {
                if let Some(w) = rows.get_mut(res.index) {
                    w.wt_status = res.wt_status;
                    w.age_date = res.age_date;
                }
            }
            pin_first(&mut rows);
            let cols = WorktreesViewDef.columns();
            Ok(render_table(None, &rows, &cols, render_worktree_row, &ctx, color))
        }
```

- [ ] **Step 2: Add tags + worktrees tests to `tests/integration.rs`**

```rust
#[test]
fn dump_tags_basic() {
    let (tmp, _repo) = setup_test_repo();
    run_git(tmp.path(), &["tag", "-a", "v1.0", "-m", "release one"]);
    let out = Command::new(env!("CARGO_BIN_EXE_git-branch-manager"))
        .args(["--repo", tmp.path().to_str().unwrap(), "--tags", "--color=never"])
        .output()
        .expect("failed to run binary");
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let s = String::from_utf8(out.stdout).unwrap();
    assert!(s.contains("Name"), "header missing: {s:?}");
    assert!(s.contains("v1.0"), "tag row missing: {s:?}");
}

#[test]
fn dump_worktrees_basic() {
    let (tmp, _repo) = setup_test_repo();
    let out = Command::new(env!("CARGO_BIN_EXE_git-branch-manager"))
        .args(["--repo", tmp.path().to_str().unwrap(), "--worktrees", "--color=never"])
        .output()
        .expect("failed to run binary");
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let s = String::from_utf8(out.stdout).unwrap();
    assert!(s.contains("Path"), "header missing: {s:?}");
    // The main working tree itself is always listed.
    assert!(s.contains(tmp.path().to_str().unwrap()), "main worktree row missing: {s:?}");
}
```

- [ ] **Step 3: Build and test (full suite + clippy)**

Run: `cargo test && cargo clippy -- -D warnings`
Expected: all tests pass; clippy clean.

- [ ] **Step 4: Commit**

```bash
git add src/dump.rs tests/integration.rs
git commit -m "feat: --tags and --worktrees dumps

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 7: Documentation and final verification

**Files:**
- Modify: `CLAUDE.md`

- [ ] **Step 1: Document the flags in `CLAUDE.md`**

In the Commands block, add:

```sh
cargo run -- --branches            # print the Branches view to stdout (fully enriched)
cargo run -- --remotes             # Remotes view to stdout
cargo run -- --tags                # Tags view to stdout
cargo run -- --worktrees           # Worktrees view to stdout
cargo run -- --branches --color=never   # plain text (for diffing / scripting)
```

And add a line near the existing `--list` mention: `--list` is a deprecated alias of `--branches`.

- [ ] **Step 2: Final full verification**

Run: `cargo build && cargo test && cargo clippy -- -D warnings`
Expected: clean build, all tests green, no clippy warnings.

- [ ] **Step 3: Manual smoke check against the perf repo**

Run: `cargo run --release -- --repo ~/workspace/remote-heavy-repo --branches --color=never | head -20`
Expected: a `base: <branch>` header, a column header row, and enriched branch rows (Branch / Remote / A/B / PR / Age / Status). No panics.

- [ ] **Step 4: Commit**

```bash
git add CLAUDE.md
git commit -m "docs: document view dump flags; note --list deprecation

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Self-review

**Spec coverage:**
- Four flags + `--color=auto|always|never` → Tasks 4 (flags) + 3 (`auto` via `IsTerminal`).
- One-flag-per-invocation usage error → Task 4 (`ArgGroup multiple(false)`), tested.
- `--list` deprecated alias → Task 4 + Task 7.
- Faithful columns/colors via existing `ColumnDef`/`RowRenderer` → Task 1 (`Line` refactor) + Task 3 (`render_table`).
- Full synchronous enrichment incl. squash + PRs (branches), enricher channels (remotes/worktrees) → Tasks 4–6.
- Deterministic default-sort ordering → `pin_first` (Tasks 4–6) matching the TUI's `ListState` pinned-first default.
- JSON additive seam → satisfied structurally: enrichment yields typed rows; rendering is a separate consumer. JSON (v2) = derive `Serialize` on the row structs + a `JsonRenderer` over the same rows + a `--format` flag. `MergeStatus` already derives `Serialize`; `TrackingStatus`/`PrInfo`/the row structs would need it. **Not in this plan** (per spec non-goal).
- Tests via `setup_test_repo()` with `--color=never` + an `--color=always` ANSI check (Task 3 unit) → covered.
- Timing/`GBM_TIMING_LOG`: out of scope — this base only has the debug-gated `/tmp/gbm-timing.log`; the dump runs the real loaders, so whatever subscriber is active captures spans. The richer opt-in lives on the perf branch.

**Placeholder scan:** every code step contains complete code; the only deferred items are explicitly the JSON v2 follow-up (out of scope) and the Task 4 stub arms, which are filled in Tasks 5–6.

**Type consistency:** `render_*_row` return `Vec<Line<'static>>` (Task 1) matches `RowRenderer<T>` (Task 1) and `render_table`'s parameter (Task 3). `ColorChoice` is defined once in `cli.rs` (Task 2) and used by `dump_render` and `dump`. View defs are unit structs (`BranchesViewDef.columns()`); loader signatures (`list_branches`, `list_remote_branches_phase1`, `spawn_remote_enricher`, `list_tags`, `list_worktrees`, `enrich_worktrees`, `fetch_open_prs`) match the verified source.
