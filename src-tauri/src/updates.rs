//! Signed updates use only the endpoint and public key embedded by the release build.
use crate::SharedCore;
use serde::Serialize;
use std::{sync::Mutex, time::Duration};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_updater::{Update, UpdaterExt};

#[derive(Default)]
pub struct PendingUpdate {
    update: Mutex<Option<Update>>,
    operation: Mutex<Operation>,
}
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Operation {
    #[default]
    Idle,
    Checking,
    Installing,
}
struct OperationGuard<'a>(&'a Mutex<Operation>);
impl Drop for OperationGuard<'_> {
    fn drop(&mut self) {
        *self.0.lock().unwrap() = Operation::Idle;
    }
}
impl PendingUpdate {
    fn begin(&self, next: Operation) -> Result<OperationGuard<'_>, String> {
        let mut operation = self.operation.lock().unwrap();
        if *operation != Operation::Idle {
            return Err("an update check or installation is already in progress".into());
        }
        *operation = next;
        Ok(OperationGuard(&self.operation))
    }
}
#[derive(Clone, Serialize)]
pub struct UpdateInfo {
    pub version: String,
    pub notes: String,
}

pub fn configured(app: &AppHandle) -> bool {
    has_signed_feed(app.config().plugins.0.get("updater"))
}

fn has_signed_feed(config: Option<&serde_json::Value>) -> bool {
    config.is_some_and(|config| {
        config
            .get("pubkey")
            .and_then(|value| value.as_str())
            .is_some_and(|key| !key.trim().is_empty())
            && config
                .get("endpoints")
                .and_then(|value| value.as_array())
                .is_some_and(|endpoints| {
                    !endpoints.is_empty()
                        && endpoints.iter().all(|endpoint| {
                            endpoint
                                .as_str()
                                .and_then(|value| tauri::Url::parse(value).ok())
                                .is_some_and(|url| {
                                    url.scheme() == "https" && url.host_str().is_some()
                                })
                        })
                })
    })
}

#[tauri::command]
pub fn update_configured(app: AppHandle) -> bool {
    configured(&app)
}

#[tauri::command]
pub async fn check_updates(app: AppHandle) -> Result<Option<UpdateInfo>, String> {
    if !configured(&app) {
        return Err(
            "This build has no signed update feed. Configure release signing to enable updates."
                .into(),
        );
    }
    let state = app.state::<PendingUpdate>();
    let _operation = state.begin(Operation::Checking)?;
    let update = app
        .updater_builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| e.to_string())?;
    let info = update.as_ref().map(|u| UpdateInfo {
        version: u.version.clone(),
        notes: u.body.clone().unwrap_or_default(),
    });
    *state.update.lock().unwrap() = update;
    Ok(info)
}

#[tauri::command]
pub async fn install_update(app: AppHandle, core: State<'_, SharedCore>) -> Result<(), String> {
    let state = app.state::<PendingUpdate>();
    let _operation = state.begin(Operation::Installing)?;
    // Keep the checked update available for retry after a download, signature,
    // or install error. The operation guard serializes checks and installations.
    let update = state
        .update
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "check for an update first".to_string())?;
    let progress_app = app.clone();
    let bytes = update
        .download(
            move |chunk, total| {
                let _ = progress_app.emit(
                    "daylog://update-progress",
                    serde_json::json!({ "chunk": chunk, "total": total }),
                );
            },
            || {},
        )
        .await
        .map_err(|e| e.to_string())?;
    {
        let mut c = core.lock().unwrap();
        let was_paused = c.paused;
        c.paused = true;
        c.flush_current();
        if c.has_pending_writes() {
            c.paused = was_paused;
            return Err(
                "Log writes failed. Resolve the storage error before installing an update.".into(),
            );
        }
        // An installer can terminate the process. Keep capture and tray
        // actions serialized until the install has finished or failed.
        if let Err(error) = update.install(bytes) {
            c.paused = was_paused;
            return Err(error.to_string());
        }
    }
    app.restart();
    #[allow(unreachable_code)]
    Ok(())
}

pub fn spawn(core: SharedCore, app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut next_check = std::time::Instant::now();
        loop {
            let enabled = core.lock().unwrap().config.auto_check_updates;
            if !enabled {
                next_check = std::time::Instant::now();
            }
            if configured(&app) && enabled && std::time::Instant::now() >= next_check {
                match check_updates(app.clone()).await {
                    Ok(Some(info)) => {
                        let _ = app.emit("daylog://update-available", info);
                    }
                    Err(error) => {
                        let _ = app.emit("daylog://update-error", error);
                    }
                    _ => {}
                }
                next_check = std::time::Instant::now() + Duration::from_secs(6 * 3600);
            }
            // Tokio is supplied by Tauri's async runtime; use a blocking sleeper without holding state.
            let _ = tauri::async_runtime::spawn_blocking(|| {
                std::thread::sleep(Duration::from_secs(30))
            })
            .await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_and_installs_are_exclusive_and_release_after_failure() {
        let state = PendingUpdate::default();
        {
            let _checking = state.begin(Operation::Checking).unwrap();
            assert!(state.begin(Operation::Checking).is_err());
            assert!(state.begin(Operation::Installing).is_err());
        }
        {
            let _installing = state.begin(Operation::Installing).unwrap();
            assert!(state.begin(Operation::Checking).is_err());
        }
        assert!(state.begin(Operation::Checking).is_ok());
    }

    #[test]
    fn unconfigured_source_build_and_invalid_feeds_are_disabled() {
        use serde_json::json;
        assert!(!has_signed_feed(None));
        for value in [
            json!(null),
            json!({}),
            json!({"pubkey":"key"}),
            json!({"pubkey":"", "endpoints":["https://example.com/latest.json"]}),
            json!({"pubkey":"key", "endpoints":[]}),
            json!({"pubkey":"key", "endpoints":[null]}),
            json!({"pubkey":"key", "endpoints":["http://example.com/latest.json"]}),
        ] {
            assert!(!has_signed_feed(Some(&value)));
        }
        assert!(has_signed_feed(Some(
            &json!({"pubkey":"key", "endpoints":["https://example.com/{{target}}/latest.json"]})
        )));
    }
}
