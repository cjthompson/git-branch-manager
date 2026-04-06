use crate::types::{BranchAction, OperationResult, TagInfo};
use chrono::{TimeZone, Utc};
use git2::{ObjectType, Repository};
use std::path::Path;
use std::process::{Command, Stdio};

/// List all tags in the repository sorted by date descending.
/// Annotated tags include the tag message; lightweight tags have message = None.
pub fn list_tags(repo: &Repository) -> Vec<TagInfo> {
    let tag_names = match repo.tag_names(None) {
        Ok(names) => names,
        Err(_) => return vec![],
    };

    let mut tags: Vec<TagInfo> = tag_names
        .iter()
        .flatten()
        .filter_map(|name| {
            let ref_name = format!("refs/tags/{name}");
            let reference = repo.find_reference(&ref_name).ok()?;

            // Check the direct target to determine if annotated.
            // Annotated tags point to a tag object; lightweight tags point to a commit.
            let target_oid = reference.target()?;
            let target_obj = repo.find_object(target_oid, None).ok()?;

            let (is_annotated, message, date, commit_hash) =
                if target_obj.kind() == Some(ObjectType::Tag) {
                    // Annotated tag
                    let tag = repo.find_tag(target_oid).ok()?;
                    let msg = tag.message().map(|m| m.trim().to_string());
                    let commit_obj = reference.peel(ObjectType::Commit).ok()?;
                    let commit = commit_obj.as_commit()?;
                    let time = commit.committer().when();
                    let date = Utc.timestamp_opt(time.seconds(), 0).single()?;
                    let hash = commit.id().to_string();
                    (true, msg, date, hash)
                } else {
                    // Lightweight tag: directly points to a commit
                    let commit = reference.peel(ObjectType::Commit).ok()?;
                    let commit = commit.as_commit()?;
                    let time = commit.committer().when();
                    let date = Utc.timestamp_opt(time.seconds(), 0).single()?;
                    let hash = commit.id().to_string();
                    (false, None, date, hash)
                };

            Some(TagInfo {
                name: name.to_string(),
                commit_hash: commit_hash[..7.min(commit_hash.len())].to_string(),
                date,
                message,
                is_annotated,
            })
        })
        .collect();

    tags.sort_by(|a, b| b.date.cmp(&a.date));
    tags
}

/// Delete a local tag.
pub fn delete_tag(repo: &Repository, tag_name: &str) -> OperationResult {
    match repo.tag_delete(tag_name) {
        Ok(()) => OperationResult {
            branch_name: tag_name.to_string(),
            action: BranchAction::DeleteTag,
            success: true,
            message: format!("Deleted tag {tag_name}"),
        },
        Err(e) => OperationResult {
            branch_name: tag_name.to_string(),
            action: BranchAction::DeleteTag,
            success: false,
            message: format!("Failed: {e}"),
        },
    }
}

/// Batch delete local tags.
pub fn delete_tags_batch(repo: &Repository, tag_names: &[String]) -> Vec<OperationResult> {
    tag_names.iter().map(|name| delete_tag(repo, name)).collect()
}

/// Batch delete remote tags with fallback to individual deletes.
pub fn delete_remote_tags_batch(repo_path: &Path, tag_names: &[String]) -> Vec<OperationResult> {
    if tag_names.is_empty() {
        return vec![];
    }

    let mut args = vec!["push", "origin", "--delete"];
    let refs: Vec<&str> = tag_names.iter().map(|s| s.as_str()).collect();
    args.extend(&refs);

    let out = Command::new("git")
        .args(&args)
        .current_dir(repo_path)
        .stdin(Stdio::null())
        .env("GIT_TERMINAL_PROMPT", "0")
        .output();

    if matches!(&out, Ok(o) if o.status.success()) {
        return tag_names
            .iter()
            .map(|name| OperationResult {
                branch_name: name.clone(),
                action: BranchAction::DeleteTagAndRemote,
                success: true,
                message: format!("Deleted remote tag {name}"),
            })
            .collect();
    }

    // Fallback to individual deletes
    tag_names
        .iter()
        .map(|name| {
            let out = Command::new("git")
                .args(["push", "origin", "--delete", name])
                .current_dir(repo_path)
                .stdin(Stdio::null())
                .env("GIT_TERMINAL_PROMPT", "0")
                .output();

            match out {
                Ok(o) if o.status.success() => OperationResult {
                    branch_name: name.clone(),
                    action: BranchAction::DeleteTagAndRemote,
                    success: true,
                    message: format!("Deleted remote tag {name}"),
                },
                _ => OperationResult {
                    branch_name: name.clone(),
                    action: BranchAction::DeleteTagAndRemote,
                    success: false,
                    message: format!("Failed to delete remote tag {name}"),
                },
            }
        })
        .collect()
}

/// Push a tag to the remote.
pub fn push_tag(repo_path: &Path, tag_name: &str) -> OperationResult {
    let out = Command::new("git")
        .args(["push", "origin", tag_name])
        .current_dir(repo_path)
        .stdin(Stdio::null())
        .env("GIT_TERMINAL_PROMPT", "0")
        .output();

    match out {
        Ok(o) if o.status.success() => OperationResult {
            branch_name: tag_name.to_string(),
            action: BranchAction::PushTag,
            success: true,
            message: format!("Pushed tag {tag_name}"),
        },
        Ok(o) => OperationResult {
            branch_name: tag_name.to_string(),
            action: BranchAction::PushTag,
            success: false,
            message: String::from_utf8_lossy(&o.stderr).trim().to_string(),
        },
        Err(e) => OperationResult {
            branch_name: tag_name.to_string(),
            action: BranchAction::PushTag,
            success: false,
            message: e.to_string(),
        },
    }
}
