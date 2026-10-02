use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use super::menu::MenuItem;
use super::modal::{draw_modal_shell, ModalActionRow, ModalFooter, ModalScroll, ModalSpec};
use crate::git::graph::{GraphCommit, GraphRefKind};
use crate::symbols::SymbolSet;
use crate::theme::Theme;
use crate::types::*;

#[derive(Debug, Clone)]
pub enum InfoModalRow {
    GraphCommit(GraphCommit),
    Branch(BranchInfo),
    Remote(RemoteBranchInfo),
    Tag(TagInfo),
    Worktree(WorktreeInfo),
    PrimaryBranchCodeCheck {
        display_ref: String,
        base_branch: String,
        primary_reference: Option<String>,
        primary_oid: Option<String>,
        outcome: Result<PrimaryBranchCodeMatch, String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InfoModalFocus {
    Info,
    Actions,
}

struct InfoField {
    label: &'static str,
    value: String,
}

/// On-screen rectangle of one info value, recorded each frame so a left-click
/// can be mapped back to the field it landed on (for click-to-copy).
#[derive(Debug, Clone)]
pub struct InfoHitRegion {
    pub rect: Rect,
    pub label: String,
    pub value: String,
}

impl InfoModalRow {
    pub fn info_field_count(&self) -> usize {
        build_fields(self).len()
    }

    pub fn info_field(&self, index: usize) -> Option<(String, String)> {
        build_fields(self)
            .get(index)
            .map(|field| (field.label.to_string(), field.value.clone()))
    }
}

/// Where a field's lines landed within the built line list, before any
/// scroll offset or pane origin is applied.
struct FieldSpan {
    label: String,
    value: String,
    start_line: u16,
    line_count: u16,
}

#[allow(clippy::too_many_arguments)]
pub fn draw_info_modal(
    frame: &mut Frame,
    row: &InfoModalRow,
    items: &[MenuItem],
    cursor: usize,
    focus: InfoModalFocus,
    info_cursor: usize,
    scroll_offset: &mut u16,
    copied_msg: Option<&str>,
    hit_regions: &mut Vec<InfoHitRegion>,
    theme: &Theme,
    symbols: &SymbolSet,
) {
    // Recorded fresh every frame; the click handler reads the latest set.
    hit_regions.clear();

    let fields = build_fields(row);
    let title = get_title(row);
    let code_check = matches!(row, InfoModalRow::PrimaryBranchCodeCheck { .. });
    let footer = if code_check {
        ModalFooter::hints(&[("Esc", "Close")])
    } else {
        ModalFooter::hints(&[
            ("Tab", "Switch"),
            ("j/k", "Navigate"),
            ("Enter", "Invoke"),
            ("Esc", "Close"),
        ])
    };
    let areas = draw_modal_shell(frame, &ModalSpec::new(title, footer, 84, 22), theme);

    let selected_field = (focus == InfoModalFocus::Info).then_some(info_cursor);
    let (mut lines, field_spans) = build_info_lines(
        &fields,
        theme,
        areas.body.width.saturating_sub(1) as usize,
        selected_field,
    );
    if let Some(msg) = copied_msg {
        lines.push(Line::from(Span::styled(
            msg.to_string(),
            theme.modal_success,
        )));
    }
    let actions_start = if code_check {
        lines.len() as u16
    } else {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("Actions", theme.modal_title)));
        lines.len() as u16
    };
    if !code_check {
        for (index, item) in items.iter().enumerate() {
            let selected = focus == InfoModalFocus::Actions && index == cursor && item.enabled;
            let mut line = ModalActionRow::new(
                item.shortcut,
                item.label.clone(),
                item.reason.clone().unwrap_or_default(),
            )
            .render_with_availability(selected, item.enabled, theme);
            line.spans.insert(
                0,
                Span::styled(
                    if selected {
                        format!("{} ", symbols.cursor_prefix)
                    } else {
                        "  ".to_string()
                    },
                    if item.enabled {
                        Style::default()
                    } else {
                        theme.modal_action_unavailable
                    },
                ),
            );
            lines.push(line);
        }
    }

    let target = match focus {
        InfoModalFocus::Info => field_spans
            .get(info_cursor)
            .map(|span| span.start_line)
            .unwrap_or_default(),
        InfoModalFocus::Actions => actions_start.saturating_add(cursor as u16),
    };
    let mut scroll = ModalScroll::default();
    scroll.offset = *scroll_offset;
    scroll.ensure_visible(target, lines.len() as u16, areas.body.height);
    *scroll_offset = scroll.offset;
    frame.render_widget(Paragraph::new(lines).scroll((scroll.offset, 0)), areas.body);

    for span in field_spans {
        let visible_start = span.start_line.max(scroll.offset);
        let visible_end = (span.start_line + span.line_count)
            .min(scroll.offset.saturating_add(areas.body.height));
        if visible_end > visible_start {
            hit_regions.push(InfoHitRegion {
                rect: Rect {
                    x: areas.body.x,
                    y: areas.body.y + visible_start - scroll.offset,
                    width: areas.body.width,
                    height: visible_end - visible_start,
                },
                label: span.label,
                value: span.value,
            });
        }
    }
}

fn get_title(row: &InfoModalRow) -> String {
    match row {
        InfoModalRow::GraphCommit(commit) => {
            let short_oid: String = commit.oid.chars().take(7).collect();
            format!("{short_oid} {}", commit.summary)
        }
        InfoModalRow::Branch(b) => b.name.clone(),
        InfoModalRow::Remote(r) => r.short_name.clone(),
        InfoModalRow::Tag(t) => t.name.clone(),
        InfoModalRow::Worktree(w) => w.path.to_string_lossy().to_string(),
        InfoModalRow::PrimaryBranchCodeCheck { display_ref, .. } => {
            format!("Code in primary branch: {display_ref}")
        }
    }
}

fn build_fields(row: &InfoModalRow) -> Vec<InfoField> {
    match row {
        InfoModalRow::GraphCommit(commit) => graph_commit_fields(commit),
        InfoModalRow::Branch(b) => branch_fields(b),
        InfoModalRow::Remote(r) => remote_fields(r),
        InfoModalRow::Tag(t) => tag_fields(t),
        InfoModalRow::Worktree(w) => worktree_fields(w),
        InfoModalRow::PrimaryBranchCodeCheck {
            display_ref,
            base_branch,
            primary_reference,
            primary_oid,
            outcome,
        } => {
            let evidence_ref = primary_reference.as_deref().unwrap_or(base_branch);
            let mut fields = Vec::new();
            if let (Some(reference), Some(oid)) = (primary_reference, primary_oid) {
                fields.push(InfoField {
                    label: "Primary",
                    value: format!("{reference} @ {oid}"),
                });
            }
            let value = match outcome {
                Ok(PrimaryBranchCodeMatch::Merged) => format!(
                    "The selected commit from {display_ref} is reachable in {evidence_ref}'s history. This is historical evidence and does not establish current-tip retention."
                ),
                Ok(PrimaryBranchCodeMatch::ContentEquivalent { commit_oid }) => format!(
                    "All changes from {display_ref}, relative to the shared ancestor, are represented at {} in {evidence_ref}'s history. The selected ref itself need not be merged; this historical evidence does not establish current-tip retention.",
                    short_oid(commit_oid)
                ),
                Ok(PrimaryBranchCodeMatch::NotFound) => format!(
                    "No available snapshot in {evidence_ref}'s history contained the complete change from {display_ref}. Rewritten or split changes may not be recognized; this does not prove absence."
                ),
                Err(error) => format!("Could not check {display_ref} in {base_branch}: {error}"),
            };
            fields.push(InfoField {
                label: "Result",
                value,
            });
            fields
        }
    }
}

fn short_oid(oid: &str) -> &str {
    oid.get(..7).unwrap_or(oid)
}

fn graph_commit_fields(commit: &GraphCommit) -> Vec<InfoField> {
    let mut fields = vec![
        InfoField {
            label: "Commit",
            value: commit.oid.clone(),
        },
        InfoField {
            label: "Summary",
            value: commit.summary.clone(),
        },
    ];

    if !commit.parents.is_empty() {
        fields.push(InfoField {
            label: "Parents",
            value: commit.parents.join(", "),
        });
    }

    // NEW: Author row — single combined "Name <email>" format
    let has_author_name = !commit.author_name.is_empty();
    let has_author_email = !commit.author_email.is_empty();
    if has_author_name || has_author_email {
        let author_value = match (has_author_name, has_author_email) {
            (true, true) => format!("{} <{}>", commit.author_name, commit.author_email),
            (true, false) => commit.author_name.clone(),
            (false, true) => commit.author_email.clone(),
            (false, false) => unreachable!(),
        };
        fields.push(InfoField {
            label: "Author",
            value: author_value,
        });
    }

    // Date row — absolute local time + relative age, when the loader provided a date.
    if let Some(authored_at) = commit.authored_at.as_ref() {
        let local = crate::types::format_local_absolute(authored_at);
        let age = crate::types::format_age(authored_at);
        fields.push(InfoField {
            label: "Date",
            value: format!("{local} ({age})"),
        });
    }

    if let Some(branch) = &commit.branch {
        fields.push(InfoField {
            label: "Branch",
            value: branch.name.clone(),
        });
    }

    for (label, kind) in [
        ("Local Refs", GraphRefKind::LocalBranch),
        ("Remote Refs", GraphRefKind::RemoteBranch),
        ("Tags", GraphRefKind::Tag),
    ] {
        let names = commit
            .refs
            .iter()
            .filter(|reference| reference.kind == kind)
            .map(|reference| reference.name.as_str())
            .collect::<Vec<_>>();
        if !names.is_empty() {
            fields.push(InfoField {
                label,
                value: names.join(", "),
            });
        }
    }

    if commit.is_possible_squash_merge {
        fields.push(InfoField {
            label: "Possible Squash Merge",
            value: "yes".into(),
        });
        if !commit.possible_squash_merge_sources.is_empty() {
            fields.push(InfoField {
                label: "Possible Squash Merge From",
                value: commit.possible_squash_merge_sources.join(", "),
            });
        }
    } else if !commit.is_cherry_picked_commit {
        if let Some(fuzzy) = commit.fuzzy_squash_match.as_ref() {
            fields.push(InfoField {
                label: "Possible Squash Merge (fuzzy)",
                value: format!("{}% similarity", fuzzy.similarity_percent),
            });
        }
    }

    fields
}

fn branch_fields(b: &BranchInfo) -> Vec<InfoField> {
    let mut fields = vec![
        InfoField {
            label: "Name",
            value: b.name.clone(),
        },
        InfoField {
            label: "Current",
            value: if b.is_current { "yes" } else { "no" }.to_string(),
        },
        InfoField {
            label: "Base",
            value: if b.is_base { "yes" } else { "no" }.to_string(),
        },
    ];

    match &b.tracking {
        TrackingStatus::Tracked { remote_ref, gone } => {
            fields.push(InfoField {
                label: "Remote",
                value: remote_ref.clone(),
            });
            if *gone {
                fields.push(InfoField {
                    label: "Tracking",
                    value: "gone".to_string(),
                });
            }
        }
        TrackingStatus::Local => {
            fields.push(InfoField {
                label: "Tracking",
                value: "local".to_string(),
            });
        }
    }

    if let Some(ahead) = b.ahead {
        fields.push(InfoField {
            label: "Ahead",
            value: ahead.to_string(),
        });
    }

    if let Some(behind) = b.behind {
        fields.push(InfoField {
            label: "Behind",
            value: behind.to_string(),
        });
    }

    fields.push(InfoField {
        label: "Merge Status",
        value: merge_status_str(&b.merge_status).to_string(),
    });

    if let Some(confidence) = &b.squash_confidence {
        fields.push(InfoField {
            label: "Confidence",
            value: squash_confidence_str(confidence),
        });
    }

    fields.push(InfoField {
        label: "Base Branch",
        value: b.base_branch.clone(),
    });

    if let Some(hash) = &b.merge_base_commit {
        fields.push(InfoField {
            label: "Merge Base",
            value: hash.clone(),
        });
    }

    fields.push(InfoField {
        label: "Last Commit",
        value: b.age_display(),
    });

    if let Some(pr) = &b.pr {
        fields.push(InfoField {
            label: "PR",
            value: format!("#{} ({})", pr.number, pr_status_str(&pr.status)),
        });
    }

    fields
}

fn remote_fields(r: &RemoteBranchInfo) -> Vec<InfoField> {
    let mut fields = vec![
        InfoField {
            label: "Full Ref",
            value: r.full_ref.clone(),
        },
        InfoField {
            label: "Remote",
            value: r.remote.clone(),
        },
        InfoField {
            label: "Short Name",
            value: r.short_name.clone(),
        },
        InfoField {
            label: "Has Local",
            value: if r.has_local { "yes" } else { "no" }.to_string(),
        },
        InfoField {
            label: "Base",
            value: if r.is_base { "yes" } else { "no" }.to_string(),
        },
    ];

    if let Some(ahead) = r.ahead {
        fields.push(InfoField {
            label: "Ahead",
            value: ahead.to_string(),
        });
    }

    if let Some(behind) = r.behind {
        fields.push(InfoField {
            label: "Behind",
            value: behind.to_string(),
        });
    }

    fields.push(InfoField {
        label: "Merge Status",
        value: merge_status_str(&r.merge_status).to_string(),
    });

    if let Some(confidence) = &r.squash_confidence {
        fields.push(InfoField {
            label: "Confidence",
            value: squash_confidence_str(confidence),
        });
    }

    fields.push(InfoField {
        label: "Last Commit",
        value: r.age_display(),
    });

    if let Some(pr) = &r.pr {
        fields.push(InfoField {
            label: "PR",
            value: format!("#{} ({})", pr.number, pr_status_str(&pr.status)),
        });
    }

    fields
}

fn tag_fields(t: &TagInfo) -> Vec<InfoField> {
    let mut fields = vec![
        InfoField {
            label: "Name",
            value: t.name.clone(),
        },
        InfoField {
            label: "Commit Hash",
            value: t.commit_hash.clone(),
        },
        InfoField {
            label: "Date",
            value: t.age_display(),
        },
        InfoField {
            label: "Annotated",
            value: if t.is_annotated { "yes" } else { "no" }.to_string(),
        },
    ];

    if let Some(msg) = &t.message {
        fields.push(InfoField {
            label: "Message",
            value: msg.clone(),
        });
    }

    fields
}

fn worktree_fields(w: &WorktreeInfo) -> Vec<InfoField> {
    let mut fields = vec![
        InfoField {
            label: "Path",
            value: w.path.to_string_lossy().to_string(),
        },
        InfoField {
            label: "Main",
            value: if w.is_main { "yes" } else { "no" }.to_string(),
        },
        InfoField {
            label: "Commit Hash",
            value: w.commit_hash.clone(),
        },
    ];

    if let Some(branch) = &w.branch {
        fields.push(InfoField {
            label: "Branch",
            value: branch.clone(),
        });
    }

    fields.push(InfoField {
        label: "Status",
        value: w.wt_status.summary(),
    });

    if let Some(ahead) = w.ahead {
        fields.push(InfoField {
            label: "Ahead",
            value: ahead.to_string(),
        });
    }

    if let Some(behind) = w.behind {
        fields.push(InfoField {
            label: "Behind",
            value: behind.to_string(),
        });
    }

    fields.push(InfoField {
        label: "Merge Status",
        value: merge_status_str(&w.merge_status).to_string(),
    });

    fields.push(InfoField {
        label: "Last Commit",
        value: w.age_display(),
    });

    if let Some(pr_status) = &w.pr {
        fields.push(InfoField {
            label: "PR",
            value: pr_status_str(pr_status).to_string(),
        });
    }

    for kind in [
        ChangedFileKind::Staged,
        ChangedFileKind::Modified,
        ChangedFileKind::Untracked,
    ] {
        let mut first_in_group = true;
        for file in w
            .wt_status
            .changed_files
            .iter()
            .filter(|file| file.kind == kind)
        {
            fields.push(InfoField {
                label: if first_in_group { kind.label() } else { "" },
                value: format!("{} ({})", file.path, file.kind.label()),
            });
            first_in_group = false;
        }
    }

    fields
}

fn merge_status_str(status: &MergeStatus) -> &'static str {
    match status {
        MergeStatus::Merged => "Merged",
        MergeStatus::InSync => "In Sync",
        MergeStatus::SquashMerged => "Squash Merged",
        MergeStatus::LocalMerged => "Local Merged",
        MergeStatus::RemoteMerged => "Remote Merged",
        MergeStatus::LocalSquashMerged => "Local Squash Merged",
        MergeStatus::RemoteSquashMerged => "Remote Squash Merged",
        MergeStatus::LikelySquashMerged => "Possible Squash Merge",
        MergeStatus::CherryPicked => "Cherry Picked",
        MergeStatus::LocalCherryPicked => "Local Cherry Picked",
        MergeStatus::RemoteCherryPicked => "Remote Cherry Picked",
        MergeStatus::Unmerged => "Unmerged",
        MergeStatus::Pending => "Pending",
    }
}

fn squash_confidence_str(confidence: &SquashConfidence) -> String {
    match confidence {
        SquashConfidence::MergeTreeConfirmed => "Merge-tree confirmed".to_string(),
        SquashConfidence::FuzzyMatch { similarity_percent } => {
            format!("Fuzzy match ({similarity_percent}%)")
        }
    }
}

fn pr_status_str(status: &PrStatus) -> &'static str {
    match status {
        PrStatus::Draft => "Draft",
        PrStatus::Open => "Open",
        PrStatus::Merged => "Merged",
        PrStatus::Closed => "Closed",
    }
}

/// Width of the label column: 15 chars of label + 1 trailing space.
const INFO_LABEL_WIDTH: usize = 16;

/// Wrap `value` to `width` columns. Words are kept whole when they fit; words
/// longer than `width` (e.g. file paths, which contain no spaces) are
/// hard-broken across lines. Always returns at least one (possibly empty) line.
fn wrap_value(value: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_len = 0usize;

    for word in value.split(' ') {
        let word_len = word.chars().count();

        // Hard-break a word that can't fit on a line by itself.
        if word_len > width {
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
                current_len = 0;
            }
            let chars: Vec<char> = word.chars().collect();
            let mut start = 0;
            while start < chars.len() {
                let end = (start + width).min(chars.len());
                lines.push(chars[start..end].iter().collect());
                start = end;
            }
            continue;
        }

        let needed = if current.is_empty() {
            word_len
        } else {
            word_len + 1
        };
        if current_len + needed > width && !current.is_empty() {
            lines.push(std::mem::take(&mut current));
            current_len = 0;
        }
        if !current.is_empty() {
            current.push(' ');
            current_len += 1;
        }
        current.push_str(word);
        current_len += word_len;
    }

    if !current.is_empty() || lines.is_empty() {
        lines.push(current);
    }
    lines
}

/// Build the info section as `label: value` lines, wrapping long values to
/// `width` with continuation lines indented under the value column.
fn build_info_lines(
    fields: &[InfoField],
    theme: &Theme,
    width: usize,
    selected_field: Option<usize>,
) -> (Vec<Line<'static>>, Vec<FieldSpan>) {
    let value_width = width.saturating_sub(INFO_LABEL_WIDTH).max(1);
    let mut lines = Vec::new();
    let mut spans = Vec::new();
    for f in fields {
        let start_line = lines.len() as u16;
        let chunks = wrap_value(&f.value, value_width);
        let line_count = chunks.len() as u16;
        for (i, chunk) in chunks.into_iter().enumerate() {
            let selected = selected_field == Some(spans.len());
            if i == 0 {
                let line = Line::from(vec![
                    Span::styled(format!("{:<15} ", f.label), theme.modal_secondary),
                    Span::styled(chunk, info_value_style(f.label, theme)),
                ]);
                lines.push(if selected {
                    line.style(theme.modal_action_selected)
                } else {
                    line
                });
            } else {
                let line = Line::from(vec![
                    Span::raw(" ".repeat(INFO_LABEL_WIDTH)),
                    Span::styled(chunk, info_value_style(f.label, theme)),
                ]);
                lines.push(if selected {
                    line.style(theme.modal_action_selected)
                } else {
                    line
                });
            }
        }
        spans.push(FieldSpan {
            label: f.label.to_string(),
            value: f.value.clone(),
            start_line,
            line_count,
        });
    }
    (lines, spans)
}

fn info_value_style(label: &str, theme: &Theme) -> Style {
    match label {
        "Commit" | "Merge Base" | "Parents" => theme.modal_commit,
        "Branch" | "Base Branch" | "Local Refs" | "Remote Refs" | "Tags" | "Remote" => {
            theme.modal_branch
        }
        "Worktree" | "Path" => theme.modal_worktree,
        _ => theme.modal_command,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone, Utc};
    use ratatui::{backend::TestBackend, Terminal};
    use std::path::PathBuf;

    fn test_worktree(changed_files: Vec<ChangedFile>) -> WorktreeInfo {
        WorktreeInfo {
            path: PathBuf::from("/tmp/worktree"),
            branch: Some("feature/test".into()),
            is_main: false,
            is_base: false,
            commit_hash: "abc123".into(),
            wt_status: WorkingTreeStatus {
                has_staged: changed_files
                    .iter()
                    .any(|file| file.kind == ChangedFileKind::Staged),
                has_modified: changed_files
                    .iter()
                    .any(|file| file.kind == ChangedFileKind::Modified),
                has_untracked: changed_files
                    .iter()
                    .any(|file| file.kind == ChangedFileKind::Untracked),
                changed_files,
            },
            age_date: Utc::now(),
            merge_status: MergeStatus::Unmerged,
            ahead: None,
            behind: None,
            pr: None,
        }
    }

    fn changed_file(path: &str, kind: ChangedFileKind) -> ChangedFile {
        ChangedFile {
            path: path.into(),
            kind,
        }
    }

    #[test]
    fn primary_code_results_show_historical_evidence_and_primary_snapshot() {
        let primary_reference = "refs/remotes/origin/main".to_string();
        let primary_oid = "0123456789abcdef".to_string();
        let outcomes = vec![
            (
                Ok(PrimaryBranchCodeMatch::Merged),
                &[
                    "historical evidence",
                    "does not establish current-tip retention",
                ][..],
            ),
            (
                Ok(PrimaryBranchCodeMatch::ContentEquivalent {
                    commit_oid: "abcdef0123456789".into(),
                }),
                &[
                    "abcdef0",
                    "relative to the shared ancestor",
                    "need not be merged",
                    "current-tip retention",
                ][..],
            ),
            (
                Ok(PrimaryBranchCodeMatch::NotFound),
                &["does not prove absence", "Rewritten or split changes"][..],
            ),
            (
                Err("shared history could not be determined".into()),
                &["Could not check", "shared history could not be determined"][..],
            ),
        ];

        for (outcome, expected_fragments) in outcomes {
            let row = InfoModalRow::PrimaryBranchCodeCheck {
                display_ref: "feature/code".into(),
                base_branch: "main".into(),
                primary_reference: Some(primary_reference.clone()),
                primary_oid: Some(primary_oid.clone()),
                outcome,
            };
            let fields = build_fields(&row);
            assert_eq!(fields[0].label, "Primary");
            assert_eq!(
                fields[0].value,
                "refs/remotes/origin/main @ 0123456789abcdef"
            );
            assert_eq!(fields[1].label, "Result");
            for fragment in expected_fragments {
                assert!(fields[1].value.contains(fragment), "{}", fields[1].value);
            }
        }
    }

    #[test]
    fn info_field_accessors_match_built_fields() {
        let row = InfoModalRow::Tag(TagInfo {
            name: "v1.2".into(),
            commit_hash: "abc123".into(),
            date: Utc::now(),
            message: Some("release".into()),
            is_annotated: true,
        });
        let fields = build_fields(&row);

        assert_eq!(row.info_field_count(), fields.len());
        for (index, field) in fields.iter().enumerate() {
            assert_eq!(
                row.info_field(index),
                Some((field.label.into(), field.value.clone()))
            );
        }
        assert_eq!(row.info_field(fields.len()), None);
    }

    #[test]
    fn worktree_changed_files_are_grouped_by_kind() {
        let worktree = test_worktree(vec![
            changed_file("modified-a", ChangedFileKind::Modified),
            changed_file("staged-a", ChangedFileKind::Staged),
            changed_file("untracked-a", ChangedFileKind::Untracked),
            changed_file("staged-b", ChangedFileKind::Staged),
            changed_file("modified-b", ChangedFileKind::Modified),
        ]);

        let fields = worktree_fields(&worktree);
        let changed_fields: Vec<(&str, &str)> = fields
            .iter()
            .filter(|field| ["staged", "modified", "untracked", ""].contains(&field.label))
            .map(|field| (field.label, field.value.as_str()))
            .collect();

        assert_eq!(
            changed_fields,
            vec![
                ("staged", "staged-a (staged)"),
                ("", "staged-b (staged)"),
                ("modified", "modified-a (modified)"),
                ("", "modified-b (modified)"),
                ("untracked", "untracked-a (untracked)"),
            ]
        );
    }

    #[test]
    fn worktree_changed_file_groups_omit_empty_kinds_and_clean_section() {
        let modified_only =
            test_worktree(vec![changed_file("modified-a", ChangedFileKind::Modified)]);
        let fields = worktree_fields(&modified_only);
        let changed_fields: Vec<(&str, &str)> = fields
            .iter()
            .filter(|field| ["staged", "modified", "untracked"].contains(&field.label))
            .map(|field| (field.label, field.value.as_str()))
            .collect();
        assert_eq!(changed_fields, vec![("modified", "modified-a (modified)")]);

        let clean_labels: Vec<&str> = worktree_fields(&test_worktree(Vec::new()))
            .iter()
            .map(|field| field.label)
            .collect();
        assert!(!clean_labels.iter().any(|label| [
            "Changed Files",
            "Staged",
            "Modified",
            "Untracked"
        ]
        .contains(label)));
    }

    #[test]
    fn worktree_changed_file_rendering_scrolls_and_keeps_copy_regions() {
        let changed_files: Vec<ChangedFile> = (0..24)
            .map(|index| changed_file(&format!("file-{index}"), ChangedFileKind::Modified))
            .collect();
        let row = InfoModalRow::Worktree(test_worktree(changed_files));
        let items = Vec::new();
        let mut scroll_offset = 0;
        let mut hit_regions = Vec::new();
        let mut terminal = Terminal::new(TestBackend::new(84, 12)).unwrap();
        let final_index = row.info_field_count() - 1;

        terminal
            .draw(|frame| {
                draw_info_modal(
                    frame,
                    &row,
                    &items,
                    0,
                    InfoModalFocus::Info,
                    final_index,
                    &mut scroll_offset,
                    None,
                    &mut hit_regions,
                    &Theme::dark(),
                    &SymbolSet::ascii(),
                )
            })
            .unwrap();

        assert!(scroll_offset > 0);
        assert!(hit_regions
            .iter()
            .any(|region| region.value == "file-23 (modified)"));

        let grouped_row = InfoModalRow::Worktree(test_worktree(vec![
            changed_file("staged", ChangedFileKind::Staged),
            changed_file("modified", ChangedFileKind::Modified),
            changed_file("untracked", ChangedFileKind::Untracked),
        ]));
        let mut grouped_regions = Vec::new();
        let mut grouped_scroll = 0;
        let mut roomy_terminal = Terminal::new(TestBackend::new(84, 30)).unwrap();
        roomy_terminal
            .draw(|frame| {
                draw_info_modal(
                    frame,
                    &grouped_row,
                    &items,
                    0,
                    InfoModalFocus::Info,
                    0,
                    &mut grouped_scroll,
                    None,
                    &mut grouped_regions,
                    &Theme::dark(),
                    &SymbolSet::ascii(),
                )
            })
            .unwrap();

        for value in [
            "staged (staged)",
            "modified (modified)",
            "untracked (untracked)",
        ] {
            assert!(grouped_regions.iter().any(|region| region.value == value));
        }
    }

    #[test]
    fn graph_commit_fields_includes_author_and_date_rows() {
        let commit = GraphCommit {
            oid: "abcdef1234567".into(),
            summary: "test".into(),
            parents: vec![],
            lane: None,
            branch: None,
            refs: vec![],
            is_possible_squash_merge: false,
            possible_squash_merge_sources: vec![],
            fuzzy_squash_match: None,
            is_cherry_picked_commit: false,
            author_name: "Jane Doe".into(),
            author_email: "jane@example.com".into(),
            authored_at: Some(Utc::now() - Duration::hours(3)),
        };
        let fields = graph_commit_fields(&commit);
        let labels: Vec<&str> = fields.iter().map(|f| f.label).collect();
        assert!(
            labels.contains(&"Author"),
            "expected Author field, got {labels:?}"
        );
        assert!(
            labels.contains(&"Date"),
            "expected Date field, got {labels:?}"
        );
        let author = fields.iter().find(|f| f.label == "Author").unwrap();
        assert_eq!(author.value, "Jane Doe <jane@example.com>");
        let date = fields.iter().find(|f| f.label == "Date").unwrap();
        assert!(
            date.value.contains("ago"),
            "expected relative age, got: {}",
            date.value
        );
    }

    #[test]
    fn graph_commit_fields_lists_all_possible_squash_sources() {
        let commit = GraphCommit {
            oid: "abcdef1234567".into(),
            summary: "squash landing".into(),
            parents: vec![],
            lane: None,
            branch: None,
            refs: vec![],
            is_possible_squash_merge: true,
            possible_squash_merge_sources: vec!["feature/auth".into(), "feature/login".into()],
            fuzzy_squash_match: None,
            is_cherry_picked_commit: false,
            author_name: String::new(),
            author_email: String::new(),
            authored_at: None,
        };

        let fields = graph_commit_fields(&commit);
        let sources = fields
            .iter()
            .find(|field| field.label == "Possible Squash Merge From")
            .expect("source field should be present for an exact squash match");
        assert_eq!(sources.value, "feature/auth, feature/login");
    }

    #[test]
    fn graph_commit_fields_shows_fuzzy_similarity_without_inventing_a_source_branch() {
        let commit = GraphCommit {
            oid: "abcdef1234567".into(),
            summary: "near squash landing".into(),
            fuzzy_squash_match: Some(crate::git::graph::FuzzySquashMatch {
                similarity_percent: 84,
            }),
            ..GraphCommit::default()
        };

        let fields = graph_commit_fields(&commit);
        let fuzzy = fields
            .iter()
            .find(|field| field.label == "Possible Squash Merge (fuzzy)")
            .expect("fuzzy similarity should be visible in commit info");
        assert_eq!(fuzzy.value, "84% similarity");
        assert!(!fields
            .iter()
            .any(|field| field.label == "Possible Squash Merge From"));
    }

    #[test]
    fn graph_commit_fields_keeps_exact_squash_and_cherry_classifications_ahead_of_fuzzy() {
        for (is_possible_squash_merge, is_cherry_picked_commit) in
            [(true, false), (false, true), (true, true)]
        {
            let commit = GraphCommit {
                is_possible_squash_merge,
                is_cherry_picked_commit,
                fuzzy_squash_match: Some(crate::git::graph::FuzzySquashMatch {
                    similarity_percent: 97,
                }),
                ..GraphCommit::default()
            };
            let fields = graph_commit_fields(&commit);
            assert!(!fields
                .iter()
                .any(|field| field.label == "Possible Squash Merge (fuzzy)"));
            if is_possible_squash_merge {
                assert!(fields
                    .iter()
                    .any(|field| field.label == "Possible Squash Merge"));
            }
        }
    }

    #[test]
    fn graph_commit_fields_omits_author_row_when_both_name_and_email_empty() {
        let commit = GraphCommit {
            oid: "x".into(),
            summary: "s".into(),
            parents: vec![],
            lane: None,
            branch: None,
            refs: vec![],
            is_possible_squash_merge: false,
            possible_squash_merge_sources: vec![],
            fuzzy_squash_match: None,
            is_cherry_picked_commit: false,
            author_name: "".into(),
            author_email: "".into(),
            authored_at: Some(Utc::now()),
        };
        let labels: Vec<&str> = graph_commit_fields(&commit)
            .iter()
            .map(|f| f.label)
            .collect();
        assert!(!labels.contains(&"Author"));
    }

    #[test]
    fn graph_commit_fields_shows_date_row_for_epoch_authored_at() {
        let commit = GraphCommit {
            oid: "x".into(),
            summary: "s".into(),
            parents: vec![],
            lane: None,
            branch: None,
            refs: vec![],
            is_possible_squash_merge: false,
            possible_squash_merge_sources: vec![],
            fuzzy_squash_match: None,
            is_cherry_picked_commit: false,
            author_name: "X".into(),
            author_email: "x@y".into(),
            authored_at: Some(Utc.timestamp_opt(0, 0).unwrap()),
        };
        let labels: Vec<&str> = graph_commit_fields(&commit)
            .iter()
            .map(|f| f.label)
            .collect();
        assert!(labels.contains(&"Date"));
    }

    #[test]
    fn graph_commit_fields_omits_date_row_when_authored_at_is_unknown() {
        let commit = GraphCommit {
            oid: "x".into(),
            summary: "s".into(),
            parents: vec![],
            lane: None,
            branch: None,
            refs: vec![],
            is_possible_squash_merge: false,
            possible_squash_merge_sources: vec![],
            fuzzy_squash_match: None,
            is_cherry_picked_commit: false,
            author_name: "X".into(),
            author_email: "x@y".into(),
            authored_at: None,
        };
        let labels: Vec<&str> = graph_commit_fields(&commit)
            .iter()
            .map(|f| f.label)
            .collect();
        assert!(!labels.contains(&"Date"));
    }

    #[test]
    fn branch_fields_includes_confidence_when_likely_squash_merged() {
        let b = BranchInfo {
            name: "feature/x".into(),
            is_current: false,
            is_base: false,
            tracking: TrackingStatus::Local,
            ahead: None,
            behind: None,
            last_commit_date: Utc::now(),
            merge_status: MergeStatus::LikelySquashMerged,
            base_branch: "main".into(),
            merge_base_commit: None,
            pr: None,
            squash_confidence: Some(SquashConfidence::FuzzyMatch {
                similarity_percent: 82,
            }),
        };
        let fields = branch_fields(&b);
        let confidence_field = fields
            .iter()
            .find(|f| f.label == "Confidence")
            .expect("Confidence field should be present");
        assert_eq!(confidence_field.value, "Fuzzy match (82%)");
    }

    #[test]
    fn branch_fields_omits_confidence_when_none() {
        let b = BranchInfo {
            name: "feature/y".into(),
            is_current: false,
            is_base: false,
            tracking: TrackingStatus::Local,
            ahead: None,
            behind: None,
            last_commit_date: Utc::now(),
            merge_status: MergeStatus::Unmerged,
            base_branch: "main".into(),
            merge_base_commit: None,
            pr: None,
            squash_confidence: None,
        };
        let fields = branch_fields(&b);
        assert!(!fields.iter().any(|f| f.label == "Confidence"));
    }

    #[test]
    fn remote_fields_includes_confidence_when_present() {
        let r = RemoteBranchInfo {
            full_ref: "origin/feature/x".into(),
            remote: "origin".into(),
            short_name: "feature/x".into(),
            has_local: false,
            is_base: false,
            last_commit_date: Utc::now(),
            merge_status: MergeStatus::LikelySquashMerged,
            ahead: None,
            behind: None,
            disjoint: false,
            pr: None,
            squash_confidence: Some(SquashConfidence::MergeTreeConfirmed),
        };
        let fields = remote_fields(&r);
        let confidence_field = fields
            .iter()
            .find(|f| f.label == "Confidence")
            .expect("Confidence field should be present");
        assert_eq!(confidence_field.value, "Merge-tree confirmed");
    }
}
