use std::path::Path;

use anyhow::{Context, bail};
use git2::{BranchType, Repository, Sort};

/// Build a squash commit message from commits reachable from `branch_name` but
/// not from `base`, in oldest-first ancestry order.
pub(super) fn squash_commit_message(
    repo_path: &Path,
    branch_name: &str,
    base: &str,
) -> anyhow::Result<String> {
    let repo = Repository::open(repo_path).context("failed to open repository")?;
    let branch = repo
        .find_branch(branch_name, BranchType::Local)
        .with_context(|| format!("failed to find local branch {branch_name}"))?;
    let base_branch = repo
        .find_branch(base, BranchType::Local)
        .with_context(|| format!("failed to find local base branch {base}"))?;
    let branch_oid = branch
        .get()
        .target()
        .with_context(|| format!("local branch {branch_name} has no target"))?;
    let base_oid = base_branch
        .get()
        .target()
        .with_context(|| format!("local base branch {base} has no target"))?;

    let mut walk = repo.revwalk().context("failed to create commit walk")?;
    walk.push(branch_oid)
        .context("failed to walk source branch")?;
    walk.hide(base_oid)
        .context("failed to exclude base commits")?;
    walk.set_sorting(Sort::TOPOLOGICAL | Sort::REVERSE)
        .context("failed to order source commits")?;

    let mut messages = Vec::new();
    let mut first_coauthor = None;
    for oid in walk {
        let oid = oid.context("failed to read source commit id")?;
        let commit = repo
            .find_commit(oid)
            .context("failed to read source commit")?;
        let message = commit
            .message()
            .with_context(|| format!("source commit {oid} has an invalid message"))?;
        let (message, coauthor) = clean_message(message);
        if first_coauthor.is_none() {
            first_coauthor = coauthor;
        }
        if message.is_empty() {
            bail!("source commit {oid} has an empty message");
        }
        messages.push(message);
    }

    if messages.is_empty() {
        bail!("branch {branch_name} has no commits beyond {base}");
    }

    let mut result = messages.join("\n\n");
    if let Some(coauthor) = first_coauthor {
        result.push_str("\n\nCo-Authored-By: ");
        result.push_str(&coauthor);
    }
    Ok(result)
}

fn clean_message(message: &str) -> (String, Option<String>) {
    let mut lines = Vec::new();
    let mut first_coauthor = None;
    for line in message.lines() {
        let trimmed = line.trim_start();
        if trimmed
            .get(.."Co-Authored-By:".len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("Co-Authored-By:"))
        {
            let value = trimmed["Co-Authored-By:".len()..].trim();
            if !value.is_empty() && first_coauthor.is_none() {
                first_coauthor = Some(value.to_string());
            }
        } else {
            lines.push(line);
        }
    }

    let start = lines.iter().position(|line| !line.trim().is_empty());
    let end = lines.iter().rposition(|line| !line.trim().is_empty());
    let cleaned = match (start, end) {
        (Some(start), Some(end)) => lines[start..=end].join("\n"),
        _ => String::new(),
    };
    (cleaned, first_coauthor)
}

#[cfg(test)]
mod tests {
    use super::clean_message;

    #[test]
    fn strips_coauthors_and_outer_blank_lines_but_preserves_other_text() {
        let message = "\n  Subject\n\n  indented body\n# literal\n  cO-aUtHoReD-bY: First <first@example.com>\nCo-Authored-By: Second <second@example.com>\n\n";
        assert_eq!(
            clean_message(message),
            (
                "  Subject\n\n  indented body\n# literal".to_string(),
                Some("First <first@example.com>".to_string())
            )
        );
    }

    #[test]
    fn no_coauthor_remains_absent() {
        assert_eq!(
            clean_message("Subject\n\nbody\n"),
            ("Subject\n\nbody".into(), None)
        );
    }
}
