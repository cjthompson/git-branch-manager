mod primary_code;
mod reachable;
mod squash;

pub use primary_code::check_ref_code_in_primary;
pub use reachable::{
    apply_merge_statuses, build_reachable_set, build_reachable_set_from_repo,
    detect_merged_branches, BaseReachable,
};
pub use squash::{is_cherry_picked, is_squash_merged, likely_squash_merged};

use anyhow::Context;
use git2::{ErrorCode, Repository};
use std::sync::atomic::{AtomicBool, Ordering};

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
