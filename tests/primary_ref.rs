use git2::{Oid, Repository, RepositoryInitOptions, Signature, Time};
use git_branch_manager::git::merge_detection::resolve_primary_ref_for_code_check;
use std::sync::atomic::AtomicBool;

fn fixture() -> (tempfile::TempDir, Repository) {
    let dir = tempfile::tempdir().unwrap();
    let mut options = RepositoryInitOptions::new();
    options.initial_head("main");
    let repo = Repository::init_opts(dir.path(), &options).unwrap();
    (dir, repo)
}

fn commit(repo: &Repository, parents: &[Oid], message: &str, timestamp: i64) -> Oid {
    let tree_id = repo.treebuilder(None).unwrap().write().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let signature = Signature::new("Test", "test@example.com", &Time::new(timestamp, 0)).unwrap();
    let parents: Vec<_> = parents
        .iter()
        .map(|oid| repo.find_commit(*oid).unwrap())
        .collect();
    let parent_refs: Vec<_> = parents.iter().collect();
    repo.commit(None, &signature, &signature, message, &tree, &parent_refs)
        .unwrap()
}

fn set_ref(repo: &Repository, name: &str, oid: Oid) {
    repo.reference(name, oid, true, "test fixture").unwrap();
}

fn assert_choice(repo: &Repository, branch: &str, expected_ref: &str, expected_oid: Oid) {
    let choice = resolve_primary_ref_for_code_check(repo, branch, None).unwrap();
    assert_eq!(choice.reference, expected_ref);
    assert_eq!(choice.oid, expected_oid);
}

#[test]
fn primary_ref_uses_existing_candidate_and_prefers_local_for_identical_tips() {
    let (_dir, repo) = fixture();
    let tip = commit(&repo, &[], "root", 100);
    set_ref(&repo, "refs/heads/release", tip);
    assert_choice(&repo, "release", "refs/heads/release", tip);
    repo.find_reference("refs/heads/release")
        .unwrap()
        .delete()
        .unwrap();
    set_ref(&repo, "refs/remotes/origin/release", tip);
    assert_choice(&repo, "release", "refs/remotes/origin/release", tip);
    set_ref(&repo, "refs/heads/release", tip);
    assert_choice(&repo, "release", "refs/heads/release", tip);
}

#[test]
fn primary_ref_prefers_descendant_even_when_its_timestamp_is_older() {
    let (_dir, repo) = fixture();
    let ancestor = commit(&repo, &[], "ancestor", 300);
    let descendant = commit(&repo, &[ancestor], "descendant with skewed clock", 100);
    set_ref(&repo, "refs/heads/main", ancestor);
    set_ref(&repo, "refs/remotes/origin/main", descendant);
    assert_choice(&repo, "main", "refs/remotes/origin/main", descendant);
    set_ref(&repo, "refs/heads/main", descendant);
    set_ref(&repo, "refs/remotes/origin/main", ancestor);
    assert_choice(&repo, "main", "refs/heads/main", descendant);
}

#[test]
fn primary_ref_prefers_longer_divergent_history_before_newer_timestamp() {
    let (_dir, repo) = fixture();
    let root = commit(&repo, &[], "root", 10);
    let first = commit(&repo, &[root], "first long side", 20);
    let longer = commit(&repo, &[first], "second long side", 30);
    let newer = commit(&repo, &[root], "shorter but newer", 900);
    set_ref(&repo, "refs/heads/main", newer);
    set_ref(&repo, "refs/remotes/origin/main", longer);
    assert_choice(&repo, "main", "refs/remotes/origin/main", longer);
    set_ref(&repo, "refs/heads/main", longer);
    set_ref(&repo, "refs/remotes/origin/main", newer);
    assert_choice(&repo, "main", "refs/heads/main", longer);
}

#[test]
fn primary_ref_counts_all_reachable_merge_parents() {
    let (_dir, repo) = fixture();
    let root = commit(&repo, &[], "root", 10);
    let left = commit(&repo, &[root], "left", 20);
    let right = commit(&repo, &[root], "right", 30);
    let merge = commit(&repo, &[left, right], "two-parent merge", 40);
    let short_first = commit(&repo, &[root], "short first", 800);
    let short_tip = commit(&repo, &[short_first], "short tip", 900);
    set_ref(&repo, "refs/heads/main", short_tip);
    set_ref(&repo, "refs/remotes/origin/main", merge);
    assert_choice(&repo, "main", "refs/remotes/origin/main", merge);
}

#[test]
fn primary_ref_uses_tip_committer_time_then_local_for_equal_history_lengths() {
    let (_dir, repo) = fixture();
    let root = commit(&repo, &[], "root", 10);
    let older = commit(&repo, &[root], "older", 100);
    let newer = commit(&repo, &[root], "newer", 200);
    set_ref(&repo, "refs/heads/main", older);
    set_ref(&repo, "refs/remotes/origin/main", newer);
    assert_choice(&repo, "main", "refs/remotes/origin/main", newer);
    set_ref(&repo, "refs/heads/main", newer);
    set_ref(&repo, "refs/remotes/origin/main", older);
    assert_choice(&repo, "main", "refs/heads/main", newer);
    let tied = commit(&repo, &[root], "same timestamp, different commit", 200);
    set_ref(&repo, "refs/remotes/origin/main", tied);
    assert_choice(&repo, "main", "refs/heads/main", newer);
}

#[test]
fn primary_ref_does_not_use_a_same_name_tag_or_another_remote() {
    let (_dir, repo) = fixture();
    let root = commit(&repo, &[], "tag-only root", 100);
    set_ref(&repo, "refs/tags/main", root);
    set_ref(&repo, "refs/remotes/upstream/main", root);
    let error = resolve_primary_ref_for_code_check(&repo, "main", None).unwrap_err();
    assert!(error.to_string().contains("has neither"));
    let branch = commit(&repo, &[root], "actual primary branch", 200);
    set_ref(&repo, "refs/heads/main", branch);
    assert_choice(&repo, "main", "refs/heads/main", branch);
}

#[test]
fn primary_ref_reports_broken_candidate_instead_of_falling_back() {
    let (_dir, repo) = fixture();
    let root = commit(&repo, &[], "valid remote", 100);
    set_ref(&repo, "refs/remotes/origin/main", root);
    repo.reference_symbolic(
        "refs/heads/main",
        "refs/heads/missing",
        true,
        "broken fixture",
    )
    .unwrap();
    let error = resolve_primary_ref_for_code_check(&repo, "main", None).unwrap_err();
    assert!(format!("{error:#}").contains("could not resolve refs/heads/main"));
    repo.find_reference("refs/heads/main")
        .unwrap()
        .delete()
        .unwrap();
    set_ref(&repo, "refs/heads/main", root);
    std::fs::write(
        repo.path().join("refs/remotes/origin/main"),
        "1234567890123456789012345678901234567890\n",
    )
    .unwrap();
    let error = resolve_primary_ref_for_code_check(&repo, "main", None).unwrap_err();
    assert!(format!("{error:#}").contains("refs/remotes/origin/main does not resolve to a commit"));
}

#[test]
fn primary_ref_honors_cancellation_and_returns_an_immutable_snapshot() {
    let (_dir, repo) = fixture();
    let root = commit(&repo, &[], "first primary", 100);
    set_ref(&repo, "refs/heads/main", root);
    let cancel = AtomicBool::new(true);
    assert!(
        resolve_primary_ref_for_code_check(&repo, "main", Some(&cancel))
            .unwrap_err()
            .to_string()
            .contains("canceled")
    );
    let snapshot = resolve_primary_ref_for_code_check(&repo, "main", None).unwrap();
    let moved = commit(&repo, &[root], "moved primary", 200);
    set_ref(&repo, "refs/heads/main", moved);
    assert_eq!(snapshot.oid, root);
    assert_choice(&repo, "main", "refs/heads/main", moved);
}

#[test]
fn primary_code_check_uses_resolved_oid_after_the_primary_ref_moves() {
    use git_branch_manager::git::merge_detection::check_ref_code_in_primary;
    use git_branch_manager::types::PrimaryBranchCodeMatch;

    let (_dir, repo) = fixture();
    let root = commit(&repo, &[], "old primary", 100);
    set_ref(&repo, "refs/heads/main", root);
    let snapshot = resolve_primary_ref_for_code_check(&repo, "main", None).unwrap();
    let blob = repo.blob(b"selected committed code\n").unwrap();
    let mut builder = repo.treebuilder(None).unwrap();
    builder.insert("feature.txt", blob, 0o100644).unwrap();
    let tree = repo.find_tree(builder.write().unwrap()).unwrap();
    let signature = Signature::new("Test", "test@example.com", &Time::new(200, 0)).unwrap();
    let parent = repo.find_commit(root).unwrap();
    let selected = repo
        .commit(None, &signature, &signature, "feature", &tree, &[&parent])
        .unwrap();
    set_ref(&repo, "refs/heads/feature", selected);
    set_ref(&repo, "refs/heads/main", selected);

    assert_eq!(
        check_ref_code_in_primary(&repo, &snapshot, "refs/heads/feature", None).unwrap(),
        PrimaryBranchCodeMatch::NotFound
    );
    let moved_snapshot = resolve_primary_ref_for_code_check(&repo, "main", None).unwrap();
    assert_eq!(
        check_ref_code_in_primary(&repo, &moved_snapshot, "refs/heads/feature", None).unwrap(),
        PrimaryBranchCodeMatch::Merged
    );
}
