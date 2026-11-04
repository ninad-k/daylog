//! Thin wrapper over active-window detection.
//!
//! This is the ONLY thing daylog senses about your activity: the name of the
//! frontmost application and its window title. There is no keyboard, mouse, or
//! clipboard hook anywhere in this crate.

use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Focus {
    pub app: String,
    pub title: String,
}

/// Returns the frontmost window's app + title, or `None` when nothing can be
/// read (no focused window, or the OS denied access — e.g. macOS Screen
/// Recording permission not yet granted).
pub fn current_focus() -> Option<Focus> {
    match active_win_pos_rs::get_active_window() {
        Ok(w) => {
            let app = if w.app_name.trim().is_empty() {
                "Unknown".to_string()
            } else {
                w.app_name
            };
            Some(Focus {
                app,
                title: w.title,
            })
        }
        Err(_) => None,
    }
}
