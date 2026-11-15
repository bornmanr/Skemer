use std::str::FromStr;

use skemer_core::models::{ApplyOutcome, ApplyRequest, HoldDecision, UpdatePlan, UpdateSource};
use skemer_core::Settings;

use crate::helper;

#[tauri::command]
pub async fn check_updates() -> Result<UpdatePlan, String> {
    helper::check().await.map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn prepare_holds(holds: Vec<String>) -> Result<HoldDecision, String> {
    helper::prepare(holds)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn apply_updates(
    sources: Vec<String>,
    holds: Vec<String>,
) -> Result<ApplyOutcome, String> {
    let sources = sources
        .iter()
        .map(|source| UpdateSource::from_str(source))
        .collect::<Result<Vec<_>, _>>()?;
    let request = ApplyRequest { sources, holds };
    helper::apply(&request)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn load_settings() -> Result<Settings, String> {
    Settings::load().map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn save_settings(settings: Settings) -> Result<(), String> {
    settings.save().map_err(|error| error.to_string())
}
