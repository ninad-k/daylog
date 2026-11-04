//! daylog — privacy-respecting personal activity logger.
//!
//! Layer map (each isolated behind a clean boundary so it can evolve alone):
//!   capture/  → senses the active window (app + title). No keyboard hooks.
//!   storage/  → plaintext JSONL or AES-GCM encrypted, behind one `Store` trait.
//!   core      → the shared state + all session/analytics logic.
//!   commands  → the Tauri IPC surface the UI calls.
//!   this file → wires it together: plugins, tray, window, capture thread.

mod capture;
mod commands;
mod config;
mod core;
mod model;
mod storage;

use crate::core::Core;
use std::sync::{Arc, Mutex};
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager,
};
use tauri_plugin_autostart::MacosLauncher;

/// The one piece of shared state. Cheap to clone (Arc); the capture thread,
/// the commands, and the tray all hold a handle to the same `Core`.
pub type SharedCore = Arc<Mutex<Core>>;

fn show_main_window(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let shared: SharedCore = Arc::new(Mutex::new(Core::bootstrap()));
    let show_on_launch = { shared.lock().unwrap().config.show_window_on_launch };

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec![]),
        ))
        .manage(shared.clone())
        .invoke_handler(tauri::generate_handler![
            commands::get_config,
            commands::set_config,
            commands::get_status,
            commands::pause,
            commands::resume,
            commands::clear_logs,
            commands::query,
            commands::analytics,
            commands::unlock,
            commands::set_encryption,
            commands::export,
            commands::reveal_log,
        ])
        .setup(move |app| {
            let handle = app.handle().clone();

            // --- System tray: quick actions without opening the window ---
            let open_i = MenuItem::with_id(app, "open", "Open daylog", true, None::<&str>)?;
            let folder_i = MenuItem::with_id(app, "folder", "Open Log Folder", true, None::<&str>)?;
            let toggle_i =
                MenuItem::with_id(app, "toggle", "Pause / Resume Logging", true, None::<&str>)?;
            let clear_i = MenuItem::with_id(app, "clear", "Clear Logs", true, None::<&str>)?;
            let sep = PredefinedMenuItem::separator(app)?;
            let quit_i = MenuItem::with_id(app, "quit", "Quit daylog", true, None::<&str>)?;
            let menu = Menu::with_items(
                app,
                &[&open_i, &folder_i, &toggle_i, &clear_i, &sep, &quit_i],
            )?;

            let _tray = TrayIconBuilder::with_id("main")
                .tooltip("daylog — activity logger")
                .icon(app.default_window_icon().cloned().unwrap())
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(move |app, event| {
                    let core = app.state::<SharedCore>();
                    match event.id.as_ref() {
                        "open" => show_main_window(app),
                        "folder" => {
                            use tauri_plugin_opener::OpenerExt;
                            let path = core.lock().unwrap().active_path();
                            let dir = path
                                .parent()
                                .map(|p| p.to_path_buf())
                                .unwrap_or(path.clone());
                            let _ = app
                                .opener()
                                .open_path(dir.display().to_string(), None::<&str>);
                        }
                        "toggle" => {
                            let status = {
                                let mut c = core.lock().unwrap();
                                c.flush_current();
                                c.paused = !c.paused;
                                c.status()
                            };
                            let _ = app.emit("daylog://status", status);
                        }
                        "clear" => {
                            let status = {
                                let mut c = core.lock().unwrap();
                                let _ = c.clear();
                                c.status()
                            };
                            let _ = app.emit("daylog://status", status);
                        }
                        "quit" => {
                            // Flush the in-progress session before exiting.
                            core.lock().unwrap().flush_current();
                            app.exit(0);
                        }
                        _ => {}
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        ..
                    } = event
                    {
                        show_main_window(tray.app_handle());
                    }
                })
                .build(app)?;

            // --- Window visibility per config (tray-only if the user wants) ---
            if let Some(w) = app.get_webview_window("main") {
                if show_on_launch {
                    let _ = w.show();
                } else {
                    let _ = w.hide();
                }
            }

            // --- Start the capture thread ---
            capture::spawn(shared.clone(), handle);
            Ok(())
        })
        // Closing the window hides it instead of quitting — logging continues
        // in the background from the tray, which is the whole point.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running daylog");
}
