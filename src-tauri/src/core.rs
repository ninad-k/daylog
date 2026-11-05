//! Shared application state. History is loaded once; checkpoints are appended
//! with a stable session id so recovery can select the latest complete record.
use crate::capture::window::Focus;
use crate::config::Config;
use crate::model::{Analytics, AppBucket, DayBucket, Event, EventFilter, Status};
use crate::storage::{self, encrypted::EncryptedStore, plaintext::PlainStore, Store};
use chrono::{DateTime, Days, Local, TimeZone, Utc};
use std::collections::HashMap;

struct SessionInProgress {
    app: String,
    title: String,
    start: DateTime<Utc>,
    last_seen: DateTime<Utc>,
    checkpoint: DateTime<Utc>,
}

pub struct Core {
    pub config: Config,
    pub paused: bool,
    store: Option<Box<dyn Store>>,
    events: Vec<Event>,
    current: Option<SessionInProgress>,
    display: Option<Focus>,
    excluded_now: bool,
    idle_now: bool,
    idle_available: bool,
    last_tick: Option<DateTime<Utc>>,
    last_purge: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
}

impl Core {
    pub fn bootstrap() -> Self {
        let mut core = match Config::load() {
            Ok(config) => {
                if let Err(e) = config.validate() {
                    let mut core = Self::new(config);
                    core.paused = true;
                    core.last_error = Some(format!(
                        "Invalid settings: {e}. Correct Settings and save to recover."
                    ));
                    return core;
                }
                Self::new(config)
            }
            Err(e) => {
                let mut core = Self::new(Config::default());
                core.paused = true;
                core.last_error = Some(format!(
                    "Cannot load settings: {e}. Repair config.json or save settings to recover."
                ));
                return core;
            }
        };
        if !core.config.encryption_enabled {
            if let Err(e) = core.open_store(None) {
                core.last_error = Some(e.to_string());
            }
        }
        core
    }

    fn new(config: Config) -> Self {
        Self {
            config,
            paused: false,
            store: None,
            events: Vec::new(),
            current: None,
            display: None,
            excluded_now: false,
            idle_now: false,
            idle_available: false,
            last_tick: None,
            last_purge: None,
            last_error: None,
        }
    }

    pub fn active_path(&self) -> std::path::PathBuf {
        storage::active_path(&self.config.log_path, self.config.encryption_enabled)
    }

    fn prepare_store(config: &Config, password: Option<&str>) -> anyhow::Result<Box<dyn Store>> {
        let path = storage::active_path(&config.log_path, config.encryption_enabled);
        if config.encryption_enabled {
            let pw = password
                .filter(|p| !p.is_empty())
                .ok_or_else(|| anyhow::anyhow!("Password required"))?;
            Ok(Box::new(EncryptedStore::open(path, pw)?))
        } else {
            Ok(Box::new(PlainStore::open(path)?))
        }
    }

    pub fn open_store(&mut self, password: Option<&str>) -> anyhow::Result<()> {
        self.config.validate()?;
        let mut store = Self::prepare_store(&self.config, password)?;
        let events = store.all()?;
        store.replace(&events)?;
        self.store = Some(store);
        self.events = events;
        self.last_error = None;
        self.purge_if_needed()?;
        Ok(())
    }

    /// Prepare and verify the destination before committing settings. Encryption
    /// changes merge history; path-only changes open independent history.
    pub fn apply_config(&mut self, config: Config, password: Option<&str>) -> anyhow::Result<()> {
        self.apply_config_with_save(config, password, Config::save)
    }

    fn apply_config_with_save(
        &mut self,
        config: Config,
        password: Option<&str>,
        save: impl FnOnce(&Config) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        config.validate()?;
        self.flush_current()?;
        let crypto_changed = config.encryption_enabled != self.config.encryption_enabled;
        let path_changed = config.log_path != self.config.log_path;
        anyhow::ensure!(
            !(crypto_changed && path_changed),
            "Change the log path and encryption separately"
        );
        if crypto_changed {
            anyhow::ensure!(
                self.store.is_some(),
                "Unlock the current log before changing encryption"
            );
            anyhow::ensure!(
                storage::active_path(&config.log_path, true) != config.log_path,
                "Plaintext and encrypted paths must differ; choose a .log path"
            );
        }
        let reopen = crypto_changed || path_changed || self.store.is_none();
        let mut prepared = if reopen {
            Some(Self::prepare_store(&config, password)?)
        } else {
            None
        };
        let mut destination_original = None;
        let new_events = if let Some(store) = prepared.as_mut() {
            let mut events = store.all()?;
            if crypto_changed {
                destination_original = Some(events.clone());
                events.extend(self.events.clone());
                events = storage::deduplicate(events);
            }
            store.replace(&events)?;
            Some(events)
        } else {
            None
        };
        if let Err(error) = save(&config) {
            if let (Some(store), Some(original)) = (prepared.as_mut(), destination_original) {
                if let Err(rollback) = store.replace(&original) {
                    return Err(anyhow::anyhow!(
                        "{error}. Destination history rollback failed: {rollback}"
                    ));
                }
            }
            return Err(error);
        }
        let old_path = self.active_path();
        self.config = config;
        if let Some(store) = prepared {
            self.store = Some(store);
        }
        if let Some(events) = new_events {
            self.events = events;
        }
        self.last_error = None;
        // Delete the source only after the destination history and settings
        // are durable. Failure is visible, never silently presented as protected.
        if crypto_changed {
            if let Err(e) = std::fs::remove_file(&old_path) {
                self.last_error = Some(format!(
                    "History migrated, but remove the old source log {}: {e}",
                    old_path.display()
                ));
            }
        }
        self.last_purge = None;
        if let Err(e) = self.purge_if_needed() {
            self.last_error = Some(e.to_string());
        }
        Ok(())
    }

    pub fn retry_storage(&mut self) -> anyhow::Result<()> {
        if self.store.is_none() {
            self.open_store(None)?;
        }
        self.flush_current()?;
        self.last_error = None;
        self.last_purge = None;
        self.purge_if_needed()?;
        Ok(())
    }

    pub fn is_locked(&self) -> bool {
        self.config.encryption_enabled && self.store.is_none()
    }

    fn is_excluded(&self, f: &Focus) -> bool {
        self.config.exclusion_enabled
            && self.config.blocklist.iter().any(|b| {
                let b = b.trim().to_lowercase();
                !b.is_empty()
                    && (f.app.to_lowercase().contains(&b) || f.title.to_lowercase().contains(&b))
            })
    }

    pub fn tick(&mut self, focus: Option<Focus>, idle_seconds: Option<u64>, now: DateTime<Utc>) {
        self.display = focus.clone();
        self.excluded_now = focus.as_ref().map(|f| self.is_excluded(f)).unwrap_or(false);
        self.idle_available = idle_seconds.is_some();
        self.idle_now = self
            .config
            .idle_threshold_seconds
            .zip(idle_seconds)
            .map(|(threshold, idle)| idle >= threshold)
            .unwrap_or(false);
        // A suspend, clock jump, or blocked capture loop must not count as work.
        let gap = self
            .last_tick
            .map(|last| {
                now < last
                    || (now - last).num_milliseconds()
                        > (self.config.poll_interval_ms.saturating_mul(3).max(10_000)) as i64
            })
            .unwrap_or(false);
        self.last_tick = Some(now);
        if gap {
            if let Err(e) = self.flush_current() {
                self.last_error = Some(e.to_string());
                self.paused = true;
                return;
            }
        }
        let skip = self.paused
            || self.store.is_none()
            || self.excluded_now
            || self.idle_now
            || focus.is_none();
        if skip {
            if !gap && !self.paused {
                if let Some(s) = self.current.as_mut() {
                    let end = if self.idle_now {
                        let idle = idle_seconds.unwrap_or(0);
                        let threshold = self.config.idle_threshold_seconds.unwrap_or(0);
                        now - chrono::Duration::seconds(
                            idle.saturating_sub(threshold).min(i64::MAX as u64) as i64,
                        )
                    } else {
                        now
                    };
                    s.last_seen = end.max(s.start);
                }
            }
            if let Err(e) = self.flush_current() {
                self.last_error = Some(e.to_string());
                self.paused = true;
            }
            if self.store.is_some() {
                if let Err(e) = self.purge_if_needed() {
                    self.last_error = Some(e.to_string());
                }
            }
            return;
        }
        let f = focus.unwrap();
        let same = self
            .current
            .as_ref()
            .map(|s| s.app == f.app && s.title == f.title)
            .unwrap_or(false);
        if same {
            let s = self.current.as_mut().unwrap();
            s.last_seen = now;
            if (now - s.checkpoint).num_seconds() >= 30 {
                if let Err(e) = self.checkpoint() {
                    self.last_error = Some(e.to_string());
                    self.paused = true;
                    return;
                }
            }
        } else {
            if let Some(s) = self.current.as_mut() {
                s.last_seen = now.max(s.last_seen);
            }
            if let Err(e) = self.flush_current() {
                self.last_error = Some(e.to_string());
                self.paused = true;
                return;
            }
            self.current = Some(SessionInProgress {
                app: f.app,
                title: f.title,
                start: now,
                last_seen: now,
                checkpoint: now,
            });
        }
        if let Err(e) = self.purge_if_needed() {
            self.last_error = Some(e.to_string());
        }
    }

    fn current_event(&self) -> Option<Event> {
        self.current
            .as_ref()
            .map(|s| Event::new(s.start, s.last_seen, s.app.clone(), s.title.clone()))
            .filter(|e| e.seconds >= 1)
    }

    fn checkpoint(&mut self) -> anyhow::Result<()> {
        if let Some(event) = self.current_event() {
            let store = self
                .store
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("Log unavailable"))?;
            store.append(&event)?;
            if let Some(old) = self.events.iter_mut().find(|e| e.id == event.id) {
                *old = event;
            } else {
                self.events.push(event);
            }
        }
        if let Some(s) = self.current.as_mut() {
            s.checkpoint = s.last_seen;
        }
        Ok(())
    }

    pub fn flush_current(&mut self) -> anyhow::Result<()> {
        self.checkpoint()?;
        self.current = None;
        Ok(())
    }

    pub fn status(&self) -> Status {
        let current = self.current_event();
        let extra = current
            .as_ref()
            .map(|e| !self.events.iter().any(|old| old.id == e.id))
            .unwrap_or(false);
        Status {
            paused: self.paused,
            encrypted: self.config.encryption_enabled,
            locked: self.is_locked(),
            excluded: self.excluded_now,
            idle: self.idle_now,
            idle_available: self.idle_available,
            capture_available: self.display.is_some(),
            error: self.last_error.clone(),
            current_app: if self.excluded_now {
                None
            } else {
                self.display.as_ref().map(|f| f.app.clone())
            },
            current_title: if self.excluded_now {
                None
            } else {
                self.display.as_ref().map(|f| f.title.clone())
            },
            events_total: self.events.len() as i64 + i64::from(extra),
            log_path: self.active_path().display().to_string(),
        }
    }

    pub fn query(&self, filter: &EventFilter) -> anyhow::Result<Vec<Event>> {
        anyhow::ensure!(self.store.is_some(), "Log is locked or unavailable");
        let mut events = self.events.clone();
        if let Some(current) = self.current_event() {
            if let Some(old) = events.iter_mut().find(|e| e.id == current.id) {
                *old = current;
            } else {
                events.push(current);
            }
        }
        events.retain(|e| filter.matches(e));
        events.sort_by_key(|e| std::cmp::Reverse(e.start));
        if let Some(limit) = filter.limit {
            events.truncate(limit);
        }
        Ok(events)
    }

    pub fn clear(&mut self) -> anyhow::Result<()> {
        self.store
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("Log is locked or unavailable"))?
            .clear()?;
        self.current = None;
        self.events.clear();
        self.last_error = None;
        Ok(())
    }

    pub fn analytics(&self, filter: &EventFilter) -> anyhow::Result<Analytics> {
        let mut matching = filter.clone();
        matching.from = None;
        matching.to = None;
        matching.limit = None;
        let events: Vec<_> = self
            .query(&matching)?
            .into_iter()
            .filter_map(|mut e| {
                if let Some(from) = filter.from {
                    e.start = e.start.max(from);
                }
                if let Some(to) = filter.to {
                    e.end = e.end.min(to);
                }
                e.seconds = (e.end - e.start).num_seconds();
                (e.seconds > 0).then_some(e)
            })
            .collect();
        let mut apps: HashMap<String, (i64, i64)> = HashMap::new();
        let mut days: HashMap<String, i64> = HashMap::new();
        let mut total = 0;
        for e in &events {
            total += e.seconds;
            let app = apps.entry(e.app.clone()).or_default();
            app.0 += e.seconds;
            app.1 += 1;
            // Split at local midnights, using calendar arithmetic across DST.
            let mut cursor = e.start;
            while cursor < e.end {
                let local = cursor.with_timezone(&Local);
                let tomorrow = local.date_naive().checked_add_days(Days::new(1)).unwrap();
                let midnight = Local
                    .from_local_datetime(&tomorrow.and_hms_opt(0, 0, 0).unwrap())
                    .earliest()
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or(cursor + chrono::Duration::hours(24));
                let end = e.end.min(midnight);
                if end <= cursor {
                    break;
                }
                *days
                    .entry(local.format("%Y-%m-%d").to_string())
                    .or_default() +=
                    (end - e.start).num_seconds() - (cursor - e.start).num_seconds();
                cursor = end;
            }
        }
        let mut per_app: Vec<_> = apps
            .into_iter()
            .map(|(app, (seconds, events))| AppBucket {
                app,
                seconds,
                events,
            })
            .collect();
        per_app.sort_by(|a, b| b.seconds.cmp(&a.seconds).then(a.app.cmp(&b.app)));
        let mut per_day: Vec<_> = days
            .into_iter()
            .map(|(date, seconds)| DayBucket { date, seconds })
            .collect();
        per_day.sort_by(|a, b| a.date.cmp(&b.date));
        Ok(Analytics {
            total_seconds: total,
            total_events: events.len() as i64,
            per_app,
            per_day,
        })
    }

    pub fn purge_if_needed(&mut self) -> anyhow::Result<()> {
        let Some(days) = self.config.auto_purge_days else {
            return Ok(());
        };
        let now = Utc::now();
        if self
            .last_purge
            .map(|last| (now - last).num_hours() < 1)
            .unwrap_or(false)
        {
            return Ok(());
        }
        let cutoff = now - chrono::Duration::days(days as i64);
        let keep: Vec<_> = self
            .events
            .iter()
            .filter(|e| e.end >= cutoff)
            .cloned()
            .collect();
        if keep.len() != self.events.len() {
            self.store
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("Log unavailable"))?
                .replace(&keep)?;
            self.events = keep;
        }
        self.last_purge = Some(now);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn setup() -> (tempfile::TempDir, Core) {
        let dir = tempfile::tempdir().unwrap();
        let mut core = Core::new(Config {
            log_path: dir.path().join("activity.log"),
            ..Config::default()
        });
        core.open_store(None).unwrap();
        (dir, core)
    }
    fn focus(app: &str) -> Option<Focus> {
        Some(Focus {
            app: app.into(),
            title: "Document".into(),
        })
    }
    fn tick(core: &mut Core, app: &str, start: DateTime<Utc>, seconds: i64) {
        core.tick(focus(app), Some(0), start + Duration::seconds(seconds));
    }

    #[test]
    fn focus_changes_preserve_boundaries_and_live_analytics() {
        let (_dir, mut core) = setup();
        let start = Utc::now();
        tick(&mut core, "Code", start, 0);
        tick(&mut core, "Code", start, 2);
        assert_eq!(
            core.analytics(&EventFilter::default())
                .unwrap()
                .total_seconds,
            2
        );
        tick(&mut core, "Browser", start, 4);
        tick(&mut core, "Browser", start, 6);
        let events = core.query(&EventFilter::default()).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].seconds, 4);
        assert_eq!(events[0].seconds, 2);
    }

    #[test]
    fn checkpoints_recover_without_duplicate_sessions() {
        let (_dir, mut core) = setup();
        let start = Utc::now();
        for second in 0..=65 {
            tick(&mut core, "Code", start, second);
        }
        let disk = core.store.as_ref().unwrap().all().unwrap();
        assert_eq!(disk.len(), 1);
        assert_eq!(disk[0].seconds, 60);
        assert_eq!(core.query(&EventFilter::default()).unwrap()[0].seconds, 65);
        core.flush_current().unwrap();
        core.open_store(None).unwrap();
        assert_eq!(core.events.len(), 1);
        assert_eq!(core.events[0].seconds, 65);
    }

    #[test]
    fn idle_and_suspend_do_not_count_away_time() {
        let (_dir, mut core) = setup();
        let start = Utc::now();
        core.config.idle_threshold_seconds = Some(30);
        for second in 0..30 {
            core.tick(
                focus("Code"),
                Some(second as u64),
                start + Duration::seconds(second),
            );
        }
        core.tick(focus("Code"), Some(32), start + Duration::seconds(32));
        assert!(core.status().idle);
        assert_eq!(core.events[0].seconds, 30);
        tick(&mut core, "Code", start, 34);
        tick(&mut core, "Code", start, 36);
        tick(&mut core, "Code", start, 3600);
        assert_eq!(
            core.analytics(&EventFilter::default())
                .unwrap()
                .total_seconds,
            32
        );
    }

    #[test]
    fn exclusions_stop_logging_and_hide_sensitive_status() {
        let (_dir, mut core) = setup();
        let start = Utc::now();
        core.config.exclusion_enabled = true;
        core.config.blocklist = vec!["secret".into()];
        tick(&mut core, "Code", start, 0);
        tick(&mut core, "Code", start, 2);
        tick(&mut core, "SECRET", start, 4);
        tick(&mut core, "SECRET", start, 8);
        assert!(core.status().excluded);
        assert!(core.status().current_app.is_none());
        assert_eq!(core.events.len(), 1);
        assert_eq!(core.events[0].seconds, 4);
    }

    #[test]
    fn failed_write_retains_session_and_pauses_until_retry() {
        let (dir, mut core) = setup();
        let start = Utc::now();
        tick(&mut core, "Code", start, 0);
        tick(&mut core, "Code", start, 2);
        std::fs::remove_file(core.active_path()).unwrap();
        tick(&mut core, "Browser", start, 4);
        assert!(core.paused);
        assert!(core.last_error.is_some());
        assert!(core.current.is_some());
        tick(&mut core, "Browser", start, 6);
        assert_eq!(core.current_event().unwrap().seconds, 4);
        std::fs::write(dir.path().join("activity.log"), "").unwrap();
        core.retry_storage().unwrap();
        assert_eq!(core.events[0].seconds, 4);
        assert!(core.last_error.is_none());
    }

    #[test]
    fn encryption_migrates_both_directions_and_removes_source() {
        let (_dir, mut core) = setup();
        let start = Utc::now();
        tick(&mut core, "Code", start, 0);
        tick(&mut core, "Code", start, 2);
        let plain = core.active_path();
        let mut config = core.config.clone();
        config.encryption_enabled = true;
        core.apply_config_with_save(config, Some("test-password"), |_| Ok(()))
            .unwrap();
        assert!(!plain.exists());
        assert_eq!(core.events.len(), 1);
        let encrypted = core.active_path();
        assert!(EncryptedStore::open(encrypted.clone(), "wrong").is_err());
        let mut config = core.config.clone();
        config.encryption_enabled = false;
        core.apply_config_with_save(config, None, |_| Ok(()))
            .unwrap();
        assert!(!encrypted.exists());
        assert!(plain.exists());
        assert_eq!(core.events.len(), 1);
    }

    #[test]
    fn failed_settings_commit_keeps_old_configuration_and_store() {
        let (_dir, mut core) = setup();
        let old = core.active_path();
        let now = Utc::now();
        tick(&mut core, "Code", now, 0);
        tick(&mut core, "Code", now, 2);
        let mut config = core.config.clone();
        config.encryption_enabled = true;
        assert!(core
            .apply_config_with_save(config, Some("test"), |_| anyhow::bail!("disk full"))
            .is_err());
        assert!(!core.config.encryption_enabled);
        assert_eq!(core.active_path(), old);
        assert!(!core.store.as_ref().unwrap().is_encrypted());
        assert!(old.exists());
        let destination = EncryptedStore::open(old.with_extension("enc"), "test").unwrap();
        assert!(destination.all().unwrap().is_empty());
        assert_eq!(core.events.len(), 1);
    }

    #[test]
    fn bad_destination_or_password_does_not_change_active_store() {
        let (_dir, mut core) = setup();
        let mut config = core.config.clone();
        config.log_path = core.active_path().join("impossible.log");
        assert!(core
            .apply_config_with_save(config, None, |_| Ok(()))
            .is_err());
        assert!(core.store.is_some());
        assert!(!core.config.encryption_enabled);
        let mut config = core.config.clone();
        config.encryption_enabled = true;
        assert!(core
            .apply_config_with_save(config, None, |_| Ok(()))
            .is_err());
        assert!(!core.config.encryption_enabled);
        assert!(core.store.is_some());
    }

    #[test]
    fn retention_rewrites_only_survivors() {
        let (_dir, mut core) = setup();
        let now = Utc::now();
        let old = Event::new(
            now - Duration::days(10),
            now - Duration::days(10) + Duration::seconds(30),
            "Old".into(),
            "".into(),
        );
        let recent = Event::new(now - Duration::seconds(60), now, "Recent".into(), "".into());
        core.events = vec![old, recent];
        core.store.as_mut().unwrap().replace(&core.events).unwrap();
        core.config.auto_purge_days = Some(7);
        core.purge_if_needed().unwrap();
        assert_eq!(core.events.len(), 1);
        assert_eq!(core.store.as_ref().unwrap().all().unwrap()[0].app, "Recent");
    }

    #[test]
    fn analytics_splits_midnight_and_clips_requested_range() {
        let (_dir, mut core) = setup();
        let midnight = Local
            .with_ymd_and_hms(2026, 1, 15, 0, 0, 0)
            .unwrap()
            .with_timezone(&Utc);
        core.events = vec![Event::new(
            midnight - Duration::seconds(10),
            midnight + Duration::seconds(20),
            "Code".into(),
            "".into(),
        )];
        let all = core.analytics(&EventFilter::default()).unwrap();
        assert_eq!(all.total_seconds, 30);
        assert_eq!(all.per_day.len(), 2);
        assert_eq!(all.per_day[0].seconds, 10);
        assert_eq!(all.per_day[1].seconds, 20);
        let range = core
            .analytics(&EventFilter {
                from: Some(midnight),
                to: Some(midnight + Duration::seconds(5)),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(range.total_seconds, 5);
        assert_eq!(range.per_day.len(), 1);
    }

    #[test]
    fn clear_drops_current_session_as_well_as_checkpoints() {
        let (_dir, mut core) = setup();
        let start = Utc::now();
        for second in 0..=35 {
            tick(&mut core, "Code", start, second);
        }
        core.clear().unwrap();
        core.flush_current().unwrap();
        assert!(core.events.is_empty());
        assert!(core.store.as_ref().unwrap().all().unwrap().is_empty());
    }
}
