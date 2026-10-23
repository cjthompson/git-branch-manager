use super::*;

pub(super) fn native_commit(
    repo: &git2::Repository,
    parent: Option<git2::Oid>,
    files: &[(&str, &str)],
    label: &str,
) -> git2::Oid {
    let mut builder = repo.treebuilder(None).unwrap();
    for (name, content) in files {
        let blob = repo.blob(content.as_bytes()).unwrap();
        builder.insert(name, blob, 0o100644).unwrap();
    }
    let tree_id = builder.write().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let sig = git2::Signature::now("test", "test@example.com").unwrap();
    let parents = parent
        .map(|id| repo.find_commit(id).unwrap())
        .into_iter()
        .collect::<Vec<_>>();
    let refs = parents.iter().collect::<Vec<_>>();
    repo.commit(None, &sig, &sig, label, &tree, &refs).unwrap()
}

pub(super) fn native_pair_fixture() -> (
    tempfile::TempDir,
    git2::Repository,
    git2::Oid,
    git2::Oid,
    GraphSnapshot,
    CacheRoot,
) {
    let dir = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(dir.path()).unwrap();
    let root = native_commit(&repo, None, &[], "root");
    let source = native_commit(&repo, Some(root), &[("picked", "picked\n")], "source");
    let base = native_commit(
        &repo,
        Some(root),
        &[("unrelated", "base\n")],
        "advance base",
    );
    let destination = native_commit(
        &repo,
        Some(base),
        &[("unrelated", "base\n"), ("picked", "picked\n")],
        "destination",
    );
    repo.reference("refs/heads/main", destination, true, "fixture")
        .unwrap();
    repo.reference("refs/heads/feature", source, true, "fixture")
        .unwrap();
    let reference = |name: &str| GraphRef {
        name: name.into(),
        kind: GraphRefKind::LocalBranch,
        has_linked_worktree: false,
        is_current: false,
        tracking: None,
        ahead_of_base: None,
    };
    // Deliberately omit the merge base and historical destination from the window.
    let snapshot = relationship_snapshot(vec![
        GraphCommit {
            oid: destination.to_string(),
            refs: vec![reference("main")],
            ..Default::default()
        },
        GraphCommit {
            oid: source.to_string(),
            refs: vec![reference("feature")],
            ..Default::default()
        },
    ]);
    let cache_root = CacheRoot::at(dir.path().join("isolated/cache"));
    (dir, repo, source, destination, snapshot, cache_root)
}

fn native_merge(
    repo: &git2::Repository,
    first: git2::Oid,
    other: git2::Oid,
    label: &str,
) -> git2::Oid {
    let first = repo.find_commit(first).unwrap();
    let other = repo.find_commit(other).unwrap();
    let tree = first.tree().unwrap();
    let sig = git2::Signature::now("test", "test@example.com").unwrap();
    repo.commit(None, &sig, &sig, label, &tree, &[&first, &other])
        .unwrap()
}

pub(super) fn merge_pair_fixture() -> (tempfile::TempDir, GraphSnapshot, CacheRoot) {
    let (dir, repo, source, destination, mut snapshot, root) = native_pair_fixture();
    let ancestor = repo.find_commit(source).unwrap().parent_id(0).unwrap();
    snapshot.commits[1].oid = native_merge(&repo, source, ancestor, "source merge").to_string();
    snapshot.commits[0].oid =
        native_merge(&repo, destination, ancestor, "destination merge").to_string();
    (dir, snapshot, root)
}
