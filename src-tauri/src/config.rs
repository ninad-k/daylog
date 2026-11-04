//! User configuration. Persisted as plain JSON in the OS config dir.
//!
//! The password is NEVER written here (or anywhere). When encryption is on,
//! the key is derived from the password at unlock time via Argon2 and only the
//! Argon2 salt lives on disk, inside the encrypted log header — see `storage`.

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

fn default_poll_ms() -> u64 {
    1500
}
fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Where the log store is written. Defaults to the OS data dir.
    pub log_path: PathBuf,

    /// Encrypt the log store at rest (AES-GCM, Argon2-derived key). Off by default.
    #[serde(default)]
    pub encryption_enabled: bool,

    /// Pause logging while a focused app/title matches any blocklist entry
    /// (case-insensitive substring). Off by default.
    #[serde(default)]
    pub exclusion_enabled: bool,

    /// Substrings matched against app name AND window title.
    #[serde(default)]
    pub blocklist: Vec<String>,

    /// Register the app to launch at login. Off by default.
    #[serde(default)]
    pub start_on_login: bool,

    /// Delete events older than N days on startup. None = keep forever.
    #[serde(default)]
    pub auto_purge_days: Option<u32>,

    /// How often the capture loop samples the active window, in milliseconds.
    #[serde(default = "default_poll_ms")]
    pub poll_interval_ms: u64,

    /// Show the main window when the app launches (otherwise tray-only).
    /// Default on — this is a visible tool, not a hidden one.
    #[serde(default = "default_true")]
    pub show_window_on_launch: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            log_path: default_log_path(),
            encryption_enabled: false,
            exclusion_enabled: false,
            blocklist: Vec::new(),
            start_on_login: false,
            auto_purge_days: None,
            poll_interval_ms: default_poll_ms(),
            show_window_on_launch: true,
        }
    }
}

/// `~/Library/Application Support/com.ninadk.daylog/activity.log` on macOS and
/// the platform equivalent elsewhere.
pub fn project_dirs() -> Option<ProjectDirs> {
    ProjectDirs::from("com", "ninadk", "daylog")
}

pub fn default_log_path() -> PathBuf {
    project_dirs()
        .map(|d| d.data_dir().join("activity.log"))
        .unwrap_or_else(|| PathBuf::from("activity.log"))
}

fn config_path() -> PathBuf {
    project_dirs()
        .map(|d| d.config_dir().join("config.json"))
        .unwrap_or_else(|| PathBuf::from("daylog-config.json"))
}

impl Config {
    pub fn load() -> Self {
        let p = config_path();
        match std::fs::read_to_string(&p) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Config::default(),
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let p = config_path();
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&p, serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }
}
