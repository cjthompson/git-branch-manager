use super::ViewItem;
use crate::types::{MergeStatus, WorkingTreeStatus, WorktreeInfo};
use std::cmp::Ordering;

/// Defines a single column in a view's table layout.
/// The compare function is used for sorting; None means not sortable.
pub struct ColumnDef<T: ViewItem> {
    pub key: &'static str,
    pub name: &'static str,
    /// Whether the column's name should appear in the table header.
    pub show_header: bool,
    pub min_width: u16,
    /// Optional row-specific minimum width, widened to the largest value in
    /// the list. Used when a compact marker is usually enough but some rows
    /// need a longer value (for example, an upstream marked `gone`).
    pub content_min_width: Option<fn(&T) -> u16>,
    /// When Some(w), use width w instead of min_width when terminal width >= 70.
    pub wide_width: Option<u16>,
    /// Hide this column when terminal width is below this threshold
    pub hide_below_width: Option<u16>,
    /// Comparison function for sorting. None = column is not sortable.
    pub compare: Option<fn(&T, &T) -> Ordering>,
}

impl<T: ViewItem> ColumnDef<T> {
    /// Resolve a column's minimum width for one row.
    pub fn min_width_for(&self, item: &T) -> u16 {
        self.content_min_width
            .map_or(self.min_width, |width| self.min_width.max(width(item)))
    }

    /// Resolve a column's minimum width for the given rows.
    pub fn min_width_for_items(&self, items: &[T]) -> u16 {
        items
            .iter()
            .map(|item| self.min_width_for(item))
            .max()
            .unwrap_or(self.min_width)
    }
}

/// Comparator: sort by last commit date (ascending = oldest first).
pub fn age_cmp<T: ViewItem>(a: &T, b: &T) -> Ordering {
    a.last_commit_date().cmp(b.last_commit_date())
}

/// Build a standard "Age" column definition for any view.
pub fn age_column<T: ViewItem>() -> ColumnDef<T> {
    ColumnDef {
        key: "age",
        name: "Age",
        show_header: true,
        min_width: 5,
        content_min_width: None,
        wide_width: Some(14),
        hide_below_width: Some(60),
        compare: Some(age_cmp),
    }
}

/// Comparator: sort by ahead count, then behind count (ascending).
pub fn ahead_behind_cmp<T: ViewItem>(a: &T, b: &T) -> Ordering {
    a.ahead()
        .unwrap_or(0)
        .cmp(&b.ahead().unwrap_or(0))
        .then(a.behind().unwrap_or(0).cmp(&b.behind().unwrap_or(0)))
}

/// Build a standard "A/B" column definition. `wide_width` fits full counts
/// (e.g. `↑1 ↓164`); `min_width` fits just the "A/B" header / arrows-only
/// compact form (see `ui::cells::ahead_behind_parts`). This column stays
/// visible at every terminal width and compacts before Age is hidden.
pub fn ahead_behind_column<T: ViewItem>() -> ColumnDef<T> {
    ColumnDef {
        key: "ahead_behind",
        name: "A/B",
        show_header: true,
        min_width: 3,
        content_min_width: None,
        wide_width: Some(8),
        hide_below_width: None,
        compare: Some(ahead_behind_cmp),
    }
}

/// Comparator: sort by PR number. Items with a PR sort before items without.
pub fn pr_cmp<T: ViewItem>(a: &T, b: &T) -> Ordering {
    match (a.pr_info().map(|p| p.number), b.pr_info().map(|p| p.number)) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

/// Build a standard "PR" column definition. `min_width` fits the "PR" header
/// and the icon-only compact form; `wide_width` fits `#` + up to 8 digits.
/// Never hidden — narrow terminals fall back to the icon instead of hiding
/// the column (see `ui::cells::pr_parts`).
pub fn pr_column<T: ViewItem>() -> ColumnDef<T> {
    ColumnDef {
        key: "pr",
        name: "PR",
        show_header: true,
        min_width: 2,
        content_min_width: None,
        wide_width: Some(9),
        hide_below_width: None,
        compare: Some(pr_cmp),
    }
}

/// Rank a MergeStatus for sorting: lower = sorted first.
pub fn merge_status_rank(status: &MergeStatus) -> u8 {
    match status {
        MergeStatus::Merged => 0,
        MergeStatus::InSync => 1,
        MergeStatus::SquashMerged => 2,
        MergeStatus::RemoteMerged => 3,
        MergeStatus::LocalMerged => 4,
        MergeStatus::RemoteSquashMerged => 5,
        MergeStatus::LocalSquashMerged => 6,
        MergeStatus::CherryPicked => 7,
        MergeStatus::RemoteCherryPicked => 8,
        MergeStatus::LocalCherryPicked => 9,
        MergeStatus::LikelySquashMerged => 10,
        MergeStatus::Unmerged => 11,
        MergeStatus::Pending => 12,
    }
}

/// Comparator: sort by merge status rank. Items without a merge status sort last.
pub fn merge_status_cmp<T: ViewItem>(a: &T, b: &T) -> Ordering {
    let rank_a = a.merge_status().map_or(u8::MAX, merge_status_rank);
    let rank_b = b.merge_status().map_or(u8::MAX, merge_status_rank);
    rank_a.cmp(&rank_b)
}

/// Overwrite `current` with `new_status` only when `new_status` is at least
/// as confident as whatever is already there (lower `merge_status_rank` =
/// more confident/definitive). Background result channels (`squash_rx`,
/// `cherry_rx`, `remote_squash_rx`, `remote_enrich_rx`) race against each
/// other and can deliver results for the same branch out of order; a later,
/// less-confident result must not clobber an already-confirmed status.
/// Returns whether the new status was accepted.
pub fn apply_merge_status_if_confident(current: &mut MergeStatus, new_status: MergeStatus) -> bool {
    if merge_status_rank(&new_status) <= merge_status_rank(current) {
        *current = new_status;
        true
    } else {
        false
    }
}

/// Build the shared merge-status column ("Merge" in every view). The name is a
/// parameter only so callers read explicitly; pass `"Merge"`.
pub fn merge_status_column<T: ViewItem>(name: &'static str) -> ColumnDef<T> {
    ColumnDef {
        key: "merge",
        name,
        show_header: true,
        min_width: 5,
        content_min_width: None,
        wide_width: Some(16),
        hide_below_width: None,
        compare: Some(merge_status_cmp),
    }
}

/// Rank a working-tree status for sorting: dirtier states sort later.
pub fn wt_status_rank(s: &WorkingTreeStatus) -> u8 {
    (s.has_staged as u8) * 4 + (s.has_modified as u8) * 2 + (s.has_untracked as u8)
}

/// Comparator: sort worktrees by working-tree dirtiness.
pub fn wt_status_cmp(a: &WorktreeInfo, b: &WorktreeInfo) -> Ordering {
    wt_status_rank(&a.wt_status).cmp(&wt_status_rank(&b.wt_status))
}

/// Build the shared worktree "Status" (working-tree dirtiness) column. This is
/// the single definition of how the Status column sizes and sorts; views use it
/// rather than declaring their own. Renders full words when wide and single
/// letters when narrow (see `ui::cells::worktree_status_line`).
pub fn worktree_status_column() -> ColumnDef<WorktreeInfo> {
    ColumnDef {
        key: "status",
        name: "Status",
        show_header: true,
        // min_width must be >= "Status".len() (6) so the header itself never
        // gets clipped; the compact single-letter values (c/s/u/t) fit easily.
        min_width: 6,
        content_min_width: None,
        wide_width: Some(9),
        hide_below_width: Some(80),
        compare: Some(wt_status_cmp),
    }
}

#[cfg(test)]
mod tests;
