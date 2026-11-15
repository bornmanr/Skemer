use std::str::FromStr;

use skemer_core::models::{ApplyOutcome, ApplyRequest, HoldDecision, UpdatePlan, UpdateSource};
use skemer_core::Settings;

use crate::{flatpak, helper};

#[tauri::command]
pub async fn check_updates() -> Result<UpdatePlan, String> {
    let settings = Settings::load().unwrap_or_default();
    let mut plan = helper::check().await.map_err(|error| error.to_string())?;
    let mut warnings = plan.warnings.clone();

    let (plan, flatpak_warnings) = tokio::task::spawn_blocking(move || {
        let flatpak_warnings = flatpak::add_flatpak(&mut plan, &settings);
        (plan, flatpak_warnings)
    })
    .await
    .map_err(|error| error.to_string())?;

    warnings.extend(flatpak_warnings);
    let mut plan = plan;
    plan.warnings = warnings;
    Ok(plan)
}

#[tauri::command]
pub async fn prepare_holds(holds: Vec<String>) -> Result<HoldDecision, String> {
    helper::prepare(holds)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn apply_updates(
    app: tauri::AppHandle,
    sources: Vec<String>,
    holds: Vec<String>,
) -> Result<ApplyOutcome, String> {
    let sources = sources
        .iter()
        .map(|source| UpdateSource::from_str(source))
        .collect::<Result<Vec<_>, _>>()?;

    let wants_user = sources.contains(&UpdateSource::FlatpakUser);
    let helper_sources: Vec<UpdateSource> = sources
        .iter()
        .copied()
        .filter(|source| *source != UpdateSource::FlatpakUser)
        .collect();

    let mut outcome = if helper_sources.is_empty() {
        empty_outcome()
    } else {
        helper::apply(
            app.clone(),
            ApplyRequest {
                sources: helper_sources,
                holds,
            },
        )
        .await
        .map_err(|error| error.to_string())?
    };

    if outcome.success && wants_user {
        match flatpak::apply_user(&app).await {
            Ok(user) => {
                outcome.log_tail = flatpak::combine_logs(outcome.log_tail, user.log_tail);
                if !user.success {
                    outcome.success = false;
                    outcome.cancelled = user.cancelled;
                    outcome.error = user.error;
                    outcome.candidates = user.candidates;
                }
            }
            Err(error) => outcome = failure_outcome(error.to_string()),
        }
    }

    Ok(outcome)
}

#[tauri::command]
pub async fn cancel_updates() -> Result<(), String> {
    flatpak::cancel_user();
    // The helper may not have a running job; cancel is still a no-op then.
    let _ = helper::cancel().await;
    Ok(())
}

#[tauri::command]
pub async fn load_settings() -> Result<Settings, String> {
    Settings::load().map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn save_settings(settings: Settings) -> Result<(), String> {
    settings.save().map_err(|error| error.to_string())
}

fn empty_outcome() -> ApplyOutcome {
    ApplyOutcome {
        success: true,
        cancelled: false,
        held_packages: Vec::new(),
        reboot_suggested: false,
        log_tail: String::new(),
        error: None,
        candidates: Vec::new(),
    }
}

fn failure_outcome(error: String) -> ApplyOutcome {
    ApplyOutcome {
        success: false,
        cancelled: false,
        held_packages: Vec::new(),
        reboot_suggested: false,
        log_tail: String::new(),
        error: Some(error),
        candidates: Vec::new(),
    }
}
