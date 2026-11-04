//! Shared data types used across the capture, storage, and UI layers.
//!
//! Keeping these in one place lets each layer evolve independently while
//! agreeing on a single on-disk / over-the-IPC representation.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A single focus session: one window/app held focus from `start` to `end`.
///
/// This is the core unit daylog records. Note what is NOT here: there is no
/// field for typed text or keystrokes. daylog answers "which app, which
/// window, how long", never "what did you type".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    /// Monotonic id (microseconds since the Unix epoch at session start).
    pub id: i64,
    /// When this window gained focus.
    pub start: DateTime<Utc>,
    /// When focus left this window (or the last heartbeat before shutdown).
    pub end: DateTime<Utc>,
    /// Application name, e.g. "Google Chrome".
    pub app: String,
    /// Window / document title, e.g. "Inbox — Gmail".
    pub title: String,
    /// Convenience duration in whole seconds (end - start).
    pub seconds: i64,
}

impl Event {
    pub fn new(start: DateTime<Utc>, end: DateTime<Utc>, app: String, title: String) -> Self {
        let seconds = (end - start).num_seconds().max(0);
        Event {
            id: start.timestamp_micros(),
            start,
            end,
            app,
            title,
            seconds,
        }
    }
}

/// Filter passed from the UI when querying the log.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct EventFilter {
    /// RFC3339 lower bound (inclusive) on `start`.
    pub from: Option<DateTime<Utc>>,
    /// RFC3339 upper bound (inclusive) on `start`.
    pub to: Option<DateTime<Utc>>,
    /// Case-insensitive substring match on app name.
    pub app: Option<String>,
    /// Case-insensitive substring match on app OR title.
    pub keyword: Option<String>,
    /// Cap on returned rows (newest first). None = all.
    pub limit: Option<usize>,
}

impl EventFilter {
    pub fn matches(&self, e: &Event) -> bool {
        if let Some(from) = self.from {
            if e.start < from {
                return false;
            }
        }
        if let Some(to) = self.to {
            if e.start > to {
                return false;
            }
        }
        if let Some(app) = &self.app {
            if !e.app.to_lowercase().contains(&app.to_lowercase()) {
                return false;
            }
        }
        if let Some(kw) = &self.keyword {
            let kw = kw.to_lowercase();
            if !e.app.to_lowercase().contains(&kw) && !e.title.to_lowercase().contains(&kw) {
                return false;
            }
        }
        true
    }
}

/// One row of the per-app time breakdown.
#[derive(Debug, Clone, Serialize)]
pub struct AppBucket {
    pub app: String,
    pub seconds: i64,
    pub events: i64,
}

/// One row of the per-day time breakdown (local date → seconds).
#[derive(Debug, Clone, Serialize)]
pub struct DayBucket {
    pub date: String,
    pub seconds: i64,
}

/// Aggregated analytics returned to the UI.
#[derive(Debug, Clone, Serialize)]
pub struct Analytics {
    pub total_seconds: i64,
    pub total_events: i64,
    pub per_app: Vec<AppBucket>,
    pub per_day: Vec<DayBucket>,
}

/// Snapshot of logger state for the status bar in the UI / tray tooltip.
#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub paused: bool,
    pub encrypted: bool,
    /// True when an encrypted store is loaded but still locked (no password this session).
    pub locked: bool,
    pub current_app: Option<String>,
    pub current_title: Option<String>,
    pub events_total: i64,
    pub log_path: String,
}
