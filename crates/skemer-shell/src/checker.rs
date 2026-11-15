use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::state::AppState;
use crate::{planner, tray};

/// Start the periodic checker. It waits briefly on startup, then checks on the
/// configured interval until the setting is "never". Saving settings wakes it
/// so a new interval takes effect immediately.
pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let mut delay = Duration::from_secs(30);

        loop {
            let settings = state.settings();
            let Some(interval) = settings.check_interval_minutes() else {
                state.settings_changed.notified().await;
                delay = Duration::from_secs(30);
                continue;
            };

            tokio::select! {
                _ = tokio::time::sleep(delay) => {
                    if !state.window_open() {
                        check(&app, &settings).await;
                    }
                    delay = Duration::from_secs(interval * 60);
                }
                _ = state.settings_changed.notified() => {
                    delay = Duration::from_secs(30);
                }
            }
        }
    });
}

/// Reap the window child when it exits so it does not linger as a zombie.
pub fn spawn_child_reaper(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let _ = state.window_open();
        }
    });
}

/// Run a check now, for the tray menu or a signal from the window process.
pub fn check_now(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let settings = state.settings();
        check(&app, &settings).await;
    });
}

/// Reload settings on SIGHUP and refresh the tray on SIGUSR1. The window child
/// sends these signals after saving settings or finishing an update.
pub fn spawn_settings_reload(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let Ok(mut hangup) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())
        else {
            return;
        };
        let Ok(mut refresh) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::user_defined1())
        else {
            return;
        };
        let state = app.state::<AppState>();

        loop {
            tokio::select! {
                _ = hangup.recv() => {
                    let settings = skemer_core::Settings::load().unwrap_or_default();
                    state.set_settings(settings);
                    state.settings_changed.notify_waiters();
                }
                _ = refresh.recv() => {
                    let settings = state.settings();
                    check(&app, &settings).await;
                }
            }
        }
    });
}

async fn check(app: &AppHandle, settings: &skemer_core::Settings) {
    match planner::build().await {
        Ok(plan) => {
            tray::update_tooltip(app, &plan);
            let _ = app.emit("skemer://plan-updated", &plan);
            if settings.notifications
                && !plan.items.is_empty()
                && !app.state::<AppState>().window_open()
            {
                notify(app, plan.items.len());
            }
        }
        Err(error) => tracing::warn!("periodic update check failed: {error:#}"),
    }
}

fn notify(app: &AppHandle, count: usize) {
    let body = if count == 1 {
        "1 update is available. Open Skemer to review it.".to_string()
    } else {
        format!("{count} updates are available. Open Skemer to review them.")
    };

    if let Err(error) = app
        .notification()
        .builder()
        .title("Skemer")
        .body(body)
        .show()
    {
        tracing::warn!("could not show update notification: {error}");
    }
}
