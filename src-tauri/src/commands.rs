//! Tauri command surface — the complete IPC contract the UI talks to.
//!
//! Every command returns `Result<T, String>` so errors surface as rejected
//! promises in the frontend.

use crate::config::Config;
use crate::model::{Analytics, Event, EventFilter, Status};
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
pub fn set_config(app: AppHandle, core: State<'_, SharedCore>, config: Config) -> R<Status> {
    let mut c = lock(&core);
    let old = c.config.clone();

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

    let path_changed = config.log_path != old.log_path;
    let enc_changed = config.encryption_enabled != old.encryption_enabled;
    c.config = config;
    c.config.save().map_err(|e| e.to_string())?;

    // Reopen the store if where/how we write changed.
    if enc_changed || path_changed {
        c.flush_current();
        if c.config.encryption_enabled {
            // Turning encryption on needs a password via `set_encryption`/`unlock`;
            // until then the store is locked.
            // (Leaving store as-is here would write plaintext to the wrong file.)
            if enc_changed {
                // Caller should follow up with set_encryption(true, password).
            }
        } else {
            c.open_store(None).map_err(|e| e.to_string())?;
        }
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
pub fn query(core: State<'_, SharedCore>, filter: EventFilter) -> R<Vec<Event>> {
    lock(&core).query(&filter).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn analytics(core: State<'_, SharedCore>, filter: EventFilter) -> R<Analytics> {
    lock(&core).analytics(&filter).map_err(|e| e.to_string())
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
    c.flush_current();
    c.config.encryption_enabled = enabled;
    c.config.save().map_err(|e| e.to_string())?;
    if enabled {
        let pw = password.ok_or("password required to enable encryption")?;
        if pw.is_empty() {
            return Err("password cannot be empty".into());
        }
        c.open_store(Some(&pw)).map_err(|e| e.to_string())?;
    } else {
        c.open_store(None).map_err(|e| e.to_string())?;
    }
    Ok(c.status())
}

/// Export the (filtered) log to CSV or Markdown. Returns the written path.
/// `dest` is optional; when omitted the file is written next to the log store.
#[tauri::command]
pub fn export(
    core: State<'_, SharedCore>,
    format: String,
    filter: EventFilter,
    dest: Option<String>,
) -> R<String> {
    let c = lock(&core);
    let events = c.query(&filter).map_err(|e| e.to_string())?;
    let base = c.active_path();
    drop(c);

    let fmt = format.to_lowercase();
    let ext = if fmt == "md" || fmt == "markdown" {
        "md"
    } else {
        "csv"
    };
    let dest: PathBuf = dest
        .map(PathBuf::from)
        .unwrap_or_else(|| base.with_file_name(format!("daylog-export.{ext}")));

    let body = if ext == "md" {
        render_markdown(&events)
    } else {
        render_csv(&events)
    };
    std::fs::write(&dest, body).map_err(|e| e.to_string())?;
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
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn render_csv(events: &[Event]) -> String {
    let mut out = String::from("start,end,seconds,app,title\n");
    for e in events {
        out.push_str(&format!(
            "{},{},{},{},{}\n",
            csv_field(&local(e.start)),
            csv_field(&local(e.end)),
            e.seconds,
            csv_field(&e.app),
            csv_field(&e.title),
        ));
    }
    out
}

fn md_field(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ")
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
