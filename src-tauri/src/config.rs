//! User configuration. Persisted as plain JSON in the OS config dir.
//!
//! The password is NEVER written here (or anywhere). When encryption is on,
//! the key is derived from the password at unlock time via Argon2 and only the
//! salt and encrypted verifier live in the log header — see `storage`.

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

fn default_poll_ms() -> u64 {
    1500
}
fn default_idle_seconds() -> Option<u64> {
    Some(300)
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

    /// Delete expired events on startup and hourly. None = keep forever.
    #[serde(default)]
    pub auto_purge_days: Option<u32>,

    /// How often the capture loop samples the active window, in milliseconds.
    #[serde(default = "default_poll_ms")]
    pub poll_interval_ms: u64,

    /// Stop counting after this many seconds without OS input. None disables it.
    #[serde(default = "default_idle_seconds")]
    pub idle_threshold_seconds: Option<u64>,

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
            idle_threshold_seconds: default_idle_seconds(),
        }
    }
}

/// `~/Library/Application Support/com.ninadk.daylog/activity.log` on macOS and
/// the platform equivalent elsewhere.
pub fn project_dirs() -> Option<ProjectDirs> {
    ProjectDirs::from("com", "ninadk", "daylog")
}

pub fn default_log_path() -> PathBuf {
    portable_root()
        .map(|p| p.join("activity.log"))
        .or_else(|| project_dirs().map(|d| d.data_dir().join("activity.log")))
        .unwrap_or_else(|| PathBuf::from("activity.log"))
}

fn portable_root() -> Option<PathBuf> {
    std::env::var_os("DAYLOG_HOME").map(PathBuf::from)
}

pub fn config_path() -> PathBuf {
    portable_root()
        .map(|p| p.join("config.json"))
        .or_else(|| project_dirs().map(|d| d.config_dir().join("config.json")))
        .unwrap_or_else(|| PathBuf::from("daylog-config.json"))
}

impl Config {
    pub fn load() -> anyhow::Result<Self> {
        let p = config_path();
        match std::fs::read_to_string(&p) {
            Ok(s) => Ok(serde_json::from_str(&s)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.log_path.is_absolute(), "Log path must be absolute");
        anyhow::ensure!(
            self.log_path.file_name().is_some(),
            "Choose a log file, not a directory"
        );
        anyhow::ensure!(!self.log_path.is_dir(), "Log path is a directory");
        anyhow::ensure!(
            (250..=60_000).contains(&self.poll_interval_ms),
            "Sampling interval must be 250–60000 ms"
        );
        anyhow::ensure!(
            self.auto_purge_days != Some(0),
            "Retention must be at least one day"
        );
        anyhow::ensure!(
            self.idle_threshold_seconds
                .is_none_or(|v| (30..=86400).contains(&v)),
            "Idle threshold must be 30–86400 seconds"
        );
        anyhow::ensure!(
            self.blocklist.len() <= 1000,
            "Blocklist is limited to 1000 entries"
        );
        Ok(())
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let p = config_path();
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        crate::storage::atomic_write(&p, &serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_settings_get_safe_defaults() {
        let config: Config =
            serde_json::from_value(serde_json::json!({"log_path": "/tmp/activity.log"})).unwrap();
        assert_eq!(config.idle_threshold_seconds, Some(300));
        assert_eq!(config.poll_interval_ms, 1500);
        assert!(!config.encryption_enabled);
        assert!(config.validate().is_ok());
    }
    #[test]
    fn invalid_settings_are_rejected_before_any_write() {
        let mut config = Config {
            log_path: PathBuf::from("relative.log"),
            ..Config::default()
        };
        assert!(config.validate().is_err());
        config.log_path = std::env::temp_dir().join("daylog-config-test.log");
        config.poll_interval_ms = 0;
        assert!(config.validate().is_err());
        config.poll_interval_ms = 1500;
        config.idle_threshold_seconds = Some(0);
        assert!(config.validate().is_err());
        config.idle_threshold_seconds = None;
        config.auto_purge_days = Some(0);
        assert!(config.validate().is_err());
    }
}
