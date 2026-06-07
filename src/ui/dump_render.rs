//! Headless table rendering for the `--branches`/`--remotes`/`--tags`/`--worktrees`
//! dump flags. Serializes the same `Line` rows the TUI draws into plain or
//! ANSI-colored fixed-width text.

use ratatui::style::{Color, Modifier, Style};

// used by render_table in Task 3
#[allow(dead_code)]
const RESET: &str = "\x1b[0m";

/// Build the SGR escape prefix for a style (modifiers, then fg, then bg).
/// Returns an empty string when the style carries no fg/bg/modifier.
// used by render_table in Task 3
#[allow(dead_code)]
pub(crate) fn sgr_prefix(style: &Style) -> String {
    let mut codes: Vec<String> = Vec::new();
    let m = style.add_modifier;
    if m.contains(Modifier::BOLD) {
        codes.push("1".into());
    }
    if m.contains(Modifier::DIM) {
        codes.push("2".into());
    }
    if m.contains(Modifier::ITALIC) {
        codes.push("3".into());
    }
    if m.contains(Modifier::UNDERLINED) {
        codes.push("4".into());
    }
    if let Some(fg) = style.fg.and_then(|c| color_code(c, true)) {
        codes.push(fg);
    }
    if let Some(bg) = style.bg.and_then(|c| color_code(c, false)) {
        codes.push(bg);
    }
    if codes.is_empty() {
        String::new()
    } else {
        format!("\x1b[{}m", codes.join(";"))
    }
}

/// SGR numeric code for a color. `fg` selects foreground (else +10 for background).
// used by sgr_prefix (consumed in Task 3)
#[allow(dead_code)]
fn color_code(c: Color, fg: bool) -> Option<String> {
    let base = |n: u8| -> String {
        if fg {
            n.to_string()
        } else {
            (n + 10).to_string()
        }
    };
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
        Color::Rgb(r, g, b) => format!(
            "{};2;{};{};{}",
            if fg { 38 } else { 48 },
            r,
            g,
            b
        ),
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
        assert_eq!(
            sgr_prefix(&Style::new().fg(Color::Indexed(141))),
            "\x1b[38;5;141m"
        );
    }
}
