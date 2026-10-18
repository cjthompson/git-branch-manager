mod fields;

use super::ref_fields::worktree_fields;
use super::*;
use chrono::Utc;
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
