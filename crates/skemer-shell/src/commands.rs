use skemer_core::{Settings, UpdatePlan};

#[tauri::command]
pub async fn check_updates() -> Result<UpdatePlan, String> {
    Ok(UpdatePlan::default())
}

#[tauri::command]
pub async fn apply_updates(sources: Vec<String>, holds: Vec<String>) -> Result<String, String> {
    let _ = (sources, holds);
    Err("apply is not implemented yet".to_string())
}

#[tauri::command]
pub async fn load_settings() -> Result<Settings, String> {
    Settings::load().map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn save_settings(settings: Settings) -> Result<(), String> {
    settings.save().map_err(|error| error.to_string())
}
