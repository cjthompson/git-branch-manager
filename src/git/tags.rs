use std::path::Path;
use std::process::Command;

use chrono::{DateTime, TimeZone, Utc};
use git2::Repository;

use crate::types::{BranchAction, OperationResult};

/// Information about a single git tag.
#[derive(Debug, Clone)]
pub struct TagInfo {
    pub name: String,
    pub commit_hash: String,
    pub date: DateTime<Utc>,
    pub message: Option<String>,
}

impl TagInfo {
    /// Human-readable age string: "3 days ago", "2 months ago", etc.
    pub fn age_display(&self) -> String {
        let duration = Utc::now() - self.date;
        let seconds = duration.num_seconds();

        if seconds < 60 {
            "just now".to_string()
        } else if seconds < 3600 {
            let mins = duration.num_minutes();
            format!("{} min{} ago", mins, if mins == 1 { "" } else { "s" })
        } else if seconds < 86400 {
            let hours = duration.num_hours();
            format!("{} hour{} ago", hours, if hours == 1 { "" } else { "s" })
        } else if seconds < 604800 {
            let days = duration.num_days();
            format!("{} day{} ago", days, if days == 1 { "" } else { "s" })
        } else if seconds < 2_592_000 {
            let weeks = duration.num_weeks();
            format!("{} week{} ago", weeks, if weeks == 1 { "" } else { "s" })
        } else if seconds < 31_536_000 {
            let months = duration.num_days() / 30;
            format!("{} month{} ago", months, if months == 1 { "" } else { "s" })
        } else {
            let years = duration.num_days() / 365;
            format!("{} year{} ago", years, if years == 1 { "" } else { "s" })
        }
    }

    /// Compact age string for narrow terminals: "3d", "2mo", etc.
    pub fn age_short(&self) -> String {
        let duration = Utc::now() - self.date;
        let seconds = duration.num_seconds();

        if seconds < 60 {
            "now".into()
        } else if seconds < 3600 {
            format!("{}m", duration.num_minutes())
        } else if seconds < 86400 {
            format!("{}h", duration.num_hours())
        } else if seconds < 604800 {
            format!("{}d", duration.num_days())
        } else if seconds < 2_592_000 {
            format!("{}w", duration.num_weeks())
        } else if seconds < 31_536_000 {
            format!("{}mo", duration.num_days() / 30)
        } else {
            format!("{}y", duration.num_days() / 365)
        }
    }
}

/// List all tags in the repository using git2.
/// Returns tags sorted by date descending (newest first).
pub fn list_tags(repo: &Repository) -> Vec<TagInfo> {
    let mut tags = Vec::new();

    let Ok(tag_names) = repo.tag_names(None) else {
        return tags;
    };

    for name in tag_names.iter().flatten() {
        let refname = format!("refs/tags/{}", name);
        let Ok(reference) = repo.find_reference(&refname) else {
            continue;
        };

        // Resolve to the target object — could be a tag object (annotated) or a commit (lightweight)
        let Ok(obj) = reference.peel(git2::ObjectType::Commit) else {
            continue;
        };

        let Ok(commit) = obj.peel_to_commit() else {
            continue;
        };

        let commit_hash = commit.id().to_string();
        let time = commit.time();
        let date = Utc
            .timestamp_opt(time.seconds(), 0)
            .single()
            .unwrap_or_else(Utc::now);

        // Check if this is an annotated tag with a message
        let message = if let Ok(tag_obj) = reference.peel(git2::ObjectType::Tag) {
            tag_obj
                .as_tag()
                .and_then(|t| t.message().map(|m| m.trim().to_string()))
        } else {
            None
        };

        tags.push(TagInfo {
            name: name.to_string(),
            commit_hash,
            date,
            message,
        });
    }

    // Sort by date descending (newest first)
    tags.sort_by(|a, b| b.date.cmp(&a.date));
    tags
}

/// Delete a local tag using git2.
pub fn delete_tag(repo: &Repository, tag_name: &str) -> OperationResult {
    match repo.tag_delete(tag_name) {
        Ok(()) => OperationResult {
            branch_name: tag_name.to_string(),
            action: BranchAction::DeleteTag,
            success: true,
            message: format!("Deleted tag {}", tag_name),
        },
        Err(e) => OperationResult {
            branch_name: tag_name.to_string(),
            action: BranchAction::DeleteTag,
            success: false,
            message: format!("Failed to delete tag: {}", e),
        },
    }
}

/// Push a tag to the remote using git CLI.
pub fn push_tag(repo_path: &Path, tag_name: &str) -> OperationResult {
    let mut cmd = Command::new("git");
    cmd.current_dir(repo_path)
        .stdin(std::process::Stdio::null())
        .env("GIT_TERMINAL_PROMPT", "0");
    match cmd
        .args(["push", "origin", tag_name])
        .output()
    {
        Ok(o) if o.status.success() => OperationResult {
            branch_name: tag_name.to_string(),
            action: BranchAction::PushTag,
            success: true,
            message: format!("Pushed tag {} to origin", tag_name),
        },
        Ok(o) => OperationResult {
            branch_name: tag_name.to_string(),
            action: BranchAction::PushTag,
            success: false,
            message: format!(
                "Push failed: {}",
                String::from_utf8_lossy(&o.stderr).trim()
            ),
        },
        Err(e) => OperationResult {
            branch_name: tag_name.to_string(),
            action: BranchAction::PushTag,
            success: false,
            message: format!("Failed to run git: {}", e),
        },
    }
}
