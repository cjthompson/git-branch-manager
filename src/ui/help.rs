use ratatui::prelude::*;
use ratatui::style::Modifier;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::theme::Theme;
use crate::view::ViewId;

use super::shared::centered_rect;

/// Common keybindings shown for all views.
const COMMON_KEYS: &[(&str, &str)] = &[
    ("j/\u{2193}", "Move down"),
    ("k/\u{2191}", "Move up"),
    ("PgUp/Dn", "Page scroll"),
    ("g/G", "Home / End"),
    ("SPACE", "Toggle selection"),
    ("a", "Select all"),
    ("n", "Deselect all"),
    ("m", "Select merged"),
    ("i", "Invert selection"),
    ("ENTER", "Operations menu"),
    ("s", "Cycle sort column"),
    ("S", "Reverse sort"),
    ("/", "Search"),
    ("\\", "Filter menu"),
    ("Tab", "Next view"),
    ("S-Tab", "Prev view"),
    ("T", "Cycle theme"),
    ("Y", "Cycle symbols"),
    (",", "Settings"),
    ("F2", "Diagnostics"),
    ("?", "Toggle help"),
    ("q", "Quit"),
];

/// Branch-view-specific keys.
const BRANCH_KEYS: &[(&str, &str)] = &[
    ("c", "Checkout"),
    ("x", "Delete cursor branch"),
    ("d", "Delete local (selected)"),
    ("D", "Delete local + remote"),
    ("p", "Push (sets upstream)"),
    ("f", "Fetch"),
    ("F", "Fetch + prune"),
    ("R", "Force recheck cache"),
];

/// Remote-view-specific keys.
const REMOTE_KEYS: &[(&str, &str)] = &[
    ("d", "Delete remote (selected)"),
    ("c", "Checkout remote"),
    ("f", "Fetch remote"),
    ("F", "Fetch + prune"),
];

/// Tags-view-specific keys.
const TAG_KEYS: &[(&str, &str)] = &[
    ("d", "Delete tag (selected)"),
    ("D", "Delete tag + remote"),
    ("p", "Push tag"),
    ("f", "Fetch"),
    ("F", "Fetch + prune"),
];

/// Worktrees-view-specific keys.
const WORKTREE_KEYS: &[(&str, &str)] = &[
    ("d", "Remove worktree"),
    ("D", "Force remove worktree"),
    ("f", "Fetch"),
    ("F", "Fetch + prune"),
];

/// Renders the help overlay on top of the current view.
pub fn draw_help(frame: &mut Frame, active_view: ViewId, theme: &Theme) {
    let area = frame.area();

    let key_style = Style::default()
        .fg(theme.accent_fg())
        .add_modifier(Modifier::BOLD);

    // Choose view-specific keys
    let view_keys: &[(&str, &str)] = match active_view {
        ViewId::Branches => BRANCH_KEYS,
        ViewId::Remotes => REMOTE_KEYS,
        ViewId::Tags => TAG_KEYS,
        ViewId::Worktrees => WORKTREE_KEYS,
    };

    // Combine: view-specific first, then common
    let section_header = format!("{} Keys", active_view.label());
    let mut all_entries: Vec<(&str, &str)> = Vec::new();
    all_entries.extend_from_slice(view_keys);
    // We'll interleave with section headers below

    let all_lines: Vec<HelpEntry> = build_help_entries(&section_header, view_keys, COMMON_KEYS);

    let col_width = 38u16;
    let separator = "  \u{2502}  "; // " | "

    // Use two-column layout if terminal is wide enough and too short for single column
    let content_height_single = all_lines.len() as u16 + 2;
    let use_two_cols = area.width >= col_width * 2 + separator.len() as u16 + 4
        && area.height < content_height_single;

    if use_two_cols {
        let mid = all_lines.len().div_ceil(2);
        let left = &all_lines[..mid];
        let right = &all_lines[mid..];

        let lines: Vec<Line> = (0..mid)
            .map(|i| {
                let mut spans = render_help_entry(&left[i], key_style, theme);
                // Pad left column to fixed width
                let left_text: String = spans.iter().map(|s| s.content.as_ref()).collect();
                let pad = col_width as usize - left_text.chars().count().min(col_width as usize);
                spans.push(Span::raw(" ".repeat(pad)));
                // Separator
                let sep_style = Style::default().add_modifier(Modifier::DIM);
                spans.push(Span::styled(separator.to_string(), sep_style));
                // Right column
                if let Some(right_entry) = right.get(i) {
                    spans.extend(render_help_entry(right_entry, key_style, theme));
                }
                Line::from(spans)
            })
            .collect();

        let width = (col_width * 2 + separator.len() as u16 + 4).min(area.width);
        let height = (mid as u16 + 2).min(area.height);
        let rect = centered_rect(width, height, area);

        let block = Block::default()
            .title("Help")
            .title_style(theme.title)
            .borders(Borders::ALL);
        let paragraph = Paragraph::new(lines).block(block);
        frame.render_widget(Clear, rect);
        frame.render_widget(paragraph, rect);
    } else {
        // Single column
        let lines: Vec<Line> = all_lines
            .iter()
            .map(|entry| Line::from(render_help_entry(entry, key_style, theme)))
            .collect();
        let width = (col_width + 4).min(area.width);
        let height = content_height_single.min(area.height);
        let rect = centered_rect(width, height, area);

        let block = Block::default()
            .title("Help")
            .title_style(theme.title)
            .borders(Borders::ALL);
        let paragraph = Paragraph::new(lines).block(block);
        frame.render_widget(Clear, rect);
        frame.render_widget(paragraph, rect);
    }
}

enum HelpEntry {
    Section(String),
    Key { key: String, desc: String },
    Blank,
}

fn build_help_entries(
    view_section: &str,
    view_keys: &[(&str, &str)],
    common_keys: &[(&str, &str)],
) -> Vec<HelpEntry> {
    let mut entries = Vec::new();

    // View-specific section
    entries.push(HelpEntry::Section(view_section.to_string()));
    for &(k, d) in view_keys {
        entries.push(HelpEntry::Key {
            key: k.to_string(),
            desc: d.to_string(),
        });
    }
    entries.push(HelpEntry::Blank);

    // Common section
    entries.push(HelpEntry::Section("Navigation & General".to_string()));
    for &(k, d) in common_keys {
        entries.push(HelpEntry::Key {
            key: k.to_string(),
            desc: d.to_string(),
        });
    }

    entries
}

fn render_help_entry<'a>(entry: &HelpEntry, key_style: Style, theme: &Theme) -> Vec<Span<'a>> {
    match entry {
        HelpEntry::Section(title) => {
            vec![Span::styled(
                title.clone(),
                theme.title.add_modifier(Modifier::BOLD),
            )]
        }
        HelpEntry::Key { key, desc } => {
            vec![
                Span::styled(format!("{:<10}", key), key_style),
                Span::styled(desc.clone(), Style::default()),
            ]
        }
        HelpEntry::Blank => {
            vec![Span::raw("")]
        }
    }
}
