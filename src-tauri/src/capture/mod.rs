//! Capture layer: a background thread that samples the active window on an
//! interval and feeds each sample into [`Core::tick`].
//!
//! It holds no logging logic of its own beyond timing — all session bookkeeping
//! lives in `core` so it can be tested without a running desktop.

pub mod window;

use crate::SharedCore;
use chrono::Utc;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

/// Spawn the capture loop. Emits a `daylog://status` event after every sample
/// so the UI can live-update its status bar.
pub fn spawn(shared: SharedCore, app: AppHandle) {
    std::thread::spawn(move || loop {
        let interval = {
            let c = shared.lock().unwrap();
            c.config.poll_interval_ms
        };

        // Sample the frontmost window (app + title only).
        let focus = window::current_focus();

        let status = {
            let mut c = shared.lock().unwrap();
            c.tick(focus, Utc::now());
            c.status()
        };
        let _ = app.emit("daylog://status", status);

        // Clamp to a sane floor so a bad config value can't busy-spin.
        std::thread::sleep(Duration::from_millis(interval.max(250)));
    });
}
