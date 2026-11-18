//! Shared data types used across the capture, storage, and UI layers.
//!
//! Keeping these in one place lets each layer evolve independently while
//! agreeing on a single on-disk / over-the-IPC representation.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A single focus session: one window/app held focus from `start` to `end`.
///
/// Text is absent by default and can only be recorded for explicitly approved
/// applications while the encrypted store is unlocked.
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
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "nonempty_text"
    )]
    pub typed_text: Option<String>,
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
            typed_text: None,
        }
    }
}

fn nonempty_text<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    Ok(Option::<String>::deserialize(deserializer)?.filter(|text| !text.is_empty()))
}

/// Filter passed from the UI when querying the log.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EventFilter {
    /// RFC3339 inclusive lower bound. Sessions overlapping the range match.
    pub from: Option<DateTime<Utc>>,
    /// RFC3339 upper bound (inclusive) on session start.
    pub to: Option<DateTime<Utc>>,
    /// Case-insensitive substring match on app name.
    pub app: Option<String>,
    /// Optional case-insensitive exact app name; independent of substring search.
    #[serde(default)]
    pub app_exact: Option<String>,
    /// Required/excluded words or phrases across app, title, and opted-in text.
    pub keyword: Option<String>,
    /// Case-insensitive category name, resolved using app category assignments.
    pub category: Option<String>,
    /// Cap on returned rows (newest first). None = all.
    pub limit: Option<usize>,
    /// Number of matching rows to skip before applying `limit`.
    pub offset: Option<usize>,
}

impl EventFilter {
    pub fn validate(&self) -> anyhow::Result<()> {
        if let (Some(from), Some(to)) = (self.from, self.to) {
            anyhow::ensure!(from <= to, "the start date must precede the end date");
        }
        Ok(())
    }

    pub fn matches(&self, e: &Event) -> bool {
        self.compile().matches(e)
    }

    /// Normalize search text once before scanning a large in-memory log.
    pub fn compile(&self) -> CompiledFilter<'_> {
        CompiledFilter {
            filter: self,
            app: self.app.as_ref().map(|app| app.trim().to_lowercase()),
            app_exact: self.app_exact.as_ref().map(|app| app.trim().to_lowercase()),
            terms: self
                .keyword
                .as_deref()
                .map(keyword_terms)
                .unwrap_or_default(),
        }
    }

    pub fn matches_with_categories(&self, e: &Event, categories: &[AppCategory]) -> bool {
        self.compile().matches_with_categories(e, categories)
    }

    /// Clip a matching session for ranged analytics. Keep its identity so it
    /// can still be recognised as the same source session by callers.
    pub fn clipped(&self, e: &Event) -> Option<Event> {
        if !self.matches(e) {
            return None;
        }
        let mut event = e.clone();
        event.start = self.from.map_or(e.start, |from| from.max(e.start));
        // `to` is inclusive, while Event.end is exclusive. Converting the
        // bound prevents a whole-second session ending at midnight from losing
        // a second when a date picker supplies 23:59:59.999999999.
        event.end = self.to.map_or(e.end, |to| {
            to.checked_add_signed(chrono::Duration::nanoseconds(1))
                .unwrap_or(to)
                .min(e.end)
        });
        event.seconds = (event.end - event.start).num_seconds().max(0);
        (event.seconds > 0).then_some(event)
    }

    /// Time inside the selected date range for an already-matching session.
    /// This avoids cloning recorded text when computing summary statistics.
    pub fn clipped_seconds(&self, event: &Event) -> i64 {
        let start = self.from.map_or(event.start, |from| from.max(event.start));
        let end = self.to.map_or(event.end, |to| {
            to.checked_add_signed(chrono::Duration::nanoseconds(1))
                .unwrap_or(to)
                .min(event.end)
        });
        (end - start).num_seconds().max(0)
    }
}

pub struct CompiledFilter<'a> {
    filter: &'a EventFilter,
    app: Option<String>,
    app_exact: Option<String>,
    terms: Vec<(String, bool)>,
}

impl CompiledFilter<'_> {
    pub fn matches(&self, e: &Event) -> bool {
        if let Some(from) = self.filter.from {
            if e.end < from || (e.end == from && e.start != e.end) {
                return false;
            }
        }
        if let Some(to) = self.filter.to {
            if e.start > to {
                return false;
            }
        }
        let app = e.app.to_lowercase();
        if self.app_exact.as_ref().is_some_and(|exact| app != *exact) {
            return false;
        }
        if let Some(needle) = &self.app {
            if !app.contains(needle) {
                return false;
            }
        }
        if !self.terms.is_empty() {
            let title = e.title.to_lowercase();
            let typed_text = e.typed_text.as_deref().unwrap_or_default().to_lowercase();
            if !self.terms.iter().all(|(term, excluded)| {
                let found = app.contains(term) || title.contains(term) || typed_text.contains(term);
                found != *excluded
            }) {
                return false;
            }
        }
        true
    }

    pub fn matches_with_categories(&self, e: &Event, categories: &[AppCategory]) -> bool {
        self.matches(e)
            && self.filter.category.as_ref().is_none_or(|category| {
                category.trim().is_empty()
                    || category_for(&e.app, categories).eq_ignore_ascii_case(category.trim())
            })
    }
}

/// Tokenize once per search expression in the query layer when possible. A
/// dangling quote simply keeps the remaining text as one phrase.
pub fn keyword_terms(query: &str) -> Vec<(String, bool)> {
    let mut terms = Vec::new();
    let mut chars = query.trim().chars().peekable();
    while chars.peek().is_some() {
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        let excluded = chars.peek() == Some(&'-');
        if excluded {
            chars.next();
        }
        let mut term = String::new();
        let mut quoted = false;
        while let Some(&c) = chars.peek() {
            chars.next();
            if c == '"' {
                quoted = !quoted;
            } else if c.is_whitespace() && !quoted {
                break;
            } else {
                term.push(c);
            }
        }
        if !term.is_empty() {
            terms.push((term.to_lowercase(), excluded));
        }
    }
    terms
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppCategory {
    pub app: String,
    pub category: String,
}

pub fn category_for(app: &str, categories: &[AppCategory]) -> String {
    categories
        .iter()
        .find(|entry| entry.app.trim().eq_ignore_ascii_case(app.trim()))
        .map(|entry| entry.category.trim())
        .filter(|category| !category.is_empty())
        .unwrap_or("Uncategorized")
        .to_owned()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TargetType {
    App,
    Category,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GoalPeriod {
    Daily,
    Weekly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GoalKind {
    Goal,
    Limit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeGoal {
    pub id: String,
    pub name: String,
    pub target_type: TargetType,
    pub target: String,
    pub period: GoalPeriod,
    pub kind: GoalKind,
    pub minutes: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedFilter {
    pub id: String,
    pub name: String,
    pub filter: EventFilter,
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

#[derive(Debug, Clone, Serialize)]
pub struct CategoryBucket {
    pub category: String,
    pub seconds: i64,
    pub events: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct WeekBucket {
    /// Local date of Monday starting this calendar week.
    pub week_start: String,
    pub seconds: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct GoalProgress {
    pub id: String,
    pub name: String,
    pub target_type: TargetType,
    pub target: String,
    pub period: GoalPeriod,
    pub kind: GoalKind,
    pub minutes: u32,
    pub seconds: i64,
    /// Percentage of the goal/limit; can exceed 100.
    pub progress: f64,
    pub reached: bool,
}

/// Aggregated analytics returned to the UI.
#[derive(Debug, Clone, Serialize)]
pub struct Analytics {
    pub total_seconds: i64,
    pub total_events: i64,
    pub per_app: Vec<AppBucket>,
    pub per_day: Vec<DayBucket>,
    pub per_category: Vec<CategoryBucket>,
    pub per_week: Vec<WeekBucket>,
    pub goals: Vec<GoalProgress>,
    pub today_seconds: i64,
    pub week_seconds: i64,
}

/// Snapshot of logger state for the status bar in the UI / tray tooltip.
#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub paused: bool,
    pub encrypted: bool,
    /// True when an encrypted store is loaded but still locked (no password this session).
    pub locked: bool,
    pub idle: bool,
    pub idle_detection_available: bool,
    pub excluded: bool,
    pub storage_error: Option<String>,
    pub current_app: Option<String>,
    pub current_title: Option<String>,
    pub events_total: i64,
    pub log_path: String,
    pub text_capture_enabled: bool,
    pub text_recording: bool,
    pub text_capture_supported: bool,
}

/// One application's activity and recorded text, with totals independent of paging.
#[derive(Debug, Clone, Serialize)]
pub struct ApplicationDetails {
    pub app: String,
    pub category: String,
    pub total_seconds: i64,
    pub total_sessions: usize,
    pub text_sessions: usize,
    pub recorded_characters: usize,
    pub last_seen: Option<DateTime<Utc>>,
    pub events: Vec<Event>,
    pub has_more: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    fn event() -> Event {
        let start = Utc.with_ymd_and_hms(2026, 11, 4, 10, 0, 0).unwrap();
        Event::new(
            start,
            start + Duration::hours(1),
            "Code".into(),
            "daylog project review".into(),
        )
    }

    #[test]
    fn combined_search_terms_phrases_and_exclusions() {
        let e = event();
        for query in [
            "code daylog",
            "\"project review\" -meeting",
            "DAYLOG -\"other project\"",
        ] {
            assert!(EventFilter {
                keyword: Some(query.into()),
                ..Default::default()
            }
            .matches(&e));
        }
        for query in ["code meeting", "\"daylog review\"", "daylog -review"] {
            assert!(!EventFilter {
                keyword: Some(query.into()),
                ..Default::default()
            }
            .matches(&e));
        }
    }

    #[test]
    fn legacy_events_default_to_no_text_and_opted_in_text_is_searchable() {
        let mut e = event();
        let legacy = serde_json::to_string(&e).unwrap();
        assert!(!legacy.contains("typed_text"));
        assert!(serde_json::from_str::<Event>(&legacy)
            .unwrap()
            .typed_text
            .is_none());
        e.typed_text = Some("A private project note\n你好".into());
        let roundtrip: Event = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        assert_eq!(roundtrip.typed_text, e.typed_text);
        assert!(EventFilter {
            keyword: Some("\"private project\" 你好".into()),
            ..EventFilter::default()
        }
        .matches(&e));
        assert!(!EventFilter {
            keyword: Some("project -private".into()),
            ..EventFilter::default()
        }
        .matches(&e));
    }

    #[test]
    fn exact_application_filter_does_not_match_similarly_named_apps() {
        let mut chrome = event();
        chrome.app = "Chrome".into();
        let mut canary = chrome.clone();
        canary.app = "Chrome Canary".into();
        let exact = EventFilter {
            app_exact: Some("cHrOmE".into()),
            ..EventFilter::default()
        };
        assert!(exact.matches(&chrome));
        assert!(!exact.matches(&canary));
        let substring = EventFilter {
            app: Some("Chrome".into()),
            ..EventFilter::default()
        };
        assert!(substring.matches(&chrome));
        assert!(substring.matches(&canary));
        let legacy: EventFilter =
            serde_json::from_str("{\"app\":\"Chrome\",\"limit\":50}").unwrap();
        assert!(legacy.app_exact.is_none());
        let roundtrip: EventFilter =
            serde_json::from_str(&serde_json::to_string(&exact).unwrap()).unwrap();
        assert_eq!(roundtrip.app_exact.as_deref(), Some("cHrOmE"));
    }

    #[test]
    fn category_and_date_filters_combine_and_clip_overlaps() {
        let e = event();
        let categories = vec![AppCategory {
            app: "code".into(),
            category: "Work".into(),
        }];
        let filter = EventFilter {
            from: Some(e.start + Duration::minutes(20)),
            to: Some(e.end - Duration::minutes(20)),
            category: Some("work".into()),
            keyword: Some("daylog".into()),
            ..Default::default()
        };
        assert!(filter.matches_with_categories(&e, &categories));
        let clipped = filter.clipped(&e).unwrap();
        assert_eq!(clipped.seconds, 1200);
        assert_eq!(clipped.id, e.id);
        assert!(!filter.matches_with_categories(&e, &[]));
    }

    #[test]
    fn saved_filters_roundtrip_and_invalid_range_is_rejected() {
        let e = event();
        let saved = SavedFilter {
            id: "one".into(),
            name: "Work".into(),
            filter: EventFilter {
                from: Some(e.end),
                to: Some(e.start),
                offset: Some(50),
                ..Default::default()
            },
        };
        let loaded: SavedFilter =
            serde_json::from_str(&serde_json::to_string(&saved).unwrap()).unwrap();
        assert_eq!(loaded.filter.offset, Some(50));
        assert!(loaded.filter.validate().is_err());
    }

    #[test]
    fn inclusive_date_end_preserves_full_second_before_midnight() {
        let start = Utc.with_ymd_and_hms(2026, 11, 4, 23, 59, 59).unwrap();
        let e = Event::new(
            start,
            start + Duration::seconds(1),
            "Code".into(),
            "x".into(),
        );
        let filter = EventFilter {
            to: Some(e.end - Duration::nanoseconds(1)),
            ..Default::default()
        };
        assert_eq!(filter.clipped(&e).unwrap().seconds, 1);
    }
}
