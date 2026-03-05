use std::path::PathBuf;
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub symbols: Option<String>,
    #[serde(default)]
    pub theme: Option<String>,
    #[serde(default)]
    pub trim_strategy: Option<String>,
    #[serde(default)]
    pub trim_min_length: Option<usize>,
}

impl Config {
    pub fn load() -> Self {
        let path = config_path();

        // Migration: check for old config path and migrate if it exists
        let old_path = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("git-bm")
            .join("config.toml");

        if old_path.exists() {
            // Create new config directory
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            // Copy old file to new path
            let _ = std::fs::copy(&old_path, &path);
            // Remove old file
            let _ = std::fs::remove_file(&old_path);
            // Try to remove old directory (ignore errors if not empty)
            if let Some(old_parent) = old_path.parent() {
                let _ = std::fs::remove_dir(old_parent);
            }
        }

        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let path = config_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(s) = toml::to_string_pretty(self) {
            let _ = std::fs::write(&path, s);
        }
    }
}

fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("git-branch-manager")
        .join("config.toml")
}
