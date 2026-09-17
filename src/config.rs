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

    /// Reload the on-disk config, apply `mutate` to it, and save the result.
    ///
    /// Avoids clobbering settings changed by a concurrent instance (config.toml
    /// is a single global path, not per-repo) or a manual edit made since this
    /// process last read the file: rather than writing back a whole in-memory
    /// snapshot that may be stale outside the field(s) `mutate` touches, this
    /// always starts from a fresh read of disk.
    pub fn update(mutate: impl FnOnce(&mut Config)) -> Config {
        let mut fresh = Self::load();
        mutate(&mut fresh);
        fresh.save();
        fresh
    }

    fn config_path() -> PathBuf {
        Self::config_dir_root()
            .join("git-branch-manager")
            .join("config.toml")
    }

    fn legacy_config_path() -> PathBuf {
        Self::config_dir_root().join("git-bm").join("config.toml")
    }

    /// The root directory config paths are resolved under. Overridable via
    /// `GBM_CONFIG_DIR` so tests can exercise `load`/`save`/`update` without
    /// touching the real user config file.
    fn config_dir_root() -> PathBuf {
        if let Ok(dir) = std::env::var("GBM_CONFIG_DIR") {
            return PathBuf::from(dir);
        }
        dirs::config_dir().unwrap_or_else(|| PathBuf::from("."))
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

    /// Serializes access to `GBM_CONFIG_DIR`-dependent tests, since env vars
    /// are process-global and `cargo test` runs tests on multiple threads.
    static CONFIG_DIR_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// `Config::update` must not clobber a field it doesn't touch, even when
    /// that field was changed on disk (e.g. by a concurrent instance, or a
    /// manual edit) after this process's own in-memory copy was loaded.
    #[test]
    fn update_preserves_untouched_fields_changed_out_of_band() {
        let _guard = CONFIG_DIR_ENV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!(
            "gbm-config-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: serialized by CONFIG_DIR_ENV_LOCK; no other test reads/writes
        // GBM_CONFIG_DIR.
        unsafe {
            std::env::set_var("GBM_CONFIG_DIR", &dir);
        }

        // This process "loads" the config early (as App does at startup)...
        let mut in_memory = Config::load();
        assert_eq!(in_memory.theme, None);

        // ...then, before this process saves anything, a concurrent instance
        // (or a manual edit) changes an unrelated field on disk.
        Config::update(|c| c.auto_fetch = Some(true));

        // This process now saves a change to a *different* field using the
        // old reload-merge-save helper, deriving the new value from its own
        // stale in-memory copy (mirroring how app.rs derives values like
        // `self.theme` before calling `Config::update`).
        in_memory.theme = Some("dracula".into());
        let saved = Config::update(|c| c.theme = in_memory.theme.clone());

        assert_eq!(saved.theme, Some("dracula".into()));
        assert_eq!(
            saved.auto_fetch,
            Some(true),
            "concurrent auto_fetch change must survive an unrelated save"
        );

        // SAFETY: still serialized by CONFIG_DIR_ENV_LOCK.
        unsafe {
            std::env::remove_var("GBM_CONFIG_DIR");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
