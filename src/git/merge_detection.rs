use crate::git::fuzzy_match;
use crate::types::{BranchInfo, MergeStatus, PrimaryBranchCodeMatch, SquashConfidence};
use anyhow::Context;
use git2::{ErrorCode, Repository};
use std::collections::HashSet;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::{field, instrument, Span};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPrimaryRef {
    pub reference: String,
    pub oid: git2::Oid,
}

fn check_cancel(cancel: Option<&AtomicBool>) -> anyhow::Result<()> {
    if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
        anyhow::bail!("primary branch code check canceled");
    }
    Ok(())
}

fn lookup_primary_candidate(
    repo: &Repository,
    reference: &str,
) -> anyhow::Result<Option<ResolvedPrimaryRef>> {
    let candidate = match repo.find_reference(reference) {
        Ok(candidate) => candidate,
        Err(error) if error.code() == ErrorCode::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("could not read {reference}")),
    };
    let resolved = candidate
        .resolve()
        .with_context(|| format!("could not resolve {reference}"))?;
    let commit = resolved
        .peel_to_commit()
        .with_context(|| format!("{reference} does not resolve to a commit"))?;
    Ok(Some(ResolvedPrimaryRef {
        reference: reference.to_string(),
        oid: commit.id(),
    }))
}

fn unique_reachable_commit_count(
    repo: &Repository,
    tip: git2::Oid,
    other: git2::Oid,
    reference: &str,
    cancel: Option<&AtomicBool>,
) -> anyhow::Result<usize> {
    let mut revwalk = repo
        .revwalk()
        .with_context(|| format!("could not walk history for {reference}"))?;
    revwalk
        .push(tip)
        .with_context(|| format!("could not start history walk for {reference}"))?;
    revwalk
        .hide(other)
        .with_context(|| format!("could not exclude shared history for {reference}"))?;
    let mut count = 0usize;
    for oid in revwalk {
        check_cancel(cancel)?;
        oid.with_context(|| format!("could not read history for {reference}"))?;
        count += 1;
    }
    Ok(count)
}

/// Select and snapshot the configured local primary branch or its origin tracking ref.
/// The selector prefers ancestry, then unique reachable commit count, then tip committer time,
/// and the local ref on an exact tie. It never fetches or consults same-name tags.
pub fn resolve_primary_ref_for_code_check(
    repo: &Repository,
    configured_base: &str,
    cancel: Option<&AtomicBool>,
) -> anyhow::Result<ResolvedPrimaryRef> {
    check_cancel(cancel)?;
    let local_name = format!("refs/heads/{configured_base}");
    let remote_name = format!("refs/remotes/origin/{configured_base}");
    let local = lookup_primary_candidate(repo, &local_name)?;
    check_cancel(cancel)?;
    let remote = lookup_primary_candidate(repo, &remote_name)?;

    match (local, remote) {
        (Some(local), None) | (None, Some(local)) => Ok(local),
        (None, None) => anyhow::bail!(
            "configured primary branch `{configured_base}` has neither `{local_name}` nor `{remote_name}`"
        ),
        (Some(local), Some(remote)) => {
            check_cancel(cancel)?;
            if local.oid == remote.oid
                || repo
                    .graph_descendant_of(local.oid, remote.oid)
                    .with_context(|| {
                        format!("could not compare {} with {}", local.reference, remote.reference)
                    })?
            {
                return Ok(local);
            }
            check_cancel(cancel)?;
            if repo
                .graph_descendant_of(remote.oid, local.oid)
                .with_context(|| {
                    format!("could not compare {} with {}", remote.reference, local.reference)
                })?
            {
                return Ok(remote);
            }

            let local_count = unique_reachable_commit_count(
                repo,
                local.oid,
                remote.oid,
                &local.reference,
                cancel,
            )?;
            let remote_count = unique_reachable_commit_count(
                repo,
                remote.oid,
                local.oid,
                &remote.reference,
                cancel,
            )?;
            if local_count != remote_count {
                return Ok(if local_count > remote_count { local } else { remote });
            }

            check_cancel(cancel)?;
            let local_time = repo
                .find_commit(local.oid)
                .with_context(|| format!("could not read tip for {}", local.reference))?
                .committer()
                .when()
                .seconds();
            let remote_time = repo
                .find_commit(remote.oid)
                .with_context(|| format!("could not read tip for {}", remote.reference))?
                .committer()
                .when()
                .seconds();
            Ok(if remote_time > local_time { remote } else { local })
        }
    }
}

/// Holds the reachable sets for both the local base branch and its remote tracking ref.
/// Used to determine whether a branch is merged, and if only into one side.
///
/// `local_tip` and `remote_tip` are the OIDs the reachable sets were built from
/// — the tip commits of the local and remote base branches. They let
/// [`Self::regular_merge_status`] distinguish a branch whose tip is *literally*
/// the base tip (no integration ever occurred → `InSync`) from a branch whose
/// tip is just an ancestor of base (`Merged`).
pub struct BaseReachable {
    pub local: HashSet<git2::Oid>,
    pub remote: HashSet<git2::Oid>,
    pub local_tip: Option<git2::Oid>,
    pub remote_tip: Option<git2::Oid>,
}

impl BaseReachable {
    /// Returns the appropriate MergeStatus for a branch OID.
    /// When no remote tracking ref exists, local is treated as authoritative (returns Merged).
    ///
    /// A branch whose tip OID is *literally* the base tip is reported as
    /// [`MergeStatus::InSync`] — the branch has no unique commits and no
    /// integration event has occurred. This is checked before the reachable-set
    /// match because a literal tip is trivially a member of the reachable set.
    pub fn regular_merge_status(&self, oid: git2::Oid) -> Option<MergeStatus> {
        // In-sync: branch tip == base tip literally. No integration event — just identical.
        if Some(oid) == self.local_tip {
            return Some(MergeStatus::InSync);
        }
        // If local tip is missing (e.g. base has no local ref but the remote ref
        // exists), an in-sync match against the remote tip is still meaningful.
        if self.local_tip.is_none() && Some(oid) == self.remote_tip {
            return Some(MergeStatus::InSync);
        }
        let in_local = self.local.contains(&oid);
        let in_remote = self.remote.contains(&oid);
        let has_remote = !self.remote.is_empty();
        match (in_local, in_remote, has_remote) {
            (true, true, _) => Some(MergeStatus::Merged),
            (true, false, false) => Some(MergeStatus::Merged), // no remote — local is truth
            (false, true, _) => Some(MergeStatus::RemoteMerged),
            (true, false, true) => Some(MergeStatus::LocalMerged),
            (false, false, _) => None,
        }
    }
}

fn build_reachable_from_ref(
    repo: &Repository,
    base_branch: &str,
) -> (HashSet<git2::Oid>, Option<git2::Oid>) {
    let oid = match repo
        .find_branch(base_branch, git2::BranchType::Local)
        .ok()
        .and_then(|b| b.get().target())
    {
        Some(oid) => oid,
        None => return (HashSet::new(), None),
    };
    (revwalk_from_oid(repo, oid), Some(oid))
}

fn build_reachable_from_remote_ref(
    repo: &Repository,
    base_branch: &str,
) -> (HashSet<git2::Oid>, Option<git2::Oid>) {
    let remote_name = format!("origin/{base_branch}");
    let oid = match repo
        .find_branch(&remote_name, git2::BranchType::Remote)
        .ok()
        .and_then(|b| b.get().target())
    {
        Some(oid) => oid,
        None => return (HashSet::new(), None),
    };
    (revwalk_from_oid(repo, oid), Some(oid))
}

fn revwalk_from_oid(repo: &Repository, oid: git2::Oid) -> HashSet<git2::Oid> {
    let mut revwalk = match repo.revwalk() {
        Ok(r) => r,
        Err(_) => return HashSet::new(),
    };
    let _ = revwalk.set_sorting(git2::Sort::NONE);
    let _ = revwalk.push(oid);
    let mut set = HashSet::new();
    for oid in revwalk.flatten() {
        set.insert(oid);
    }
    set
}

/// Detect which branches have been regular-merged into the base branch using git2.
/// Modifies branch merge_status in place from Unmerged to Merged where applicable.
#[instrument(
    skip(repo, branches),
    fields(
        base_branch,
        branch_count = branches.len(),
        base_oid = field::Empty,
        base_lookup_result = field::Empty,
        candidate_count = field::Empty,
        checked_count = field::Empty,
        skipped_base_count = field::Empty,
        skipped_current_count = field::Empty,
        find_branch_error_count = field::Empty,
        missing_target_count = field::Empty,
        merged_count = field::Empty,
        unmerged_count = field::Empty,
    )
)]
/// Returns a BaseReachable with reachable sets for both local and remote base,
/// so callers can reuse it for merge-base computation and squash detection.
pub fn detect_merged_branches(
    repo: &Repository,
    base_branch: &str,
    branches: &mut [BranchInfo],
) -> anyhow::Result<BaseReachable> {
    let span = Span::current();

    let (local_reachable, local_tip) = build_reachable_from_ref(repo, base_branch);
    let (remote_reachable, remote_tip) = build_reachable_from_remote_ref(repo, base_branch);

    if local_reachable.is_empty() && remote_reachable.is_empty() {
        span.record("base_lookup_result", "find_branch_error");
        return Err(anyhow::anyhow!("base branch not found: {base_branch}"));
    }
    span.record("base_lookup_result", "success");

    let base_reachable = BaseReachable {
        local: local_reachable,
        remote: remote_reachable,
        local_tip,
        remote_tip,
    };

    let candidates: Vec<(usize, git2::Oid)> = branches
        .iter()
        .enumerate()
        .filter(|(_, b)| !b.is_base && !b.is_current)
        .filter_map(|(i, b)| {
            repo.find_branch(&b.name, git2::BranchType::Local)
                .and_then(|br| {
                    br.get()
                        .target()
                        .ok_or_else(|| git2::Error::from_str("no target"))
                })
                .ok()
                .map(|oid| (i, oid))
        })
        .collect();

    let skipped_base_count = branches.iter().filter(|b| b.is_base).count();
    let skipped_current_count = branches.iter().filter(|b| b.is_current).count();
    let candidate_count = branches
        .iter()
        .filter(|b| !b.is_base && !b.is_current)
        .count();
    let find_branch_error_count = candidate_count.saturating_sub(candidates.len());
    let missing_target_count = 0usize;

    span.record("candidate_count", candidate_count);
    span.record("skipped_base_count", skipped_base_count);
    span.record("skipped_current_count", skipped_current_count);
    span.record("find_branch_error_count", find_branch_error_count);
    span.record("missing_target_count", missing_target_count);

    if candidates.is_empty() {
        span.record("checked_count", 0u64);
        span.record("merged_count", 0u64);
        span.record("unmerged_count", 0u64);
        return Ok(base_reachable);
    }

    let mut merged_count = 0usize;
    let mut unmerged_count = 0usize;
    for (i, branch_oid) in &candidates {
        if let Some(status) = base_reachable.regular_merge_status(*branch_oid) {
            merged_count += 1;
            branches[*i].merge_status = status;
        } else {
            unmerged_count += 1;
        }
    }
    span.record("checked_count", candidates.len() as u64);
    span.record("merged_count", merged_count);
    span.record("unmerged_count", unmerged_count);
    Ok(base_reachable)
}

/// Build the reachable sets for both local and remote base using an already-open repository.
/// Call this when you already have a repo handle on the current thread.
#[instrument(skip(repo), fields(reachable_count = field::Empty))]
pub fn build_reachable_set_from_repo(repo: &Repository, base_branch: &str) -> BaseReachable {
    let (local, local_tip) = build_reachable_from_ref(repo, base_branch);
    let (remote, remote_tip) = build_reachable_from_remote_ref(repo, base_branch);
    BaseReachable {
        local,
        remote,
        local_tip,
        remote_tip,
    }
}

/// Build the reachable sets for both local and remote base by opening a fresh Repository.
/// Intended for background-thread use: Repository is !Send, so callers open their own handle
/// rather than sharing the main thread's repo.
pub fn build_reachable_set(repo_path: &Path, base_branch: &str) -> BaseReachable {
    let repo = match git2::Repository::open(repo_path) {
        Ok(r) => r,
        Err(_) => {
            return BaseReachable {
                local: HashSet::new(),
                remote: HashSet::new(),
                local_tip: None,
                remote_tip: None,
            }
        }
    };
    build_reachable_set_from_repo(&repo, base_branch)
}

/// Apply merge statuses to branches using a prebuilt BaseReachable.
/// Used when the reachable set was built in a parallel thread via build_reachable_set,
/// so we re-resolve each branch tip with the provided (main-thread) repo handle.
pub fn apply_merge_statuses(
    repo: &Repository,
    branches: &mut [BranchInfo],
    base_reachable: &BaseReachable,
) {
    if base_reachable.local.is_empty() && base_reachable.remote.is_empty() {
        return;
    }
    for branch in branches.iter_mut() {
        if branch.is_base || branch.is_current {
            continue;
        }
        if let Ok(b) = repo.find_branch(&branch.name, git2::BranchType::Local) {
            if let Some(oid) = b.get().target() {
                if let Some(status) = base_reachable.regular_merge_status(oid) {
                    branch.merge_status = status;
                }
            }
        }
    }
}

/// Check whether a selected committed ref's aggregate change appears in the
/// historical snapshots reachable from an already-resolved primary ref.
pub fn check_ref_code_in_primary(
    repo: &Repository,
    primary: &ResolvedPrimaryRef,
    selected_ref: &str,
    cancel: Option<&AtomicBool>,
) -> anyhow::Result<PrimaryBranchCodeMatch> {
    check_cancel(cancel)?;
    let selected_oid = repo
        .revparse_single(selected_ref)
        .with_context(|| format!("could not resolve selected ref {selected_ref}"))?
        .peel_to_commit()
        .with_context(|| format!("selected ref {selected_ref} does not resolve to a commit"))?
        .id();

    let comparison_odb = git2::Odb::new_ext(repo.object_format())
        .context("could not create isolated comparison object database")?;
    let object_path = repo.commondir().join("objects");
    comparison_odb
        .add_disk_alternate(&object_path.to_string_lossy())
        .with_context(|| format!("could not read objects from {}", object_path.display()))?;
    let comparison_repo = Repository::from_odb(comparison_odb)
        .context("could not create isolated comparison repository")?;
    let clean_config = git2::Config::new().context("could not create isolated merge config")?;
    comparison_repo
        .set_config(&clean_config)
        .context("could not isolate merge-driver configuration")?;
    let odb = comparison_repo
        .odb()
        .context("could not read isolated comparison object database")?;
    let mempack = odb
        .add_new_mempack_backend(1000)
        .context("could not add scratch in-memory object backend")?;

    // Keep history walks and graph queries on the caller's repository: it may
    // carry shallow graft metadata that the scratch repository deliberately lacks.
    let base = repo
        .find_commit(primary.oid)
        .with_context(|| format!("could not read chosen primary tip {}", primary.reference))?;
    let selected = repo
        .find_commit(selected_oid)
        .with_context(|| format!("could not read selected commit {selected_oid}"))?;
    check_cancel(cancel)?;

    if base.id() == selected.id()
        || repo
            .graph_descendant_of(base.id(), selected.id())
            .with_context(|| {
                format!(
                    "could not compare chosen primary {} with {selected_ref}",
                    primary.reference
                )
            })?
    {
        return Ok(PrimaryBranchCodeMatch::Merged);
    }

    check_cancel(cancel)?;
    let merge_bases = repo
        .merge_bases(base.id(), selected.id())
        .with_context(|| {
            format!(
                "could not find shared history between {} and {selected_ref}",
                primary.reference
            )
        })?;
    if merge_bases.len() != 1 {
        anyhow::bail!(
            "expected one shared history base between {} and {selected_ref}, found {}",
            primary.reference,
            merge_bases.len()
        );
    }

    let ancestor_tree = repo
        .find_commit(merge_bases[0])
        .context("could not read shared history commit")?
        .tree()
        .context("could not read shared history tree")?;
    let selected_tree = selected
        .tree()
        .with_context(|| format!("could not read selected ref tree {selected_ref}"))?;

    let mut revwalk = repo
        .revwalk()
        .context("could not start primary history walk")?;
    revwalk
        .push(base.id())
        .with_context(|| format!("could not walk history for {}", primary.reference))?;
    revwalk
        .set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME)
        .context("could not order primary history walk")?;

    let mut visited_trees = HashSet::new();
    for oid in revwalk {
        check_cancel(cancel)?;
        let oid = oid.context("could not read primary history walk")?;
        let history_commit = repo
            .find_commit(oid)
            .with_context(|| format!("could not read primary history commit {oid}"))?;
        let history_tree = history_commit
            .tree()
            .with_context(|| format!("could not read tree for primary commit {oid}"))?;
        if !visited_trees.insert(history_tree.id()) {
            continue;
        }

        let found = {
            let scratch_ancestor = comparison_repo
                .find_tree(ancestor_tree.id())
                .context("could not load shared history tree into scratch repository")?;
            let scratch_history =
                comparison_repo
                    .find_tree(history_tree.id())
                    .with_context(|| {
                        format!("could not load primary tree {oid} into scratch repository")
                    })?;
            let scratch_selected = comparison_repo
                .find_tree(selected_tree.id())
                .context("could not load selected tree into scratch repository")?;
            configure_scratch_merge_attributes(&comparison_repo, &odb)?;
            let merged = comparison_repo
                .merge_trees(&scratch_ancestor, &scratch_history, &scratch_selected, None)
                .with_context(|| {
                    format!("could not compare selected changes at primary commit {oid}")
                })?;
            if merged.has_conflicts() {
                false
            } else {
                let mut options = git2::DiffOptions::new();
                options.ignore_filemode(false).ignore_submodules(false);
                let diff = comparison_repo
                    .diff_tree_to_index(Some(&scratch_history), Some(&merged), Some(&mut options))
                    .with_context(|| {
                        format!("could not compare merged changes at primary commit {oid}")
                    })?;
                diff.deltas().len() == 0
            }
        };
        mempack
            .reset()
            .context("could not clear temporary merge objects")?;
        if found {
            return Ok(PrimaryBranchCodeMatch::ContentEquivalent {
                commit_oid: oid.to_string(),
            });
        }
    }
    Ok(PrimaryBranchCodeMatch::NotFound)
}

fn configure_scratch_merge_attributes(
    repo: &Repository,
    odb: &git2::Odb<'_>,
) -> anyhow::Result<()> {
    // A synthetic index makes merge-driver selection deterministic even if the
    // user's worktree, global config, or info/attributes file chooses `union`.
    let attributes = b"* merge=text\n";
    let blob_oid = odb
        .write(git2::ObjectType::Blob, attributes)
        .context("could not write scratch merge attributes")?;
    let mut index = git2::Index::new().context("could not create scratch merge index")?;
    repo.set_index(&mut index)
        .context("could not install scratch merge attributes")?;
    index
        .add(&git2::IndexEntry {
            ctime: git2::IndexTime::new(0, 0),
            mtime: git2::IndexTime::new(0, 0),
            dev: 0,
            ino: 0,
            mode: 0o100644,
            uid: 0,
            gid: 0,
            file_size: attributes.len() as u32,
            id: blob_oid,
            flags: 0,
            flags_extended: 0,
            path: b".gitattributes".to_vec(),
        })
        .context("could not add scratch merge attributes")?;
    Ok(())
}

/// Detect if a branch was squash-merged into the base branch using git CLI.
/// Uses commit-tree + cherry to check if the branch's tree content already exists in base.
///
/// `merge_base` is the branch's already-known merge base with `base_branch` (the
/// value computed once from the in-memory reachable set in `fill_merge_base_commits`).
/// When `Some`, it is used directly and the per-call `git merge-base` subprocess is
/// skipped entirely — this avoids re-walking history for every candidate, which is
/// catastrophic on branches whose merge base is far back or absent (disjoint
/// histories force `git merge-base` to walk the full graph). When `None`, we fall
/// back to computing it via `git merge-base` (used by the remote path, which does
/// not precompute merge bases).
#[instrument(skip(repo_path))]
pub fn is_squash_merged(
    repo_path: &Path,
    base_branch: &str,
    branch_name: &str,
    commit_hash: Option<&str>,
    merge_base: Option<&str>,
) -> bool {
    let git = |args: &[&str]| -> Option<String> {
        let out = Command::new("git")
            .args(args)
            .current_dir(repo_path)
            .stdin(std::process::Stdio::null())
            .output()
            .ok()?;
        if out.status.success() {
            Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
        } else {
            None
        }
    };

    let branchish = commit_hash.unwrap_or(branch_name);

    // Step 1: find merge-base. Prefer the precomputed value; only shell out when absent.
    let ancestor = match merge_base {
        Some(mb) if !mb.is_empty() => mb.to_string(),
        _ => match git(&["merge-base", base_branch, branchish]) {
            Some(a) if !a.is_empty() => a,
            _ => return false,
        },
    };

    // Step 2: create temp commit-tree
    let tree_spec = format!("{branchish}^{{tree}}");
    let temp_commit = match git(&["commit-tree", &tree_spec, "-p", &ancestor, "-m", "_"]) {
        Some(c) if !c.is_empty() => c,
        _ => return false,
    };

    // Step 3: cherry check
    match git(&["cherry", base_branch, &temp_commit]) {
        Some(result) => result.starts_with('-'),
        None => false,
    }
}

/// True when every commit unique to `branch_name` relative to `merge_base`
/// has already landed in `base_branch` via individual cherry-picks.
///
/// Unlike `is_squash_merged`, this needs no synthetic `commit-tree`: `git
/// cherry <base> <branchish> <limit>` already walks every real commit between
/// `limit` (exclusive) and `branchish`, prefixing each line with `-` (its
/// patch-id is already reachable from `base_branch`) or `+` (it is not).
/// Every non-empty line must start with `-` for the branch to count as fully
/// cherry-picked; a single `+` (or any git failure) fails closed to `false`,
/// mirroring `is_squash_merged`'s fail-closed semantics.
#[instrument(skip(repo_path))]
pub fn is_cherry_picked(
    repo_path: &Path,
    base_branch: &str,
    branch_name: &str,
    commit_hash: Option<&str>,
    merge_base: Option<&str>,
) -> bool {
    let git = |args: &[&str]| -> Option<String> {
        let out = Command::new("git")
            .args(args)
            .current_dir(repo_path)
            .stdin(std::process::Stdio::null())
            .output()
            .ok()?;
        if out.status.success() {
            Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
        } else {
            None
        }
    };

    let branchish = commit_hash.unwrap_or(branch_name);

    let ancestor = match merge_base {
        Some(mb) if !mb.is_empty() => mb.to_string(),
        _ => match git(&["merge-base", base_branch, branchish]) {
            Some(a) if !a.is_empty() => a,
            _ => return false,
        },
    };

    match git(&["cherry", base_branch, branchish, &ancestor]) {
        Some(result) => {
            for line in result.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if !line.starts_with('-') {
                    return false;
                }
            }
            // Empty output (no lines to evaluate) means there are no commits
            // unique to the branch in the (tip, merge-base] window — every
            // commit reachable from branchish via that window has its
            // patch-id already in upstream. Counts as fully cherry-picked.
            true
        }
        None => false,
    }
}

/// Confirm that replaying `branchish` onto `base_branch` produces base's tree.
/// Fail closed on conflicts, missing refs, unsupported git, or other failures.
fn merge_tree_confirms(repo_path: &Path, base_branch: &str, branchish: &str) -> bool {
    let git = |args: &[&str]| -> Option<String> {
        let out = Command::new("git")
            .args(args)
            .current_dir(repo_path)
            .stdin(std::process::Stdio::null())
            .output()
            .ok()?;
        if out.status.success() {
            Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
        } else {
            None
        }
    };

    let base_tree = match git(&["rev-parse", &format!("{base_branch}^{{tree}}")]) {
        Some(t) if !t.is_empty() => t,
        _ => return false,
    };
    let written_tree = match git(&["merge-tree", "--write-tree", base_branch, branchish]) {
        Some(t) if !t.is_empty() => t,
        _ => return false,
    };
    written_tree == base_tree
}

/// Compute a raw diff compatible with `fuzzy_match::score`.
fn compute_diff(repo_path: &Path, merge_base: &str, refish: &str) -> Option<Vec<u8>> {
    let out = Command::new("git")
        .current_dir(repo_path)
        .args([
            "diff",
            "--binary",
            "--full-index",
            "--no-ext-diff",
            "--no-textconv",
            merge_base,
            refish,
            "--",
        ])
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() || out.stdout.is_empty() {
        return None;
    }
    Some(out.stdout)
}

/// Return a confidence classification for content that is likely already
/// represented in the base branch when exact patch-id matching is absent.
#[instrument(skip(repo_path))]
pub fn likely_squash_merged(
    repo_path: &Path,
    base_branch: &str,
    branch_name: &str,
    commit_hash: Option<&str>,
    merge_base: Option<&str>,
) -> Option<SquashConfidence> {
    let git = |args: &[&str]| -> Option<String> {
        let out = Command::new("git")
            .args(args)
            .current_dir(repo_path)
            .stdin(std::process::Stdio::null())
            .output()
            .ok()?;
        if out.status.success() {
            Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
        } else {
            None
        }
    };

    let branchish = commit_hash.unwrap_or(branch_name);
    let ancestor = match merge_base {
        Some(mb) if !mb.is_empty() => {
            match git(&["rev-parse", "--verify", &format!("{mb}^{{commit}}")]) {
                Some(a) if !a.is_empty() => a,
                _ => return None,
            }
        }
        _ => match git(&["merge-base", base_branch, branchish]) {
            Some(a) if !a.is_empty() => a,
            _ => return None,
        },
    };

    if merge_tree_confirms(repo_path, base_branch, branchish) {
        return Some(SquashConfidence::MergeTreeConfirmed);
    }

    let branch_diff = compute_diff(repo_path, &ancestor, branchish)?;
    let base_diff = compute_diff(repo_path, &ancestor, base_branch)?;
    let fscore = fuzzy_match::score(&branch_diff, &base_diff)?;
    let similarity_percent = fuzzy_match::classify(&fscore)?;
    Some(SquashConfidence::FuzzyMatch { similarity_percent })
}
