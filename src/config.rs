use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    pub symbols: Option<String>,
    pub theme: Option<String>,
    pub sort_column: Option<String>,
    pub sort_asc: Option<bool>,
    pub auto_fetch: Option<bool>,
    pub load_worktrees_on_launch: Option<bool>,
}

impl Config {
    pub fn load() -> Self {
        let path = Self::config_path();

        // Try new path first, then legacy path
        let content = fs::read_to_string(&path)
            .or_else(|_| fs::read_to_string(Self::legacy_config_path()))
            .unwrap_or_default();

        if content.is_empty() {
            return Self::default();
        }

        toml::from_str(&content).unwrap_or_default()
    }

    pub fn save(&self) {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(content) = toml::to_string(self) {
            let _ = fs::write(&path, content);
        }
    }

    /// Convert the configured sort column name to a column index (0-5).
    /// Maps "name"(0), "remote"(1), "ahead"(2), "pr"(3), "age"(4), "status"(5).
    pub fn sort_column_index(&self) -> Option<usize> {
        self.sort_column.as_deref().and_then(|s| match s {
            "name" => Some(0),
            "remote" => Some(1),
            "ahead" => Some(2),
            "pr" => Some(3),
            "age" => Some(4),
            "status" => Some(5),
            _ => None,
        })
    }

    /// Convert a column index (0-5) to the sort column name string.
    /// Maps 0→"name", 1→"remote", 2→"ahead", 3→"pr", 4→"age", 5→"status".
    pub fn sort_column_name(idx: usize) -> Option<&'static str> {
        match idx {
            0 => Some("name"),
            1 => Some("remote"),
            2 => Some("ahead"),
            3 => Some("pr"),
            4 => Some("age"),
            5 => Some("status"),
            _ => None,
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
}
