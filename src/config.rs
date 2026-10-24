use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub const DEFAULT_URL: &str = "https://dev-null.sh";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    /// Whose Following log to build from public data, and whose profile `g p` opens, when not signed in.
    #[serde(default)]
    pub username: Option<String>,
    /// Whether the device key was approved on the web.
    #[serde(default)]
    pub signed_in: bool,
    /// "auto" (the default) follows the terminal's background; anything else is a theme's id, like "windows-98".
    #[serde(default)]
    pub theme: Option<String>,
}

impl Config {
    /// DEVNULL_THEME wins over the saved setting.
    pub fn theme(&self) -> String {
        std::env::var("DEVNULL_THEME")
            .ok()
            .filter(|t| !t.is_empty())
            .or_else(|| self.theme.clone())
            .unwrap_or_else(|| "auto".into())
            .to_lowercase()
    }
}

pub fn dir() -> PathBuf {
    if let Ok(dir) = std::env::var("DEVNULL_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("devnull")
}

pub fn key_path() -> PathBuf {
    dir().join("device_ed25519")
}

impl Config {
    pub fn load() -> Self {
        fs::read_to_string(dir().join("config.json"))
            .ok()
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        fs::create_dir_all(dir()).context("Could not create the config directory")?;
        fs::write(dir().join("config.json"), serde_json::to_string_pretty(self)?).context("Could not save the config")
    }
}

/// A commit being written, kept across restarts.
pub fn load_draft() -> Option<String> {
    fs::read_to_string(dir().join("draft.txt")).ok().filter(|d| !d.trim().is_empty())
}

pub fn save_draft(draft: Option<&str>) {
    let path = dir().join("draft.txt");
    match draft.filter(|d| !d.trim().is_empty()) {
        Some(draft) => {
            let _ = fs::create_dir_all(dir());
            let _ = fs::write(path, draft);
        }
        None => {
            let _ = fs::remove_file(path);
        }
    }
}

pub fn base_url() -> String {
    std::env::var("DEVNULL_URL")
        .unwrap_or_else(|_| DEFAULT_URL.to_string())
        .trim_end_matches('/')
        .to_string()
}
