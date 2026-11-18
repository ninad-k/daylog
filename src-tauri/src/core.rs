//! Shared tracking state, storage transitions, and filtered insights.
use crate::capture::window::Focus;
use crate::config::Config;
use crate::model::{category_for, Analytics, ApplicationDetails, Event, EventFilter, Status};
use crate::storage::{self, encrypted::EncryptedStore, plaintext::PlainStore, Store};
use chrono::{DateTime, Utc};
use std::time::Duration;

struct SessionInProgress {
    id: i64,
    app: String,
    title: String,
    start: DateTime<Utc>,
    last_seen: DateTime<Utc>,
    typed_text: String,
    typed_chars: usize,
}

pub struct Core {
    pub config: Config,
    pub paused: bool,
    store: Option<Box<dyn Store>>,
    current: Option<SessionInProgress>,
    pending: Vec<Event>,
    idle_corrections: Vec<(DateTime<Utc>, DateTime<Utc>)>,
    display: Option<Focus>,
    excluded_now: bool,
    idle: bool,
    idle_detection_available: bool,
    pub storage_error: Option<String>,
    last_tick: Option<DateTime<Utc>>,
    last_purge: DateTime<Utc>,
    last_session_id: i64,
    config_load_failed: bool,
}

impl Core {
    pub fn bootstrap() -> Self {
        Self::from_loaded_config(Config::load())
    }

    fn from_loaded_config(config: anyhow::Result<Config>) -> Self {
        match config {
            Ok(config) => Self::from_config(config),
            Err(error) => {
                let mut core = Self::empty(Config::default());
                core.paused = true;
                core.config_load_failed = true;
                core.storage_error = Some(format!("Configuration could not be loaded: {error:#}. Logging is stopped. Correct the configuration file or explicitly save valid settings to recover."));
                core
            }
        }
    }

    fn from_config(config: Config) -> Self {
        let mut core = Self::empty(config);
        if !core.config.encryption_enabled {
            if let Err(e) = core.open_store(None) {
                core.storage_error = Some(e.to_string());
            }
        }
        core
    }

    fn empty(config: Config) -> Self {
        Self {
            config,
            paused: false,
            store: None,
            current: None,
            pending: Vec::new(),
            idle_corrections: Vec::new(),
            display: None,
            excluded_now: false,
            idle: false,
            idle_detection_available: false,
            storage_error: None,
            last_tick: None,
            last_purge: Utc::now(),
            last_session_id: i64::MIN,
            config_load_failed: false,
        }
    }

    pub fn active_path(&self) -> std::path::PathBuf {
        storage::active_path(&self.config.log_path, self.config.encryption_enabled)
    }

    pub fn open_store(&mut self, password: Option<&str>) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.config_load_failed,
            "correct the configuration file or save valid settings before opening storage"
        );
        anyhow::ensure!(
            self.pending.is_empty(),
            "retry pending log writes before reopening storage"
        );
        let store: Box<dyn Store> = if self.config.encryption_enabled {
            let pw = password
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("password required"))?;
            Box::new(EncryptedStore::open(self.active_path(), pw)?)
        } else {
            Box::new(PlainStore::open(self.active_path())?)
        };
        self.last_session_id = self.last_session_id.max(
            store
                .events()
                .iter()
                .map(|event| event.id)
                .max()
                .unwrap_or(i64::MIN),
        );
        self.store = Some(store);
        self.storage_error = None;
        self.purge_if_needed();
        Ok(())
    }

    pub fn is_locked(&self) -> bool {
        self.config.encryption_enabled && self.store.is_none()
    }

    pub fn has_pending_writes(&self) -> bool {
        !self.pending.is_empty()
    }

    pub fn apply_config(
        &mut self,
        mut config: Config,
        password: Option<&str>,
    ) -> anyhow::Result<()> {
        config.email.last_sent = self.config.email.last_sent.clone();
        config.email.sent_periods = self.config.email.sent_periods.clone();
        config.validate()?;
        if self.config_load_failed {
            let path = storage::active_path(&config.log_path, config.encryption_enabled);
            let store: Box<dyn Store> = if config.encryption_enabled {
                Box::new(EncryptedStore::open(
                    path,
                    password.ok_or_else(|| {
                        anyhow::anyhow!("enter the encryption password to recover settings")
                    })?,
                )?)
            } else {
                Box::new(PlainStore::open(path)?)
            };
            config.save()?;
            self.config = config;
            self.store = Some(store);
            self.config_load_failed = false;
            self.storage_error = None;
            self.purge_if_needed();
            return Ok(());
        }
        let changed = config.log_path != self.config.log_path
            || config.encryption_enabled != self.config.encryption_enabled;
        if changed {
            self.flush_current();
            anyhow::ensure!(
                self.pending.is_empty(),
                "log write failed; storage settings were not changed"
            );
            let source = self.store.as_ref().ok_or_else(|| {
                anyhow::anyhow!("unlock the log before changing storage settings")
            })?;
            let outcome = storage::migrate(
                source.as_ref(),
                storage::active_path(&config.log_path, config.encryption_enabled),
                config.encryption_enabled,
                password,
                || config.save(),
            )?;
            self.store = Some(outcome.store);
            self.storage_error = outcome.cleanup_warning;
        } else {
            config.save()?;
        }
        self.config = config;
        self.purge_if_needed();
        Ok(())
    }

    pub fn set_encryption(&mut self, enabled: bool, password: Option<&str>) -> anyhow::Result<()> {
        if enabled == self.config.encryption_enabled {
            return Ok(());
        }
        let mut config = self.config.clone();
        config.encryption_enabled = enabled;
        self.apply_config(config, password)
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

    pub fn typing_allowed(&self, focus: &Focus) -> bool {
        self.config.text_capture_enabled
            && self.config.encryption_enabled
            && self
                .store
                .as_ref()
                .is_some_and(|store| store.is_encrypted())
            && !self.paused
            && self.pending.is_empty()
            && !self.is_excluded(focus)
            && (self.config.text_capture_all_apps
                || self
                    .config
                    .text_capture_apps
                    .iter()
                    .any(|app| app.trim().eq_ignore_ascii_case(focus.app.trim())))
    }

    fn begin_session(&mut self, focus: &Focus, now: DateTime<Utc>) {
        self.last_session_id = now
            .timestamp_micros()
            .max(self.last_session_id.saturating_add(1));
        self.current = Some(SessionInProgress {
            id: self.last_session_id,
            app: focus.app.clone(),
            title: focus.title.clone(),
            start: now,
            last_seen: now,
            typed_text: String::new(),
            typed_chars: 0,
        });
    }

    /// Accept text only after the native capture guard has validated its field.
    /// Recheck consent and the sampled window context while holding Core's lock.
    pub fn record_text(&mut self, focus: &Focus, text: &str, now: DateTime<Utc>) -> bool {
        if !self.typing_allowed(focus)
            || !self
                .display
                .as_ref()
                .is_some_and(|display| display.app == focus.app && display.title == focus.title)
            || self.current.as_ref().is_some_and(|session| {
                session.app != focus.app || session.title != focus.title || now < session.start
            })
        {
            return false;
        }
        if self.current.is_none() {
            self.begin_session(focus, now);
        }
        self.idle = false;
        for character in text.chars() {
            if character == '\u{0008}' {
                let session = self.current.as_mut().unwrap();
                if session.typed_text.pop().is_some() {
                    session.typed_chars -= 1;
                }
                session.last_seen = session.last_seen.max(now);
                continue;
            }
            if character.is_control() && !matches!(character, '\n' | '\r' | '\t') {
                continue;
            }
            if self.current.as_ref().unwrap().typed_chars >= 16 * 1024 {
                self.flush_current();
                self.begin_session(focus, now);
            }
            let session = self.current.as_mut().unwrap();
            session.last_seen = session.last_seen.max(now);
            session.typed_text.push(character);
            session.typed_chars += 1;
        }
        true
    }

    pub fn tick_with_idle(
        &mut self,
        focus: Option<Focus>,
        now: DateTime<Utc>,
        idle: Option<Duration>,
    ) {
        self.retry_pending();
        self.display = focus.clone();
        self.excluded_now = focus.as_ref().is_some_and(|f| self.is_excluded(f));
        self.idle_detection_available = idle.is_some();
        let was_idle = self.idle;
        self.idle = self
            .config
            .idle_threshold_seconds
            .zip(idle)
            .is_some_and(|(threshold, elapsed)| elapsed.as_secs() >= threshold);
        let mut idle_correction_error = None;
        if self.idle {
            if let Some(elapsed) = idle {
                if let Ok(delta) = chrono::Duration::from_std(elapsed) {
                    if let Some(last_input) = now.checked_sub_signed(delta) {
                        if let Some(session) = &mut self.current {
                            session.last_seen =
                                session.last_seen.min(last_input).max(session.start);
                        }
                        if !was_idle {
                            self.idle_corrections.push((last_input, now));
                        }
                    }
                }
            }
        }
        // Keep the original observed interval if storage is temporarily
        // unavailable. Retrying after activity resumes must not trim new work.
        while let Some(&(last_input, observed_at)) = self.idle_corrections.first() {
            if let Err(error) = self.trim_idle_tail(last_input, observed_at) {
                let message = format!("Idle correction failed: {error}");
                self.storage_error = Some(message.clone());
                idle_correction_error = Some(message);
                break;
            }
            self.idle_corrections.remove(0);
            if self
                .storage_error
                .as_deref()
                .is_some_and(|error| error.starts_with("Idle correction failed:"))
            {
                self.storage_error = None;
            }
        }
        // A suspended machine or a stalled capture call must not create hours of focus time.
        let gap = self.last_tick.is_some_and(|last| {
            (now - last).num_milliseconds() > (self.config.poll_interval_ms * 3).max(5000) as i64
                || now < last
        });
        self.last_tick = Some(now);
        if gap {
            self.flush_current();
        }
        if self.paused
            || self.store.is_none()
            || self.excluded_now
            || self.idle
            || focus.is_none()
            || !self.pending.is_empty()
        {
            self.flush_current();
            // A successful append must not hide a failed history correction;
            // retain the error while a correction is still pending.
            if idle_correction_error.is_some() {
                self.storage_error = idle_correction_error;
            }
            return;
        }
        let f = focus.unwrap();
        if let Some(s) = &mut self.current {
            s.last_seen = now;
        }
        match &self.current {
            Some(s) if s.app == f.app && s.title == f.title => {}
            _ => {
                self.flush_current();
                if self.pending.is_empty() {
                    self.begin_session(&f, now);
                }
            }
        }
        if (now - self.last_purge).num_hours() >= 1 {
            self.purge_if_needed();
        }
        if idle_correction_error.is_some() {
            self.storage_error = idle_correction_error;
        }
    }

    fn trim_idle_tail(
        &mut self,
        last_input: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        // Focus can move without input (for example a notification window).
        // Such sessions may already have been flushed before the idle threshold
        // is reached. Correct them alongside the live session, preserving IDs.
        // A future event may belong to imported history or a changed clock;
        // do not alter it based on the current OS idle observation.
        let overlaps =
            |event: &Event| event.end > last_input && event.end <= now && event.start < now;
        let trim = |event: &mut Event| {
            if !overlaps(event) {
                return true;
            }
            let has_text = event
                .typed_text
                .as_ref()
                .is_some_and(|text| !text.is_empty());
            if event.start >= last_input && !has_text {
                return false;
            }
            event.end = last_input.max(event.start);
            event.seconds = (event.end - event.start).num_seconds().max(0);
            event.seconds > 0 || has_text
        };
        self.pending.retain_mut(trim);
        if let Some(store) = self.store.as_mut() {
            if store.events().iter().any(overlaps) {
                let mut events = store.events().to_vec();
                events.retain_mut(trim);
                store.replace(&events)?;
            }
        }
        Ok(())
    }

    fn retry_pending(&mut self) {
        let Some(store) = self.store.as_mut() else {
            return;
        };
        while let Some(event) = self.pending.first() {
            match store.append(event) {
                Ok(()) => {
                    self.pending.remove(0);
                    self.storage_error = None;
                }
                Err(e) => {
                    self.storage_error = Some(format!("Log write failed: {e}"));
                    break;
                }
            }
        }
    }

    pub fn flush_current(&mut self) {
        if let Some(s) = self.current.take() {
            if (s.last_seen - s.start).num_seconds() >= 1 || !s.typed_text.is_empty() {
                let mut event = Event::new(s.start, s.last_seen, s.app, s.title);
                event.id = s.id;
                event.typed_text = (!s.typed_text.is_empty()).then_some(s.typed_text);
                self.pending.push(event);
                self.pending.sort_by_key(|event| (event.start, event.id));
            }
        }
        self.retry_pending();
    }

    fn live_event(&self) -> Option<Event> {
        self.current
            .as_ref()
            .filter(|s| (s.last_seen - s.start).num_seconds() >= 1 || !s.typed_text.is_empty())
            .map(|s| {
                let mut event = Event::new(s.start, s.last_seen, s.app.clone(), s.title.clone());
                event.id = s.id;
                event.typed_text = (!s.typed_text.is_empty()).then(|| s.typed_text.clone());
                event
            })
    }

    pub fn status(&self) -> Status {
        Status {
            paused: self.paused,
            encrypted: self.config.encryption_enabled,
            locked: self.is_locked(),
            current_app: self.display.as_ref().map(|f| f.app.clone()),
            current_title: self
                .display
                .as_ref()
                .filter(|_| !self.excluded_now)
                .map(|f| f.title.clone()),
            events_total: self.store.as_ref().map_or(0, |s| s.len() as i64)
                + self.pending.len() as i64
                + i64::from(self.live_event().is_some()),
            log_path: self.active_path().display().to_string(),
            idle: self.idle,
            idle_detection_available: self.idle_detection_available,
            excluded: self.excluded_now,
            storage_error: self.storage_error.clone(),
            text_capture_enabled: self.config.text_capture_enabled,
            text_recording: crate::capture::typing::active()
                && self
                    .display
                    .as_ref()
                    .is_some_and(|focus| self.typing_allowed(focus)),
            text_capture_supported: crate::capture::typing::supported(),
        }
    }

    pub fn query(&self, filter: &EventFilter) -> anyhow::Result<Vec<Event>> {
        let limit = filter.limit.unwrap_or(usize::MAX);
        let mut skip = filter.offset.unwrap_or(0);
        let mut rows = Vec::new();
        self.visit_matching(filter, |event| {
            if limit == 0 {
                return false;
            }
            if skip > 0 {
                skip -= 1;
                return true;
            }
            rows.push(event.clone());
            rows.len() < limit
        })?;
        Ok(rows)
    }

    /// Iterate cached/pending/live matches in newest-first order, cloning only
    /// when the caller chooses to retain an event. Return false to stop early.
    fn visit_matching(
        &self,
        filter: &EventFilter,
        mut visit: impl FnMut(&Event) -> bool,
    ) -> anyhow::Result<()> {
        filter.validate()?;
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("log is locked or storage unavailable"))?;
        let matcher = filter.compile();
        let live = self.live_event();
        let mut sources = [store.events(), self.pending.as_slice(), live.as_slice()]
            .map(|events| events.iter().rev().peekable());
        // Merge the persisted, retry-pending, and live histories in newest-first
        // order. Pagination stops the scan early and clones only returned rows.
        // Comparing all three heads also handles a clock adjustment that puts a
        // live session before the newest persisted event.
        loop {
            let newest = sources
                .iter_mut()
                .enumerate()
                .filter_map(|(index, events)| {
                    events.peek().map(|event| (index, event.start, event.id))
                })
                .max_by_key(|(_, start, id)| (*start, *id));
            let Some((index, _, _)) = newest else {
                break;
            };
            let event = sources[index].next().unwrap();
            if !matcher.matches_with_categories(event, &self.config.app_categories) {
                continue;
            }
            if !visit(event) {
                break;
            }
        }
        Ok(())
    }

    pub fn application_details(
        &self,
        app: &str,
        filter: &EventFilter,
        text_only: bool,
    ) -> anyhow::Result<ApplicationDetails> {
        let app = app.trim();
        anyhow::ensure!(!app.is_empty(), "an application name is required");
        let mut scoped = filter.clone();
        scoped.app = None;
        scoped.app_exact = Some(app.to_owned());
        let limit = scoped.limit.unwrap_or(50).clamp(1, 200);
        let offset = scoped.offset.unwrap_or(0);
        let mut details = ApplicationDetails {
            app: app.to_owned(),
            category: category_for(app, &self.config.app_categories),
            total_seconds: 0,
            total_sessions: 0,
            text_sessions: 0,
            recorded_characters: 0,
            last_seen: None,
            events: Vec::new(),
            has_more: false,
        };
        let mut display_matches = 0usize;
        self.visit_matching(&scoped, |event| {
            details.total_sessions = details.total_sessions.saturating_add(1);
            details.total_seconds = details
                .total_seconds
                .saturating_add(scoped.clipped_seconds(event));
            details.last_seen = Some(
                details
                    .last_seen
                    .map_or(event.end, |last| last.max(event.end)),
            );
            let text = event.typed_text.as_deref().filter(|text| !text.is_empty());
            if let Some(text) = text {
                details.text_sessions = details.text_sessions.saturating_add(1);
                details.recorded_characters = details
                    .recorded_characters
                    .saturating_add(text.chars().count());
            }
            if !text_only || text.is_some() {
                if display_matches >= offset && details.events.len() < limit {
                    details.events.push(event.clone());
                }
                display_matches = display_matches.saturating_add(1);
            }
            true
        })?;
        details.has_more = display_matches.saturating_sub(offset) > details.events.len();
        Ok(details)
    }

    pub fn clear(&mut self) -> anyhow::Result<()> {
        self.store
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("log is locked"))?
            .clear()?;
        self.current = None;
        self.pending.clear();
        self.idle_corrections.clear();
        self.storage_error = None;
        self.last_session_id = i64::MIN;
        Ok(())
    }

    pub fn clear_recorded_text(&mut self) -> anyhow::Result<()> {
        self.clear_recorded_text_with(Config::save)
    }

    fn clear_recorded_text_with(
        &mut self,
        persist: impl FnOnce(&Config) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.store.is_some(),
            "unlock the log before deleting recorded text"
        );
        let mut config = self.config.clone();
        config.text_capture_enabled = false;
        persist(&config)?;
        self.config = config;
        let store = self
            .store
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("unlock the log before deleting recorded text"))?;
        let mut events = store.events().to_vec();
        for event in &mut events {
            event.typed_text = None;
        }
        if let Err(error) = store.replace(&events) {
            self.storage_error = Some(format!(
                "Text recording is disabled, but deleting recorded text failed: {error}"
            ));
            return Err(error);
        }
        for event in &mut self.pending {
            event.typed_text = None;
        }
        if let Some(session) = &mut self.current {
            session.typed_text.clear();
            session.typed_chars = 0;
        }
        self.storage_error = None;
        Ok(())
    }

    pub fn analytics(&self, filter: &EventFilter) -> anyhow::Result<Analytics> {
        let mut unlimited = filter.clone();
        unlimited.limit = None;
        unlimited.offset = None;
        let events: Vec<Event> = self
            .query(&unlimited)?
            .iter()
            .filter_map(|e| filter.clipped(e))
            .collect();
        let now = Utc::now();
        let mut analytics =
            crate::insights::analytics(&events, &self.config.app_categories, &[], now);
        let all = self.query(&EventFilter::default())?;
        analytics.goals = crate::insights::goal_progress(
            &all,
            &self.config.app_categories,
            &self.config.time_goals,
            now,
        );
        Ok(analytics)
    }

    pub fn purge_if_needed(&mut self) {
        self.last_purge = Utc::now();
        let (Some(days), Some(store)) = (self.config.auto_purge_days, self.store.as_mut()) else {
            return;
        };
        let cutoff = Utc::now() - chrono::Duration::days(days as i64);
        if store.events().iter().any(|e| e.start < cutoff) {
            let keep: Vec<_> = store
                .events()
                .iter()
                .filter(|e| e.start >= cutoff)
                .cloned()
                .collect();
            if let Err(e) = store.replace(&keep) {
                self.storage_error = Some(format!("Auto-purge failed: {e}"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::AppCategory;
    use chrono::TimeZone;
    fn core(name: &str) -> Core {
        Core::from_config(Config {
            log_path: std::env::temp_dir()
                .join(format!("daylog-core-{}-{name}.log", std::process::id())),
            ..Config::default()
        })
    }
    fn focus() -> Option<Focus> {
        Some(Focus {
            app: "Code".into(),
            title: "test.rs".into(),
        })
    }

    fn text_core(name: &str) -> Core {
        let path = std::env::temp_dir().join(format!(
            "daylog-text-core-{}-{}-{name}.log",
            std::process::id(),
            rand::random::<u64>()
        ));
        let mut core = Core::from_config(Config {
            log_path: path,
            encryption_enabled: true,
            text_capture_enabled: true,
            text_capture_apps: vec!["Code".into()],
            ..Config::default()
        });
        core.open_store(Some("test encryption password")).unwrap();
        core
    }

    #[test]
    fn text_capture_requires_consent_encrypted_store_and_current_approved_context() {
        let mut c = text_core("consent");
        let t = Utc::now();
        let f = focus().unwrap();
        assert!(!c.record_text(&f, "before sampled focus", t));
        c.tick_with_idle(Some(f.clone()), t, Some(Duration::ZERO));
        assert!(c.typing_allowed(&f));
        c.paused = true;
        assert!(!c.record_text(&f, "paused", t));
        c.paused = false;
        let other = Focus {
            app: "Other".into(),
            title: f.title.clone(),
        };
        assert!(!c.record_text(&other, "unapproved", t));
        let stale = Focus {
            app: f.app.clone(),
            title: "different document".into(),
        };
        assert!(!c.record_text(&stale, "stale", t));
        c.config.exclusion_enabled = true;
        c.config.blocklist = vec!["test.rs".into()];
        assert!(!c.record_text(&f, "excluded", t));
        c.config.exclusion_enabled = false;
        c.config.text_capture_enabled = false;
        assert!(!c.record_text(&f, "disabled", t));
        c.config.text_capture_enabled = true;
        c.config.encryption_enabled = false;
        assert!(!c.record_text(&f, "plaintext config", t));
        c.config.encryption_enabled = true;
        let plain_path = c.config.log_path.clone();
        c.store = Some(Box::new(PlainStore::open(plain_path.clone()).unwrap()));
        assert!(!c.record_text(&f, "plaintext store despite encrypted flag", t));
        assert_eq!(std::fs::metadata(&plain_path).unwrap().len(), 0);
        std::fs::remove_file(plain_path).unwrap();
        c.store = None;
        assert!(!c.record_text(&f, "locked", t));
        assert!(c.query(&EventFilter::default()).is_err());
        std::fs::remove_file(c.active_path()).unwrap();
    }

    #[test]
    fn all_app_scope_accepts_unlisted_apps_without_bypassing_capture_safeguards() {
        let mut c = text_core("all-app-scope");
        let t = Utc::now();
        let focus = Focus {
            app: "ExampleEditor".into(),
            title: "An ordinary document".into(),
        };
        c.config.text_capture_apps.clear();
        c.tick_with_idle(Some(focus.clone()), t, Some(Duration::ZERO));
        assert!(!c.typing_allowed(&focus));
        assert!(!c.record_text(&focus, "not selected", t));
        c.config.text_capture_all_apps = true;
        assert!(c.typing_allowed(&focus));
        assert!(c.record_text(&focus, "ordinary sample", t));
        c.paused = true;
        assert!(!c.typing_allowed(&focus));
        assert!(!c.record_text(&focus, "paused", t));
        c.paused = false;
        c.config.exclusion_enabled = true;
        c.config.blocklist = vec!["ordinary document".into()];
        assert!(!c.typing_allowed(&focus));
        assert!(!c.record_text(&focus, "excluded", t));
        c.config.exclusion_enabled = false;
        c.config.text_capture_enabled = false;
        assert!(!c.typing_allowed(&focus));
        c.config.text_capture_enabled = true;
        c.config.encryption_enabled = false;
        assert!(!c.typing_allowed(&focus));
        c.config.encryption_enabled = true;
        let path = c.active_path();
        c.store = Some(Box::new(
            PlainStore::open(c.config.log_path.clone()).unwrap(),
        ));
        assert!(
            !c.typing_allowed(&focus),
            "a stale config flag cannot authorize plaintext recording"
        );
        c.store = None;
        assert!(!c.typing_allowed(&focus));
        assert!(!c.record_text(&focus, "locked", t));
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(c.config.log_path).unwrap();
    }

    #[test]
    fn configuration_load_failure_never_opens_a_default_plaintext_store() {
        let mut c =
            Core::from_loaded_config(Err(anyhow::anyhow!("damaged encrypted configuration")));
        assert!(c.paused);
        assert!(c.store.is_none());
        assert!(c.config_load_failed);
        assert!(c
            .status()
            .storage_error
            .unwrap()
            .contains("Logging is stopped"));
        assert!(c.open_store(Some("anything")).is_err());
        c.tick_with_idle(focus(), Utc::now(), Some(Duration::ZERO));
        assert!(c.current.is_none());
        assert!(c.store.is_none());
    }

    #[test]
    fn unicode_backspace_and_text_only_sessions_are_persisted_encrypted() {
        let mut c = text_core("unicode");
        let t = Utc::now();
        let f = focus().unwrap();
        c.tick_with_idle(Some(f.clone()), t, Some(Duration::ZERO));
        assert!(c.record_text(&f, "é🙂x\u{0008}\u{0008}한\n", t));
        let live = c.query(&EventFilter::default()).unwrap();
        assert_eq!(live[0].typed_text.as_deref(), Some("é한\n"));
        assert_eq!(live[0].seconds, 0);
        c.flush_current();
        let rows = c
            .query(&EventFilter {
                from: Some(t),
                keyword: Some("한".into()),
                ..EventFilter::default()
            })
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].typed_text.as_deref(), Some("é한\n"));
        assert_eq!(
            c.analytics(&EventFilter::default()).unwrap().total_seconds,
            0
        );
        assert_eq!(
            c.analytics(&EventFilter::default()).unwrap().total_events,
            0
        );
        let path = c.active_path();
        assert!(!std::fs::read_to_string(&path).unwrap().contains("é한"));
        drop(c);
        let reopened = EncryptedStore::open(path.clone(), "test encryption password").unwrap();
        assert_eq!(reopened.events()[0].typed_text.as_deref(), Some("é한\n"));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn long_text_splits_without_loss_or_duplicate_session_ids() {
        let mut c = text_core("chunks");
        let t = Utc::now();
        let f = focus().unwrap();
        c.tick_with_idle(Some(f.clone()), t, Some(Duration::ZERO));
        let text = "🙂".repeat(2 * 16 * 1024 + 3);
        assert!(c.record_text(&f, &text, t));
        c.flush_current();
        let rows = c.query(&EventFilter::default()).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(
            rows.iter()
                .map(|event| event.id)
                .collect::<std::collections::HashSet<_>>()
                .len(),
            3
        );
        let reconstructed = rows
            .iter()
            .rev()
            .filter_map(|event| event.typed_text.as_deref())
            .collect::<String>();
        assert_eq!(reconstructed, text);
        assert!(rows
            .iter()
            .all(|event| event.typed_text.as_ref().unwrap().chars().count() <= 16 * 1024));
        std::fs::remove_file(c.active_path()).unwrap();
    }

    #[test]
    fn deleting_recorded_text_preserves_activity_and_disables_recording() {
        let mut c = text_core("clear-text");
        let t = Utc::now();
        let f = focus().unwrap();
        c.tick_with_idle(Some(f.clone()), t, Some(Duration::ZERO));
        c.record_text(&f, "first text", t + chrono::Duration::seconds(1));
        c.flush_current();
        c.tick_with_idle(
            Some(f.clone()),
            t + chrono::Duration::seconds(2),
            Some(Duration::ZERO),
        );
        c.record_text(&f, "live text", t + chrono::Duration::seconds(3));
        let before = c.analytics(&EventFilter::default()).unwrap().total_seconds;
        assert!(c
            .clear_recorded_text_with(|_| Err(anyhow::anyhow!("configuration failure")))
            .is_err());
        assert!(c.config.text_capture_enabled);
        assert!(c
            .query(&EventFilter::default())
            .unwrap()
            .iter()
            .all(|event| event.typed_text.is_some()));
        c.clear_recorded_text_with(|config| {
            assert!(!config.text_capture_enabled);
            Ok(())
        })
        .unwrap();
        assert!(!c.config.text_capture_enabled);
        assert_eq!(
            c.analytics(&EventFilter::default()).unwrap().total_seconds,
            before
        );
        assert!(c
            .query(&EventFilter::default())
            .unwrap()
            .iter()
            .all(|event| event.typed_text.is_none()));
        assert!(!c.record_text(&f, "disabled after clear", t + chrono::Duration::seconds(4)));
        std::fs::remove_file(c.active_path()).unwrap();
    }

    #[test]
    fn failed_text_deletion_keeps_history_but_disables_recording() {
        struct UnavailableStore(Vec<Event>);
        impl Store for UnavailableStore {
            fn append(&mut self, _: &Event) -> anyhow::Result<()> {
                unreachable!()
            }
            fn events(&self) -> &[Event] {
                &self.0
            }
            fn replace(&mut self, _: &[Event]) -> anyhow::Result<()> {
                Err(anyhow::anyhow!("disk unavailable"))
            }
            fn path(&self) -> std::path::PathBuf {
                std::path::PathBuf::new()
            }
            fn relocate(&mut self, _: std::path::PathBuf) {}
            fn is_encrypted(&self) -> bool {
                true
            }
        }
        let mut c = text_core("failed-clear");
        let path = c.active_path();
        let t = Utc::now();
        let mut event = Event::new(t, t, "Code".into(), "document".into());
        event.typed_text = Some("preserved on failed deletion".into());
        c.store = Some(Box::new(UnavailableStore(vec![event])));
        assert!(c.clear_recorded_text_with(|_| Ok(())).is_err());
        assert!(!c.config.text_capture_enabled);
        assert_eq!(
            c.store.as_ref().unwrap().events()[0].typed_text.as_deref(),
            Some("preserved on failed deletion")
        );
        assert!(c
            .storage_error
            .unwrap()
            .contains("deleting recorded text failed"));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn idle_correction_retains_recorded_text_and_original_identity() {
        let mut c = text_core("text-idle");
        let t = Utc::now();
        let mut event = Event::new(
            t,
            t + chrono::Duration::seconds(10),
            "Code".into(),
            "test.rs".into(),
        );
        event.typed_text = Some("actual input".into());
        let id = event.id;
        c.store.as_mut().unwrap().append(&event).unwrap();
        c.trim_idle_tail(t, t + chrono::Duration::seconds(20))
            .unwrap();
        let rows = c.query(&EventFilter::default()).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, id);
        assert_eq!(rows[0].seconds, 0);
        assert_eq!(rows[0].typed_text.as_deref(), Some("actual input"));
        std::fs::remove_file(c.active_path()).unwrap();
    }

    #[test]
    fn application_details_scopes_exactly_and_keeps_totals_across_activity_and_text_pages() {
        let mut c = text_core("details-paging");
        c.config.app_categories = vec![AppCategory {
            app: "Chrome".into(),
            category: "Work".into(),
        }];
        let t = Utc::now();
        let make = |start, end, app: &str, text: Option<&str>| {
            let mut event = Event::new(
                t + chrono::Duration::seconds(start),
                t + chrono::Duration::seconds(end),
                app.into(),
                "Work report".into(),
            );
            event.typed_text = text.map(str::to_owned);
            event
        };
        for event in [
            make(0, 10, "Chrome", Some("a🙂é")),
            make(20, 40, "Chrome", None),
            make(50, 80, "Chrome", Some("你好")),
            make(90, 190, "Chrome Canary", Some("other application")),
        ] {
            c.store.as_mut().unwrap().append(&event).unwrap();
        }
        let filter = EventFilter {
            app: Some("ignored substring".into()),
            app_exact: Some("Chrome Canary".into()),
            limit: Some(1),
            ..EventFilter::default()
        };
        let first = c.application_details("cHrOmE", &filter, false).unwrap();
        assert_eq!(first.category, "Work");
        assert_eq!(first.total_sessions, 3);
        assert_eq!(first.total_seconds, 60);
        assert_eq!(first.text_sessions, 2);
        assert_eq!(first.recorded_characters, 5);
        assert_eq!(first.last_seen, Some(t + chrono::Duration::seconds(80)));
        assert_eq!(first.events.len(), 1);
        assert_eq!(first.events[0].typed_text.as_deref(), Some("你好"));
        assert!(first.has_more);
        let second_filter = EventFilter {
            offset: Some(1),
            ..filter.clone()
        };
        let activity_second = c
            .application_details("Chrome", &second_filter, false)
            .unwrap();
        assert!(activity_second.events[0].typed_text.is_none());
        assert!(activity_second.has_more);
        let text_second = c
            .application_details("Chrome", &second_filter, true)
            .unwrap();
        assert_eq!(text_second.events[0].typed_text.as_deref(), Some("a🙂é"));
        assert!(!text_second.has_more);
        assert_eq!(text_second.total_sessions, first.total_sessions);
        assert_eq!(text_second.total_seconds, first.total_seconds);
        assert_eq!(text_second.recorded_characters, first.recorded_characters);
        let exhausted = c
            .application_details(
                "Chrome",
                &EventFilter {
                    offset: Some(usize::MAX),
                    ..filter
                },
                true,
            )
            .unwrap();
        assert!(exhausted.events.is_empty());
        assert!(!exhausted.has_more);
        assert_eq!(exhausted.total_sessions, 3);
        std::fs::remove_file(c.active_path()).unwrap();
    }

    #[test]
    fn application_details_includes_live_and_pending_text_and_rejects_locked_store() {
        let mut c = text_core("details-live");
        let t = Utc::now();
        let f = focus().unwrap();
        let stored = Event::new(
            t - chrono::Duration::seconds(50),
            t - chrono::Duration::seconds(30),
            "Code".into(),
            "Old document".into(),
        );
        c.store.as_mut().unwrap().append(&stored).unwrap();
        c.tick_with_idle(Some(f.clone()), t, Some(Duration::ZERO));
        c.record_text(&f, "live🙂", t + chrono::Duration::seconds(2));
        let mut pending = Event::new(
            t - chrono::Duration::seconds(20),
            t - chrono::Duration::seconds(10),
            "Code".into(),
            "Pending document".into(),
        );
        pending.typed_text = Some("pending".into());
        c.pending.push(pending);
        let details = c
            .application_details("Code", &EventFilter::default(), false)
            .unwrap();
        assert_eq!(details.total_sessions, 3);
        assert_eq!(details.total_seconds, 32);
        assert_eq!(details.text_sessions, 2);
        assert_eq!(details.recorded_characters, 12);
        assert_eq!(details.events[0].typed_text.as_deref(), Some("live🙂"));
        assert_eq!(details.last_seen, Some(t + chrono::Duration::seconds(2)));
        assert!(!details.has_more);
        let path = c.active_path();
        c.store = None;
        assert!(c
            .application_details("Code", &EventFilter::default(), false)
            .is_err());
        assert!(c
            .application_details("Code", &EventFilter::default(), true)
            .is_err());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn application_details_combines_keyword_dates_and_category_with_clipped_time() {
        let mut c = text_core("details-filtered");
        c.config.app_categories = vec![AppCategory {
            app: "Code".into(),
            category: "Work".into(),
        }];
        let t = Utc::now();
        let mut event = Event::new(
            t,
            t + chrono::Duration::seconds(100),
            "Code".into(),
            "Project notes".into(),
        );
        event.typed_text = Some("searchable private note".into());
        c.store.as_mut().unwrap().append(&event).unwrap();
        let filter = EventFilter {
            from: Some(t + chrono::Duration::seconds(20)),
            to: Some(t + chrono::Duration::seconds(80) - chrono::Duration::nanoseconds(1)),
            keyword: Some("\"private note\" -meeting".into()),
            category: Some("work".into()),
            limit: Some(0),
            ..EventFilter::default()
        };
        let details = c.application_details("Code", &filter, true).unwrap();
        assert_eq!(details.total_seconds, 60);
        assert_eq!(details.total_sessions, 1);
        assert_eq!(details.events.len(), 1, "a zero limit is clamped to one");
        assert_eq!(
            details.events[0].seconds, 100,
            "displayed sessions retain their full duration"
        );
        assert_eq!(
            details.events[0].typed_text.as_deref(),
            Some("searchable private note")
        );
        let absent = c
            .application_details(
                "Code",
                &EventFilter {
                    category: Some("Other".into()),
                    ..filter
                },
                true,
            )
            .unwrap();
        assert_eq!(absent.total_sessions, 0);
        assert!(absent.last_seen.is_none());
        assert!(c
            .application_details("  ", &EventFilter::default(), false)
            .is_err());
        std::fs::remove_file(c.active_path()).unwrap();
    }

    #[test]
    fn application_details_defaults_to_fifty_rows_and_clamps_large_pages() {
        let mut c = core("details-page-limits");
        c.clear().unwrap();
        let t = Utc::now();
        let events: Vec<_> = (0..205)
            .map(|index| {
                Event::new(
                    t + chrono::Duration::seconds(index * 2),
                    t + chrono::Duration::seconds(index * 2 + 1),
                    "Chrome".into(),
                    "tab".into(),
                )
            })
            .collect();
        c.store.as_mut().unwrap().replace(&events).unwrap();
        let default = c
            .application_details("Chrome", &EventFilter::default(), false)
            .unwrap();
        assert_eq!(default.events.len(), 50);
        assert_eq!(default.total_sessions, 205);
        assert!(default.has_more);
        let large = c
            .application_details(
                "Chrome",
                &EventFilter {
                    limit: Some(usize::MAX),
                    ..EventFilter::default()
                },
                false,
            )
            .unwrap();
        assert_eq!(large.events.len(), 200);
        assert_eq!(large.total_seconds, 205);
        assert!(large.has_more);
        let text = c
            .application_details("Chrome", &EventFilter::default(), true)
            .unwrap();
        assert!(text.events.is_empty());
        assert!(!text.has_more);
        assert_eq!(text.total_sessions, 205);
        std::fs::remove_file(c.active_path()).unwrap();
    }
    #[test]
    fn idle_cuts_session_at_last_input_and_resumes_without_gap() {
        let mut c = core("idle");
        c.clear().unwrap();
        let t = Utc.timestamp_opt(1700000000, 0).unwrap();
        for seconds in 0..=310 {
            c.tick_with_idle(
                focus(),
                t + chrono::Duration::seconds(seconds),
                Some(Duration::from_secs(seconds.saturating_sub(10).max(0) as u64)),
            );
        }
        assert!(c.status().idle);
        let rows = c.query(&EventFilter::default()).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].seconds, 10);
        c.tick_with_idle(
            focus(),
            t + chrono::Duration::seconds(311),
            Some(Duration::ZERO),
        );
        c.tick_with_idle(
            focus(),
            t + chrono::Duration::seconds(312),
            Some(Duration::ZERO),
        );
        assert_eq!(
            c.analytics(&EventFilter::default()).unwrap().total_seconds,
            11
        );
        let _ = std::fs::remove_file(c.active_path());
    }

    #[test]
    fn failed_combined_encryption_and_path_change_preserves_all_settings_and_histories() {
        let mut c = core("atomic-encryption-settings");
        c.clear().unwrap();
        let source_path = c.active_path();
        let destination_base = source_path.with_extension("new.log");
        let destination_path = storage::active_path(&destination_base, true);
        let _ = std::fs::remove_file(&destination_path);
        let t = Utc::now();
        let event = Event::new(
            t,
            t + chrono::Duration::seconds(10),
            "Code".into(),
            "history.rs".into(),
        );
        c.store.as_mut().unwrap().append(&event).unwrap();
        {
            let mut destination =
                EncryptedStore::open(destination_path.clone(), "correct password").unwrap();
            destination.append(&event).unwrap();
        }
        let old_source = std::fs::read(&source_path).unwrap();
        let old_destination = std::fs::read(&destination_path).unwrap();
        let old_config = serde_json::to_value(&c.config).unwrap();
        let mut requested = c.config.clone();
        requested.encryption_enabled = true;
        requested.log_path = destination_base;
        requested.idle_threshold_seconds = Some(600);
        assert!(c
            .apply_config(requested, Some("incorrect password"))
            .is_err());
        assert_eq!(serde_json::to_value(&c.config).unwrap(), old_config);
        assert_eq!(std::fs::read(&source_path).unwrap(), old_source);
        assert_eq!(std::fs::read(&destination_path).unwrap(), old_destination);
        assert!(!c.store.as_ref().unwrap().is_encrypted());
        assert_eq!(c.store.as_ref().unwrap().len(), 1);
        let _ = std::fs::remove_file(source_path);
        let _ = std::fs::remove_file(destination_path);
    }
    #[test]
    fn sleep_gap_is_not_counted_and_live_session_is_visible() {
        let mut c = core("sleep");
        c.clear().unwrap();
        let t = Utc::now();
        c.tick_with_idle(focus(), t, None);
        c.tick_with_idle(focus(), t + chrono::Duration::seconds(2), None);
        assert_eq!(c.query(&EventFilter::default()).unwrap()[0].seconds, 2);
        c.tick_with_idle(focus(), t + chrono::Duration::hours(8), None);
        assert_eq!(
            c.analytics(&EventFilter::default()).unwrap().total_seconds,
            2
        );
        let _ = std::fs::remove_file(c.active_path());
    }

    #[test]
    fn idle_trims_sessions_flushed_by_programmatic_focus_changes() {
        let mut c = core("idle-focus-changes");
        c.clear().unwrap();
        let t = Utc.timestamp_opt(1700000000, 0).unwrap();
        for seconds in 0..=311i64 {
            let window = if (50..80).contains(&seconds) {
                Some(Focus {
                    app: "Notification".into(),
                    title: "Automatic popup".into(),
                })
            } else {
                focus()
            };
            c.tick_with_idle(
                window,
                t + chrono::Duration::seconds(seconds),
                Some(Duration::from_secs(seconds.saturating_sub(10).max(0) as u64)),
            );
        }
        let rows = c.query(&EventFilter::default()).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, t.timestamp_micros());
        assert_eq!(rows[0].app, "Code");
        assert_eq!(rows[0].seconds, 10);
        assert!(c.status().idle);
        let _ = std::fs::remove_file(c.active_path());
    }

    #[test]
    fn idle_trim_preserves_ids_and_future_history_and_trims_pending() {
        let mut c = core("idle-trim-history");
        c.clear().unwrap();
        let t = Utc::now();
        let event = |start, end| {
            Event::new(
                t + chrono::Duration::seconds(start),
                t + chrono::Duration::seconds(end),
                "Code".into(),
                "test.rs".into(),
            )
        };
        let mut crossing = event(-5, 15);
        crossing.id = 42;
        for row in [event(-30, -20), crossing, event(30, 40)] {
            c.store.as_mut().unwrap().append(&row).unwrap();
        }
        let mut pending = event(-10, 10);
        pending.id = 99;
        c.pending = vec![pending, event(10, 20)];
        c.trim_idle_tail(t, t + chrono::Duration::seconds(25))
            .unwrap();
        let stored = c.store.as_ref().unwrap().events();
        assert_eq!(stored.len(), 3);
        assert_eq!(stored[0].seconds, 10);
        assert_eq!(stored[1].id, 42);
        assert_eq!(stored[1].seconds, 5);
        assert_eq!(stored[1].end, t);
        assert_eq!(stored[2].start, t + chrono::Duration::seconds(30));
        assert_eq!(stored[2].seconds, 10);
        assert_eq!(c.pending.len(), 1);
        assert_eq!(c.pending[0].id, 99);
        assert_eq!(c.pending[0].seconds, 10);
        let _ = std::fs::remove_file(c.active_path());
    }

    #[test]
    fn failed_idle_correction_retries_after_activity_resumes_without_trimming_new_work() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };

        struct TemporarilyUnavailableStore {
            events: Vec<Event>,
            replacements: Arc<AtomicUsize>,
        }
        impl Store for TemporarilyUnavailableStore {
            fn append(&mut self, event: &Event) -> anyhow::Result<()> {
                self.events.push(event.clone());
                self.events.sort_by_key(|event| (event.start, event.id));
                Ok(())
            }
            fn events(&self) -> &[Event] {
                &self.events
            }
            fn replace(&mut self, events: &[Event]) -> anyhow::Result<()> {
                anyhow::ensure!(
                    self.replacements.fetch_add(1, Ordering::SeqCst) >= 2,
                    "disk busy"
                );
                self.events = events.to_vec();
                Ok(())
            }
            fn path(&self) -> std::path::PathBuf {
                std::path::PathBuf::new()
            }
            fn relocate(&mut self, _path: std::path::PathBuf) {}
        }

        let mut c = core("idle-retry-resume");
        let t = Utc::now();
        let replacements = Arc::new(AtomicUsize::new(0));
        c.store = Some(Box::new(TemporarilyUnavailableStore {
            events: vec![Event::new(
                t - chrono::Duration::seconds(10),
                t + chrono::Duration::seconds(10),
                "Code".into(),
                "old.rs".into(),
            )],
            replacements: replacements.clone(),
        }));
        c.tick_with_idle(
            focus(),
            t + chrono::Duration::seconds(300),
            Some(Duration::from_secs(300)),
        );
        assert!(c.status().idle);
        assert!(c
            .storage_error
            .as_deref()
            .unwrap()
            .starts_with("Idle correction failed:"));
        c.tick_with_idle(
            focus(),
            t + chrono::Duration::seconds(301),
            Some(Duration::ZERO),
        );
        assert!(!c.status().idle);
        assert!(c.storage_error.is_some());
        c.tick_with_idle(
            focus(),
            t + chrono::Duration::seconds(302),
            Some(Duration::ZERO),
        );
        assert!(c.storage_error.is_none());
        assert!(c.idle_corrections.is_empty());
        assert_eq!(replacements.load(Ordering::SeqCst), 3);
        assert_eq!(c.store.as_ref().unwrap().events()[0].seconds, 10);
        assert_eq!(
            c.analytics(&EventFilter::default()).unwrap().total_seconds,
            11
        );
        c.tick_with_idle(
            focus(),
            t + chrono::Duration::seconds(303),
            Some(Duration::ZERO),
        );
        assert_eq!(replacements.load(Ordering::SeqCst), 3);
        let _ = std::fs::remove_file(c.active_path());
    }

    #[test]
    fn combined_filter_paging_merges_stored_pending_and_live_sessions() {
        let mut c = core("query-merge");
        c.clear().unwrap();
        c.config.app_categories = vec![
            AppCategory {
                app: "Code".into(),
                category: "Work".into(),
            },
            AppCategory {
                app: "Terminal".into(),
                category: "Work".into(),
            },
        ];
        let t = Utc.with_ymd_and_hms(2026, 11, 4, 10, 0, 0).unwrap();
        let make_event = |seconds: i64, app: &str, title: &str| {
            Event::new(
                t + chrono::Duration::seconds(seconds),
                t + chrono::Duration::seconds(seconds + 1),
                app.into(),
                title.into(),
            )
        };
        // Include out-of-order persisted input and a newer excluded category.
        for event in [
            make_event(20, "Terminal", "project"),
            make_event(10, "Code", "project"),
            make_event(25, "Browser", "project"),
        ] {
            c.store.as_mut().unwrap().append(&event).unwrap();
        }
        c.pending.push(make_event(15, "Code", "project"));
        // A backwards clock adjustment can place live work before stored work.
        c.current = Some(SessionInProgress {
            id: (t + chrono::Duration::seconds(5)).timestamp_micros(),
            app: "Code".into(),
            title: "project".into(),
            start: t + chrono::Duration::seconds(5),
            last_seen: t + chrono::Duration::seconds(6),
            typed_text: String::new(),
            typed_chars: 0,
        });
        let filter = EventFilter {
            category: Some("Work".into()),
            keyword: Some("project -meeting".into()),
            offset: Some(1),
            limit: Some(2),
            ..Default::default()
        };
        let page = c.query(&filter).unwrap();
        assert_eq!(
            page.iter()
                .map(|event| (event.start - t).num_seconds())
                .collect::<Vec<_>>(),
            vec![15, 10]
        );
        let all = c
            .query(&EventFilter {
                limit: None,
                offset: None,
                ..filter.clone()
            })
            .unwrap();
        assert_eq!(
            all.iter()
                .map(|event| (event.start - t).num_seconds())
                .collect::<Vec<_>>(),
            vec![20, 15, 10, 5]
        );
        assert!(c
            .query(&EventFilter {
                limit: Some(0),
                ..filter
            })
            .unwrap()
            .is_empty());
        let _ = std::fs::remove_file(c.active_path());
    }
}
