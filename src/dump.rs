//! Synchronous, non-interactive view dumps. Runs each view's loaders to
//! completion (draining background enrichers inline), orders rows like the TUI,
//! and renders via `ui::dump_render`.

use anyhow::Result;
use git2::Repository;
use std::cmp::Reverse;
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
    rows.sort_by_key(|b| Reverse(b.is_pinned()));
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
