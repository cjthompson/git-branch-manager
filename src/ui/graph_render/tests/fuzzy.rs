use super::*;

#[test]
fn graph_commit_row_shows_fuzzy_score_after_short_oid() {
    for score in [84, 97, 99] {
        let lines = render_commit_lines(fuzzy_commit(score), 100, 0);
        let row = lines
            .iter()
            .find(|line| line.contains("abcdef0"))
            .expect("commit row should be visible");

        assert!(
            row.contains("o abcdef0"),
            "ordinary DAG marker should remain: {row}"
        );
        assert!(
            row.contains(&format!("[fuzzy squash {score}%] fuzzy landing")),
            "fuzzy annotation should follow the short oid for {score}%: {row}"
        );
    }
}

#[test]
fn graph_commit_row_keeps_dag_glyph_and_fixed_ref_geometry_when_fuzzy_label_scrolls() {
    let initial = render_commit_lines(fuzzy_commit(84), 34, 0);
    let initial_row = initial
        .iter()
        .find(|line| line.contains("abcdef0"))
        .expect("commit row should be visible");
    assert!(initial_row.contains("o abcdef0"), "got: {initial_row}");
    assert!(!initial_row.contains("[fuzzy squash 84%]"));

    let scrolled = render_commit_lines(fuzzy_commit(84), 34, 8);
    let row = scrolled
        .iter()
        .find(|line| line.contains("[fuzzy squash 84%]"))
        .expect("horizontal scrolling should reveal the clipped fuzzy label");
    let header = scrolled
        .iter()
        .find(|line| line.contains("LRT|State"))
        .expect("narrow graph should keep the compact LRT/State header");
    assert!(
        row.starts_with('│'),
        "row should retain its outer border: {row}"
    );
    assert!(
        row.contains('o'),
        "fuzzy status must keep the ordinary DAG glyph: {row}"
    );
    assert_eq!(
        row.find('|'),
        header.find("LRT|State").map(|label_start| label_start - 1),
        "fuzzy annotation must not shift the fixed LRT/State columns"
    );
}

#[test]
fn graph_commit_row_omits_fuzzy_label_without_a_match() {
    let lines = render_commit_lines(
        GraphCommit {
            oid: "abcdef0123456789".into(),
            summary: "ordinary commit".into(),
            ..GraphCommit::default()
        },
        100,
        0,
    );
    let row = lines
        .iter()
        .find(|line| line.contains("abcdef0"))
        .expect("commit row should be visible");
    assert!(row.contains("ordinary commit"), "got: {row}");
    assert!(!row.contains("[fuzzy squash"), "got: {row}");
}

#[test]
fn graph_commit_row_gives_exact_squash_and_cherry_labels_precedence_over_fuzzy() {
    for (is_possible_squash_merge, is_cherry_picked_commit) in
        [(true, false), (false, true), (true, true)]
    {
        let mut commit = fuzzy_commit(97);
        if is_possible_squash_merge {
            commit
                .relationships
                .push(crate::git::graph::GraphRelationship {
                    kind: crate::git::graph::RelationshipKind::SquashMerge,
                    matching: crate::git::graph::RelationshipMatch::Exact,
                    destination_oid: commit.oid.clone(),
                    destination_refs: vec!["main".into()],
                    source_oid: "1111111111111111111111111111111111111111".into(),
                    source_refs: vec!["feature/exact".into()],
                });
        }
        if is_cherry_picked_commit {
            commit
                .relationships
                .push(crate::git::graph::GraphRelationship {
                    kind: crate::git::graph::RelationshipKind::CherryPick,
                    matching: crate::git::graph::RelationshipMatch::Exact,
                    source_oid: commit.oid.clone(),
                    source_refs: vec!["feature/picked".into()],
                    destination_oid: "4444444444444444444444444444444444444444".into(),
                    destination_refs: vec!["main".into()],
                });
        }
        let lines = render_commit_lines(commit, 100, 0);
        let row = lines
            .iter()
            .find(|line| line.contains("abcdef0"))
            .expect("commit row should be visible");
        assert!(!row.contains("[fuzzy squash"), "got: {row}");
    }
}
