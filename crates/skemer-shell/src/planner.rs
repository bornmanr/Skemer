use anyhow::Result;
use skemer_core::{Settings, UpdatePlan};

use crate::{flatpak, helper};

/// Build one complete plan: the helper's system plan plus the enabled Flatpak
/// scopes. Used by the Tauri command and the periodic checker.
pub async fn build() -> Result<UpdatePlan> {
    let settings = Settings::load().unwrap_or_default();
    let mut plan = helper::check().await?;
    let mut warnings = plan.warnings.clone();

    let (plan, flatpak_warnings) = tokio::task::spawn_blocking(move || {
        let flatpak_warnings = flatpak::add_flatpak(&mut plan, &settings);
        (plan, flatpak_warnings)
    })
    .await?;

    warnings.extend(flatpak_warnings);
    let mut plan = plan;
    plan.warnings = warnings;
    Ok(plan)
}
