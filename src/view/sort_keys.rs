use super::column::ColumnDef;
use super::ViewItem;
use crate::config::Config;

/// Find the column index for a given key string.
pub fn index_for_key<T: ViewItem>(columns: &[ColumnDef<T>], key: &str) -> Option<usize> {
    columns.iter().position(|c| c.key == key)
}

/// Get the key string for a given column index.
pub fn key_for_index<T: ViewItem>(columns: &[ColumnDef<T>], idx: usize) -> Option<&'static str> {
    columns.get(idx).map(|c| c.key)
}

/// Old (pre-migration) shared index scheme used by the legacy top-level
/// sort_column string. Only used to bridge legacy configs on load.
fn legacy_index(key: &str) -> Option<usize> {
    match key {
        "name" => Some(0),
        "remote" => Some(1),
        "ahead" => Some(2),
        "pr" => Some(3),
        "age" => Some(4),
        "status" => Some(5),
        _ => None,
    }
}

/// Branches' old column order, before Up moved ahead of Branch. Preserve the
/// meaning of legacy sort keys independently of the current display order.
const LEGACY_BRANCH_KEYS: [&str; 6] = ["name", "remote", "ahead_behind", "pr", "age", "merge"];

/// Translate a legacy shared sort index to the pre-reorder Remotes column key.
/// Remotes used to place Name at index 0 and Local at index 1.
fn legacy_remote_key(idx: usize) -> Option<&'static str> {
    match idx {
        0 => Some("name"),
        1 => Some("local"),
        2 => Some("ahead_behind"),
        3 => Some("pr"),
        4 => Some("age"),
        5 => Some("merge"),
        _ => None,
    }
}

/// If a config was written before per-view sort fields existed (only has the
/// legacy top-level `sort_column`/`sort_asc`), populate Branches' and Remotes'
/// per-view fields from it (those were the only two views the legacy fields
/// ever applied to). No-ops if per-view fields are already present, or if
/// there's no legacy value. Does not write to disk — caller may `config.save()`
/// afterward if it wants migration persisted immediately.
pub fn migrate_legacy_config(config: &mut Config) {
    if config.sort_column_branches.is_some() || config.sort_column_remotes.is_some() {
        return; // already has per-view data, nothing to migrate
    }
    let Some(legacy) = config.sort_column.as_deref() else {
        return;
    };
    let Some(idx) = legacy_index(legacy) else {
        return;
    };

    config.sort_column_branches = LEGACY_BRANCH_KEYS.get(idx).map(|key| (*key).to_string());
    config.sort_column_remotes = legacy_remote_key(idx).map(str::to_string);
    config.sort_asc_branches = config.sort_asc;
    config.sort_asc_remotes = config.sort_asc;
}

/// All (column, ascending) states reachable by cycling, in order, plus
/// `(None, true)` as the "no sort" state at the start of the cycle.
pub fn sort_state_cycle<T: ViewItem>(columns: &[ColumnDef<T>]) -> Vec<(Option<usize>, bool)> {
    let mut v = vec![(None, true)];
    for (i, col) in columns.iter().enumerate() {
        if col.compare.is_some() {
            v.push((Some(i), true));
            v.push((Some(i), false));
        }
    }
    v
}

/// Format a view's configured sort as a human-readable string:
/// "<Column Name> (asc|desc)" or "none".
pub fn display_string<T: ViewItem>(
    columns: &[ColumnDef<T>],
    key: Option<&str>,
    asc: bool,
) -> String {
    match key.and_then(|k| columns.iter().find(|c| c.key == k)) {
        Some(col) => format!("{} ({})", col.name, if asc { "asc" } else { "desc" }),
        None => "none".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_for_key_branches() {
        let cols = crate::view::branches::BranchesViewDef.columns();
        assert_eq!(index_for_key(&cols, "remote"), Some(0));
        assert_eq!(index_for_key(&cols, "name"), Some(1));
        assert_eq!(index_for_key(&cols, "ahead_behind"), Some(2));
        assert_eq!(index_for_key(&cols, "pr"), Some(3));
        assert_eq!(index_for_key(&cols, "age"), Some(4));
        assert_eq!(index_for_key(&cols, "merge"), Some(5));
    }

    #[test]
    fn index_for_key_remotes() {
        let cols = crate::view::remotes::RemotesViewDef.columns();
        assert_eq!(index_for_key(&cols, "local"), Some(0));
        assert_eq!(index_for_key(&cols, "name"), Some(1));
        assert_eq!(index_for_key(&cols, "ahead_behind"), Some(2));
        assert_eq!(index_for_key(&cols, "pr"), Some(3));
        assert_eq!(index_for_key(&cols, "age"), Some(4));
        assert_eq!(index_for_key(&cols, "merge"), Some(5));
    }

    #[test]
    fn index_for_key_tags() {
        let cols = crate::view::tags::TagsViewDef.columns();
        assert_eq!(index_for_key(&cols, "name"), Some(0));
        assert_eq!(index_for_key(&cols, "hash"), Some(1));
        assert_eq!(index_for_key(&cols, "age"), Some(2));
        assert_eq!(index_for_key(&cols, "message"), Some(3));
    }

    #[test]
    fn index_for_key_worktrees() {
        let cols = crate::view::worktrees::WorktreesViewDef.columns();
        assert_eq!(index_for_key(&cols, "path"), Some(0));
        assert_eq!(index_for_key(&cols, "branch"), Some(1));
        assert_eq!(index_for_key(&cols, "status"), Some(2));
        assert_eq!(index_for_key(&cols, "age"), Some(3));
        assert_eq!(index_for_key(&cols, "merge"), Some(4));
    }

    #[test]
    fn key_for_index_branches() {
        let cols = crate::view::branches::BranchesViewDef.columns();
        assert_eq!(key_for_index(&cols, 0), Some("remote"));
        assert_eq!(key_for_index(&cols, 1), Some("name"));
        assert_eq!(key_for_index(&cols, 2), Some("ahead_behind"));
        assert_eq!(key_for_index(&cols, 3), Some("pr"));
        assert_eq!(key_for_index(&cols, 4), Some("age"));
        assert_eq!(key_for_index(&cols, 5), Some("merge"));
    }

    #[test]
    fn roundtrip_branches() {
        let cols = crate::view::branches::BranchesViewDef.columns();
        for (idx, col) in cols.iter().enumerate() {
            assert_eq!(index_for_key(&cols, col.key), Some(idx));
            assert_eq!(key_for_index(&cols, idx), Some(col.key));
        }
    }

    #[test]
    fn roundtrip_remotes() {
        let cols = crate::view::remotes::RemotesViewDef.columns();
        for (idx, col) in cols.iter().enumerate() {
            assert_eq!(index_for_key(&cols, col.key), Some(idx));
            assert_eq!(key_for_index(&cols, idx), Some(col.key));
        }
    }

    #[test]
    fn migrate_legacy_config_basic() {
        let mut config = Config {
            sort_column: Some("age".into()),
            sort_asc: Some(false),
            ..Default::default()
        };
        migrate_legacy_config(&mut config);

        assert_eq!(config.sort_column_branches, Some("age".into()));
        assert_eq!(config.sort_column_remotes, Some("age".into()));
        assert_eq!(config.sort_asc_branches, Some(false));
        assert_eq!(config.sort_asc_remotes, Some(false));
        assert_eq!(config.sort_column_tags, None);
        assert_eq!(config.sort_column_worktrees, None);
    }

    #[test]
    fn migrate_legacy_config_keeps_branch_and_up_sort_meanings() {
        for (legacy_key, expected_branch_key, expected_remote_key) in [
            ("name", "name", "name"),
            ("remote", "remote", "local"),
            ("ahead", "ahead_behind", "ahead_behind"),
            ("pr", "pr", "pr"),
            ("age", "age", "age"),
            ("status", "merge", "merge"),
        ] {
            let mut config = Config {
                sort_column: Some(legacy_key.into()),
                ..Default::default()
            };

            migrate_legacy_config(&mut config);

            assert_eq!(
                config.sort_column_branches.as_deref(),
                Some(expected_branch_key),
                "legacy Branches sort {legacy_key:?}"
            );
            assert_eq!(
                config.sort_column_remotes.as_deref(),
                Some(expected_remote_key),
                "legacy Remotes sort {legacy_key:?}"
            );
        }
    }

    #[test]
    fn migrate_legacy_config_noop_when_already_migrated() {
        let mut config = Config {
            sort_column: Some("age".into()),
            sort_asc: Some(false),
            sort_column_branches: Some("name".into()),
            sort_asc_branches: Some(true),
            ..Default::default()
        };
        migrate_legacy_config(&mut config);

        // Should not overwrite
        assert_eq!(config.sort_column_branches, Some("name".into()));
        assert_eq!(config.sort_asc_branches, Some(true));
    }

    #[test]
    fn migrate_legacy_config_noop_when_no_legacy() {
        let mut config = Config::default();
        migrate_legacy_config(&mut config);

        assert_eq!(config.sort_column_branches, None);
        assert_eq!(config.sort_column_remotes, None);
    }

    #[test]
    fn sort_state_cycle_branches() {
        let cols = crate::view::branches::BranchesViewDef.columns();
        let cycle = sort_state_cycle(&cols);

        // First state is (None, true)
        assert_eq!(cycle[0], (None, true));

        // Then alternating asc/desc for each sortable column
        // Branches: remote (Up), name (Branch), ahead_behind, pr, age, merge.
        assert!(!cycle.is_empty());
        assert!(cycle.iter().any(|&(col, asc)| col == Some(0) && asc)); // Up asc
        assert!(cycle.iter().any(|&(col, asc)| col == Some(0) && !asc)); // Up desc
        assert!(cycle.iter().any(|&(col, asc)| col == Some(1) && asc)); // Branch asc
    }

    #[test]
    fn display_string_with_key() {
        let cols = crate::view::branches::BranchesViewDef.columns();
        let result = display_string(&cols, Some("name"), true);
        assert_eq!(result, "Branch (asc)");

        let result = display_string(&cols, Some("age"), false);
        assert_eq!(result, "Age (desc)");
    }

    #[test]
    fn display_string_without_key() {
        let cols = crate::view::branches::BranchesViewDef.columns();
        let result = display_string(&cols, None, true);
        assert_eq!(result, "none");
    }
}
