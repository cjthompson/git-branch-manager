use super::status_text::{branch_like_summary, format_branch_like};
use super::*;
use crate::config::Config;
use crate::git::graph;
use crate::types::*;
use crate::ui::commit_details::CommitDetailsFocus;
use crate::ui::modal::ModalScroll;
use crate::view::graph::GraphState;
use crate::view::list_state::ListState;
use chrono::Utc;

fn empty_renderer<T>(
    _: &T,
    _: usize,
    _: bool,
    _: bool,
    _: &[usize],
    _: &crate::ui::list_render::CellContext,
) -> Vec<Line<'static>> {
    Vec::new()
}

fn render_commit_details_overlay(commit: graph::GraphCommit) -> String {
    use crate::git::commit_details::{CommitDetailMode, CommitDetails};

    let oid = commit.oid.clone();
    let summary = commit.summary.clone();
    let overlay = Overlay::CommitDetails {
        commit,
        details: CommitDetails {
            oid: oid.clone(),
            summary,
            author_name: String::new(),
            author_email: String::new(),
            authored_at: None,
            message_lines: Vec::new(),
            mode: CommitDetailMode::Commit { oid },
            files: Vec::new(),
            branch_log: Vec::new(),
        },
        items: Vec::new(),
        cursor: 0,
        file_cursor: 0,
        focus: CommitDetailsFocus::Files,
        scroll: ModalScroll::default(),
    };
    render_frame(Some(overlay), &mut GraphState::new())
}

fn render_frame(mut overlay: Option<Overlay>, graph: &mut GraphState) -> String {
    use crate::job_queue::JobStatusView;
    use crate::symbols::SymbolSet;
    use crate::theme::Theme;
    use crate::ui::info_modal::InfoHitRegion;
    use ratatui::{backend::TestBackend, Terminal};

    let theme = Theme::dark();
    let symbols = SymbolSet::ascii();
    let config = Config::default();
    let mut info_hit_regions: Vec<InfoHitRegion> = Vec::new();
    let mut info_modal_scroll_offset = 0;
    let mut branches = ListState::<BranchInfo>::empty();
    let mut remotes = ListState::<RemoteBranchInfo>::empty();
    let mut tags = ListState::<TagInfo>::empty();
    let mut worktrees = ListState::<WorktreeInfo>::empty();
    let mut terminal = Terminal::new(TestBackend::new(120, 36)).unwrap();

    terminal
        .draw(|frame| {
            let mut context = RenderContext {
                active_view: ViewId::Graph,
                overlay: overlay.as_mut(),
                toast: None,
                theme: &theme,
                symbols: &symbols,
                config: &config,
                info_copied_msg: None,
                info_hit_regions: &mut info_hit_regions,
                info_modal_scroll_offset: &mut info_modal_scroll_offset,
                job_status: JobStatusView {
                    visible: false,
                    current_label: None,
                    current_progress: None,
                    queued_count: 0,
                    targets_done: 0,
                    targets_total: 0,
                    summary: None,
                },
                branches: &mut branches,
                remotes: &mut remotes,
                tags: &mut tags,
                worktrees: &mut worktrees,
                graph,
                base_branch: "main",
                branch_columns: &[],
                remote_columns: &[],
                tag_columns: &[],
                worktree_columns: &[],
                active_filter_tokens: &[],
                render_branch_row: empty_renderer::<BranchInfo>,
                render_remote_row: empty_renderer::<RemoteBranchInfo>,
                render_tag_row: empty_renderer::<TagInfo>,
                render_worktree_row: empty_renderer::<WorktreeInfo>,
            };
            draw(frame, &mut context);
        })
        .unwrap();

    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

fn branch(name: &str, status: MergeStatus) -> BranchInfo {
    BranchInfo {
        name: name.to_string(),
        is_current: false,
        is_base: false,
        tracking: TrackingStatus::Local,
        ahead: None,
        behind: None,
        last_commit_date: Utc::now(),
        merge_status: status,
        base_branch: "main".into(),
        merge_base_commit: None,
        pr: None,
        squash_confidence: None,
    }
}

#[test]
fn branch_like_summary_counts_statuses_and_selection() {
    let mut state = ListState::new(vec![
        branch("a", MergeStatus::Merged),
        branch("b", MergeStatus::SquashMerged),
        branch("c", MergeStatus::Unmerged),
        branch("d", MergeStatus::Merged),
        branch("e", MergeStatus::CherryPicked),
    ]);
    // Select the first two.
    state.selected_mut()[0] = true;
    state.selected_mut()[1] = true;

    let (total, selected, merged, squashed, cherry_picked) = branch_like_summary(&state);
    assert_eq!(total, 5);
    assert_eq!(selected, 2);
    assert_eq!(merged, 2);
    assert_eq!(squashed, 1);
    assert_eq!(cherry_picked, 1);
}

#[test]
fn branch_like_summary_empty() {
    let state: ListState<BranchInfo> = ListState::new(vec![]);
    assert_eq!(branch_like_summary(&state), (0, 0, 0, 0, 0));
}

#[test]
fn format_branch_like_preserves_shape() {
    let text = format_branch_like("branches", (4, 2, 2, 1, 1), "[/]search [q]uit");
    assert_eq!(
        text,
        " 4 branches | 2 selected | 2 merged | 1 squashed | 1 cherry-picked \u{2014} [/]search [q]uit"
    );
}

#[test]
fn commit_details_overlay_renders_fuzzy_similarity_from_the_graph_commit() {
    use crate::git::graph::GraphCommit;
    let rendered = render_commit_details_overlay(GraphCommit {
        relationships: vec![crate::git::graph::GraphRelationship {
            kind: crate::git::graph::RelationshipKind::SquashMerge,
            matching: crate::git::graph::RelationshipMatch::Fuzzy {
                similarity_percent: 84,
            },
            destination_oid: "abcdef1234567890".into(),
            destination_refs: vec!["main".into()],
            source_oid: "3333333333333333333333333333333333333333".into(),
            source_refs: Vec::new(),
        }],
        oid: "abcdef1234567890".into(),
        summary: "near squash landing".into(),
        ..GraphCommit::default()
    });
    assert!(
        rendered.contains("Possible squash merge (fuzzy): 84% similarity"),
        "commit details overlay should include fuzzy similarity metadata; got: {rendered}"
    );
}

#[test]
fn commit_details_overlay_hides_fuzzy_metadata_without_a_match() {
    let rendered = render_commit_details_overlay(graph::GraphCommit {
        oid: "abcdef1234567890".into(),
        summary: "ordinary commit".into(),
        ..graph::GraphCommit::default()
    });
    assert!(!rendered.contains("Possible squash merge (fuzzy)"));
}

#[test]
fn commit_details_overlay_keeps_exact_squash_and_cherry_classifications_ahead_of_fuzzy() {
    for (is_possible_squash_merge, is_cherry_picked_commit) in
        [(true, false), (false, true), (true, true)]
    {
        let rendered = render_commit_details_overlay(graph::GraphCommit {
            oid: "abcdef1234567890".into(),
            summary: "classified commit".into(),
            relationships: {
                let mut pairs = vec![
                    crate::git::graph::GraphRelationship {
                        kind: crate::git::graph::RelationshipKind::SquashMerge,
                        matching: crate::git::graph::RelationshipMatch::Exact,
                        destination_oid: "abcdef1234567890".into(),
                        destination_refs: vec!["main".into()],
                        source_oid: "1111111111111111111111111111111111111111".into(),
                        source_refs: Vec::new(),
                    },
                    crate::git::graph::GraphRelationship {
                        kind: crate::git::graph::RelationshipKind::SquashMerge,
                        matching: crate::git::graph::RelationshipMatch::Fuzzy {
                            similarity_percent: 97,
                        },
                        destination_oid: "abcdef1234567890".into(),
                        destination_refs: vec!["main".into()],
                        source_oid: "3333333333333333333333333333333333333333".into(),
                        source_refs: Vec::new(),
                    },
                ];
                if !is_possible_squash_merge {
                    pairs.retain(|r| r.matching != crate::git::graph::RelationshipMatch::Exact);
                }
                if is_cherry_picked_commit {
                    pairs.push(crate::git::graph::GraphRelationship {
                        kind: crate::git::graph::RelationshipKind::CherryPick,
                        matching: crate::git::graph::RelationshipMatch::Exact,
                        source_oid: "abcdef1234567890".into(),
                        source_refs: vec!["feature/picked".into()],
                        destination_oid: "4444444444444444444444444444444444444444".into(),
                        destination_refs: vec!["main".into()],
                    });
                }
                pairs
            },
            ..graph::GraphCommit::default()
        });
        assert!(!rendered.contains("Possible squash merge (fuzzy)"));
    }
}
#[test]
fn commit_details_overlay_shows_captured_cherry_pick_provenance() {
    let rendered = render_commit_details_overlay(graph::GraphCommit {
        oid: "ddddddd0000000000000000000000000000000000".into(),
        relationships: vec![graph::GraphRelationship {
            kind: graph::RelationshipKind::CherryPick,
            matching: graph::RelationshipMatch::Exact,
            destination_oid: "ddddddd0000000000000000000000000000000000".into(),
            destination_refs: vec!["main".into()],
            source_oid: "abcdef1234567890000000000000000000000000".into(),
            source_refs: vec!["feature/gone".into(), "feature/renamed".into()],
        }],
        ..Default::default()
    });
    assert!(rendered.contains("Cherry-picked From abcdef1 (feature/gone, feature/renamed)"));
}

#[test]
fn graph_status_line_shows_ahead_of_base_count() {
    use crate::git::graph::{
        GraphCommit, GraphLine, GraphRef, GraphRefCounts, GraphRefKind, GraphSnapshot, GraphSource,
    };
    let mut graph = GraphState::new();
    graph.apply_result(Ok(GraphSnapshot {
        source: GraphSource::Gleisbau,
        commits: vec![GraphCommit {
            oid: "1111111111111111111111111111111111111111".into(),
            summary: "feature work".into(),
            refs: vec![GraphRef {
                name: "feature".into(),
                kind: GraphRefKind::LocalBranch,
                has_linked_worktree: false,
                is_current: false,
                tracking: None,
                ahead_of_base: Some(1),
            }],
            ..GraphCommit::default()
        }],
        lines: vec![GraphLine {
            graph: "*".into(),
            commit_index: Some(0),
        }],
        ref_counts: GraphRefCounts { local: 1, remote: 0 },
        max_count: 500,
        includes_remotes: false,
        generation: None,
    }));
    let rendered = render_frame(None, &mut graph);
    assert!(
        rendered.contains("1 ahead of main"),
        "graph status line should include the ahead-of-base count; got: {rendered}"
    );
}
