#[derive(Debug, Clone)]
pub struct SymbolSet {
    pub name: &'static str,
    pub checkbox_on: &'static str,
    pub checkbox_off: &'static str,
    pub cursor_prefix: &'static str,
    pub arrow_up: &'static str,
    pub arrow_down: &'static str,
    pub current_branch: &'static str,
    pub status_merged: &'static str,
    pub status_squash_merged: &'static str,
    pub status_unmerged: &'static str,
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
            status_merged: "+",
            status_squash_merged: "~",
            status_unmerged: "-",
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
            status_merged: "\u{2714}",        // heavy check mark
            status_squash_merged: "\u{2248}", // almost equal to
            status_unmerged: "\u{2718}",      // heavy ballot X
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
            status_merged: "\u{f126}",        // nerd font code-fork (merged)
            status_squash_merged: "\u{25cf}", // solid circle (squash-merged)
            status_unmerged: "\u{f00d}",      // nerd font x-mark
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
}
