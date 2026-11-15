use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;

use crate::paths;

/// User-facing Skemer settings. `check_interval_hours = 0` means "never".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub check_interval_hours: u32,
    pub notifications: bool,
    pub autostart: bool,
    pub close_to_tray: bool,
    pub include_flatpak_system: bool,
    pub include_flatpak_user: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            check_interval_hours: 6,
            notifications: true,
            autostart: true,
            close_to_tray: true,
            include_flatpak_system: true,
            include_flatpak_user: true,
        }
    }
}

impl Settings {
    pub fn load() -> Result<Self> {
        let Some(path) = paths::config_file() else {
            return Ok(Self::default());
        };

        match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text)
                .with_context(|| format!("parse settings file {}", path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => {
                Err(error).with_context(|| format!("read settings file {}", path.display()))
            }
        }
    }

    pub fn save(&self) -> Result<()> {
        let Some(path) = paths::config_file() else {
            anyhow::bail!("no XDG config directory is available");
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create settings directory {}", parent.display()))?;
        }
        let text = serde_json::to_string_pretty(self)?;
        fs::write(&path, text)
            .with_context(|| format!("write settings file {}", path.display()))?;
        Ok(())
    }

    pub fn check_interval_minutes(&self) -> Option<u64> {
        if self.check_interval_hours == 0 {
            None
        } else {
            Some(u64::from(self.check_interval_hours) * 60)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_agreed_ux() {
        let settings = Settings::default();
        assert_eq!(settings.check_interval_hours, 6);
        assert!(settings.notifications);
        assert!(settings.autostart);
        assert!(settings.close_to_tray);
        assert!(settings.include_flatpak_system);
        assert!(settings.include_flatpak_user);
    }

    #[test]
    fn zero_interval_means_never() {
        let settings = Settings {
            check_interval_hours: 0,
            ..Settings::default()
        };
        assert_eq!(settings.check_interval_minutes(), None);
    }

    #[test]
    fn interval_is_converted_to_minutes() {
        let settings = Settings {
            check_interval_hours: 3,
            ..Settings::default()
        };
        assert_eq!(settings.check_interval_minutes(), Some(180));
    }
}
