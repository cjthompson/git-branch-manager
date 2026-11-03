use super::spawn_phase1_loader;
use crate::app::Phase1Msg;
use git_branch_manager::git::cache::{BranchCache, CacheRoot};
use git_branch_manager::types::{BranchInfo, MergeStatus};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("failed to spawn git");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn commit(dir: &Path, file: &str) {
    fs::write(dir.join(file), file).unwrap();
    git(dir, &["add", file]);
    git(dir, &["commit", "-m", file]);
}

fn init_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    git(p, &["init", "-b", "main"]);
    git(p, &["config", "user.name", "Phase1 Test"]);
    git(p, &["config", "user.email", "phase1@example.com"]);
    git(p, &["config", "commit.gpgsign", "false"]);
    commit(p, "README.md");
    dir
}

struct MixedRepo {
    repo: tempfile::TempDir,
    _remote: tempfile::TempDir,
    main_initial: String,
    main_tip: String,
    merged_tip: String,
}

/// main (tracked, in sync), feature-tracked (pushed, 1 ahead),
/// feature-unmerged (local only), feature-merged (merged into main and pushed).
fn mixed_repo() -> MixedRepo {
    let repo = init_repo();
    let p = repo.path();
    let remote = tempfile::tempdir().unwrap();
    git(remote.path(), &["init", "--bare"]);
    git(
        p,
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    );
    let main_initial = git(p, &["rev-parse", "HEAD"]);
    git(p, &["push", "-u", "origin", "main"]);

    git(p, &["switch", "-c", "feature-tracked"]);
    commit(p, "tracked.txt");
    git(p, &["push", "-u", "origin", "feature-tracked"]);
    commit(p, "tracked-ahead.txt");
    git(p, &["switch", "main"]);

    git(p, &["switch", "-c", "feature-unmerged", "main"]);
    commit(p, "unmerged.txt");
    git(p, &["switch", "main"]);

    git(p, &["switch", "-c", "feature-merged", "main"]);
    commit(p, "merged.txt");
    let merged_tip = git(p, &["rev-parse", "HEAD"]);
    git(p, &["switch", "main"]);
    git(
        p,
        &[
            "merge",
            "--no-ff",
            "feature-merged",
            "-m",
            "merge feature-merged",
        ],
    );
    git(p, &["push", "origin", "main"]);
    let main_tip = git(p, &["rev-parse", "main"]);

    MixedRepo {
        repo,
        _remote: remote,
        main_initial,
        main_tip,
        merged_tip,
    }
}

fn drain(rx: &Receiver<Phase1Msg>) -> Vec<Phase1Msg> {
    let mut out = Vec::new();
    loop {
        match rx.recv_timeout(Duration::from_secs(60)) {
            Ok(m) => out.push(m),
            Err(RecvTimeoutError::Disconnected) => return out,
            Err(RecvTimeoutError::Timeout) => panic!("phase1 loader stalled"),
        }
    }
}

type Counts = (Option<u32>, Option<u32>);

#[derive(Default)]
struct Outcome {
    kinds: Vec<&'static str>,
    fast: Vec<BranchInfo>,
    fast_base_tip: Option<String>,
    ahead_behind: Vec<(String, Counts)>,
    merge_bases: Vec<(String, String)>,
    merge_statuses: Vec<(String, MergeStatus)>,
}

fn outcome(msgs: Vec<Phase1Msg>) -> Outcome {
    let mut out = Outcome::default();
    for msg in msgs {
        match msg {
            Phase1Msg::Fast(branches, cache_for_app, _) => {
                out.kinds.push("Fast");
                out.fast = branches;
                out.fast_base_tip = cache_for_app.mb_data.base_tip.clone();
            }
            Phase1Msg::AheadBehind(updates) => {
                out.kinds.push("AheadBehind");
                out.ahead_behind = updates
                    .into_iter()
                    .map(|(name, ahead, behind)| (name, (ahead, behind)))
                    .collect();
            }
            Phase1Msg::MergeBaseCommits(updates) => {
                out.kinds.push("MergeBaseCommits");
                out.merge_bases = updates;
            }
            Phase1Msg::MergeStatuses(updates) => {
                out.kinds.push("MergeStatuses");
                out.merge_statuses = updates;
            }
        }
    }
    out
}

fn by_name<V: Clone>(pairs: &[(String, V)]) -> BTreeMap<String, V> {
    let map: BTreeMap<String, V> = pairs.iter().cloned().collect();
    assert_eq!(map.len(), pairs.len(), "duplicate branch entries");
    map
}

fn expect<V: Clone>(pairs: &[(&str, V)]) -> BTreeMap<String, V> {
    pairs
        .iter()
        .map(|(name, value)| (name.to_string(), value.clone()))
        .collect()
}

#[test]
fn mixed_branches_emit_fast_then_ahead_behind_and_merge_detection() {
    let fixture = mixed_repo();
    let cache = tempfile::tempdir().unwrap();
    let rx = spawn_phase1_loader(fixture.repo.path(), "main", &CacheRoot::at(cache.path()));
    let out = outcome(drain(&rx));

    assert_eq!(out.kinds.len(), 4, "kinds: {:?}", out.kinds);
    assert_eq!(out.kinds[0], "Fast");
    for kind in ["AheadBehind", "MergeBaseCommits", "MergeStatuses"] {
        assert_eq!(
            out.kinds.iter().filter(|k| **k == kind).count(),
            1,
            "{kind}"
        );
    }
    let pos = |kind: &str| out.kinds.iter().position(|k| *k == kind).unwrap();
    assert!(pos("MergeBaseCommits") < pos("MergeStatuses"));

    assert_eq!(out.fast.len(), 4);
    let fast_names: Vec<&str> = out.fast.iter().map(|b| b.name.as_str()).collect();
    for name in [
        "main",
        "feature-tracked",
        "feature-unmerged",
        "feature-merged",
    ] {
        assert!(fast_names.contains(&name), "missing {name}");
    }
    for b in out.fast.iter().filter(|b| !b.is_base && !b.is_current) {
        assert_eq!(b.merge_status, MergeStatus::Pending, "{}", b.name);
    }
    assert!(out.fast.iter().all(|b| b.merge_base_commit.is_none()));
    assert!(out
        .fast
        .iter()
        .all(|b| b.ahead.is_none() && b.behind.is_none()));

    assert_eq!(
        by_name(&out.ahead_behind),
        expect(&[
            ("main", (Some(0), Some(0))),
            ("feature-tracked", (Some(1), Some(0))),
        ]),
    );

    assert_eq!(
        by_name(&out.merge_bases),
        expect(&[
            ("feature-tracked", fixture.main_initial[..8].to_string()),
            ("feature-unmerged", fixture.main_initial[..8].to_string()),
            ("feature-merged", fixture.merged_tip[..8].to_string()),
        ]),
    );

    assert_eq!(
        by_name(&out.merge_statuses),
        expect(&[
            ("main", MergeStatus::Unmerged),
            ("feature-tracked", MergeStatus::Pending),
            ("feature-unmerged", MergeStatus::Pending),
            ("feature-merged", MergeStatus::Merged),
        ]),
    );
}

#[test]
fn base_only_repo_skips_merge_detection() {
    let repo = init_repo();
    let cache = tempfile::tempdir().unwrap();
    let rx = spawn_phase1_loader(repo.path(), "main", &CacheRoot::at(cache.path()));
    let out = outcome(drain(&rx));

    assert_eq!(out.kinds, ["Fast", "AheadBehind"]);
    let names: Vec<&str> = out.fast.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(names, ["main"]);
    assert!(out.ahead_behind.is_empty());
}

#[test]
fn second_run_with_unchanged_base_tip_restores_cached_values() {
    let fixture = mixed_repo();
    let cache = tempfile::tempdir().unwrap();
    let cache_root = CacheRoot::at(cache.path());

    let first = outcome(drain(&spawn_phase1_loader(
        fixture.repo.path(),
        "main",
        &cache_root,
    )));

    let repo = git2::Repository::open(fixture.repo.path()).unwrap();
    let base_tip = repo
        .find_branch("main", git2::BranchType::Local)
        .unwrap()
        .get()
        .target()
        .unwrap();
    let branch_tip = repo
        .find_branch("feature-unmerged", git2::BranchType::Local)
        .unwrap()
        .get()
        .target()
        .unwrap();
    assert_eq!(base_tip.to_string(), fixture.main_tip);

    let cached_merge_base = fixture.merged_tip[..8].to_string();
    assert_ne!(cached_merge_base, fixture.main_initial[..8]);
    let mut seeded_cache = BranchCache::load_for_base(fixture.repo.path(), "main", &cache_root);
    seeded_cache.set_base_tip(Some(base_tip.to_string()));
    seeded_cache.insert(
        "feature-unmerged",
        &MergeStatus::Merged,
        &branch_tip.to_string(),
    );
    seeded_cache.insert_merge_base(branch_tip, base_tip, Some(cached_merge_base.clone()));
    seeded_cache.save();

    let second = outcome(drain(&spawn_phase1_loader(
        fixture.repo.path(),
        "main",
        &cache_root,
    )));

    assert_eq!(
        second.fast_base_tip.as_deref(),
        Some(fixture.main_tip.as_str())
    );
    let first_merge_bases = by_name(&first.merge_bases);
    assert_eq!(
        first_merge_bases.get("feature-unmerged"),
        Some(&fixture.main_initial[..8].to_string()),
    );
    let second_merge_bases = by_name(&second.merge_bases);
    assert_eq!(
        second_merge_bases.get("feature-unmerged"),
        Some(&cached_merge_base),
    );

    let first_merge_statuses = by_name(&first.merge_statuses);
    assert_eq!(
        first_merge_statuses.get("feature-tracked"),
        Some(&MergeStatus::Pending),
    );
    assert_eq!(
        first_merge_statuses.get("feature-unmerged"),
        Some(&MergeStatus::Pending),
    );
    let second_merge_statuses = by_name(&second.merge_statuses);
    assert_eq!(
        second_merge_statuses.get("feature-tracked"),
        Some(&MergeStatus::Pending),
    );
    assert_eq!(
        second_merge_statuses.get("feature-unmerged"),
        Some(&MergeStatus::Merged),
    );

    let db_files = fs::read_dir(cache.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("git-bm-repo-cache-") && n.ends_with(".sqlite3"))
        .count();
    assert_eq!(db_files, 1);
}
