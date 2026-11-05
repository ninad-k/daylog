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

/// Apply all settings in one operation; encryption changes require an unlocked
/// source and a verified destination. Roll back autostart if preparation fails.
#[tauri::command]
pub fn save_settings(
    app: AppHandle,
    core: State<'_, SharedCore>,
    config: Config,
    password: Option<String>,
) -> R<Status> {
    config.validate().map_err(|e| e.to_string())?;
    let mut c = lock(&core);
    let old_autostart = c.config.start_on_login;
    let changed = old_autostart != config.start_on_login;
    if changed {
        let manager = app.autolaunch();
        if config.start_on_login {
            manager.enable()
        } else {
            manager.disable()
        }
        .map_err(|e| format!("Start on login: {e}"))?;
    }
    if let Err(e) = c.apply_config(config, password.as_deref()) {
        if changed {
            let manager = app.autolaunch();
            let rollback = if old_autostart {
                manager.enable()
            } else {
                manager.disable()
            };
            if let Err(rollback) = rollback {
                return Err(format!("{e}. Autostart rollback failed: {rollback}"));
            }
        }
        return Err(e.to_string());
    }
    Ok(c.status())
}

#[tauri::command]
pub fn pause(core: State<'_, SharedCore>) -> R<Status> {
    let mut c = lock(&core);
    c.flush_current().map_err(|e| e.to_string())?;
    c.paused = true;
    Ok(c.status())
}

#[tauri::command]
pub fn resume(core: State<'_, SharedCore>) -> R<Status> {
    let mut c = lock(&core);
    c.retry_storage().map_err(|e| e.to_string())?;
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

#[tauri::command]
pub fn retry_storage(core: State<'_, SharedCore>) -> R<Status> {
    let mut c = lock(&core);
    c.retry_storage().map_err(|e| e.to_string())?;
    Ok(c.status())
}

/// Unlock an encrypted store with the session password (wrong password errors).
#[tauri::command]
pub fn unlock(core: State<'_, SharedCore>, password: String) -> R<Status> {
    let mut c = lock(&core);
    if !c.is_locked() {
        return Ok(c.status());
    }
    c.open_store(Some(&password)).map_err(|e| e.to_string())?;
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
    let protected = [
        c.config.log_path.clone(),
        crate::storage::active_path(&c.config.log_path, true),
        crate::config::config_path(),
        crate::config::config_path().with_file_name("daylog.lock"),
    ];
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
    if protected.iter().any(|path| same_destination(&dest, path)) {
        return Err("Export cannot overwrite daylog history or configuration".into());
    }
    crate::storage::atomic_write(&dest, body.as_bytes()).map_err(|e| e.to_string())?;
    Ok(dest.display().to_string())
}

fn same_destination(a: &std::path::Path, b: &std::path::Path) -> bool {
    fn resolved(path: &std::path::Path) -> PathBuf {
        path.canonicalize().unwrap_or_else(|_| {
            let parent = path.parent().unwrap_or(std::path::Path::new("."));
            parent
                .canonicalize()
                .unwrap_or_else(|_| parent.to_path_buf())
                .join(path.file_name().unwrap_or_default())
        })
    }
    resolved(a) == resolved(b)
}

fn local(ts: chrono::DateTime<chrono::Utc>) -> String {
    Local
        .from_utc_datetime(&ts.naive_utc())
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

fn csv_field(s: &str) -> String {
    // RFC-4180-ish quoting.
    let protected;
    let s = if s.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        protected = format!("'{s}");
        protected.as_str()
    } else {
        s
    };
    if s.contains([',', '"', '\n', '\r']) {
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
    let mut out = String::new();
    for character in s.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\n' | '\r' => out.push(' '),
            '\\' | '|' | '[' | ']' | '!' | '`' | '*' | '_' => {
                out.push('\\');
                out.push(character);
            }
            c => out.push(c),
        }
    }
    out
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exports_escape_cells_and_neutralize_spreadsheet_formulas() {
        assert_eq!(csv_field("a,\"b\""), "\"a,\"\"b\"\"\"");
        assert_eq!(csv_field("=1+1"), "'=1+1");
        assert_eq!(csv_field("line\rbreak"), "\"line\rbreak\"");
        assert_eq!(md_field("a|b\nc"), "a\\|b c");
    }
}
