//! Application core: the single source of truth the capture loop, the Tauri
//! commands, and the tray all share (behind one `Mutex`).
//!
//! It owns the config, the active [`Store`], and the in-progress focus session.

use crate::capture::window::Focus;
use crate::config::Config;
use crate::model::{Analytics, AppBucket, DayBucket, Event, EventFilter, Status};
use crate::storage::{self, encrypted::EncryptedStore, plaintext::PlainStore, Store};
use chrono::{DateTime, Local, Utc};
use std::collections::HashMap;

/// A focus session that hasn't ended yet.
struct SessionInProgress {
    app: String,
    title: String,
    start: DateTime<Utc>,
    last_seen: DateTime<Utc>,
}

pub struct Core {
    pub config: Config,
    pub paused: bool,
    /// `None` means the active store is encrypted but still locked this session.
    store: Option<Box<dyn Store>>,
    current: Option<SessionInProgress>,
    /// Last observed focus, kept only for live status display (never logged
    /// when excluded/paused).
    display: Option<Focus>,
    excluded_now: bool,
}

impl Core {
    /// Build the core from saved config. An encrypted store starts locked:
    /// the user must supply the password via `unlock` before logging begins.
    pub fn bootstrap() -> Self {
        let config = Config::load();
        let mut core = Core {
            config,
            paused: false,
            store: None,
            current: None,
            display: None,
            excluded_now: false,
        };
        if !core.config.encryption_enabled {
            core.store = PlainStore::open(core.active_path())
                .ok()
                .map(|s| Box::new(s) as Box<dyn Store>);
        }
        core.purge_if_needed();
        core
    }

    pub fn active_path(&self) -> std::path::PathBuf {
        storage::active_path(&self.config.log_path, self.config.encryption_enabled)
    }

    /// (Re)open the store. For encrypted mode a password is required.
    pub fn open_store(&mut self, password: Option<&str>) -> anyhow::Result<()> {
        let path = self.active_path();
        if self.config.encryption_enabled {
            let pw = password.ok_or_else(|| anyhow::anyhow!("password required"))?;
            let s = EncryptedStore::open(path, pw)?;
            self.store = Some(Box::new(s));
        } else {
            let s = PlainStore::open(path)?;
            self.store = Some(Box::new(s));
        }
        self.purge_if_needed();
        Ok(())
    }

    pub fn is_locked(&self) -> bool {
        self.config.encryption_enabled && self.store.is_none()
    }

    fn is_excluded(&self, f: &Focus) -> bool {
        if !self.config.exclusion_enabled {
            return false;
        }
        let app = f.app.to_lowercase();
        let title = f.title.to_lowercase();
        self.config.blocklist.iter().any(|b| {
            let b = b.trim().to_lowercase();
            !b.is_empty() && (app.contains(&b) || title.contains(&b))
        })
    }

    /// One sample from the capture loop. `now` is injected so this is unit-testable.
    ///
    /// Capture logic in one place:
    ///  - If we're not logging (paused / locked / no window / excluded app in
    ///    focus), end any running session and record nothing.
    ///  - Otherwise extend the current session if the window is unchanged, or
    ///    flush it and start a new one when focus moves.
    pub fn tick(&mut self, focus: Option<Focus>, now: DateTime<Utc>) {
        self.display = focus.clone();
        self.excluded_now = focus.as_ref().map(|f| self.is_excluded(f)).unwrap_or(false);

        let logging = !self.paused && self.store.is_some();
        let skip = !logging || self.excluded_now || focus.is_none();
        if skip {
            self.flush_current();
            return;
        }

        let f = focus.unwrap();
        match &mut self.current {
            Some(s) if s.app == f.app && s.title == f.title => {
                s.last_seen = now;
            }
            _ => {
                self.flush_current();
                self.current = Some(SessionInProgress {
                    app: f.app,
                    title: f.title,
                    start: now,
                    last_seen: now,
                });
            }
        }
    }

    /// Persist the running session (if any) and clear it. Sessions shorter than
    /// one second (a window merely passed through) are dropped.
    pub fn flush_current(&mut self) {
        if let Some(s) = self.current.take() {
            if (s.last_seen - s.start).num_seconds() >= 1 {
                if let Some(store) = self.store.as_mut() {
                    let e = Event::new(s.start, s.last_seen, s.app, s.title);
                    let _ = store.append(&e); // best-effort; a failed write shouldn't crash capture
                }
            }
        }
    }

    pub fn status(&self) -> Status {
        let events_total = self
            .store
            .as_ref()
            .and_then(|s| s.all().ok())
            .map(|v| v.len() as i64)
            .unwrap_or(0);
        Status {
            paused: self.paused,
            encrypted: self.config.encryption_enabled,
            locked: self.is_locked(),
            current_app: self.display.as_ref().map(|f| f.app.clone()),
            current_title: self.display.as_ref().map(|f| f.title.clone()),
            events_total,
            log_path: self.active_path().display().to_string(),
        }
    }

    pub fn query(&self, filter: &EventFilter) -> anyhow::Result<Vec<Event>> {
        match &self.store {
            Some(s) => s.query(filter),
            None => Err(anyhow::anyhow!("log is locked")),
        }
    }

    pub fn clear(&mut self) -> anyhow::Result<()> {
        self.current = None;
        match self.store.as_mut() {
            Some(s) => s.clear(),
            None => Err(anyhow::anyhow!("log is locked")),
        }
    }

    /// Aggregate stored events into per-app and per-day time buckets.
    pub fn analytics(&self, filter: &EventFilter) -> anyhow::Result<Analytics> {
        let events = self.query(filter)?;
        let mut app_secs: HashMap<String, (i64, i64)> = HashMap::new();
        let mut day_secs: HashMap<String, i64> = HashMap::new();
        let mut total = 0i64;
        for e in &events {
            total += e.seconds;
            let a = app_secs.entry(e.app.clone()).or_insert((0, 0));
            a.0 += e.seconds;
            a.1 += 1;
            // Bucket by LOCAL calendar day so the chart matches the user's clock.
            let day = e.start.with_timezone(&Local).format("%Y-%m-%d").to_string();
            *day_secs.entry(day).or_insert(0) += e.seconds;
        }
        let mut per_app: Vec<AppBucket> = app_secs
            .into_iter()
            .map(|(app, (seconds, events))| AppBucket {
                app,
                seconds,
                events,
            })
            .collect();
        per_app.sort_by_key(|a| std::cmp::Reverse(a.seconds));

        let mut per_day: Vec<DayBucket> = day_secs
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

    /// Drop events older than `auto_purge_days` (if set). Best-effort: rewrites
    /// the store with only the surviving events.
    pub fn purge_if_needed(&mut self) {
        let Some(days) = self.config.auto_purge_days else {
            return;
        };
        let Some(store) = self.store.as_mut() else {
            return;
        };
        let cutoff = Utc::now() - chrono::Duration::days(days as i64);
        let Ok(all) = store.all() else { return };
        if all.iter().all(|e| e.start >= cutoff) {
            return; // nothing to drop
        }
        let keep: Vec<Event> = all.into_iter().filter(|e| e.start >= cutoff).collect();
        if store.clear().is_ok() {
            for e in &keep {
                let _ = store.append(e);
            }
        }
    }
}
