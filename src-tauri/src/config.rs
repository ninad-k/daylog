//! User configuration. Persisted as plain JSON in the OS config dir.
//!
//! The password is NEVER written here (or anywhere). When encryption is on,
//! the key is derived from the password at unlock time via Argon2 and only the
//! Argon2 salt lives on disk, inside the encrypted log header — see `storage`.

use crate::model::{AppCategory, SavedFilter, TimeGoal};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

fn default_poll_ms() -> u64 {
    1500
}
fn default_true() -> bool {
    true
}
fn default_idle() -> Option<u64> {
    Some(300)
}
fn default_smtp_port() -> u16 {
    587
}
fn default_email_hour() -> u32 {
    9
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EmailConfig {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub implicit_tls: bool,
    pub username: String,
    pub sender: String,
    pub recipient: String,
    pub weekly: bool,
    pub hour: u32,
    pub last_sent: Option<String>,
    #[serde(default)]
    pub sent_periods: Vec<String>,
}
impl Default for EmailConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            host: String::new(),
            port: default_smtp_port(),
            implicit_tls: false,
            username: String::new(),
            sender: String::new(),
            recipient: String::new(),
            weekly: false,
            hour: default_email_hour(),
            last_sent: None,
            sent_periods: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Where the log store is written. Defaults to the OS data dir.
    pub log_path: PathBuf,

    /// Encrypt the log store at rest (AES-GCM, Argon2-derived key). Off by default.
    #[serde(default)]
    pub encryption_enabled: bool,

    /// Explicit opt-in only; typed text requires an unlocked encrypted store.
    #[serde(default)]
    pub text_capture_enabled: bool,
    /// Broader opt-in scope; native field/process safeguards still apply.
    #[serde(default)]
    pub text_capture_all_apps: bool,
    #[serde(default)]
    pub text_capture_apps: Vec<String>,

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

    #[serde(default = "default_idle")]
    pub idle_threshold_seconds: Option<u64>,
    #[serde(default)]
    pub app_categories: Vec<AppCategory>,
    #[serde(default)]
    pub time_goals: Vec<TimeGoal>,
    #[serde(default)]
    pub saved_filters: Vec<SavedFilter>,
    #[serde(default)]
    pub email: EmailConfig,
    #[serde(default)]
    pub auto_check_updates: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            log_path: default_log_path(),
            encryption_enabled: false,
            text_capture_enabled: false,
            text_capture_all_apps: false,
            text_capture_apps: Vec::new(),
            exclusion_enabled: false,
            blocklist: Vec::new(),
            start_on_login: false,
            auto_purge_days: None,
            poll_interval_ms: default_poll_ms(),
            show_window_on_launch: true,
            idle_threshold_seconds: default_idle(),
            app_categories: Vec::new(),
            time_goals: Vec::new(),
            saved_filters: Vec::new(),
            email: EmailConfig::default(),
            auto_check_updates: false,
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
    // A development run can use a separate profile without touching an installed logger.
    #[cfg(debug_assertions)]
    if let Some(path) = std::env::var_os("DAYLOG_DEV_CONFIG") {
        return PathBuf::from(path);
    }
    project_dirs()
        .map(|d| d.config_dir().join("config.json"))
        .unwrap_or_else(|| PathBuf::from("daylog-config.json"))
}

impl Config {
    pub fn load() -> anyhow::Result<Self> {
        Self::load_from_path(&config_path())
    }

    fn load_from_path(path: &std::path::Path) -> anyhow::Result<Self> {
        use anyhow::Context;
        let content = match std::fs::read_to_string(path) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default())
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("could not read configuration at {}", path.display()))
            }
        };
        let config: Self = serde_json::from_str(&content)
            .with_context(|| format!("invalid configuration at {}", path.display()))?;
        config
            .validate()
            .with_context(|| format!("invalid settings in {}", path.display()))?;
        Ok(config)
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let p = config_path();
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        crate::storage::atomic_write(&p, &serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.text_capture_enabled || self.encryption_enabled,
            "typed text requires encryption; disable text capture before disabling encryption"
        );
        anyhow::ensure!(
            !self.text_capture_enabled
                || self.text_capture_all_apps
                || self
                    .text_capture_apps
                    .iter()
                    .any(|app| !app.trim().is_empty()),
            "approve at least one application or select all-app mode before enabling typed text"
        );
        let mut approved = std::collections::HashSet::new();
        for app in &self.text_capture_apps {
            anyhow::ensure!(
                !app.trim().is_empty(),
                "approved application names cannot be empty"
            );
            anyhow::ensure!(
                approved.insert(app.trim().to_lowercase()),
                "approved application names must be unique"
            );
        }
        anyhow::ensure!(
            !self.log_path.as_os_str().is_empty(),
            "log path is required"
        );
        anyhow::ensure!(self.log_path.is_absolute(), "log path must be absolute");
        anyhow::ensure!(
            self.log_path.extension().and_then(|s| s.to_str()) != Some("enc"),
            "base log path must not end in .enc"
        );
        anyhow::ensure!(
            (250..=60_000).contains(&self.poll_interval_ms),
            "sampling interval must be 250–60000 ms"
        );
        anyhow::ensure!(
            self.auto_purge_days
                .is_none_or(|days| (1..=365_000).contains(&days)),
            "auto-purge must be 1–365000 days"
        );
        anyhow::ensure!(
            self.idle_threshold_seconds
                .is_none_or(|s| (15..=86400).contains(&s)),
            "idle threshold must be 15–86400 seconds"
        );
        let mut category_apps = std::collections::HashSet::new();
        for mapping in &self.app_categories {
            anyhow::ensure!(
                !mapping.app.trim().is_empty() && !mapping.category.trim().is_empty(),
                "app and category are required"
            );
            anyhow::ensure!(
                category_apps.insert(mapping.app.trim().to_lowercase()),
                "each app can have only one category"
            );
        }
        let mut goal_ids = std::collections::HashSet::new();
        for goal in &self.time_goals {
            anyhow::ensure!(
                !goal.id.is_empty()
                    && !goal.name.trim().is_empty()
                    && !goal.target.trim().is_empty()
                    && goal.minutes > 0,
                "goals need a name, target, and positive duration"
            );
            anyhow::ensure!(goal_ids.insert(goal.id.trim()), "goal IDs must be unique");
        }
        let mut filter_ids = std::collections::HashSet::new();
        for saved in &self.saved_filters {
            anyhow::ensure!(
                !saved.id.trim().is_empty() && !saved.name.trim().is_empty(),
                "saved filters need a name and ID"
            );
            anyhow::ensure!(
                filter_ids.insert(saved.id.trim()),
                "saved filter IDs must be unique"
            );
            saved.filter.validate()?;
        }
        anyhow::ensure!(self.email.hour <= 23, "email hour must be 0–23");
        if self.email.enabled {
            crate::reports::validate_email(&self.email)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{EventFilter, GoalKind, GoalPeriod, TargetType};
    use chrono::{Duration, Utc};

    #[test]
    fn legacy_config_loads_new_feature_defaults() {
        let mut value = serde_json::to_value(Config::default()).unwrap();
        for field in [
            "text_capture_enabled",
            "text_capture_all_apps",
            "text_capture_apps",
            "idle_threshold_seconds",
            "app_categories",
            "time_goals",
            "saved_filters",
            "email",
            "auto_check_updates",
        ] {
            value.as_object_mut().unwrap().remove(field);
        }
        let loaded: Config = serde_json::from_value(value).unwrap();
        assert_eq!(loaded.idle_threshold_seconds, Some(300));
        assert!(loaded.app_categories.is_empty());
        assert!(loaded.time_goals.is_empty());
        assert!(loaded.saved_filters.is_empty());
        assert!(!loaded.email.enabled);
        assert!(!loaded.auto_check_updates);
        assert!(!loaded.text_capture_enabled);
        assert!(!loaded.text_capture_all_apps);
        assert!(loaded.text_capture_apps.is_empty());
        assert!(loaded.validate().is_ok());
    }

    #[test]
    fn sampling_idle_and_retention_bounds_are_validated() {
        let mut config = Config::default();
        for poll in [250, 1500, 60_000] {
            config.poll_interval_ms = poll;
            assert!(config.validate().is_ok());
        }
        for poll in [0, 249, 60_001, u64::MAX] {
            config.poll_interval_ms = poll;
            assert!(config.validate().is_err());
        }
        config.poll_interval_ms = 1500;
        for idle in [None, Some(15), Some(300), Some(86400)] {
            config.idle_threshold_seconds = idle;
            assert!(config.validate().is_ok());
        }
        for idle in [Some(0), Some(14), Some(86401), Some(u64::MAX)] {
            config.idle_threshold_seconds = idle;
            assert!(config.validate().is_err());
        }
        config.idle_threshold_seconds = None;
        config.auto_purge_days = Some(0);
        assert!(config.validate().is_err());
        config.auto_purge_days = Some(1);
        assert!(config.validate().is_ok());
        config.auto_purge_days = Some(365_000);
        assert!(config.validate().is_ok());
        config.auto_purge_days = Some(365_001);
        assert!(config.validate().is_err());
        config.auto_purge_days = Some(u32::MAX);
        assert!(config.validate().is_err());
    }

    #[test]
    fn typed_text_requires_encryption_and_explicit_distinct_approved_apps() {
        let mut config = Config {
            text_capture_enabled: true,
            ..Config::default()
        };
        assert!(config.validate().is_err());
        config.encryption_enabled = true;
        assert!(config.validate().is_err());
        config.text_capture_apps = vec!["Code".into()];
        assert!(config.validate().is_ok());
        config.text_capture_apps.push(" code ".into());
        assert!(config.validate().is_err());
        config.text_capture_apps = vec!["   ".into()];
        assert!(config.validate().is_err());
        config.text_capture_enabled = false;
        config.text_capture_apps.clear();
        config.encryption_enabled = false;
        assert!(config.validate().is_ok());
    }

    #[test]
    fn all_application_text_capture_requires_encryption_and_defaults_off_for_legacy_profiles() {
        let mut config = Config {
            encryption_enabled: true,
            text_capture_enabled: true,
            text_capture_all_apps: true,
            ..Config::default()
        };
        assert!(config.text_capture_apps.is_empty());
        assert!(config.validate().is_ok());
        config.encryption_enabled = false;
        assert!(config.validate().is_err());
        config.encryption_enabled = true;
        config.text_capture_all_apps = false;
        assert!(config.validate().is_err());
        config.text_capture_apps = vec!["Notepad".into()];
        assert!(config.validate().is_ok());
        let mut legacy = serde_json::to_value(&config).unwrap();
        legacy
            .as_object_mut()
            .unwrap()
            .remove("text_capture_all_apps");
        let loaded: Config = serde_json::from_value(legacy).unwrap();
        assert!(!loaded.text_capture_all_apps);
        assert!(loaded.text_capture_enabled);
        assert!(loaded.validate().is_ok());
    }

    #[test]
    fn unreadable_or_invalid_configuration_never_silently_defaults() {
        let path = std::env::temp_dir().join(format!(
            "daylog-config-{}-{}.json",
            std::process::id(),
            rand::random::<u64>()
        ));
        assert!(!Config::load_from_path(&path).unwrap().encryption_enabled);
        std::fs::write(&path, "{\"encryption_enabled\": true, broken").unwrap();
        assert!(Config::load_from_path(&path).is_err());
        let mut config = Config {
            encryption_enabled: true,
            ..Config::default()
        };
        std::fs::write(&path, serde_json::to_vec(&config).unwrap()).unwrap();
        assert!(Config::load_from_path(&path).unwrap().encryption_enabled);
        config.text_capture_enabled = true;
        std::fs::write(&path, serde_json::to_vec(&config).unwrap()).unwrap();
        assert!(Config::load_from_path(&path).is_err());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn category_goal_and_saved_filter_values_are_validated_together() {
        let mut config = Config::default();
        config.app_categories.push(AppCategory {
            app: "Code".into(),
            category: "Work".into(),
        });
        config.time_goals.push(TimeGoal {
            id: "work".into(),
            name: "Focus time".into(),
            target_type: TargetType::Category,
            target: "Work".into(),
            period: GoalPeriod::Daily,
            kind: GoalKind::Goal,
            minutes: 60,
        });
        let now = Utc::now();
        config.saved_filters.push(SavedFilter {
            id: "work".into(),
            name: "Work today".into(),
            filter: EventFilter {
                category: Some("Work".into()),
                from: Some(now - Duration::days(1)),
                to: Some(now),
                ..Default::default()
            },
        });
        assert!(config.validate().is_ok());
        config.time_goals[0].minutes = 0;
        assert!(config.validate().is_err());
        config.time_goals[0].minutes = 60;
        config.app_categories[0].category = "   ".into();
        assert!(config.validate().is_err());
        config.app_categories[0].category = "Work".into();
        config.saved_filters[0].filter.to = Some(now - Duration::days(2));
        assert!(config.validate().is_err());
    }

    #[test]
    fn log_paths_and_email_schedule_are_validated() {
        let mut config = Config {
            log_path: PathBuf::from("relative.log"),
            ..Config::default()
        };
        assert!(config.validate().is_err());
        config.log_path = default_log_path().with_extension("enc");
        assert!(config.validate().is_err());
        config.log_path = default_log_path();
        config.email.hour = 24;
        assert!(config.validate().is_err());
        config.email.hour = 23;
        assert!(config.validate().is_ok());
        config.email.enabled = true;
        assert!(config.validate().is_err());
        config.email.host = "smtp.example.com".into();
        config.email.sender = "me@example.com".into();
        config.email.recipient = "me@example.com".into();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn duplicate_ids_and_category_assignments_are_rejected() {
        let mut config = Config {
            app_categories: vec![
                AppCategory {
                    app: "Code".into(),
                    category: "Work".into(),
                },
                AppCategory {
                    app: " code ".into(),
                    category: "Leisure".into(),
                },
            ],
            ..Config::default()
        };
        assert!(config.validate().is_err());
        config.app_categories.clear();
        let goal = TimeGoal {
            id: "focus".into(),
            name: "Focus".into(),
            target_type: TargetType::App,
            target: "Code".into(),
            period: GoalPeriod::Daily,
            kind: GoalKind::Goal,
            minutes: 10,
        };
        config.time_goals = vec![goal.clone(), goal];
        assert!(config.validate().is_err());
        config.time_goals.clear();
        let saved = SavedFilter {
            id: "work".into(),
            name: "Work".into(),
            filter: EventFilter::default(),
        };
        config.saved_filters = vec![saved.clone(), saved];
        assert!(config.validate().is_err());
        config.saved_filters[1].id = "second".into();
        assert!(config.validate().is_ok());
        config.saved_filters[1].name = "  ".into();
        assert!(config.validate().is_err());
    }
}
