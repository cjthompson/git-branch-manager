use super::*;

#[test]
fn graph_commit_default_has_empty_author_fields_and_unknown_date() {
    let c = GraphCommit::default();
    assert_eq!(c.author_name, "");
    assert_eq!(c.author_email, "");
    assert_eq!(c.authored_at, None);
}

#[test]
fn git_cli_loader_parses_author_and_author_date_from_format() {
    use crate::git::graph::{load_graph_with_squash_annotations, GraphLoadOptions};

    // Build a repo where gleisbau fails (shallow). Forces CLI fallback.
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    std::process::Command::new("git")
        .args(["init", "-q", "-b", "main"])
        .current_dir(dir)
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["config", "user.email", "cli@example.com"])
        .current_dir(dir)
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["config", "user.name", "CLI Bot"])
        .current_dir(dir)
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["commit", "--allow-empty", "-q", "-m", "tip"])
        .current_dir(dir)
        .output()
        .unwrap();
    let head = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir)
        .output()
        .unwrap();
    let head_oid = String::from_utf8(head.stdout).unwrap().trim().to_string();
    std::fs::write(dir.join(".git/shallow"), format!("{head_oid}\n")).unwrap();

    let test_cache = tempfile::tempdir().unwrap();
    let snapshot = load_graph_with_squash_annotations(
        dir,
        GraphLoadOptions {
            cache_root: CacheRoot::at(test_cache.path()),
            ..GraphLoadOptions::default()
        },
    )
    .unwrap();
    assert!(matches!(
        snapshot.source,
        crate::git::graph::GraphSource::GitCliFallback { .. }
    ));
    let tip = snapshot.commits.last().expect("graph non-empty");
    assert_eq!(tip.author_name, "CLI Bot");
    assert_eq!(tip.author_email, "cli@example.com");
    assert!(tip.authored_at.is_some());
}

#[test]
fn gleisbau_loader_populates_author_and_author_date() {
    use crate::git::graph::{load_graph_with_squash_annotations, GraphLoadOptions};

    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let init = std::process::Command::new("git")
        .args(["init", "-q", "-b", "main"])
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(init.status.success(), "git init failed");
    std::process::Command::new("git")
        .args(["config", "user.email", "gleisbau@example.com"])
        .current_dir(dir)
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["config", "user.name", "Gleisbau Bot"])
        .current_dir(dir)
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["commit", "--allow-empty", "-q", "-m", "tip"])
        .current_dir(dir)
        .output()
        .unwrap();

    let test_cache = tempfile::tempdir().unwrap();
    let snapshot = load_graph_with_squash_annotations(
        dir,
        GraphLoadOptions {
            cache_root: CacheRoot::at(test_cache.path()),
            ..GraphLoadOptions::default()
        },
    )
    .unwrap();
    assert!(matches!(
        snapshot.source,
        crate::git::graph::GraphSource::Gleisbau
    ));
    let tip = snapshot.commits.last().expect("graph non-empty");
    assert_eq!(tip.author_name, "Gleisbau Bot");
    assert_eq!(tip.author_email, "gleisbau@example.com");
    assert!(tip.authored_at.is_some());
}

#[test]
fn git_cli_loader_preserves_control_characters_in_author_fields() {
    use crate::git::graph::{load_graph_with_squash_annotations, GraphLoadOptions};

    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let init = std::process::Command::new("git")
        .args(["init", "-q", "-b", "main"])
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(init.status.success(), "git init failed");

    let author_name = "CLI\x1fBot";
    let author_email = "cli\x1f@example.com";
    for (key, value) in [("user.email", author_email), ("user.name", author_name)] {
        let configured = std::process::Command::new("git")
            .args(["config", key, value])
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(configured.status.success(), "git config {key} failed");
    }
    let committed = std::process::Command::new("git")
        .args(["commit", "--allow-empty", "-q", "-m", "tip"])
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(committed.status.success(), "git commit failed");

    let head = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir)
        .output()
        .unwrap();
    let head_oid = String::from_utf8(head.stdout).unwrap().trim().to_string();
    std::fs::write(dir.join(".git/shallow"), format!("{head_oid}\n")).unwrap();

    let test_cache = tempfile::tempdir().unwrap();
    let snapshot = load_graph_with_squash_annotations(
        dir,
        GraphLoadOptions {
            cache_root: CacheRoot::at(test_cache.path()),
            ..GraphLoadOptions::default()
        },
    )
    .unwrap();
    assert!(matches!(
        snapshot.source,
        crate::git::graph::GraphSource::GitCliFallback { .. }
    ));
    let tip = snapshot.commits.last().expect("graph non-empty");
    assert_eq!(tip.author_name, author_name);
    assert_eq!(tip.author_email, author_email);
}

#[test]
fn round_line_style_uses_gleisbau_round_characters() {
    use gleisbau::settings::Characters;

    let settings = gleisbau_settings(false, GraphLineStyle::Round, None).unwrap();

    assert_eq!(settings.characters.chars, Characters::round().chars);
}

#[test]
fn gleisbau_settings_with_base_branch_puts_base_in_order_group_zero() {
    let settings = gleisbau_settings(false, GraphLineStyle::Thin, Some("main")).unwrap();

    assert_eq!(settings.branches.order[0].as_str(), "^main$");
}

#[test]
fn gleisbau_settings_without_base_branch_keeps_none_semantics() {
    let settings = gleisbau_settings(false, GraphLineStyle::Thin, None).unwrap();

    assert!(settings.branches.order.is_empty());
}

#[test]
fn gleisbau_settings_with_special_chars_escapes_base_branch_name() {
    let settings =
        gleisbau_settings(false, GraphLineStyle::Thin, Some("release/1.0")).unwrap();

    let pattern = &settings.branches.order[0];
    assert!(pattern.is_match("release/1.0"));
    assert!(!pattern.is_match("release/10"));
}

#[test]
fn symbol_sets_only_select_rounded_lines_for_powerline() {
    assert_eq!(
        GraphLineStyle::from_symbol_name("powerline"),
        GraphLineStyle::Round
    );
    assert_eq!(
        GraphLineStyle::from_symbol_name("unicode"),
        GraphLineStyle::Thin
    );
    assert_eq!(
        GraphLineStyle::from_symbol_name("ascii"),
        GraphLineStyle::Thin
    );
}
