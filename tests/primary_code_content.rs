use git2::{Oid, Repository, RepositoryInitOptions, Signature, Time};
use git_branch_manager::git::merge_detection::{
    check_ref_code_in_primary, resolve_primary_ref_for_code_check, ResolvedPrimaryRef,
};
use git_branch_manager::types::PrimaryBranchCodeMatch as Match;
use std::sync::atomic::AtomicBool;

fn fixture() -> (tempfile::TempDir, Repository) {
    let dir = tempfile::tempdir().unwrap();
    let mut options = RepositoryInitOptions::new();
    options.initial_head("main");
    let repo = Repository::init_opts(dir.path(), &options).unwrap();
    (dir, repo)
}

fn commit(repo: &Repository, parents: &[Oid], name: &str, files: &[(&str, &[u8], i32)]) -> Oid {
    let mut builder = repo.treebuilder(None).unwrap();
    for (path, content, mode) in files {
        builder
            .insert(path, repo.blob(content).unwrap(), *mode)
            .unwrap();
    }
    let tree = repo.find_tree(builder.write().unwrap()).unwrap();
    let signature = Signature::new("Test", "test@example.com", &Time::new(1000, 0)).unwrap();
    let parents: Vec<_> = parents
        .iter()
        .map(|oid| repo.find_commit(*oid).unwrap())
        .collect();
    let parent_refs: Vec<_> = parents.iter().collect();
    repo.commit(None, &signature, &signature, name, &tree, &parent_refs)
        .unwrap()
}

fn set_ref(repo: &Repository, name: &str, oid: Oid) {
    repo.reference(name, oid, true, "fixture").unwrap();
}

fn check(repo: &Repository) -> anyhow::Result<Match> {
    let primary = resolve_primary_ref_for_code_check(repo, "main", None)?;
    check_ref_code_in_primary(repo, &primary, "refs/heads/feature", None)
}

#[test]
fn primary_code_requires_exact_binary_content() {
    let (_dir, repo) = fixture();
    let root = commit(&repo, &[], "root", &[]);
    let source = commit(
        &repo,
        &[root],
        "source",
        &[("data.bin", b"\0source", 0o100644)],
    );
    let different = commit(
        &repo,
        &[root],
        "different",
        &[("data.bin", b"\0other", 0o100644)],
    );
    set_ref(&repo, "refs/heads/feature", source);
    set_ref(&repo, "refs/heads/main", different);
    assert_eq!(check(&repo).unwrap(), Match::NotFound);
    let same = commit(
        &repo,
        &[different],
        "integrated",
        &[
            ("data.bin", b"\0source", 0o100644),
            ("other", b"other", 0o100644),
        ],
    );
    set_ref(&repo, "refs/heads/main", same);
    assert_eq!(
        check(&repo).unwrap(),
        Match::ContentEquivalent {
            commit_oid: same.to_string()
        }
    );
}

#[test]
fn primary_code_includes_modes_even_when_config_ignores_them() {
    let (_dir, repo) = fixture();
    repo.config()
        .unwrap()
        .set_bool("core.filemode", false)
        .unwrap();
    let root = commit(&repo, &[], "root", &[("tool", b"code", 0o100644)]);
    let source = commit(&repo, &[root], "executable", &[("tool", b"code", 0o100755)]);
    let unchanged = commit(
        &repo,
        &[root],
        "other code",
        &[("tool", b"code", 0o100644), ("other", b"other", 0o100644)],
    );
    set_ref(&repo, "refs/heads/feature", source);
    set_ref(&repo, "refs/heads/main", unchanged);
    assert_eq!(check(&repo).unwrap(), Match::NotFound);
    let same = commit(
        &repo,
        &[unchanged],
        "mode integrated",
        &[("tool", b"code", 0o100755), ("other", b"other", 0o100644)],
    );
    set_ref(&repo, "refs/heads/main", same);
    assert_eq!(
        check(&repo).unwrap(),
        Match::ContentEquivalent {
            commit_oid: same.to_string()
        }
    );
}

#[test]
fn primary_code_recognizes_rename_with_unrelated_changes() {
    let (_dir, repo) = fixture();
    let root = commit(&repo, &[], "root", &[("old.txt", b"content", 0o100644)]);
    let source = commit(
        &repo,
        &[root],
        "rename",
        &[("new.txt", b"content", 0o100644)],
    );
    let primary = commit(
        &repo,
        &[root],
        "rename and other",
        &[
            ("new.txt", b"content", 0o100644),
            ("other", b"other", 0o100644),
        ],
    );
    set_ref(&repo, "refs/heads/feature", source);
    set_ref(&repo, "refs/heads/main", primary);
    assert_eq!(
        check(&repo).unwrap(),
        Match::ContentEquivalent {
            commit_oid: primary.to_string()
        }
    );
}

#[test]
fn primary_code_errors_are_distinct_from_nonmatches_and_cancellation() {
    let (_dir, repo) = fixture();
    let root = commit(&repo, &[], "root", &[]);
    set_ref(&repo, "refs/heads/main", root);
    let primary = resolve_primary_ref_for_code_check(&repo, "main", None).unwrap();
    let error = check_ref_code_in_primary(&repo, &primary, "refs/heads/missing", None).unwrap_err();
    assert!(format!("{error:#}").contains("could not resolve selected ref refs/heads/missing"));
    let unrelated = commit(&repo, &[], "unrelated", &[("other", b"other", 0o100644)]);
    set_ref(&repo, "refs/heads/feature", unrelated);
    assert!(format!("{:#}", check(&repo).unwrap_err()).contains("shared history"));
    let broken = ResolvedPrimaryRef {
        reference: "refs/heads/main".into(),
        oid: Oid::from_str("1234567890123456789012345678901234567890").unwrap(),
    };
    let error = check_ref_code_in_primary(&repo, &broken, "refs/heads/feature", None).unwrap_err();
    assert!(format!("{error:#}").contains("could not read chosen primary tip"));
    let error = check_ref_code_in_primary(
        &repo,
        &primary,
        "refs/heads/missing",
        Some(&AtomicBool::new(true)),
    )
    .unwrap_err();
    assert!(error.to_string().contains("canceled"));
}

#[test]
fn primary_code_preserves_available_shallow_history() {
    let (dir, repo) = fixture();
    let old_root = commit(&repo, &[], "outside shallow history", &[]);
    let boundary = commit(&repo, &[old_root], "boundary", &[]);
    let source = commit(
        &repo,
        &[boundary],
        "source",
        &[("feature", b"source", 0o100644)],
    );
    let primary = commit(
        &repo,
        &[boundary],
        "manual integration",
        &[
            ("feature", b"source", 0o100644),
            ("other", b"other", 0o100644),
        ],
    );
    set_ref(&repo, "refs/heads/feature", source);
    set_ref(&repo, "refs/heads/main", primary);
    std::fs::write(repo.path().join("shallow"), format!("{boundary}\n")).unwrap();
    let hex = old_root.to_string();
    let missing = repo.path().join("objects").join(&hex[..2]).join(&hex[2..]);
    drop(repo);
    std::fs::remove_file(missing).unwrap();
    let repo = Repository::open(dir.path()).unwrap();
    assert!(repo.is_shallow());
    assert_eq!(
        check(&repo).unwrap(),
        Match::ContentEquivalent {
            commit_oid: primary.to_string()
        }
    );
}
