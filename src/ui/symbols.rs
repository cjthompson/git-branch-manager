#[derive(Debug, Clone, Copy)]
pub struct SymbolSet {
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

pub static ASCII: SymbolSet = SymbolSet {
    checkbox_on: "[x]",
    checkbox_off: "[ ]",
    cursor_prefix: ">",
    arrow_up: "+",
    arrow_down: "-",
    current_branch: "*",
    status_merged: "+",
    status_squash_merged: "~",
    status_unmerged: "-",
};

pub static UNICODE: SymbolSet = SymbolSet {
    checkbox_on: "\u{25c9}",   // ◉
    checkbox_off: "\u{25ef}",  // ◯
    cursor_prefix: "\u{276f}", // ❯
    arrow_up: "\u{2191}",      // ↑
    arrow_down: "\u{2193}",    // ↓
    current_branch: "\u{25cf}", // ●
    status_merged: "\u{2714}",        // ✔
    status_squash_merged: "\u{2248}", // ≈
    status_unmerged: "\u{2718}",      // ✘
};

pub static POWERLINE: SymbolSet = SymbolSet {
    checkbox_on: "\u{f046}",
    checkbox_off: "\u{f096}",
    cursor_prefix: "\u{e0b1}",
    arrow_up: "\u{f062}",
    arrow_down: "\u{f063}",
    current_branch: "\u{e0a0}",
    status_merged: "\u{f00c}",        // nerd font check
    status_squash_merged: "\u{f0ab}", // nerd font compress/squash
    status_unmerged: "\u{f00d}",      // nerd font x-mark
};

pub fn detect() -> &'static SymbolSet {
    let term = std::env::var("TERM_PROGRAM").unwrap_or_default();
    match term.as_str() {
        "iTerm.app" | "WezTerm" | "kitty" | "Alacritty" => &POWERLINE,
        _ => &UNICODE,
    }
}

pub fn from_name(name: &str) -> &'static SymbolSet {
    match name {
        "ascii" => &ASCII,
        "unicode" => &UNICODE,
        "powerline" => &POWERLINE,
        _ => detect(),
    }
}
