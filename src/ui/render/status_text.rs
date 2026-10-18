use crate::types::MergeStatus;
use crate::view::list_state::ListState;
use crate::view::ViewId;
use crate::view::ViewItem;

use super::RenderContext;

/// Build default status bar text based on the active view.
pub(super) fn default_status_text(ctx: &RenderContext) -> String {
    match ctx.active_view {
        ViewId::Graph => {
            let commits = ctx.graph.snapshot().map(|snapshot| snapshot.commits.len()).unwrap_or(0);
            let (local, remote) = ctx
                .graph
                .snapshot()
                .map(|snapshot| (snapshot.ref_counts.local, snapshot.ref_counts.remote))
                .unwrap_or_default();
            format!(
                " {commits} commits | {local} local refs | {remote} remote refs — [j/k]move [g/G]home/end [Enter]details/actions [o]ptions [L]older [r]eload [?]help [q]uit"
            )
        }
        ViewId::Branches => format_branch_like(
            "branches",
            branch_like_summary(ctx.branches),
            "[/]search [\\]filter [g]graph [c]heckout [d]el [D]el+remote [p]ush [f]etch [F2]diag [?]help [q]uit",
        ),
        ViewId::Remotes => format_branch_like(
            "remote branches",
            branch_like_summary(ctx.remotes),
            "[/]search [\\]filter [g]graph [c]heckout [d]el [f]etch [?]help [q]uit",
        ),
        ViewId::Tags => {
            let total = ctx.tags.items().len();
            format!(
                " {} tags \u{2014} [/]search [\\]filter [g]graph [d]el [D]el+remote [p]ush [f]etch [?]help [q]uit",
                total
            )
        }
        ViewId::Worktrees => {
            let total = ctx.worktrees.items().len();
            format!(
                " {} worktrees \u{2014} [/]search [g]graph [d]el [D]force-del [f]etch [?]help [q]uit",
                total
            )
        }
    }
}

/// Counts `(total, selected, merged, squashed, cherry_picked)` for a branch-like list view.
/// Works for any item type whose `ViewItem::merge_status` is populated.
pub(super) fn branch_like_summary<T: ViewItem>(state: &ListState<T>) -> (usize, usize, usize, usize, usize) {
    let total = state.items().len();
    let selected = state.selected().iter().filter(|&&s| s).count();
    let merged = state
        .items()
        .iter()
        .filter(|i| i.merge_status() == Some(&MergeStatus::Merged))
        .count();
    let squashed = state
        .items()
        .iter()
        .filter(|i| i.merge_status() == Some(&MergeStatus::SquashMerged))
        .count();
    let cherry_picked = state
        .items()
        .iter()
        .filter(|i| i.merge_status() == Some(&MergeStatus::CherryPicked))
        .count();
    (total, selected, merged, squashed, cherry_picked)
}

/// Formats a branch-like status bar line from a noun, summary counts, and the
/// view's shortcut suffix. Branches and Remotes share this formatter; only the
/// noun and the shortcut list differ between them.
pub(super) fn format_branch_like(
    noun: &str,
    summary: (usize, usize, usize, usize, usize),
    shortcuts: &str,
) -> String {
    let (total, selected, merged, squashed, cherry_picked) = summary;
    format!(
        " {total} {noun} | {selected} selected | {merged} merged | {squashed} squashed | {cherry_picked} cherry-picked \u{2014} {shortcuts}"
    )
}
