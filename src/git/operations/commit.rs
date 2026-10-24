use std::path::Path;
use std::process::Stdio;

use crate::types::{BranchAction, OperationResult};

use super::git_cmd;

/// Cherry-pick one immutable commit object onto the current checkout.
pub fn cherry_pick_commit(repo_path: &Path, selected_oid: &str) -> OperationResult {
    let fail = |message: String| OperationResult {
        branch_name: selected_oid.to_string(),
        action: BranchAction::CherryPickCommit,
        success: false,
        message,
        failure: None,
    };

    let repo = match git2::Repository::open(repo_path) {
        Ok(repo) => repo,
        Err(error) => return fail(format!("Failed to open repository: {error}")),
    };
    if repo.state() != git2::RepositoryState::Clean {
        return fail(format!(
            "Cannot cherry-pick while another Git operation is in progress ({:?})",
            repo.state()
        ));
    }
    let mut status_options = git2::StatusOptions::new();
    status_options
        .include_untracked(true)
        .recurse_untracked_dirs(true);
    match repo.statuses(Some(&mut status_options)) {
        Ok(statuses) if statuses.is_empty() => {}
        Ok(_) => return fail("Cannot cherry-pick with a dirty index or working tree".into()),
        Err(error) => return fail(format!("Failed to inspect working tree: {error}")),
    }

    let oid = match git2::Oid::from_str(selected_oid) {
        Ok(oid) if oid.to_string().eq_ignore_ascii_case(selected_oid) => oid,
        _ => return fail(format!("Invalid full commit OID: {selected_oid}")),
    };
    let commit = match repo.find_commit(oid) {
        Ok(commit) => commit,
        Err(error) => return fail(format!("Commit {selected_oid} is unavailable: {error}")),
    };
    if commit.parent_count() > 1 {
        return fail(format!(
            "Cannot cherry-pick merge commit {selected_oid}: requires mainline selection"
        ));
    }

    let output = git_cmd(repo_path)
        .arg("cherry-pick")
        .arg(selected_oid)
        .stdin(Stdio::null())
        .output();
    match output {
        Ok(output) if output.status.success() => OperationResult::success(
            selected_oid,
            BranchAction::CherryPickCommit,
            format!("Cherry-picked commit {selected_oid}"),
        ),
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            if git2::Repository::open(repo_path)
                .is_ok_and(|repo| repo.state() == git2::RepositoryState::CherryPick)
            {
                let abort = git_cmd(repo_path).args(["cherry-pick", "--abort"]).output();
                let abort_note = match abort {
                    Ok(abort) if abort.status.success() => "Cherry-pick aborted.".to_string(),
                    Ok(abort) => format!(
                        "Cherry-pick abort failed: {}",
                        String::from_utf8_lossy(&abort.stderr).trim()
                    ),
                    Err(error) => format!("Cherry-pick abort failed: {error}"),
                };
                fail(format!("Cherry-pick failed: {stderr}\n{abort_note}"))
            } else if stderr.is_empty() {
                fail(format!("Cherry-pick failed with status {}", output.status))
            } else {
                fail(stderr)
            }
        }
        Err(error) => fail(format!("Failed to run git cherry-pick: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("temp repo");
        git(dir.path(), &["init", "-b", "main"]);
        git(dir.path(), &["config", "user.email", "test@example.com"]);
        git(dir.path(), &["config", "user.name", "Test User"]);
        std::fs::write(dir.path().join("base.txt"), "base\n").unwrap();
        git(dir.path(), &["add", "base.txt"]);
        git(dir.path(), &["commit", "-m", "base"]);
        dir
    }

    #[test]
    fn rejects_dirty_worktree_without_touching_user_changes() {
        let dir = repo();
        let selected = git(dir.path(), &["rev-parse", "HEAD"]);
        std::fs::write(dir.path().join("base.txt"), "user change\n").unwrap();

        let result = cherry_pick_commit(dir.path(), &selected);

        assert!(!result.success);
        assert!(result.message.contains("dirty index or working tree"));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("base.txt")).unwrap(),
            "user change\n"
        );
    }

    #[test]
    fn rejects_merge_commit_without_changing_checkout() {
        let dir = repo();
        git(dir.path(), &["checkout", "-b", "feature"]);
        std::fs::write(dir.path().join("feature.txt"), "feature\n").unwrap();
        git(dir.path(), &["add", "feature.txt"]);
        git(dir.path(), &["commit", "-m", "feature"]);
        git(dir.path(), &["checkout", "main"]);
        std::fs::write(dir.path().join("main.txt"), "main\n").unwrap();
        git(dir.path(), &["add", "main.txt"]);
        git(dir.path(), &["commit", "-m", "main change"]);
        git(dir.path(), &["merge", "--no-ff", "-m", "merge", "feature"]);
        let merge_oid = git(dir.path(), &["rev-parse", "HEAD"]);

        let result = cherry_pick_commit(dir.path(), &merge_oid);

        assert!(!result.success);
        assert!(result.message.contains("requires mainline selection"));
        assert_eq!(git(dir.path(), &["rev-parse", "HEAD"]), merge_oid);
    }

    #[test]
    fn preserves_an_existing_git_operation() {
        let dir = repo();
        git(dir.path(), &["checkout", "-b", "feature"]);
        std::fs::write(dir.path().join("feature.txt"), "feature\n").unwrap();
        git(dir.path(), &["add", "feature.txt"]);
        git(dir.path(), &["commit", "-m", "feature"]);
        let selected = git(dir.path(), &["rev-parse", "HEAD"]);
        git(dir.path(), &["checkout", "main"]);
        std::fs::write(dir.path().join("main.txt"), "main\n").unwrap();
        git(dir.path(), &["add", "main.txt"]);
        git(dir.path(), &["commit", "-m", "main change"]);
        git(dir.path(), &["merge", "--no-commit", "feature"]);

        let result = cherry_pick_commit(dir.path(), &selected);

        assert!(!result.success);
        assert!(result
            .message
            .contains("another Git operation is in progress"));
        assert!(!git(dir.path(), &["rev-parse", "--verify", "MERGE_HEAD"]).is_empty());
    }

    #[test]
    fn cherry_picks_a_root_commit() {
        let dir = repo();
        let blob = Command::new("git")
            .args(["hash-object", "-w", "--stdin"])
            .current_dir(dir.path())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("start git hash-object");
        use std::io::Write;
        let mut blob = blob;
        blob.stdin
            .as_mut()
            .unwrap()
            .write_all(b"root content\n")
            .unwrap();
        let blob = blob.wait_with_output().unwrap();
        assert!(blob.status.success());
        let blob_oid = String::from_utf8_lossy(&blob.stdout).trim().to_string();
        let tree = Command::new("git")
            .args(["mktree"])
            .current_dir(dir.path())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("start git mktree");
        let mut tree = tree;
        writeln!(
            tree.stdin.as_mut().unwrap(),
            "100644 blob {blob_oid}\troot.txt"
        )
        .unwrap();
        let tree = tree.wait_with_output().unwrap();
        assert!(tree.status.success());
        let tree_oid = String::from_utf8_lossy(&tree.stdout).trim().to_string();
        let root_oid = git(
            dir.path(),
            &["commit-tree", &tree_oid, "-m", "independent root"],
        );

        let result = cherry_pick_commit(dir.path(), &root_oid);

        assert!(result.success, "{result:?}");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("root.txt")).unwrap(),
            "root content\n"
        );
    }

    #[test]
    fn rejects_invalid_full_oid_without_changing_head() {
        let dir = repo();
        let head = git(dir.path(), &["rev-parse", "HEAD"]);

        let result = cherry_pick_commit(dir.path(), "0000000000000000000000000000000000000000");

        assert!(!result.success);
        assert!(result.message.contains("unavailable"));
        assert_eq!(git(dir.path(), &["rev-parse", "HEAD"]), head);
    }

    #[test]
    fn aborts_its_conflict_and_preserves_the_pre_pick_checkout() {
        let dir = repo();
        git(dir.path(), &["checkout", "-b", "feature"]);
        std::fs::write(dir.path().join("collision.txt"), "feature content\n").unwrap();
        git(dir.path(), &["add", "collision.txt"]);
        git(dir.path(), &["commit", "-m", "feature adds collision"]);
        let selected = git(dir.path(), &["rev-parse", "HEAD"]);
        git(dir.path(), &["checkout", "main"]);
        std::fs::write(dir.path().join("collision.txt"), "current content\n").unwrap();
        git(dir.path(), &["add", "collision.txt"]);
        git(dir.path(), &["commit", "-m", "current adds collision"]);
        let pre_pick_head = git(dir.path(), &["rev-parse", "HEAD"]);

        let result = cherry_pick_commit(dir.path(), &selected);

        assert!(!result.success);
        assert!(result.message.contains("Cherry-pick failed"));
        assert!(result.message.contains("Cherry-pick aborted"), "{result:?}");
        assert_eq!(git(dir.path(), &["rev-parse", "HEAD"]), pre_pick_head);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("collision.txt")).unwrap(),
            "current content\n"
        );
        assert_eq!(
            git2::Repository::open(dir.path()).unwrap().state(),
            git2::RepositoryState::Clean
        );
    }
}
