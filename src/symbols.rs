#[derive(Debug, Clone)]
pub struct SymbolSet {
    pub name: &'static str,
    pub checkbox_on: &'static str,
    pub checkbox_off: &'static str,
    pub cursor_prefix: &'static str,
    pub arrow_up: &'static str,
    pub arrow_down: &'static str,
    pub current_branch: &'static str,
    /// Marker for an ordinary commit in the Graph tab.
    pub graph_commit: &'static str,
    /// Marker for a base commit that may be a squash-merge landing commit.
    pub graph_squash_commit: &'static str,
    /// Marker for a branch-tip commit that landed via individual cherry-picks.
    pub graph_cherry_commit: &'static str,
    /// Marker for a merge commit in the Graph tab.
    pub graph_merge: &'static str,
    /// Left-facing merge connector used by the Powerline graph renderer.
    pub graph_arrow_left: &'static str,
    /// Right-facing merge connector used by the graph renderer.
    pub graph_arrow_right: &'static str,
    pub graph_remote_ref: &'static str,
    pub graph_tag_ref: &'static str,
    pub status_merged: &'static str,
    pub status_in_sync: &'static str,
    pub status_squash_merged: &'static str,
    pub status_cherry_picked: &'static str,
    pub status_unmerged: &'static str,
    pub status_local_suffix: &'static str,
    pub status_remote_suffix: &'static str,
    /// Shown in the A/B column for a branch that shares no history with the base
    /// (no merge base); its ahead/behind counts would be the full, misleading
    /// history sizes, so we render this marker instead.
    pub disjoint: &'static str,
    /// Compact stand-in for the PR number when the PR column is too narrow to
    /// show digits; colored via `theme.pr_draft/open/merged/closed`.
    pub pr_indicator: &'static str,
    /// Shown when a tracking counterpart exists: Branches' upstream-exists
    /// indicator and Remotes' local-branch-exists indicator.
    pub tracking_link: &'static str,
}

impl SymbolSet {
    pub fn ascii() -> Self {
        Self {
            name: "ascii",
            checkbox_on: "[x]",
            checkbox_off: "[ ]",
            cursor_prefix: ">",
            arrow_up: "+",
            arrow_down: "-",
            current_branch: "*",
            graph_commit: "o",
            graph_squash_commit: "~",
            graph_cherry_commit: "c",
            graph_merge: "+",
            graph_arrow_left: "<",
            graph_arrow_right: ">",
            graph_remote_ref: "@",
            graph_tag_ref: "#",
            status_merged: "+",
            status_in_sync: "=",
            status_squash_merged: "~",
            status_cherry_picked: "c",
            status_unmerged: "-",
            status_local_suffix: "^",
            status_remote_suffix: "v",
            disjoint: "!=",
            pr_indicator: "P",
            tracking_link: "<>",
        }
    }

    pub fn unicode() -> Self {
        Self {
            name: "unicode",
            checkbox_on: "\u{25c9}",          // filled circle
            checkbox_off: "\u{25ef}",         // empty circle
            cursor_prefix: "\u{276f}",        // heavy right-pointing angle quotation mark
            arrow_up: "\u{2191}",             // up arrow
            arrow_down: "\u{2193}",           // down arrow
            current_branch: "\u{25cf}",       // black circle
            graph_commit: "\u{25cf}",         // black circle
            graph_squash_commit: "\u{2248}",  // almost equal to
            graph_cherry_commit: "\u{2605}",  // black star
            graph_merge: "\u{25cb}",          // white circle
            graph_arrow_left: "\u{25c0}",     // black left-pointing triangle
            graph_arrow_right: "\u{25b6}",    // black right-pointing triangle
            graph_remote_ref: "\u{2601}",     // cloud
            graph_tag_ref: "\u{2311}",        // tag marker
            status_merged: "\u{2714}",        // heavy check mark
            status_in_sync: "\u{2261}",       // identical to (≡)
            status_squash_merged: "\u{2248}", // almost equal to
            status_cherry_picked: "\u{2605}", // black star
            status_unmerged: "\u{2718}",      // heavy ballot X
            status_local_suffix: "\u{2191}",  // ↑ upwards arrow
            status_remote_suffix: "\u{2193}", // ↓ downwards arrow
            disjoint: "\u{2260}",             // not equal to (no shared history)
            pr_indicator: "\u{21c4}",         // ⇄ rightwards arrow over leftwards arrow
            tracking_link: "\u{1f517}",       // 🔗 link
        }
    }

    pub fn powerline() -> Self {
        Self {
            name: "powerline",
            checkbox_on: "\u{f058}",          // nerd font check-circle
            checkbox_off: "\u{f111}",         // nerd font circle
            cursor_prefix: "\u{e0b1}",        // powerline right arrow thin
            arrow_up: "\u{f062}",             // nerd font arrow-up
            arrow_down: "\u{f063}",           // nerd font arrow-down
            current_branch: "\u{e0a0}",       // powerline branch
            graph_commit: "\u{25cf}",         // medium filled circle
            graph_squash_commit: "\u{2248}",  // almost equal to
            graph_cherry_commit: "\u{f005}",  // nerd font fa-star
            graph_merge: "\u{f407}",          // nerd font git-merge
            graph_arrow_left: "\u{25c0}",     // black left-pointing triangle
            graph_arrow_right: "\u{25b6}",    // black right-pointing triangle
            graph_remote_ref: "\u{f0c2}",     // nerd font cloud
            graph_tag_ref: "\u{f02b}",        // nerd font tag
            status_merged: "\u{f126}",        // nerd font code-fork (merged)
            status_in_sync: "\u{f441}",       // nerd font nf-dev-equals
            status_squash_merged: "\u{25cf}", // solid circle (squash-merged)
            status_cherry_picked: "\u{f005}", // nerd font fa-star
            status_unmerged: "\u{f00d}",      // nerd font x-mark
            status_local_suffix: "\u{2191}",  // ↑ upwards arrow
            status_remote_suffix: "\u{2193}", // ↓ downwards arrow
            disjoint: "\u{2260}",             // not equal to (no shared history)
            pr_indicator: "\u{f407}",         // nerd font oct-git-pull-request
            tracking_link: "\u{f0c1}",        // nerd font link
        }
    }

    pub fn from_name(name: &str) -> Self {
        match name {
            "ascii" => Self::ascii(),
            "unicode" => Self::unicode(),
            "powerline" => Self::powerline(),
            _ => Self::detect(),
        }
    }

    /// Auto-detect the best symbol set based on terminal
    pub fn detect() -> Self {
        let term = std::env::var("TERM_PROGRAM").unwrap_or_default();
        match term.as_str() {
            "iTerm.app" | "WezTerm" | "kitty" | "Alacritty" => Self::powerline(),
            _ => Self::unicode(),
        }
    }

    /// Cycle to the next symbol set
    pub fn next(&self) -> Self {
        match self.name {
            "ascii" => Self::unicode(),
            "unicode" => Self::powerline(),
            "powerline" => Self::ascii(),
            _ => Self::ascii(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbol_cycle() {
        let s = SymbolSet::ascii();
        let s2 = s.next();
        assert_eq!(s2.name, "unicode");
        assert_eq!(s2.checkbox_on, "\u{25c9}");
        let s3 = s2.next();
        assert_eq!(s3.name, "powerline");
        assert_eq!(s3.checkbox_on, "\u{f058}");
        let s4 = s3.next();
        assert_eq!(s4.name, "ascii");
        assert_eq!(s4.checkbox_on, "[x]");
    }

    #[test]
    fn from_name_ascii() {
        let s = SymbolSet::from_name("ascii");
        assert_eq!(s.checkbox_on, "[x]");
        assert_eq!(s.name, "ascii");
    }

    #[test]
    fn from_name_unicode() {
        let s = SymbolSet::from_name("unicode");
        assert_eq!(s.checkbox_on, "\u{25c9}");
        assert_eq!(s.name, "unicode");
    }

    #[test]
    fn from_name_powerline() {
        let s = SymbolSet::from_name("powerline");
        assert_eq!(s.name, "powerline");
    }

    #[test]
    fn powerline_graph_commit_is_a_compact_filled_circle() {
        let s = SymbolSet::powerline();
        assert_eq!(s.graph_commit, "\u{25cf}");
        assert_eq!(s.graph_merge, "\u{f407}");
    }

    #[test]
    fn from_name_unknown_falls_back_to_detect() {
        let s = SymbolSet::from_name("unknown");
        // Should return either unicode or powerline depending on terminal
        assert!(s.name == "unicode" || s.name == "powerline");
    }

    #[test]
    fn ascii_symbols_are_plain() {
        let s = SymbolSet::ascii();
        assert_eq!(s.cursor_prefix, ">");
        assert_eq!(s.arrow_up, "+");
        assert_eq!(s.arrow_down, "-");
        assert_eq!(s.current_branch, "*");
    }

    #[test]
    fn unicode_symbols_are_special() {
        let s = SymbolSet::unicode();
        assert_eq!(s.arrow_up, "\u{2191}");
        assert_eq!(s.arrow_down, "\u{2193}");
    }

    #[test]
    fn every_set_defines_a_disjoint_marker() {
        assert_eq!(SymbolSet::ascii().disjoint, "!=");
        assert_eq!(SymbolSet::unicode().disjoint, "\u{2260}");
        assert_eq!(SymbolSet::powerline().disjoint, "\u{2260}");
    }

    #[test]
    fn every_set_defines_an_in_sync_marker() {
        assert_eq!(SymbolSet::ascii().status_in_sync, "=");
        assert_eq!(SymbolSet::unicode().status_in_sync, "\u{2261}");
        assert_eq!(SymbolSet::powerline().status_in_sync, "\u{f441}");
    }

    #[test]
    fn graph_ref_markers_are_exact_and_width_safe() {
        let sets = [
            SymbolSet::ascii(),
            SymbolSet::unicode(),
            SymbolSet::powerline(),
        ];
        assert_eq!(
            (sets[0].graph_remote_ref, sets[0].graph_tag_ref),
            ("@", "#")
        );
        assert_eq!(
            (sets[1].graph_remote_ref, sets[1].graph_tag_ref),
            ("☁", "⌑")
        );
        assert_eq!(sets[2].graph_remote_ref, "\u{f0c2}");
        assert_eq!(sets[2].graph_tag_ref, "\u{f02b}");
        for set in sets {
            assert_eq!(ratatui::text::Span::raw(set.graph_remote_ref).width(), 1);
            assert_eq!(ratatui::text::Span::raw(set.graph_tag_ref).width(), 1);
        }
    }

    #[test]
    fn possible_squash_commit_markers_are_distinct_and_width_safe() {
        let sets = [
            SymbolSet::ascii(),
            SymbolSet::unicode(),
            SymbolSet::powerline(),
        ];
        assert_eq!(sets[0].graph_squash_commit, "~");
        assert_eq!(sets[1].graph_squash_commit, "≈");
        assert_eq!(sets[2].graph_squash_commit, "≈");
        for set in sets {
            assert_ne!(set.graph_squash_commit, set.graph_commit);
            assert_ne!(set.graph_squash_commit, set.graph_merge);
            assert_eq!(ratatui::text::Span::raw(set.graph_squash_commit).width(), 1);
        }
    }

    #[test]
    fn cherry_commit_markers_are_distinct_and_width_safe() {
        for symbols in [SymbolSet::ascii(), SymbolSet::unicode(), SymbolSet::powerline()] {
            assert_ne!(symbols.graph_cherry_commit, symbols.status_squash_merged);
            assert_ne!(symbols.status_cherry_picked, symbols.status_squash_merged);
            assert_ne!(symbols.graph_cherry_commit, symbols.graph_squash_commit);
            assert_ne!(symbols.graph_cherry_commit, symbols.graph_commit);
            assert_ne!(symbols.graph_cherry_commit, symbols.graph_merge);
            assert_eq!(
                ratatui::text::Span::raw(symbols.graph_cherry_commit).width(),
                1
            );
            assert_eq!(
                ratatui::text::Span::raw(symbols.status_cherry_picked).width(),
                1
            );
        }
    }
}
