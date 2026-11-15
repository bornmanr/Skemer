use std::collections::VecDeque;
use std::process::Stdio;
use std::sync::{Mutex, OnceLock};

use anyhow::{anyhow, Context, Result};
use skemer_core::models::{ApplyOutcome, UpdatePlan};
use skemer_core::sources::{flatpak_available, FlatpakCli, FlatpakScope, FlatpakSource};
use skemer_core::{plan, CancelHandle, Settings};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;

static USER_CANCEL: OnceLock<Mutex<Option<CancelHandle>>> = OnceLock::new();

fn user_cancel() -> &'static Mutex<Option<CancelHandle>> {
    USER_CANCEL.get_or_init(|| Mutex::new(None))
}

struct CancelGuard;

impl Drop for CancelGuard {
    fn drop(&mut self) {
        if let Ok(mut slot) = user_cancel().lock() {
            *slot = None;
        }
    }
}

/// Add the enabled Flatpak scopes to a plan. Returns warnings for scopes that
/// could not be read.
pub fn add_flatpak(plan: &mut UpdatePlan, settings: &Settings) -> Vec<String> {
    let mut warnings = Vec::new();
    if !flatpak_available() {
        return warnings;
    }

    let mut scopes = Vec::new();
    if settings.include_flatpak_system {
        scopes.push(FlatpakScope::System);
    }
    if settings.include_flatpak_user {
        scopes.push(FlatpakScope::User);
    }

    for scope in scopes {
        match FlatpakCli.check(scope) {
            Ok(items) => plan.items.extend(items),
            Err(error) => warnings.push(format!("{}: {error}", scope.source().label())),
        }
    }

    plan.items.sort_by(|left, right| {
        left.source
            .cmp(&right.source)
            .then_with(|| left.name.cmp(&right.name))
    });
    plan.total_download_bytes = plan.items.iter().map(|item| item.download_bytes).sum();
    plan.reboot_suggested =
        plan.reboot_suggested || plan.items.iter().any(|item| item.reboot_suggested);

    warnings
}

/// Update the user Flatpak installation as the desktop user.
pub async fn apply_user(app: &AppHandle) -> Result<ApplyOutcome> {
    let cancel = CancelHandle::default();
    {
        let mut slot = user_cancel()
            .lock()
            .map_err(|_| anyhow!("the Flatpak cancel lock is poisoned"))?;
        *slot = Some(cancel.clone());
    }
    let _guard = CancelGuard;

    let _ = app.emit(
        "skemer://update-progress",
        serde_json::json!({
            "percent": 95,
            "message": "Updating Flatpak user apps",
        }),
    );

    let mut command = tokio::process::Command::new("flatpak");
    command
        .arg("--user")
        .arg("update")
        .arg("-y")
        .arg("--noninteractive")
        .env("LC_ALL", "C")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);

    let mut child = command.spawn().context("run flatpak --user update")?;
    cancel.set_pid(child.id());
    if cancel.is_cancelled() {
        cancel.cancel();
    }

    let (line_tx, mut line_rx) = mpsc::channel::<String>(256);
    if let Some(stdout) = child.stdout.take() {
        let sender = line_tx.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if sender.send(line).await.is_err() {
                    break;
                }
            }
        });
    }
    if let Some(stderr) = child.stderr.take() {
        let sender = line_tx.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if sender.send(line).await.is_err() {
                    break;
                }
            }
        });
    }
    drop(line_tx);

    let wait = tokio::spawn(async move { child.wait().await });

    let mut log_tail: VecDeque<String> = VecDeque::with_capacity(200);
    while let Some(line) = line_rx.recv().await {
        let line = line.trim_end().to_string();
        if line.is_empty() {
            continue;
        }
        let _ = app.emit("skemer://update-log", serde_json::json!({ "line": line }));
        if log_tail.len() == 200 {
            log_tail.pop_front();
        }
        log_tail.push_back(line);
    }

    let status = wait.await.context("join the Flatpak wait task")??;
    let cancelled = cancel.is_cancelled() && !status.success();
    cancel.set_pid(None);

    let log = log_tail.into_iter().collect::<Vec<_>>().join("\n");
    let success = status.success() && !cancelled;
    let error = if success {
        None
    } else if cancelled {
        Some("Update cancelled.".to_string())
    } else {
        Some(plan::failure_message(&log))
    };

    let _ = app.emit(
        "skemer://update-progress",
        serde_json::json!({
            "percent": if success { 100 } else { 95 },
            "message": if success {
                "Update complete"
            } else if cancelled {
                "Update cancelled"
            } else {
                "Flatpak user update failed"
            },
        }),
    );

    Ok(ApplyOutcome {
        success,
        cancelled,
        held_packages: Vec::new(),
        reboot_suggested: false,
        log_tail: log,
        error,
        candidates: Vec::new(),
    })
}

/// Ask the local Flatpak update to stop.
pub fn cancel_user() {
    if let Ok(slot) = user_cancel().lock() {
        if let Some(handle) = slot.as_ref() {
            handle.cancel();
        }
    }
}

pub fn combine_logs(first: String, second: String) -> String {
    if first.is_empty() {
        second
    } else if second.is_empty() {
        first
    } else {
        format!("{first}\n{second}")
    }
}
