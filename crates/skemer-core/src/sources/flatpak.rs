use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::models::{UpdateItem, UpdatePlan, UpdateSource};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FlatpakScope {
    System,
    User,
}

impl FlatpakScope {
    pub fn source(self) -> UpdateSource {
        match self {
            Self::System => UpdateSource::FlatpakSystem,
            Self::User => UpdateSource::FlatpakUser,
        }
    }
}

pub trait FlatpakSource {
    fn check(&self, scope: FlatpakScope) -> Result<UpdatePlan>;
}

pub struct FlatpakCli;

impl FlatpakSource for FlatpakCli {
    fn check(&self, _scope: FlatpakScope) -> Result<UpdatePlan> {
        anyhow::bail!("flatpak check is not implemented yet")
    }
}

/// Parse tab-separated `flatpak remote-ls --updates` output. The caller should
/// request the columns `application,version,branch,origin,size`.
pub fn parse_remote_ls(output: &str, scope: FlatpakScope) -> Vec<UpdateItem> {
    let mut items = Vec::new();

    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let columns: Vec<&str> = line.split('\t').collect();
        if columns.len() < 5 {
            continue;
        }
        let application = columns[0].trim();
        if application.is_empty() {
            continue;
        }
        let version = columns[1].trim();
        let branch = columns[2].trim();
        let origin = columns[3].trim();
        let size = columns[4].trim();

        items.push(UpdateItem {
            id: format!("flatpak:{}:{}", scope_label(scope), application),
            source: scope.source(),
            name: application.to_string(),
            repo: if origin.is_empty() {
                "flathub".to_string()
            } else {
                origin.to_string()
            },
            current_version: "installed".to_string(),
            new_version: if branch.is_empty() {
                version.to_string()
            } else {
                format!("{version} ({branch})")
            },
            download_bytes: parse_size(size),
            installed_bytes: 0,
            summary: String::new(),
            reboot_suggested: false,
            warning: None,
        });
    }

    items
}

fn scope_label(scope: FlatpakScope) -> &'static str {
    match scope {
        FlatpakScope::System => "system",
        FlatpakScope::User => "user",
    }
}

fn parse_size(value: &str) -> u64 {
    let value = value.trim();
    if value.is_empty() {
        return 0;
    }

    let split = value
        .find(|character: char| !character.is_ascii_digit() && character != '.')
        .unwrap_or(value.len());
    let (number, unit) = value.split_at(split);
    let Ok(number) = number.parse::<f64>() else {
        return 0;
    };

    let multiplier = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "b" => 1.0,
        "kb" => 1000.0,
        "mb" => 1000.0 * 1000.0,
        "gb" => 1000.0 * 1000.0 * 1000.0,
        "kib" => 1024.0,
        "mib" => 1024.0 * 1024.0,
        "gib" => 1024.0 * 1024.0 * 1024.0,
        _ => 1.0,
    };

    (number * multiplier) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tab_separated_updates() {
        let output = "org.gimp.GIMP\t3.0.4\tstable\tflathub\t64.0 MiB\n\
                      org.freedesktop.Platform\t24.08.2\t24.08\tflathub\t73.0 MiB";
        let items = parse_remote_ls(output, FlatpakScope::System);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].name, "org.gimp.GIMP");
        assert_eq!(items[0].new_version, "3.0.4 (stable)");
        assert_eq!(items[0].repo, "flathub");
        assert_eq!(items[0].download_bytes, 64 * 1024 * 1024);
        assert_eq!(items[1].source, UpdateSource::FlatpakSystem);
    }

    #[test]
    fn user_scope_gets_its_own_source() {
        let items = parse_remote_ls("app\t1\tstable\tflathub\t1 MiB", FlatpakScope::User);
        assert_eq!(items[0].source, UpdateSource::FlatpakUser);
    }

    #[test]
    fn malformed_lines_are_ignored() {
        assert!(parse_remote_ls("only-one-column", FlatpakScope::System).is_empty());
        assert!(parse_remote_ls("", FlatpakScope::System).is_empty());
    }
}
