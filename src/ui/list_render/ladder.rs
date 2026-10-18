use ratatui::layout::Alignment;

use crate::view::column::ColumnDef;
use crate::view::ViewItem;

/// One column's sizing inputs for the responsive compaction-ladder decision
/// (BL-022 stage 1). Deliberately independent of `ColumnDef<T>`'s `T: ViewItem`
/// generic so the ladder algorithm — and its tests — don't need a concrete
/// row type.
pub(super) struct LadderColumn {
    pub(super) key: &'static str,
    pub(super) min_width: u16,
    pub(super) wide_width: Option<u16>,
    pub(super) is_stretchy: bool,
}

impl LadderColumn {
    fn wide_or_min(&self) -> u16 {
        self.wide_width.unwrap_or(self.min_width)
    }
}

/// The ladder's last rung: every column compact, including ones with no
/// named tier (e.g. worktree Status, stretchy wide floors). This matches the
/// pre-ladder behavior's single "short" state, kept as the ultimate fallback
/// when even demoting every named tier doesn't leave the stretchy column
/// enough room.
pub(super) const FULLY_COMPACT_LEVEL: u8 = 3;

/// Whether the column identified by `key` should render in its compact form
/// at the given ladder `level`. Follows BL-022's priority order as directed
/// for this task: Age is demoted first (level 1), then Merge (level 2), then
/// everything else together — including A/B and PR, which BL-022's own text
/// lists together as one "least important" tier — at `FULLY_COMPACT_LEVEL`
/// (level 3).
pub(super) fn demoted_at_level(key: &str, level: u8) -> bool {
    match level {
        0 => false,
        1 => key == "age",
        2 => matches!(key, "age" | "merge"),
        _ => true,
    }
}

/// Resolve the compaction ladder level (`0..=FULLY_COMPACT_LEVEL`) for a set
/// of visible columns at a given width.
///
/// Walks the ladder from level 0 (nothing compact) upward, stopping at the
/// first level where the stretchy (Branch/Path) column gets at least its own
/// wide floor and at least as much space as the fixed columns combined —
/// the same ">=" room check the old binary toggle used, now re-evaluated one
/// level at a time instead of once. `area_width` is the raw outer terminal
/// width (for the flat `< 70` floor, same as before); `available` is the row
/// width left after the border/highlight-symbol columns are removed (i.e.
/// `columns_area.width`), used for the arithmetic itself.
pub(super) fn resolve_ladder_level(
    columns: &[LadderColumn],
    area_width: u16,
    available: u32,
) -> u8 {
    let gaps = columns.len() as u32; // N+1 segments (checkbox + N columns) -> N gaps
    let mut level = 0u8;
    while level < FULLY_COMPACT_LEVEL {
        let (mut stretchy_wide_floor, mut fixed_total) = (0u32, 0u32);
        for col in columns {
            let w = if demoted_at_level(col.key, level) {
                col.min_width
            } else {
                col.wide_or_min()
            } as u32;
            if col.is_stretchy {
                stretchy_wide_floor += w;
            } else {
                fixed_total += w;
            }
        }
        let stretchy_actual = available.saturating_sub(3 + gaps + fixed_total); // 3 = checkbox
        let room_ok = stretchy_actual >= stretchy_wide_floor && stretchy_actual >= fixed_total;
        if room_ok && area_width >= 70 {
            break;
        }
        level += 1;
    }
    level
}

/// Decide whether a header label should render right-aligned, given whether
/// right alignment is wanted at all (Age / the last column, to match their
/// data cells) and the column's actual resolved width.
///
/// Right alignment truncates overflow by dropping *leading* characters
/// (ratatui preserves the tail), which reads as garbage — e.g. "Merge"
/// becomes "erge" — when a narrow terminal forces the column below its
/// label's width. Falling back to left alignment in that case truncates the
/// tail instead, producing a legible partial word (e.g. "Merg").
pub(super) fn header_alignment(
    wants_right: bool,
    label_len: usize,
    resolved_width: u16,
) -> Alignment {
    if wants_right && resolved_width as usize >= label_len {
        Alignment::Right
    } else {
        Alignment::Left
    }
}

/// Return the indices of columns that remain visible at the given terminal
/// width. Kept separate so the final narrow-width fallback can be tested
/// against a real view's column definitions.
pub(super) fn visible_column_indices<T: ViewItem>(
    columns: &[ColumnDef<T>],
    area_width: u16,
) -> Vec<usize> {
    columns
        .iter()
        .enumerate()
        .filter(|(_, col)| {
            col.hide_below_width
                .is_none_or(|threshold| area_width >= threshold)
        })
        .map(|(i, _)| i)
        .collect()
}
