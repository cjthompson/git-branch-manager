//! Synchronous, non-interactive view dumps. Runs each view's loaders to
//! completion (draining background enrichers inline), orders rows like the TUI,
//! and renders via `ui::dump_render`.

use anyhow::Result;
use git2::Repository;
use std::cmp::Reverse;
use std::path::Path;

use git_branch_manager::cli::ColorChoice;
use git_branch_manager::config::Config;
use git_branch_manager::git::{branch, github, tags, worktree};
use git_branch_manager::symbols::SymbolSet;
use git_branch_manager::theme::Theme;
use git_branch_manager::ui::dump_render::{render_table, DUMP_AREA_WIDTH};
use git_branch_manager::ui::list_render::CellContext;
use git_branch_manager::view::branches::BranchesViewDef;
use git_branch_manager::view::remotes::RemotesViewDef;
use git_branch_manager::view::tags::TagsViewDef;
use git_branch_manager::view::worktrees::WorktreesViewDef;
use git_branch_manager::view::ViewItem;

use crate::app::render_branch_row;
use crate::app::render_remote_row;
use crate::app::render_tag_row;
use crate::app::render_worktree_row;

#[derive(Clone, Copy, Debug)]
pub enum DumpView {
    Branches,
    Remotes,
    Tags,
    Worktrees,
}

/// Stable pinned-first reorder, preserving the loader's within-group (date)
/// order. Callers that must distinguish among pinned rows (e.g. branches put the
/// base first) apply an additional stable sort after calling this.
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
            // Base branch first among the pinned rows, matching the TUI's
            // ListState ordering (it pins the base to index 0). A stable sort
            // by is_base keeps the rest (current branch, then date-ordered
            // non-pinned) in pin_first's order.
            rows.sort_by_key(|b| Reverse(b.is_base));
            let cols = BranchesViewDef.columns();
            Ok(render_table(Some(base), &rows, &cols, render_branch_row, &ctx, color))
        }
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
    }
}
