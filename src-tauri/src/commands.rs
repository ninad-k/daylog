//! Tauri command surface — the complete IPC contract the UI talks to.
//!
//! Every command returns `Result<T, String>` so errors surface as rejected
//! promises in the frontend.

use crate::config::Config;
use crate::model::{Analytics, ApplicationDetails, Event, EventFilter, Status};
use crate::SharedCore;
use chrono::{Local, TimeZone};
use std::path::PathBuf;
use tauri::{AppHandle, State};
use tauri_plugin_autostart::ManagerExt;

type R<T> = Result<T, String>;

fn lock(core: &SharedCore) -> std::sync::MutexGuard<'_, crate::core::Core> {
    core.lock().unwrap()
}

#[tauri::command]
pub fn get_config(core: State<'_, SharedCore>) -> R<Config> {
    Ok(lock(&core).config.clone())
}

#[tauri::command]
pub fn get_status(core: State<'_, SharedCore>) -> R<Status> {
    Ok(lock(&core).status())
}

/// Persist a new config and apply the parts that need action right now:
/// start-on-login registration, and reopening the store when the path or
/// encryption flag changed.
#[tauri::command]
pub fn set_config(
    app: AppHandle,
    core: State<'_, SharedCore>,
    config: Config,
    password: Option<String>,
) -> R<Status> {
    let mut c = lock(&core);
    let old = c.config.clone();
    config.validate().map_err(|e| e.to_string())?;
    // Apply start-on-login via the autostart plugin.
    if config.start_on_login != old.start_on_login {
        let m = app.autolaunch();
        let res = if config.start_on_login {
            m.enable()
        } else {
            m.disable()
        };
        res.map_err(|e| format!("start-on-login: {e}"))?;
    }

    if let Err(e) = c.apply_config(config, password.as_deref()) {
        if old.start_on_login {
            let _ = app.autolaunch().enable();
        } else {
            let _ = app.autolaunch().disable();
        }
        return Err(e.to_string());
    }
    Ok(c.status())
}

#[tauri::command]
pub fn pause(core: State<'_, SharedCore>) -> R<Status> {
    let mut c = lock(&core);
    c.flush_current();
    c.paused = true;
    Ok(c.status())
}

#[tauri::command]
pub fn resume(core: State<'_, SharedCore>) -> R<Status> {
    let mut c = lock(&core);
    c.paused = false;
    Ok(c.status())
}

#[tauri::command]
pub fn clear_logs(core: State<'_, SharedCore>) -> R<Status> {
    let mut c = lock(&core);
    c.clear().map_err(|e| e.to_string())?;
    Ok(c.status())
}

#[tauri::command]
pub fn clear_recorded_text(core: State<'_, SharedCore>) -> R<Status> {
    let mut c = lock(&core);
    c.clear_recorded_text().map_err(|e| e.to_string())?;
    Ok(c.status())
}

#[tauri::command]
pub fn query(core: State<'_, SharedCore>, filter: EventFilter) -> R<Vec<Event>> {
    lock(&core).query(&filter).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn analytics(core: State<'_, SharedCore>, filter: EventFilter) -> R<Analytics> {
    lock(&core).analytics(&filter).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn application_details(
    core: State<'_, SharedCore>,
    app: String,
    filter: EventFilter,
    text_only: Option<bool>,
) -> R<ApplicationDetails> {
    lock(&core)
        .application_details(&app, &filter, text_only.unwrap_or(false))
        .map_err(|error| error.to_string())
}

/// Unlock an encrypted store with the session password (wrong password errors).
#[tauri::command]
pub fn unlock(core: State<'_, SharedCore>, password: String) -> R<Status> {
    let mut c = lock(&core);
    c.open_store(Some(&password)).map_err(|e| e.to_string())?;
    Ok(c.status())
}

/// Turn encryption on (needs a password) or off, then reopen the store.
#[tauri::command]
pub fn set_encryption(
    core: State<'_, SharedCore>,
    enabled: bool,
    password: Option<String>,
) -> R<Status> {
    let mut c = lock(&core);
    c.set_encryption(enabled, password.as_deref())
        .map_err(|e| e.to_string())?;
    Ok(c.status())
}

/// Export the (filtered) log to CSV or Markdown. Returns the written path.
/// `dest` is optional; when omitted the file is written next to the log store.
#[tauri::command]
pub fn export(
    core: State<'_, SharedCore>,
    format: String,
    mut filter: EventFilter,
    dest: Option<String>,
) -> R<String> {
    let c = lock(&core);
    filter.limit = None;
    filter.offset = None;
    let events = c.query(&filter).map_err(|e| e.to_string())?;
    let base = c.active_path();

    let fmt = format.to_lowercase();
    let ext = if fmt == "md" || fmt == "markdown" {
        "md"
    } else {
        "csv"
    };
    let dest: PathBuf = dest
        .map(PathBuf::from)
        .unwrap_or_else(|| base.with_file_name(format!("daylog-export.{ext}")));
    validate_export_path(&c.config.log_path, &dest)?;
    drop(c);

    let body = if ext == "md" {
        render_markdown(&events)
    } else {
        render_csv(&events)
    };
    crate::storage::atomic_write(&dest, body.as_bytes()).map_err(|e| e.to_string())?;
    Ok(dest.display().to_string())
}

fn local(ts: chrono::DateTime<chrono::Utc>) -> String {
    Local
        .from_utc_datetime(&ts.naive_utc())
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

fn csv_field(s: &str) -> String {
    // RFC-4180-ish quoting.
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn csv_text(s: &str) -> String {
    if s.trim_start().starts_with(['=', '+', '-', '@']) || s.starts_with(['\t', '\r']) {
        csv_field(&format!("'{s}"))
    } else {
        csv_field(s)
    }
}

fn validate_export_path(base: &std::path::Path, destination: &std::path::Path) -> R<()> {
    for protected in [base.to_path_buf(), crate::storage::active_path(base, true)] {
        let same = protected == destination
            || (protected.exists()
                && destination.exists()
                && std::fs::canonicalize(&protected).ok()
                    == std::fs::canonicalize(destination).ok());
        if same {
            return Err("Choose an export file other than the activity log.".into());
        }
    }
    Ok(())
}

fn render_csv(events: &[Event]) -> String {
    let mut out = String::from("start,end,seconds,app,title\n");
    for e in events {
        out.push_str(&format!(
            "{},{},{},{},{}\n",
            csv_field(&local(e.start)),
            csv_field(&local(e.end)),
            e.seconds,
            csv_text(&e.app),
            csv_text(&e.title),
        ));
    }
    out
}

fn md_field(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('|', "\\|")
        .replace(['\n', '\r'], " ")
}

fn render_markdown(events: &[Event]) -> String {
    let mut out = String::from(
        "# daylog export\n\n| Start | End | Seconds | App | Title |\n|---|---|---|---|---|\n",
    );
    for e in events {
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            md_field(&local(e.start)),
            md_field(&local(e.end)),
            e.seconds,
            md_field(&e.app),
            md_field(&e.title),
        ));
    }
    out
}

/// Reveal the active log file in the OS file manager.
#[tauri::command]
pub fn reveal_log(app: AppHandle, core: State<'_, SharedCore>) -> R<()> {
    use tauri_plugin_opener::OpenerExt;
    let path = lock(&core).active_path();
    let dir = path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or(path.clone());
    app.opener()
        .open_path(dir.display().to_string(), None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn summary_preview(core: State<'_, SharedCore>, weekly: bool) -> R<String> {
    crate::reports::preview(&core, weekly).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn export_summary(core: State<'_, SharedCore>, weekly: bool, dest: String) -> R<String> {
    let body = crate::reports::preview(&core, weekly).map_err(|e| e.to_string())?;
    let path = PathBuf::from(dest);
    validate_export_path(&lock(&core).config.log_path, &path)?;
    crate::storage::atomic_write(&path, body.as_bytes()).map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}

#[tauri::command]
pub fn email_status(
    core: State<'_, SharedCore>,
    runtime: State<'_, std::sync::Arc<crate::reports::EmailRuntime>>,
) -> crate::reports::EmailStatus {
    runtime.status(&lock(&core).config.email)
}

#[tauri::command]
pub fn set_email_password(
    core: State<'_, SharedCore>,
    runtime: State<'_, std::sync::Arc<crate::reports::EmailRuntime>>,
    password: String,
) {
    runtime.set_password(password, &lock(&core).config.email);
}

#[tauri::command]
pub async fn send_summary_email(
    core: State<'_, SharedCore>,
    runtime: State<'_, std::sync::Arc<crate::reports::EmailRuntime>>,
) -> R<()> {
    let core = core.inner().clone();
    let runtime = runtime.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::reports::send_summary(&core, &runtime, false)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exports_quote_newlines_and_neutralize_spreadsheet_formulas() {
        assert_eq!(
            csv_text("hello, \"world\"\r\n"),
            "\"hello, \"\"world\"\"\r\n\""
        );
        assert_eq!(csv_text("=1+1"), "'=1+1");
        assert_eq!(csv_text("  @SUM(1)"), "'  @SUM(1)");
        assert_eq!(md_field("<script>|x\r\ny"), "&lt;script&gt;\\|x  y");
    }
    #[test]
    fn exports_cannot_overwrite_activity_files() {
        let base = std::env::temp_dir().join("daylog-protected.log");
        assert!(validate_export_path(&base, &base).is_err());
        assert!(validate_export_path(&base, &base.with_extension("enc")).is_err());
        assert!(validate_export_path(&base, &base.with_extension("csv")).is_ok());
    }

    #[test]
    fn exports_omit_recorded_text() {
        let now = chrono::Utc::now();
        let mut event = Event::new(
            now,
            now + chrono::Duration::seconds(2),
            "Editor".into(),
            "Document".into(),
        );
        event.typed_text = Some("private captured content".into());
        assert!(!render_csv(&[event.clone()]).contains("private captured content"));
        assert!(!render_markdown(&[event]).contains("private captured content"));
    }
}
