use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tracing::{info, warn};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    pub symbols: Option<String>,
    pub theme: Option<String>,
    pub sort_column: Option<String>, // legacy; kept only for migration
    pub sort_asc: Option<bool>,      // legacy; kept only for migration
    pub sort_column_branches: Option<String>,
    pub sort_asc_branches: Option<bool>,
    pub sort_column_remotes: Option<String>,
    pub sort_asc_remotes: Option<bool>,
    pub sort_column_tags: Option<String>,
    pub sort_asc_tags: Option<bool>,
    pub sort_column_worktrees: Option<String>,
    pub sort_asc_worktrees: Option<bool>,
    pub auto_fetch: Option<bool>,
    pub load_worktrees_on_launch: Option<bool>,
    pub include_remotes: Option<bool>,
    /// Silently verify and correct the cache in the background on launch.
    /// Defaults to enabled (`None`/absent means on) — unlike `auto_fetch`, it
    /// has no visible cost in the common case (no network I/O, no toast, no
    /// overlay). Set to `false` to disable on very large repos where the
    /// per-launch squash-merge recomputation is undesirable.
    pub verify_cache_on_launch: Option<bool>,
}

impl Config {
    pub fn load() -> Self {
        let path = Self::config_path();
        let legacy = Self::legacy_config_path();

        // Try new path first, then legacy path
        let (used_path, content) = match fs::read_to_string(&path) {
            Ok(c) => (path, c),
            Err(new_err) => match fs::read_to_string(&legacy) {
                Ok(c) => {
                    info!(
                        new_path = %path.display(),
                        legacy_path = %legacy.display(),
                        new_error = %new_err,
                        "config: new path unreadable, falling back to legacy"
                    );
                    (legacy, c)
                }
                Err(legacy_err) => {
                    warn!(
                        new_path = %path.display(),
                        legacy_path = %legacy.display(),
                        new_error = %new_err,
                        legacy_error = %legacy_err,
                        "config: both paths unreadable, returning defaults"
                    );
                    return Self::default();
                }
            },
        };

        if content.is_empty() {
            info!(path = %used_path.display(), "config: file empty, returning defaults");
            return Self::default();
        }

        match toml::from_str(&content) {
            Ok(parsed) => {
                info!(
                    path = %used_path.display(),
                    bytes = content.len(),
                    "config: loaded"
                );
                parsed
            }
            Err(e) => {
                warn!(
                    path = %used_path.display(),
                    bytes = content.len(),
                    error = %e,
                    "config: parse failed, returning defaults"
                );
                Self::default()
            }
        }
    }

    pub fn save(&self) {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            if let Err(e) = fs::create_dir_all(parent) {
                warn!(
                    path = %path.display(),
                    error = %e,
                    "config save: failed to create parent dir"
                );
                return;
            }
        }
        let content = match toml::to_string(self) {
            Ok(c) => c,
            Err(e) => {
                warn!(error = %e, "config save: serialize failed");
                return;
            }
        };
        info!(
            path = %path.display(),
            bytes = content.len(),
            "config save"
        );
        if let Err(e) = fs::write(&path, &content) {
            warn!(
                path = %path.display(),
                error = %e,
                "config save: write failed"
            );
        }
    }

    fn config_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("git-branch-manager")
            .join("config.toml")
    }

    fn legacy_config_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("git-bm")
            .join("config.toml")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults() {
        let c = Config::default();
        assert_eq!(c.symbols, None);
        assert_eq!(c.theme, None);
        assert_eq!(c.auto_fetch, None);
        assert_eq!(c.include_remotes, None);
    }

    #[test]
    fn config_roundtrip_toml() {
        let c = Config {
            theme: Some("dracula".into()),
            auto_fetch: Some(true),
            ..Default::default()
        };
        let toml_str = toml::to_string(&c).unwrap();
        let parsed: Config = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.theme, Some("dracula".into()));
        assert_eq!(parsed.auto_fetch, Some(true));
    }

    #[test]
    fn config_roundtrip_preserves_graph_remote_ref_preference() {
        let parsed: Config = toml::from_str("include_remotes = true\n").unwrap();
        let serialized = toml::to_string(&parsed).unwrap();

        assert_eq!(parsed.include_remotes, Some(true));
        assert!(serialized.contains("include_remotes = true"));
    }
}
