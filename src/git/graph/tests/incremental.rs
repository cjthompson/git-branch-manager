use super::*;

#[test]
fn repository_state_uses_canonical_typed_refs_and_observes_head_changes() {
    let dir = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(dir.path()).unwrap();
    let mut index = repo.index().unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let sig = git2::Signature::now("test", "test@example.com").unwrap();
    let oid = repo.commit(Some("refs/heads/main"), &sig, &sig, "root", &tree, &[]).unwrap();
    repo.reference("refs/heads/feature/foo", oid, true, "test").unwrap();
    repo.reference("refs/remotes/origin/feature/foo", oid, true, "test").unwrap();
    repo.reference("refs/remotes/origin/origin/topic", oid, true, "test").unwrap();
    repo.reference("refs/remotes/upstream/topic", oid, true, "test").unwrap();
    repo.reference("refs/tags/feature/foo", oid, true, "test").unwrap();
    repo.set_head("refs/heads/main").unwrap();

    let before = GraphRepositoryState::capture(dir.path()).unwrap();
    assert!(before.refs.keys().any(|id| id.full_name == "refs/heads/feature/foo"));
    assert!(before.refs.keys().any(|id| id.full_name == "refs/remotes/origin/feature/foo"));
    assert!(before.refs.keys().any(|id| id.full_name == "refs/tags/feature/foo"));
    assert_eq!(before.head_ref.as_deref(), Some("refs/heads/main"));

    repo.set_head("refs/heads/feature/foo").unwrap();
    repo.find_reference("refs/remotes/origin/origin/topic").unwrap().delete().unwrap();
    let after = GraphRepositoryState::capture(dir.path()).unwrap();
    let delta = GraphRepositoryDelta::between(before, after);
    assert!(delta.head_changed);
    assert!(delta.added_refs.is_empty());
    assert!(delta.removed_refs.iter().any(|(id, _)| id.full_name == "refs/remotes/origin/origin/topic"));
    assert!(!delta.removed_refs.iter().any(|(id, _)| id.full_name == "refs/remotes/upstream/topic"));
}

#[test]
fn incremental_update_adds_new_tip_history_without_dropping_cached_commits() {
    let dir = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(dir.path()).unwrap();
    let tree_id = repo.index().unwrap().write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let sig = git2::Signature::now("test", "test@example.com").unwrap();
    let root = repo.commit(Some("refs/heads/main"), &sig, &sig, "root", &tree, &[]).unwrap();
    repo.set_head("refs/heads/main").unwrap();
    let options = GraphLoadOptions { max_count: 10, ..GraphLoadOptions::default() };
    let snapshot = load_graph(dir.path(), options.clone()).unwrap();
    let before = GraphRepositoryState::capture(dir.path()).unwrap();
    let parent = repo.find_commit(root).unwrap();
    let next = repo.commit(None, &sig, &sig, "feature", &tree, &[&parent]).unwrap();
    repo.reference("refs/heads/feature/new", next, true, "test").unwrap();
    let after = GraphRepositoryState::capture(dir.path()).unwrap();
    let updated = update_graph_incrementally(
        dir.path(), snapshot, &GraphRepositoryDelta::between(before, after), options,
    ).unwrap();
    assert!(updated.commits.iter().any(|commit| commit.oid == next.to_string()));
    assert!(updated.commits.iter().any(|commit| commit.oid == root.to_string()));
    assert!(updated.commits.iter().flat_map(|commit| &commit.refs).any(|reference| reference.name == "feature/new"));
}

#[test]
fn remote_inclusive_incremental_update_keeps_history_from_additional_roots() {
    let dir = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(dir.path()).unwrap();
    let tree_id = repo.index().unwrap().write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let sig = git2::Signature::now("test", "test@example.com").unwrap();
    let main = repo.commit(Some("refs/heads/main"), &sig, &sig, "main root", &tree, &[]).unwrap();
    let stash = main;
    repo.reference("refs/stash", stash, true, "test stash").unwrap();
    repo.set_head("refs/heads/main").unwrap();
    let options = GraphLoadOptions { max_count: 10, include_remotes: true, ..GraphLoadOptions::default() };
    let before = GraphRepositoryState::capture(dir.path()).unwrap();
    assert_eq!(before.additional_roots.get("refs/stash"), Some(&stash.to_string()));
    let snapshot = load_graph(dir.path(), options.clone()).unwrap();

    let parent = repo.find_commit(main).unwrap();
    let next = repo.commit(None, &sig, &sig, "new branch tip", &tree, &[&parent]).unwrap();
    repo.reference("refs/heads/feature/new", next, true, "test").unwrap();
    let after = GraphRepositoryState::capture(dir.path()).unwrap();
    let updated = update_graph_incrementally(
        dir.path(), snapshot, &GraphRepositoryDelta::between(before, after), options,
    ).unwrap();

    assert!(updated.commits.iter().any(|commit| commit.oid == stash.to_string()));
    assert!(updated.commits.iter().any(|commit| commit.oid == next.to_string()));
}

#[test]
fn local_tracking_metadata_uses_configured_upstream_not_matching_short_name() {
    let dir = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(dir.path()).unwrap();
    let tree_id = repo.index().unwrap().write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let sig = git2::Signature::now("test", "test@example.com").unwrap();
    let root = repo.commit(Some("refs/heads/main"), &sig, &sig, "root", &tree, &[]).unwrap();
    let parent = repo.find_commit(root).unwrap();
    let local = repo.commit(Some("refs/heads/topic"), &sig, &sig, "local", &tree, &[&parent]).unwrap();
    let origin = repo.commit(None, &sig, &sig, "origin", &tree, &[&parent]).unwrap();
    repo.reference("refs/remotes/origin/topic", origin, true, "test").unwrap();
    repo.reference("refs/remotes/upstream/topic", root, true, "test").unwrap();
    repo.remote("upstream", "https://example.invalid/upstream.git").unwrap();
    repo.find_branch("topic", git2::BranchType::Local).unwrap().set_upstream(Some("upstream/topic")).unwrap();
    repo.set_head("refs/heads/topic").unwrap();

    let refs = collect_ref_data(dir.path(), true, Some("main")).unwrap();
    let local_ref = refs.refs_by_oid.get(&local.to_string()).unwrap().iter()
        .find(|reference| reference.kind == GraphRefKind::LocalBranch).unwrap();
    let tracking = local_ref.tracking.as_ref().unwrap();
    assert_eq!((tracking.ahead, tracking.behind), (1, 0));
}
