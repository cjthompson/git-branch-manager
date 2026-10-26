//! Remote from git config; never assumes a name, and ignores a value naming no remote.

use std::path::Path;

use git2::Repository;

use crate::types::{BranchAction, FailureCause, OperationResult};

/// Outcome of choosing a remote for a push or a tag operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushTarget {
    /// Push to `remote`; `set_upstream` means the push must record tracking config.
    Resolved { remote: String, set_upstream: bool },
    /// Several remotes and none configured. Picking one is a separate task.
    NeedsChoice(Vec<String>),
    /// No remotes are defined.
    NoRemote,
}

fn remote_names(repo: &Repository) -> Vec<String> {
    let Ok(names) = repo.remotes() else {
        return Vec::new();
    };
    (0..names.len())
        .filter_map(|index| names.get(index).ok().flatten().map(str::to_owned))
        .collect()
}

fn configured_remote(repo: &Repository, key: &str, names: &[String]) -> Option<String> {
    let value = repo.config().ok()?.get_string(key).ok()?;
    names.contains(&value).then_some(value)
}

fn upstream_remote(repo: &Repository, branch: &str, names: &[String]) -> Option<String> {
    configured_remote(repo, &format!("branch.{branch}.remote"), names)
}

fn push_default_remote(repo: &Repository, names: &[String]) -> Option<String> {
    configured_remote(repo, "remote.pushDefault", names)
}

/// Remote whose `branch` is deleted: the branch's configured upstream, and no other.
pub fn resolve_delete(repo: &Repository, branch: &str) -> Result<String, String> {
    let names = remote_names(repo);
    if names.is_empty() {
        return Err(format!(
            "Cannot delete remote branch {branch}: no remotes are configured"
        ));
    }
    upstream_remote(repo, branch, &names).ok_or_else(|| {
        format!(
            "Cannot delete remote branch {branch}: no upstream is configured (branch.{branch}.remote), so the remote branch is left alone"
        )
    })
}

/// Delete target from an explicit remote when given, else from the branch's upstream.
pub fn resolve_delete_for(
    repo: &Repository,
    explicit: Option<&str>,
    branch: &str,
) -> Result<String, String> {
    match explicit {
        Some(remote) => Ok(remote.to_owned()),
        None => resolve_delete(repo, branch),
    }
}

/// Remote a branch is pushed to: upstream, then `remote.pushDefault`, then the only remote.
pub fn resolve_push(repo: &Repository, branch: &str) -> PushTarget {
    let names = remote_names(repo);
    if names.is_empty() {
        return PushTarget::NoRemote;
    }
    if let Some(remote) = upstream_remote(repo, branch, &names) {
        return PushTarget::Resolved {
            remote,
            set_upstream: false,
        };
    }
    if let Some(remote) = push_default_remote(repo, &names) {
        return PushTarget::Resolved {
            remote,
            set_upstream: true,
        };
    }
    if let [only] = names.as_slice() {
        return PushTarget::Resolved {
            remote: only.clone(),
            set_upstream: true,
        };
    }
    PushTarget::NeedsChoice(names)
}

/// Remote for tag push and delete: `remote.pushDefault`, then the only remote.
pub fn resolve_tag(repo: &Repository) -> PushTarget {
    let names = remote_names(repo);
    if names.is_empty() {
        return PushTarget::NoRemote;
    }
    if let Some(remote) = push_default_remote(repo, &names) {
        return PushTarget::Resolved {
            remote,
            set_upstream: false,
        };
    }
    if let [only] = names.as_slice() {
        return PushTarget::Resolved {
            remote: only.clone(),
            set_upstream: false,
        };
    }
    PushTarget::NeedsChoice(names)
}

/// Remote a branch is fetched from: upstream, then the only remote. Read-only.
pub fn resolve_fetch(repo: &Repository, branch: &str) -> Result<String, String> {
    let names = remote_names(repo);
    match names.as_slice() {
        [] => Err(format!("Cannot fetch {branch}: no remotes are configured")),
        _ => upstream_remote(repo, branch, &names)
            .or_else(|| match names.as_slice() {
                [only] => Some(only.clone()),
                _ => None,
            })
            .ok_or_else(|| needs_choice_message(&format!("fetch {branch}"), &names)),
    }
}

fn needs_choice_message(subject: &str, names: &[String]) -> String {
    format!(
        "Cannot {subject}: several remotes are configured ({}) and none is set for it; a remote must be chosen",
        names.join(", ")
    )
}

/// `(remote, set_upstream)` for a push or tag action, or the message explaining why it cannot run.
pub fn require_resolved(
    target: Result<PushTarget, String>,
    subject: &str,
) -> Result<(String, bool), String> {
    match target {
        Ok(PushTarget::Resolved {
            remote,
            set_upstream,
        }) => Ok((remote, set_upstream)),
        Ok(PushTarget::NeedsChoice(names)) => Err(needs_choice_message(subject, &names)),
        Ok(PushTarget::NoRemote) => Err(format!("Cannot {subject}: no remotes are configured")),
        Err(error) => Err(error),
    }
}

/// Push target for `branch`, read from the repository at `repo_path`.
pub fn push_target(repo_path: &Path, branch: &str) -> Result<PushTarget, String> {
    let repo = Repository::open(repo_path).map_err(|e| format!("Failed to open repo: {e}"))?;
    Ok(resolve_push(&repo, branch))
}

/// Fetch remote for `branch`, read from the repository at `repo_path`.
pub fn fetch_target(repo_path: &Path, branch: &str) -> Result<String, String> {
    let repo = Repository::open(repo_path).map_err(|e| format!("Failed to open repo: {e}"))?;
    resolve_fetch(&repo, branch)
}

/// Tag push/delete target for the repository at `repo_path`.
pub fn tag_target(repo_path: &Path) -> Result<PushTarget, String> {
    let repo = Repository::open(repo_path).map_err(|e| format!("Failed to open repo: {e}"))?;
    Ok(resolve_tag(&repo))
}

/// Failure result for an action that cannot run because no remote was resolved.
pub fn unresolved_result(name: &str, action: BranchAction, message: String) -> OperationResult {
    OperationResult::failure(
        name,
        action,
        FailureCause::Other {
            raw_message: message.clone(),
        },
        message,
    )
}

/// Failure result for a remote-only action invoked without the remote it names.
pub fn remote_required(name: &str, action: BranchAction) -> OperationResult {
    unresolved_result(
        name,
        action,
        format!("No remote specified for {}", action.label()),
    )
}

#[cfg(test)]
mod tests;
