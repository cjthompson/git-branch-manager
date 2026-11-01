use super::compute_ahead_behind;
use chrono::Utc;
use git_branch_manager::git::cache::BranchCache;
use git_branch_manager::types::{BranchInfo, MergeStatus, TrackingStatus};

type Row = (String, Option<u32>, Option<u32>);

fn branch(name: &str, tracking: TrackingStatus) -> BranchInfo {
    BranchInfo {
        name: name.to_string(),
        is_current: false,
        is_base: false,
        tracking,
        ahead: None,
        behind: None,
        last_commit_date: Utc::now(),
        merge_status: MergeStatus::Unmerged,
        base_branch: "main".to_string(),
        merge_base_commit: None,
        pr: None,
        squash_confidence: None,
    }
}

fn tracked(remote_ref: &str, gone: bool) -> TrackingStatus {
    TrackingStatus::Tracked {
        remote_ref: remote_ref.to_string(),
        gone,
    }
}

fn commit(repo: &git2::Repository, parent: Option<git2::Oid>, label: &str) -> git2::Oid {
    let tree_oid = repo.treebuilder(None).unwrap().write().unwrap();
    let tree = repo.find_tree(tree_oid).unwrap();
    let sig = git2::Signature::now("test", "test@example.com").unwrap();
    let parent_commit = parent.map(|p| repo.find_commit(p).unwrap());
    let parents: Vec<&git2::Commit> = parent_commit.iter().collect();
    repo.commit(None, &sig, &sig, label, &tree, &parents)
        .unwrap()
}

fn setup() -> (tempfile::TempDir, git2::Repository, BranchCache) {
    let dir = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(dir.path()).unwrap();
    let cache = BranchCache::load_from_path(dir.path().join("cache.sqlite3"));
    (dir, repo, cache)
}

/// Adds origin (no network is used) and branch.<name> upstream config pointing at refs/remotes/origin/<name>.
fn configure_upstream(repo: &git2::Repository, name: &str) {
    if repo.find_remote("origin").is_err() {
        repo.remote("origin", "https://invalid.invalid/repo.git")
            .unwrap();
    }
    let mut config = repo.config().unwrap();
    config
        .set_str(&format!("branch.{name}.remote"), "origin")
        .unwrap();
    config
        .set_str(
            &format!("branch.{name}.merge"),
            &format!("refs/heads/{name}"),
        )
        .unwrap();
}

/// Local feat is 2 commits ahead of and upstream is 1 commit ahead of the shared root.
/// Returns (local tip, upstream tip).
fn feat_topology(repo: &git2::Repository) -> (git2::Oid, git2::Oid) {
    let root = commit(repo, None, "root");
    let upstream = commit(repo, Some(root), "upstream-1");
    let local_1 = commit(repo, Some(root), "local-1");
    let local_2 = commit(repo, Some(local_1), "local-2");
    repo.reference("refs/heads/feat", local_2, true, "")
        .unwrap();
    repo.reference("refs/remotes/origin/feat", upstream, true, "")
        .unwrap();
    configure_upstream(repo, "feat");
    (local_2, upstream)
}

#[test]
fn tracked_branch_with_upstream_computes_and_caches_counts() {
    let (_dir, repo, mut cache) = setup();
    let (local_tip, upstream_tip) = feat_topology(&repo);
    let input = [branch("feat", tracked("origin/feat", false))];

    let out = compute_ahead_behind(&repo, &input, &mut cache);

    assert_eq!(out, vec![("feat".to_string(), Some(2), Some(1))]);
    assert_eq!(
        cache.lookup_ahead_behind(local_tip, upstream_tip),
        Some((2, 1))
    );
}

#[test]
fn cached_ahead_behind_is_returned_without_recomputing() {
    let (_dir, repo, mut cache) = setup();
    let (local_tip, upstream_tip) = feat_topology(&repo);
    cache.insert_ahead_behind(local_tip, upstream_tip, 7, 9);
    let input = [branch("feat", tracked("origin/feat", false))];

    let first = compute_ahead_behind(&repo, &input, &mut cache);
    let second = compute_ahead_behind(&repo, &input, &mut cache);

    assert_eq!(first, vec![("feat".to_string(), Some(7), Some(9))]);
    assert_eq!(second, first);
    assert_eq!(
        cache.lookup_ahead_behind(local_tip, upstream_tip),
        Some((7, 9))
    );
}

#[test]
fn gone_upstream_is_skipped_and_not_cached() {
    let (_dir, repo, mut cache) = setup();
    let (local_tip, upstream_tip) = feat_topology(&repo);
    let input = [branch("feat", tracked("origin/feat", true))];

    let out = compute_ahead_behind(&repo, &input, &mut cache);

    assert_eq!(out, Vec::<Row>::new());
    assert_eq!(cache.lookup_ahead_behind(local_tip, upstream_tip), None);
}

#[test]
fn local_only_branch_is_skipped() {
    let (_dir, repo, mut cache) = setup();
    let (local_tip, upstream_tip) = feat_topology(&repo);
    let input = [branch("feat", TrackingStatus::Local)];

    let out = compute_ahead_behind(&repo, &input, &mut cache);

    assert_eq!(out, Vec::<Row>::new());
    assert_eq!(cache.lookup_ahead_behind(local_tip, upstream_tip), None);
}

#[test]
fn tracked_branch_missing_from_repo_is_skipped() {
    let (_dir, repo, mut cache) = setup();
    let input = [branch("ghost", tracked("origin/ghost", false))];

    let out = compute_ahead_behind(&repo, &input, &mut cache);

    assert_eq!(out, Vec::<Row>::new());
}

#[test]
fn tracked_branch_without_upstream_config_is_skipped() {
    let (_dir, repo, mut cache) = setup();
    let root = commit(&repo, None, "root");
    let solo_tip = commit(&repo, Some(root), "solo-tip");
    repo.reference("refs/heads/solo", solo_tip, true, "")
        .unwrap();
    repo.reference("refs/remotes/origin/solo", root, true, "")
        .unwrap();
    let input = [branch("solo", tracked("origin/solo", false))];

    let out = compute_ahead_behind(&repo, &input, &mut cache);

    assert_eq!(out, Vec::<Row>::new());
}

#[test]
fn upstream_ref_pointing_at_blob_is_skipped() {
    let (_dir, repo, mut cache) = setup();
    let root = commit(&repo, None, "root");
    let local_tip = commit(&repo, Some(root), "blob-up-local");
    let blob = repo.blob(b"not a commit").unwrap();
    repo.reference("refs/heads/blob-up", local_tip, true, "")
        .unwrap();
    repo.reference("refs/remotes/origin/blob-up", blob, true, "")
        .unwrap();
    configure_upstream(&repo, "blob-up");
    let local_branch = repo
        .find_branch("blob-up", git2::BranchType::Local)
        .unwrap();
    let upstream_branch = local_branch.upstream().unwrap();
    assert!(local_branch.get().peel_to_commit().is_ok());
    assert!(upstream_branch.get().peel_to_commit().is_err());
    let input = [branch("blob-up", tracked("origin/blob-up", false))];

    let out = compute_ahead_behind(&repo, &input, &mut cache);

    assert_eq!(out, Vec::<Row>::new());
}

#[test]
fn local_branch_ref_pointing_at_blob_is_skipped() {
    let (_dir, repo, mut cache) = setup();
    let root = commit(&repo, None, "root");
    let upstream_tip = commit(&repo, Some(root), "blob-local-upstream");
    let blob = repo.blob(b"not a commit").unwrap();
    repo.reference("refs/heads/blob-local", blob, true, "")
        .unwrap();
    repo.reference("refs/remotes/origin/blob-local", upstream_tip, true, "")
        .unwrap();
    configure_upstream(&repo, "blob-local");
    let local_branch = repo
        .find_branch("blob-local", git2::BranchType::Local)
        .unwrap();
    let upstream_branch = local_branch.upstream().unwrap();
    assert!(local_branch.get().peel_to_commit().is_err());
    assert!(upstream_branch.get().peel_to_commit().is_ok());
    let input = [branch("blob-local", tracked("origin/blob-local", false))];

    let out = compute_ahead_behind(&repo, &input, &mut cache);

    assert_eq!(out, Vec::<Row>::new());
}

#[test]
fn graph_error_keeps_branch_with_none_counts() {
    let (dir, repo, mut cache) = setup();
    let root = commit(&repo, None, "graph-error-root");
    let local_tip = commit(&repo, Some(root), "graph-error-local");
    let upstream_tip = commit(&repo, Some(root), "graph-error-upstream");
    repo.reference("refs/heads/feat", local_tip, true, "")
        .unwrap();
    repo.reference("refs/remotes/origin/feat", upstream_tip, true, "")
        .unwrap();
    configure_upstream(&repo, "feat");

    // Remove the shared root's loose object so the merge-base walk cannot read it.
    let objects_dir = repo.path().join("objects");
    drop(repo);
    let root_hex = root.to_string();
    std::fs::remove_file(objects_dir.join(&root_hex[..2]).join(&root_hex[2..])).unwrap();
    let repo = git2::Repository::open(dir.path()).unwrap();
    let input = [branch("feat", tracked("origin/feat", false))];

    let out = compute_ahead_behind(&repo, &input, &mut cache);

    assert_eq!(out, vec![("feat".to_string(), None, None)]);
    assert_eq!(cache.lookup_ahead_behind(local_tip, upstream_tip), None);
}
