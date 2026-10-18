use super::{check_cancel, ResolvedPrimaryRef};
use crate::types::PrimaryBranchCodeMatch;
use anyhow::Context;
use git2::Repository;
use std::collections::HashSet;
use std::sync::atomic::AtomicBool;

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
