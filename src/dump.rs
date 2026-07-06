//! Synchronous, non-interactive view dumps. Runs each view's loaders to
//! completion (draining background enrichers inline), orders rows like the TUI,
//! and renders via `ui::dump_render`.

use anyhow::Result;
use git2::Repository;
use std::cmp::Reverse;
use std::path::Path;

use git_branch_manager::cli::ColorChoice;
use git_branch_manager::config::Config;
use git_branch_manager::git::{branch, cache, github, squash_loader, tags, worktree};
use git_branch_manager::symbols::SymbolSet;
use git_branch_manager::theme::Theme;
use git_branch_manager::types::MergeStatus;
use git_branch_manager::ui::dump_render::{render_table, DUMP_AREA_WIDTH};
use git_branch_manager::ui::list_render::CellContext;
use git_branch_manager::view::branches::BranchesViewDef;
use git_branch_manager::view::list_state;
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
    let symbols = SymbolSet::from_name(
        symbols_override
            .or(config.symbols.as_deref())
            .unwrap_or("auto"),
    );
    let ctx = CellContext {
        theme: &theme,
        symbols: &symbols,
        area_width: DUMP_AREA_WIDTH,
        compact: false,
        data_col_widths: Vec::new(),
        // Unbounded so paths render in full; the dump auto-grows column 0 to fit.
        first_col_width: u16::MAX,
    };

    match view {
        DumpView::Branches => {
            let mut rows = branch::list_branches(repo, base)?;
            let pr_map = github::fetch_open_prs_checked(repo_path).unwrap_or_else(|e| {
                eprintln!("note: PR data unavailable ({e}); PR column left blank");
                Default::default()
            });
            for b in &mut rows {
                b.pr = pr_map.get(&b.name).cloned();
            }
            pin_first(&mut rows);
            // Base branch first among the pinned rows, matching the TUI's
            // ListState ordering (it pins the base to index 0). A stable sort
            // by is_base keeps the rest (current branch, then date-ordered
            // non-pinned) in pin_first's order.
            rows.sort_by_key(|b| Reverse(b.is_base));

            // Apply configured sort (if any) to non-pinned items
            let cols = BranchesViewDef.columns();
            if let Some(col_idx) = config.sort_column_index() {
                if let Some(column) = cols.get(col_idx) {
                    if let Some(compare) = column.compare {
                        let ascending = config.sort_asc.unwrap_or(true);
                        list_state::sort_items(&mut rows, compare, ascending);
                    }
                }
            }

            Ok(render_table(
                Some(base),
                &rows,
                &cols,
                render_branch_row,
                &ctx,
                color,
            ))
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
                    r.disjoint = res.disjoint;
                }
            }

            // Remote squash-merge pass, mirroring the TUI (app.rs::spawn_remote_load
            // builds the candidates; the drain in App::tick folds results back).
            // Candidate key is the remote `full_ref`; the commit hash is the peeled
            // commit of `refs/remotes/<full_ref>`; results map back by `full_ref`.
            // The TUI builds candidates from the phase-1 Pending state; here the
            // enricher has already run synchronously, so the equivalent set is the
            // non-base remotes that came back Unmerged (squash detection only ever
            // flips Unmerged -> SquashMerged). Uses the same per-repo BranchCache.
            // Remote branches don't precompute a merge base, so the slot is None and
            // is_squash_merged falls back to `git merge-base`.
            let candidates: Vec<(String, String, Option<String>)> = rows
                .iter()
                .filter(|b| b.merge_status == MergeStatus::Unmerged && !b.is_base)
                .filter_map(|b| {
                    let refname = format!("refs/remotes/{}", b.full_ref);
                    repo.find_reference(&refname)
                        .ok()
                        .and_then(|r| r.peel_to_commit().ok())
                        .map(|c| (b.full_ref.clone(), c.id().to_string(), None))
                })
                .collect();
            if !candidates.is_empty() {
                let remote_cache = cache::BranchCache::load(repo_path);
                let squash_rx = squash_loader::spawn_squash_checker(
                    repo_path.to_path_buf(),
                    base.to_string(),
                    candidates,
                    remote_cache,
                );
                for res in squash_rx.iter() {
                    if !matches!(res.status, MergeStatus::Unmerged | MergeStatus::Pending) {
                        if let Some(r) = rows.iter_mut().find(|r| r.full_ref == res.branch_name) {
                            r.merge_status = res.status;
                        }
                    }
                }
            }

            let pr_map = github::fetch_open_prs_checked(repo_path).unwrap_or_else(|e| {
                eprintln!("note: PR data unavailable ({e}); PR column left blank");
                Default::default()
            });
            for r in &mut rows {
                r.pr = pr_map.get(&r.short_name).cloned();
            }
            pin_first(&mut rows);

            // Apply configured sort (if any) to non-pinned items
            let cols = RemotesViewDef.columns();
            if let Some(col_idx) = config.sort_column_index() {
                if let Some(column) = cols.get(col_idx) {
                    if let Some(compare) = column.compare {
                        let ascending = config.sort_asc.unwrap_or(true);
                        list_state::sort_items(&mut rows, compare, ascending);
                    }
                }
            }

            Ok(render_table(
                None,
                &rows,
                &cols,
                render_remote_row,
                &ctx,
                color,
            ))
        }
        DumpView::Tags => {
            let mut rows = tags::list_tags(repo);
            pin_first(&mut rows);
            let cols = TagsViewDef.columns();
            Ok(render_table(
                None,
                &rows,
                &cols,
                render_tag_row,
                &ctx,
                color,
            ))
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
            // Correlate merge status from the branch list (same data the
            // Branches view shows). Graceful degrade: on error, worktrees keep
            // their Unmerged default.
            if let Ok(branches) = branch::list_branches(repo, base) {
                worktree::apply_branch_merge_status(&mut rows, &branches);
            }
            pin_first(&mut rows);
            let cols = WorktreesViewDef.columns();
            Ok(render_table(
                None,
                &rows,
                &cols,
                render_worktree_row,
                &ctx,
                color,
            ))
        }
    }
}
