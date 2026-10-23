use super::*;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

/// Builds `feature` off `root`, lands it on `main` (squash or cherry-pick), then
/// leaves the work only as `refs/remotes/origin/feature`. Returns the feature tip.
fn remote_only_fixture(squash: bool) -> (tempfile::TempDir, CacheRoot, String) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    git(path, &["init", "-b", "main"]);
    git(path, &["config", "user.name", "Test"]);
    git(path, &["config", "user.email", "test@example.com"]);
    git(path, &["commit", "--allow-empty", "-m", "root"]);
    git(path, &["checkout", "-b", "feature"]);
    let feature_files = if squash { 3 } else { 1 };
    for index in 0..feature_files {
        let name = format!("feature-{index}.txt");
        std::fs::write(path.join(&name), format!("content {index}\n")).unwrap();
        git(path, &["add", &name]);
        git(path, &["commit", "-m", &name]);
    }
    git(path, &["checkout", "main"]);
    std::fs::write(path.join("unrelated.txt"), "unrelated\n").unwrap();
    git(path, &["add", "unrelated.txt"]);
    git(path, &["commit", "-m", "unrelated"]);
    if squash {
        git(path, &["merge", "--squash", "feature"]);
        git(path, &["commit", "-m", "squash landing"]);
    } else {
        git(path, &["cherry-pick", "feature"]);
    }
    git(path, &["update-ref", "refs/remotes/origin/feature", "feature"]);
    let feature_tip = git(path, &["rev-parse", "feature"]);
    git(path, &["branch", "-D", "feature"]);
    let cache_root = CacheRoot::at(path.join("cache"));
    (dir, cache_root, feature_tip)
}

fn load(path: &Path, cache_root: &CacheRoot, include_remotes: bool) -> GraphSnapshot {
    load_graph(
        path,
        GraphLoadOptions {
            base_branch: Some("main".into()),
            include_remotes,
            cache_root: cache_root.clone(),
            ..Default::default()
        },
    )
    .unwrap()
}

fn assert_remote_window(snapshot: &GraphSnapshot, path: &Path) {
    assert!(snapshot.includes_remotes);
    let root = git(path, &["rev-list", "--max-parents=0", "HEAD"]);
    assert!(
        snapshot.commits.iter().any(|commit| commit.oid == root),
        "root commit missing from snapshot"
    );
    assert!(
        snapshot.commits.iter().any(|commit| {
            commit.refs.iter().any(|reference| {
                reference.kind == GraphRefKind::RemoteBranch && reference.name == "origin/feature"
            })
        }),
        "origin/feature missing from snapshot"
    );
}

fn compute(
    path: &Path,
    snapshot: &GraphSnapshot,
    cache_root: &CacheRoot,
) -> Vec<GraphRelationship> {
    compute_relationships(
        path,
        snapshot,
        Some("main"),
        cache_root,
        &EnrichmentCancel::never(),
    )
}

#[test]
fn remote_only_squash_merged_branch_gets_squash_relationship() {
    let (dir, cache_root, feature_tip) = remote_only_fixture(true);
    let path = dir.path();
    let snapshot = load(path, &cache_root, true);
    assert_remote_window(&snapshot, path);

    let relationships = compute(path, &snapshot, &cache_root);
    let squash = relationships
        .iter()
        .filter(|rel| rel.kind == RelationshipKind::SquashMerge)
        .collect::<Vec<_>>();
    assert_eq!(squash.len(), 1, "{relationships:?}");
    let rel = squash[0];
    assert_eq!(rel.matching, RelationshipMatch::Exact);
    assert_eq!(rel.source_oid, feature_tip);
    assert_eq!(rel.source_refs, vec!["origin/feature".to_string()]);
    assert_eq!(rel.destination_refs, vec!["main".to_string()]);
    assert_eq!(rel.destination_oid, git(path, &["rev-parse", "main"]));
}

#[test]
fn remote_only_cherry_picked_branch_gets_cherry_relationship() {
    let (dir, cache_root, feature_tip) = remote_only_fixture(false);
    let path = dir.path();
    let snapshot = load(path, &cache_root, true);
    assert_remote_window(&snapshot, path);

    let relationships = compute(path, &snapshot, &cache_root);
    let rel = relationships
        .iter()
        .find(|rel| rel.kind == RelationshipKind::CherryPick && rel.source_oid == feature_tip)
        .unwrap_or_else(|| panic!("no cherry relationship: {relationships:?}"));
    assert_eq!(rel.matching, RelationshipMatch::Exact);
    assert_eq!(rel.source_refs, vec!["origin/feature".to_string()]);
    assert_eq!(rel.destination_refs, vec!["main".to_string()]);
    assert_eq!(rel.destination_oid, git(path, &["rev-parse", "main"]));
}

#[test]
fn remote_tips_ignored_when_snapshot_excludes_remotes() {
    for squash in [true, false] {
        let (dir, cache_root, _) = remote_only_fixture(squash);
        let path = dir.path();

        let mut snapshot = load(path, &cache_root, true);
        assert!(
            !compute(path, &snapshot, &cache_root).is_empty(),
            "squash={squash}: remotes-on snapshot should produce relationships"
        );
        snapshot.includes_remotes = false;
        assert!(
            compute(path, &snapshot, &cache_root).is_empty(),
            "squash={squash}: includes_remotes=false must ignore remote tips"
        );

        let off = load(path, &cache_root, false);
        assert!(
            compute(path, &off, &cache_root).is_empty(),
            "squash={squash}: remotes-off load must produce no relationships"
        );
    }
}
