//! Opt-in summary email. Credentials live in memory for this app session only.
use crate::{
    config::EmailConfig,
    model::{Analytics, EventFilter},
    SharedCore,
};
use chrono::{Datelike, Duration, Local, NaiveDate, TimeZone, Timelike, Utc};
use lettre::{
    message::header::ContentType, transport::smtp::authentication::Credentials, Message,
    SmtpTransport, Transport,
};
use serde::Serialize;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use zeroize::Zeroizing;

#[derive(Default, Clone, Serialize)]
pub struct EmailStatus {
    pub ready: bool,
    pub sending: bool,
    pub last_error: Option<String>,
    pub last_success: Option<String>,
}
#[derive(Default)]
pub struct EmailRuntime {
    inner: Mutex<EmailSession>,
}
#[derive(Default)]
struct EmailSession {
    credentials: Option<SessionCredentials>,
    status: EmailStatus,
    delivered: HashSet<String>,
}
struct SessionCredentials {
    transport: TransportIdentity,
    password: Zeroizing<String>,
}
#[derive(PartialEq, Eq)]
struct TransportIdentity {
    host: String,
    port: u16,
    implicit_tls: bool,
    username: String,
}
impl From<&EmailConfig> for TransportIdentity {
    fn from(config: &EmailConfig) -> Self {
        Self {
            host: config.host.trim().to_lowercase(),
            port: config.port,
            implicit_tls: config.implicit_tls,
            username: config.username.clone(),
        }
    }
}
impl EmailRuntime {
    pub fn set_password(&self, password: String, config: &EmailConfig) {
        self.inner.lock().unwrap().credentials = if password.is_empty() {
            None
        } else {
            Some(SessionCredentials {
                transport: TransportIdentity::from(config),
                password: Zeroizing::new(password),
            })
        };
    }
    pub fn status(&self, config: &EmailConfig) -> EmailStatus {
        let session = self.inner.lock().unwrap();
        let mut status = session.status.clone();
        status.ready = config.username.is_empty()
            || session.credentials.as_ref().is_some_and(|credentials| {
                credentials.transport == TransportIdentity::from(config)
            });
        status
    }

    fn password_for(&self, config: &EmailConfig) -> anyhow::Result<Option<String>> {
        if config.username.is_empty() {
            return Ok(None);
        }
        self.inner
            .lock()
            .unwrap()
            .credentials
            .as_ref()
            .filter(|credentials| credentials.transport == TransportIdentity::from(config))
            .map(|credentials| Some(credentials.password.to_string()))
            .ok_or_else(|| {
                anyhow::anyhow!("enter the SMTP password for the current server and account")
            })
    }

    fn should_send(&self, config: &EmailConfig, label: &str) -> bool {
        let key = delivery_key(config, label);
        !self.inner.lock().unwrap().delivered.contains(&key)
            && !config.sent_periods.contains(&key)
            && config.last_sent.as_deref() != Some(label)
    }

    fn remember_delivery(&self, config: &EmailConfig, label: &str) {
        self.inner
            .lock()
            .unwrap()
            .delivered
            .insert(delivery_key(config, label));
    }
}

fn delivery_key(config: &EmailConfig, label: &str) -> String {
    serde_json::to_string(&(
        config.host.trim().to_lowercase(),
        config.port,
        config.implicit_tls,
        &config.username,
        &config.sender,
        &config.recipient,
        label,
    ))
    .unwrap()
}

fn record_delivery(current: &mut EmailConfig, sent: &EmailConfig, label: &str) {
    let key = delivery_key(sent, label);
    if !current.sent_periods.contains(&key) {
        current.sent_periods.push(key);
    }
    // Enough history for changing between daily and weekly schedules, bounded
    // so this metadata does not grow forever with the activity history.
    if current.sent_periods.len() > 128 {
        current
            .sent_periods
            .drain(..current.sent_periods.len() - 128);
    }
    if delivery_key(current, label) == delivery_key(sent, label) && current.weekly == sent.weekly {
        current.last_sent = Some(label.to_owned());
    }
}

pub fn validate_email(config: &EmailConfig) -> anyhow::Result<()> {
    anyhow::ensure!(
        !config.host.is_empty()
            && config.host.trim() == config.host
            && !config.host.chars().any(|c| c.is_whitespace()
                || c.is_control()
                || matches!(c, '/' | '\\' | '@' | '?' | '#')),
        "enter an SMTP hostname"
    );
    anyhow::ensure!(config.port != 0, "SMTP port must be positive");
    let _: lettre::message::Mailbox = config
        .sender
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid sender email"))?;
    let _: lettre::message::Mailbox = config
        .recipient
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid recipient email"))?;
    Ok(())
}

fn midnight(date: NaiveDate) -> anyhow::Result<chrono::DateTime<Utc>> {
    let midnight = date
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| anyhow::anyhow!("invalid summary date"))?;
    for minute in 0..1440 {
        if let Some(value) = Local
            .from_local_datetime(&(midnight + Duration::minutes(minute)))
            .earliest()
        {
            return Ok(value.with_timezone(&Utc));
        }
    }
    Err(anyhow::anyhow!("local calendar date is unavailable"))
}

pub fn period_filter(
    weekly: bool,
    previous: bool,
    now: chrono::DateTime<Local>,
) -> anyhow::Result<(String, EventFilter)> {
    let today = now.date_naive();
    let start = if weekly {
        today - Duration::days(today.weekday().num_days_from_monday() as i64)
    } else {
        today
    };
    let length = if weekly { 7 } else { 1 };
    let start = start - Duration::days(if previous { length } else { 0 });
    let end = start + Duration::days(length);
    Ok((
        format!("{}:{start}", if weekly { "weekly" } else { "daily" }),
        EventFilter {
            from: Some(midnight(start)?),
            to: Some(midnight(end)? - Duration::nanoseconds(1)),
            ..EventFilter::default()
        },
    ))
}

pub fn render_summary(label: &str, analytics: &Analytics) -> String {
    fn duration(s: i64) -> String {
        format!("{}h {}m", s / 3600, (s % 3600) / 60)
    }
    let mut text = format!(
        "daylog summary — {label}\n\nTracked time: {}\nFocus sessions: {}\n\nApplications\n",
        duration(analytics.total_seconds),
        analytics.total_events
    );
    for app in &analytics.per_app {
        text.push_str(&format!("{}: {}\n", app.app, duration(app.seconds)));
    }
    text.push_str("\nCategories\n");
    for category in &analytics.per_category {
        text.push_str(&format!(
            "{}: {}\n",
            category.category,
            duration(category.seconds)
        ));
    }
    text.push_str("\nDays\n");
    for day in &analytics.per_day {
        text.push_str(&format!("{}: {}\n", day.date, duration(day.seconds)));
    }
    text.push_str("\nGenerated locally by daylog. Window titles are omitted.\n");
    text
}

pub fn preview(core: &SharedCore, weekly: bool) -> anyhow::Result<String> {
    let (label, filter) = period_filter(weekly, false, Local::now())?;
    let analytics = core.lock().unwrap().analytics(&filter)?;
    Ok(render_summary(&label, &analytics))
}

pub fn send_summary(
    core: &SharedCore,
    runtime: &EmailRuntime,
    scheduled: bool,
) -> anyhow::Result<()> {
    {
        let mut session = runtime.inner.lock().unwrap();
        let status = &mut session.status;
        anyhow::ensure!(!status.sending, "an email is already being sent");
        status.sending = true;
    }
    let mut delivered = false;
    let result = (|| {
        let (config, label, body, password) = {
            let core = core.lock().unwrap();
            let config = core.config.email.clone();
            anyhow::ensure!(config.enabled, "enable summary emails in Settings first");
            validate_email(&config)?;
            let (label, filter) = period_filter(config.weekly, scheduled, Local::now())?;
            if scheduled && !runtime.should_send(&config, &label) {
                return Ok(());
            }
            let password = runtime.password_for(&config)?;
            let analytics = core.analytics(&filter)?;
            (
                config,
                label.clone(),
                render_summary(&label, &analytics),
                password,
            )
        };
        let message = Message::builder()
            .from(config.sender.parse()?)
            .to(config.recipient.parse()?)
            .subject(format!("daylog {label}"))
            .header(ContentType::TEXT_PLAIN)
            .body(body)?;
        // Both modes enforce certificate-validated TLS; plaintext SMTP is deliberately unavailable.
        let mut builder = if config.implicit_tls {
            SmtpTransport::relay(&config.host)?
        } else {
            SmtpTransport::starttls_relay(&config.host)?
        };
        builder = builder
            .port(config.port)
            .timeout(Some(std::time::Duration::from_secs(30)));
        if let Some(password) = password {
            builder = builder.credentials(Credentials::new(config.username.clone(), password));
        }
        builder.build().send(&message)?;
        delivered = true;
        if scheduled {
            // Remember successful SMTP delivery before touching disk. A config
            // write failure must not resend the same email every retry period.
            runtime.remember_delivery(&config, &label);
            let mut core = core.lock().unwrap();
            record_delivery(&mut core.config.email, &config, &label);
            core.config.save().map_err(|error| {
                anyhow::anyhow!("Email was sent, but delivery history could not be saved: {error}")
            })?;
        }
        Ok(())
    })();
    let mut session = runtime.inner.lock().unwrap();
    let status = &mut session.status;
    status.sending = false;
    if delivered {
        status.last_success = Some(Local::now().to_rfc3339());
    }
    match &result {
        Ok(()) => {
            status.last_error = None;
        }
        Err(e) => {
            status.last_error = Some(e.to_string());
        }
    }
    result
}

pub fn spawn(core: SharedCore, runtime: Arc<EmailRuntime>) {
    std::thread::spawn(move || {
        let mut retry_after = std::time::Instant::now();
        loop {
            let now = Local::now();
            let due = {
                let core = core.lock().unwrap();
                let email = &core.config.email;
                email.enabled
                    && now.hour() >= email.hour
                    && !core.is_locked()
                    && runtime.status(email).ready
                    && period_filter(email.weekly, true, now)
                        .is_ok_and(|(label, _)| runtime.should_send(email, &label))
            };
            if due && std::time::Instant::now() >= retry_after {
                let _ = send_summary(&core, &runtime, true);
                retry_after = std::time::Instant::now() + std::time::Duration::from_secs(900);
            }
            std::thread::sleep(std::time::Duration::from_secs(30));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn previous_week_is_complete_monday_to_monday() {
        let now = Local.with_ymd_and_hms(2026, 11, 4, 12, 0, 0).unwrap();
        let (label, filter) = period_filter(true, true, now).unwrap();
        assert_eq!(label, "weekly:2026-10-26");
        assert_eq!(
            filter
                .from
                .unwrap()
                .with_timezone(&Local)
                .date_naive()
                .to_string(),
            "2026-10-26"
        );
        assert_eq!(
            filter
                .to
                .unwrap()
                .with_timezone(&Local)
                .date_naive()
                .to_string(),
            "2026-11-01"
        );
    }
    #[test]
    fn email_validation_rejects_invalid_addresses_and_hosts() {
        let mut c = EmailConfig {
            host: "smtp.example.com".into(),
            sender: "me@example.com".into(),
            recipient: "me@example.com".into(),
            ..EmailConfig::default()
        };
        assert!(validate_email(&c).is_ok());
        c.host = "https://example.com".into();
        assert!(validate_email(&c).is_err());
    }

    fn email_config() -> EmailConfig {
        EmailConfig {
            enabled: true,
            host: "smtp.example.com".into(),
            username: "account".into(),
            sender: "me@example.com".into(),
            recipient: "me@example.com".into(),
            ..EmailConfig::default()
        }
    }

    #[test]
    fn smtp_secret_is_bound_to_server_port_tls_and_account() {
        let runtime = EmailRuntime::default();
        let config = email_config();
        runtime.set_password("test-secret".into(), &config);
        assert!(runtime.status(&config).ready);
        assert_eq!(
            runtime.password_for(&config).unwrap().as_deref(),
            Some("test-secret")
        );
        let mut changed = config.clone();
        changed.host = "other.example.com".into();
        assert!(!runtime.status(&changed).ready);
        assert!(runtime.password_for(&changed).is_err());
        changed = config.clone();
        changed.port = 465;
        assert!(runtime.password_for(&changed).is_err());
        changed = config.clone();
        changed.implicit_tls = true;
        assert!(runtime.password_for(&changed).is_err());
        changed = config.clone();
        changed.username = "other-account".into();
        assert!(runtime.password_for(&changed).is_err());
        changed.username.clear();
        assert_eq!(runtime.password_for(&changed).unwrap(), None);
        runtime.set_password(String::new(), &config);
        assert!(!runtime.status(&config).ready);
    }

    #[test]
    fn successful_delivery_stays_deduplicated_without_a_saved_marker() {
        let runtime = EmailRuntime::default();
        let config = email_config();
        let label = "daily:2026-11-03";
        assert!(runtime.should_send(&config, label));
        runtime.remember_delivery(&config, label);
        // Disk persistence may have failed, so config still has no marker.
        assert!(config.last_sent.is_none());
        assert!(config.sent_periods.is_empty());
        assert!(!runtime.should_send(&config, label));
        assert!(runtime.should_send(&config, "daily:2026-11-04"));
    }

    #[test]
    fn persisted_delivery_history_survives_switching_schedule_modes() {
        let runtime = EmailRuntime::default();
        let mut current = email_config();
        let daily = current.clone();
        record_delivery(&mut current, &daily, "daily:2026-11-03");
        current.weekly = true;
        let weekly = current.clone();
        record_delivery(&mut current, &weekly, "weekly:2026-10-26");
        current.weekly = false;
        let loaded: EmailConfig =
            serde_json::from_str(&serde_json::to_string(&current).unwrap()).unwrap();
        assert!(!runtime.should_send(&loaded, "daily:2026-11-03"));
        assert!(!runtime.should_send(&loaded, "weekly:2026-10-26"));
        assert!(runtime.should_send(&loaded, "daily:2026-11-04"));
        for index in 0..200 {
            record_delivery(&mut current, &daily, &format!("daily:{index}"));
        }
        assert_eq!(current.sent_periods.len(), 128);
    }

    #[test]
    fn delivery_record_does_not_overwrite_changed_server_or_schedule() {
        let sent = email_config();
        let mut current = sent.clone();
        current.host = "new.example.com".into();
        current.weekly = true;
        record_delivery(&mut current, &sent, "daily:2026-11-03");
        assert_eq!(current.host, "new.example.com");
        assert!(current.weekly);
        assert!(current.last_sent.is_none());
        assert!(current
            .sent_periods
            .contains(&delivery_key(&sent, "daily:2026-11-03")));
    }

    #[test]
    fn summary_includes_totals_without_window_titles() {
        use crate::model::{AppCategory, Event};
        let start = Utc.with_ymd_and_hms(2026, 11, 4, 12, 0, 0).unwrap();
        let event = Event::new(
            start,
            start + Duration::minutes(60),
            "Code".into(),
            "PRIVATE_DOCUMENT_TITLE".into(),
        );
        let report = crate::insights::analytics(
            &[event],
            &[AppCategory {
                app: "Code".into(),
                category: "Work".into(),
            }],
            &[],
            start,
        );
        let summary = render_summary("daily:2026-11-04", &report);
        assert!(summary.contains("Tracked time: 1h 0m"));
        assert!(summary.contains("Code: 1h 0m"));
        assert!(summary.contains("Work: 1h 0m"));
        assert!(!summary.contains("PRIVATE_DOCUMENT_TITLE"));
    }
}
