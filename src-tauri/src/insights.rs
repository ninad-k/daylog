//! Calendar-aware summaries and time-goal progress. Events are split at local
//! midnight so a focus session spanning days does not all accrue to its start.

use crate::model::{
    category_for, Analytics, AppBucket, AppCategory, CategoryBucket, DayBucket, Event, GoalPeriod,
    GoalProgress, TargetType, TimeGoal, WeekBucket,
};
use chrono::{DateTime, Datelike, Days, Duration, Local, NaiveDate, TimeZone, Utc};
use std::collections::BTreeMap;

pub fn analytics(
    events: &[Event],
    categories: &[AppCategory],
    goals: &[TimeGoal],
    now: DateTime<Utc>,
) -> Analytics {
    aggregate(events, categories, goals, now, &Local)
}

pub fn goal_progress(
    events: &[Event],
    categories: &[AppCategory],
    goals: &[TimeGoal],
    now: DateTime<Utc>,
) -> Vec<GoalProgress> {
    goals_in_zone(events, categories, goals, now, &Local)
}

fn monday(date: NaiveDate) -> NaiveDate {
    date - Duration::days(i64::from(date.weekday().num_days_from_monday()))
}

/// Some timezones advance clocks at midnight. Find the first representable
/// instant of the requested date rather than assuming every day has 24 hours.
fn day_start<T: TimeZone>(date: NaiveDate, timezone: &T) -> Option<DateTime<Utc>> {
    let midnight = date.and_hms_opt(0, 0, 0)?;
    for minute in 0..1440 {
        let local = midnight.checked_add_signed(Duration::minutes(minute))?;
        if let Some(value) = timezone.from_local_datetime(&local).earliest() {
            return Some(value.with_timezone(&Utc));
        }
    }
    None
}

fn split_days<T: TimeZone>(event: &Event, timezone: &T) -> Vec<(NaiveDate, i64)> {
    let mut result = Vec::new();
    let mut cursor = event.start;
    // Allocate the original whole-second duration without rounding separately
    // at each midnight (which can otherwise lose a second on fractional times).
    let mut accounted_seconds = 0;
    while cursor < event.end {
        let date = cursor.with_timezone(timezone).date_naive();
        let mut next_date = date.checked_add_days(Days::new(1));
        let mut boundary = None;
        // A timezone can skip an entire local calendar date after a political
        // offset change. Look forward until an actual next day exists.
        while let Some(candidate) = next_date {
            if let Some(start) = day_start(candidate, timezone) {
                if start > cursor {
                    boundary = Some(start);
                    break;
                }
            }
            next_date = candidate.checked_add_days(Days::new(1));
        }
        let end = boundary.map_or(event.end, |value| value.min(event.end));
        let cumulative_seconds = (end - event.start).num_seconds().max(0);
        let seconds = cumulative_seconds - accounted_seconds;
        if seconds > 0 {
            result.push((date, seconds));
        }
        accounted_seconds = cumulative_seconds;
        cursor = end;
    }
    result
}

fn aggregate<T: TimeZone>(
    events: &[Event],
    categories: &[AppCategory],
    goals: &[TimeGoal],
    now: DateTime<Utc>,
    timezone: &T,
) -> Analytics {
    let mut apps: BTreeMap<String, (i64, i64)> = BTreeMap::new();
    let mut category_totals: BTreeMap<String, (i64, i64)> = BTreeMap::new();
    let mut days: BTreeMap<NaiveDate, i64> = BTreeMap::new();
    let mut weeks: BTreeMap<NaiveDate, i64> = BTreeMap::new();
    let mut total_seconds = 0;
    let mut total_events = 0;
    for event in events {
        let seconds = (event.end - event.start).num_seconds().max(0);
        if seconds == 0 {
            continue;
        }
        total_seconds += seconds;
        total_events += 1;
        let app = apps.entry(event.app.clone()).or_default();
        app.0 += seconds;
        app.1 += 1;
        let category = category_totals
            .entry(category_for(&event.app, categories))
            .or_default();
        category.0 += seconds;
        category.1 += 1;
        for (date, seconds) in split_days(event, timezone) {
            *days.entry(date).or_default() += seconds;
            *weeks.entry(monday(date)).or_default() += seconds;
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
    per_app.sort_by(|a, b| b.seconds.cmp(&a.seconds).then_with(|| a.app.cmp(&b.app)));
    let mut per_category: Vec<_> = category_totals
        .into_iter()
        .map(|(category, (seconds, events))| CategoryBucket {
            category,
            seconds,
            events,
        })
        .collect();
    per_category.sort_by(|a, b| {
        b.seconds
            .cmp(&a.seconds)
            .then_with(|| a.category.cmp(&b.category))
    });
    let today = now.with_timezone(timezone).date_naive();
    let today_seconds = *days.get(&today).unwrap_or(&0);
    let week_seconds = *weeks.get(&monday(today)).unwrap_or(&0);
    Analytics {
        total_seconds,
        total_events,
        per_app,
        per_day: days
            .into_iter()
            .map(|(date, seconds)| DayBucket {
                date: date.to_string(),
                seconds,
            })
            .collect(),
        per_category,
        per_week: weeks
            .into_iter()
            .map(|(week, seconds)| WeekBucket {
                week_start: week.to_string(),
                seconds,
            })
            .collect(),
        goals: goals_in_zone(events, categories, goals, now, timezone),
        today_seconds,
        week_seconds,
    }
}

fn goals_in_zone<T: TimeZone>(
    events: &[Event],
    categories: &[AppCategory],
    goals: &[TimeGoal],
    now: DateTime<Utc>,
    timezone: &T,
) -> Vec<GoalProgress> {
    let today = now.with_timezone(timezone).date_naive();
    goals
        .iter()
        .map(|goal| {
            let date = match goal.period {
                GoalPeriod::Daily => today,
                GoalPeriod::Weekly => monday(today),
            };
            let start = day_start(date, timezone).unwrap_or(now);
            let seconds = events
                .iter()
                .filter(|event| match goal.target_type {
                    TargetType::App => event.app.eq_ignore_ascii_case(goal.target.trim()),
                    TargetType::Category => category_for(&event.app, categories)
                        .eq_ignore_ascii_case(goal.target.trim()),
                })
                .map(|event| {
                    let from = event.start.max(start);
                    let to = event.end.min(now);
                    (to - from).num_seconds().max(0)
                })
                .sum();
            let target_seconds = i64::from(goal.minutes) * 60;
            GoalProgress {
                id: goal.id.clone(),
                name: goal.name.clone(),
                target_type: goal.target_type,
                target: goal.target.clone(),
                period: goal.period,
                kind: goal.kind,
                minutes: goal.minutes,
                seconds,
                progress: if target_seconds > 0 {
                    seconds as f64 / target_seconds as f64 * 100.0
                } else {
                    0.0
                },
                reached: target_seconds > 0 && seconds >= target_seconds,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::GoalKind;
    use chrono::{FixedOffset, Timelike};

    fn event(start: DateTime<Utc>, minutes: i64, app: &str) -> Event {
        Event::new(
            start,
            start + Duration::minutes(minutes),
            app.into(),
            "Window".into(),
        )
    }

    #[test]
    fn midnight_and_monday_split_in_local_timezone() {
        let zone = FixedOffset::east_opt(4 * 3600).unwrap();
        // Sunday 23:30 local through Monday 00:30 local.
        let start = Utc.with_ymd_and_hms(2026, 11, 1, 19, 30, 0).unwrap();
        let report = aggregate(
            &[event(start, 60, "Code")],
            &[],
            &[],
            start + Duration::hours(1),
            &zone,
        );
        assert_eq!(report.total_seconds, 3600);
        assert_eq!(report.total_events, 1);
        assert_eq!(report.per_day.len(), 2);
        assert_eq!(report.per_day[0].date, "2026-11-01");
        assert_eq!(report.per_day[1].date, "2026-11-02");
        assert!(report.per_day.iter().all(|day| day.seconds == 1800));
        assert_eq!(report.per_week.len(), 2);
        assert_eq!(report.per_week[0].week_start, "2026-10-26");
        assert_eq!(report.per_week[1].week_start, "2026-11-02");
        assert_eq!(report.today_seconds, 1800);
        assert_eq!(report.week_seconds, 1800);
    }

    #[test]
    fn daily_and_weekly_goals_clip_at_calendar_start_and_now() {
        let now = Utc.with_ymd_and_hms(2026, 11, 4, 1, 0, 0).unwrap();
        let categories = vec![AppCategory {
            app: "code".into(),
            category: "Work".into(),
        }];
        let events = [
            event(now - Duration::hours(2), 90, "Code"),
            event(now - Duration::days(1), 60, "Code"),
            // Exclude the future portion of a malformed/imported event.
            event(now - Duration::minutes(10), 30, "Chat"),
        ];
        let make_goal = |id: &str, target_type, target: &str, period, kind| TimeGoal {
            id: id.into(),
            name: id.into(),
            target_type,
            target: target.into(),
            period,
            kind,
            minutes: 30,
        };
        let goals = [
            make_goal(
                "daily",
                TargetType::App,
                "Code",
                GoalPeriod::Daily,
                GoalKind::Goal,
            ),
            make_goal(
                "weekly",
                TargetType::Category,
                "work",
                GoalPeriod::Weekly,
                GoalKind::Limit,
            ),
            make_goal(
                "future",
                TargetType::App,
                "Chat",
                GoalPeriod::Daily,
                GoalKind::Limit,
            ),
        ];
        let progress = goals_in_zone(&events, &categories, &goals, now, &Utc);
        assert_eq!(progress[0].seconds, 1800);
        assert_eq!(progress[0].progress, 100.0);
        assert!(progress[0].reached);
        assert_eq!(progress[1].seconds, 9000);
        assert_eq!(progress[1].progress, 500.0);
        assert!(progress[1].reached);
        assert_eq!(progress[2].seconds, 600);
        assert!(!progress[2].reached);
    }

    #[test]
    fn fractional_midnight_duration_is_not_lost() {
        let start = Utc
            .with_ymd_and_hms(2026, 11, 4, 23, 59, 59)
            .unwrap()
            .with_nanosecond(500_000_000)
            .unwrap();
        let e = Event::new(
            start,
            start + Duration::seconds(2),
            "Code".into(),
            "x".into(),
        );
        let parts = split_days(&e, &Utc);
        assert_eq!(parts.iter().map(|(_, seconds)| *seconds).sum::<i64>(), 2);
    }

    #[test]
    fn category_totals_and_empty_reports() {
        let now = Utc.with_ymd_and_hms(2026, 11, 4, 12, 0, 0).unwrap();
        let categories = vec![AppCategory {
            app: "Code".into(),
            category: "Work".into(),
        }];
        let report = aggregate(
            &[event(now, 30, "Code"), event(now, 10, "Chat")],
            &categories,
            &[],
            now,
            &Utc,
        );
        assert_eq!(report.per_category[0].category, "Work");
        assert_eq!(report.per_category[0].seconds, 1800);
        assert_eq!(report.per_category[1].category, "Uncategorized");
        let empty = aggregate(&[], &[], &[], now, &Utc);
        assert_eq!(empty.total_seconds, 0);
        assert_eq!(empty.total_events, 0);
        assert!(empty.per_day.is_empty());
    }
}
