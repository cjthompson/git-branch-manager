use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use crate::theme::Theme;
use crate::view::ViewId;

use super::modal::{draw_modal_shell, ModalFooter, ModalScroll, ModalSpec};

/// Common keybindings shown for all views.
const COMMON_KEYS: &[(&str, &str)] = &[
    ("j/\u{2193}", "Move down"),
    ("k/\u{2191}", "Move up"),
    ("PgUp/Dn", "Page scroll"),
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
    ("x", "Cancel running job"),
    ("X", "Clear queued jobs"),
    ("?", "Toggle help"),
    ("q", "Quit"),
];

/// Branch-view-specific keys.
const BRANCH_KEYS: &[(&str, &str)] = &[
    ("c", "Checkout"),
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

/// Graph-view-specific keys.
const GRAPH_KEYS: &[(&str, &str)] = &[
    ("g/G", "Home / End"),
    ("h/l or ←/→", "Scroll commit text and refs"),
    ("o", "Graph options"),
    ("L", "Load 500 older commits"),
    ("r", "Reload graph"),
];

/// Graph-view concept notes shown in the help overlay.
const GRAPH_CONCEPTS: &[&str] = &[
    "Graph = DAG + live-ref pane",
    "o toggles remote refs (saved)",
    "L adds +500 commits (session)",
    "Falls back to git log if needed",
];

/// Renders the help overlay on top of the current view.
pub fn draw_help(frame: &mut Frame, active_view: ViewId, scroll: &mut usize, theme: &Theme) {
    let key_style = theme.modal_key;

    // Choose view-specific keys
    let view_keys: &[(&str, &str)] = match active_view {
        ViewId::Graph => GRAPH_KEYS,
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

    let all_lines: Vec<HelpEntry> = build_help_entries(
        &section_header,
        view_keys,
        COMMON_KEYS,
        (active_view != ViewId::Graph).then_some(("g", "Jump selected ref to Graph")),
        if active_view == ViewId::Graph {
            GRAPH_CONCEPTS
        } else {
            &[]
        },
    );

    let col_width = 38u16;
    let separator = "  \u{2502}  "; // " | "

    // Two-column layout is the default whenever the terminal is wide enough
    // to fit both columns plus the separator; fall back to single-column
    // only when the terminal is too narrow. (Previously this was inverted:
    // single-column was the default and two-column only kicked in when the
    // terminal was too short for single-column.)
    let use_two_cols = frame.area().width >= col_width * 2 + separator.chars().count() as u16 + 4;
    let areas = draw_modal_shell(
        frame,
        &ModalSpec::new(
            "Help",
            ModalFooter::hints(&[("j/k", "Scroll"), ("PgUp/Dn", "Page"), ("Esc", "Close")]),
            82,
            22,
        ),
        theme,
    );

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
                let sep_style = theme.dim;
                spans.push(Span::styled(separator.to_string(), sep_style));
                // Right column
                if let Some(right_entry) = right.get(i) {
                    spans.extend(render_help_entry(right_entry, key_style, theme));
                }
                Line::from(spans)
            })
            .collect();

        let offset = clamp_scroll(scroll, mid as u16, areas.body.height);
        frame.render_widget(Paragraph::new(lines).scroll((offset, 0)), areas.body);
    } else {
        // Single column
        let lines: Vec<Line> = all_lines
            .iter()
            .map(|entry| Line::from(render_help_entry(entry, key_style, theme)))
            .collect();
        let offset = clamp_scroll(scroll, all_lines.len() as u16, areas.body.height);
        frame.render_widget(Paragraph::new(lines).scroll((offset, 0)), areas.body);
    }
}

/// Bounds the persisted Help offset to the rows that can occupy the shell body.
fn clamp_scroll(scroll: &mut usize, total_rows: u16, viewport_rows: u16) -> u16 {
    let mut modal_scroll = ModalScroll::default();
    modal_scroll.offset = (*scroll).min(u16::MAX as usize) as u16;
    modal_scroll.clamp(total_rows, viewport_rows);
    *scroll = modal_scroll.offset as usize;
    modal_scroll.offset
}

enum HelpEntry {
    Section(String),
    Key { key: String, desc: String },
    Note(String),
    Blank,
}

fn build_help_entries(
    view_section: &str,
    view_keys: &[(&str, &str)],
    common_keys: &[(&str, &str)],
    list_graph_key: Option<(&str, &str)>,
    concepts: &[&str],
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
    if !concepts.is_empty() {
        entries.push(HelpEntry::Blank);
        entries.push(HelpEntry::Section("Graph Concepts".to_string()));
        for note in concepts {
            entries.push(HelpEntry::Note(note.to_string()));
        }
    }
    entries.push(HelpEntry::Blank);

    // Common section
    entries.push(HelpEntry::Section("Navigation & General".to_string()));
    if let Some((key, description)) = list_graph_key {
        entries.push(HelpEntry::Key {
            key: key.to_string(),
            desc: description.to_string(),
        });
    }
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
            vec![Span::styled(title.clone(), theme.modal_title)]
        }
        HelpEntry::Key { key, desc } => {
            vec![
                Span::styled(format!("{:<10}", key), key_style),
                Span::styled(desc.clone(), theme.modal_secondary),
            ]
        }
        HelpEntry::Note(text) => vec![Span::styled(text.clone(), theme.modal_secondary)],
        HelpEntry::Blank => {
            vec![Span::raw("")]
        }
    }
}
