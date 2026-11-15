use std::collections::HashMap;
use std::process::Command;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::models::{UpdateItem, UpdateSource};

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

    pub fn flag(self) -> &'static str {
        match self {
            Self::System => "--system",
            Self::User => "--user",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
        }
    }
}

pub trait FlatpakSource {
    fn check(&self, scope: FlatpakScope) -> Result<Vec<UpdateItem>>;
}

pub struct FlatpakCli;

impl FlatpakSource for FlatpakCli {
    fn check(&self, scope: FlatpakScope) -> Result<Vec<UpdateItem>> {
        let installed = run(scope, &["list", "--columns=application,branch,version"])?;
        let updates = run(
            scope,
            &[
                "remote-ls",
                "--updates",
                "--columns=application,name,branch,origin,version,download-size,description",
            ],
        )?;
        Ok(parse_updates(&updates, &parse_installed(&installed), scope))
    }
}

/// True when a usable Flatpak command is on PATH.
pub fn flatpak_available() -> bool {
    Command::new("flatpak")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn run(scope: FlatpakScope, args: &[&str]) -> Result<String> {
    let output = Command::new("flatpak")
        .arg(scope.flag())
        .args(args)
        .output()
        .with_context(|| format!("run flatpak {}", args.join(" ")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        if stderr.contains("No installations")
            || stderr.contains("No user installations")
            || stderr.contains("No system installations")
        {
            return Ok(String::new());
        }
        anyhow::bail!("flatpak {} failed: {stderr}", args.join(" "));
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Map `(application, branch)` to the installed version. The version may be
/// empty for runtimes.
pub fn parse_installed(output: &str) -> HashMap<(String, String), String> {
    let mut installed = HashMap::new();
    for line in output.lines() {
        let columns: Vec<&str> = line.split('\t').collect();
        if columns.len() < 3 {
            continue;
        }
        let application = columns[0].trim();
        let branch = columns[1].trim();
        let version = columns[2].trim();
        if application.is_empty() {
            continue;
        }
        installed.insert(
            (application.to_string(), branch.to_string()),
            version.to_string(),
        );
    }
    installed
}

/// Parse tab-separated `flatpak remote-ls --updates` output. The columns are
/// `application,name,branch,origin,version,download-size,description`.
pub fn parse_updates(
    output: &str,
    installed: &HashMap<(String, String), String>,
    scope: FlatpakScope,
) -> Vec<UpdateItem> {
    let mut items = Vec::new();

    for line in output.lines() {
        let columns: Vec<&str> = line.split('\t').collect();
        if columns.len() < 6 {
            continue;
        }
        let application = columns[0].trim();
        if application.is_empty() {
            continue;
        }
        let name = columns[1].trim();
        let branch = columns[2].trim();
        let origin = columns[3].trim();
        let version = columns[4].trim();
        let size = columns[5].trim();
        let description = columns.get(6).map(|value| value.trim()).unwrap_or("");

        let key = (application.to_string(), branch.to_string());
        let installed_version = installed.get(&key).map(String::as_str).unwrap_or("");
        let current_version = if !installed_version.is_empty() {
            installed_version.to_string()
        } else if !branch.is_empty() {
            branch.to_string()
        } else {
            "installed".to_string()
        };
        let new_version = if !version.is_empty() {
            version.to_string()
        } else {
            "update".to_string()
        };
        let summary = if !description.is_empty() {
            description.to_string()
        } else if !name.is_empty() && !branch.is_empty() {
            format!("{name} ({branch})")
        } else if !name.is_empty() {
            name.to_string()
        } else if !branch.is_empty() {
            format!("Flatpak runtime or application ({branch})")
        } else {
            "Flatpak runtime or application".to_string()
        };

        items.push(UpdateItem {
            id: format!("flatpak:{}:{application}:{branch}", scope.label()),
            source: scope.source(),
            name: application.to_string(),
            repo: if origin.is_empty() {
                "flathub".to_string()
            } else {
                origin.to_string()
            },
            current_version,
            new_version,
            download_bytes: parse_size(size),
            installed_bytes: 0,
            summary: summary.to_string(),
            reboot_suggested: false,
            warning: None,
        });
    }

    items.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.id.cmp(&right.id))
    });
    items
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
    fn parses_installed_refs() {
        let output = "org.gimp.GIMP\tstable\t3.0.4\n\
                      org.freedesktop.Platform\t24.08\t\n";
        let installed = parse_installed(output);
        assert_eq!(
            installed.get(&("org.gimp.GIMP".to_string(), "stable".to_string())),
            Some(&"3.0.4".to_string())
        );
        assert_eq!(
            installed.get(&("org.freedesktop.Platform".to_string(), "24.08".to_string())),
            Some(&String::new())
        );
    }

    #[test]
    fn parses_app_and_runtime_updates() {
        let installed = parse_installed(
            "org.gimp.GIMP\tstable\t3.0.2\n\
             org.freedesktop.Platform\t24.08\t24.08.1\n",
        );
        let output = "org.gimp.GIMP\tGIMP\tstable\tflathub\t3.0.4\t64.0 MB\tImage editor\n\
                      org.freedesktop.Platform\tPlatform\t24.08\tflathub\t24.08.2\t73.0 MB\t";
        let items = parse_updates(output, &installed, FlatpakScope::System);

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].name, "org.freedesktop.Platform");
        assert_eq!(items[0].source, UpdateSource::FlatpakSystem);
        assert_eq!(items[0].current_version, "24.08.1");
        assert_eq!(items[0].new_version, "24.08.2");
        assert_eq!(items[0].summary, "Platform (24.08)");

        assert_eq!(items[1].name, "org.gimp.GIMP");
        assert_eq!(items[1].current_version, "3.0.2");
        assert_eq!(items[1].new_version, "3.0.4");
        assert_eq!(items[1].summary, "Image editor");
        assert_eq!(items[1].download_bytes, 64_000_000);
    }

    #[test]
    fn missing_versions_fall_back_to_the_branch() {
        let installed = parse_installed("org.kde.Platform\t6.10\t\n");
        let output = "org.kde.Platform\tkde platform\t6.10\tflathub\t\t392.1 MB\t\n";
        let items = parse_updates(output, &installed, FlatpakScope::User);
        assert_eq!(items[0].current_version, "6.10");
        assert_eq!(items[0].new_version, "update");
        assert_eq!(items[0].summary, "kde platform (6.10)");
        assert_eq!(items[0].source, UpdateSource::FlatpakUser);
    }

    #[test]
    fn malformed_lines_are_ignored() {
        assert!(parse_installed("only-one-column").is_empty());
        assert!(parse_updates("", &HashMap::new(), FlatpakScope::System).is_empty());
        assert!(parse_updates("only-one-column", &HashMap::new(), FlatpakScope::System).is_empty());
    }

    #[test]
    fn sizes_are_parsed() {
        assert_eq!(parse_size("147.9 MB"), 147_900_000);
        assert_eq!(parse_size("75.8 kB"), 75_800);
        assert_eq!(parse_size("1.5 GiB"), 1_610_612_736);
        assert_eq!(parse_size(""), 0);
    }

    #[test]
    #[ignore = "reads the host Flatpak installation"]
    fn reads_the_host_flatpak_user() {
        if !flatpak_available() {
            return;
        }
        let items = FlatpakCli
            .check(FlatpakScope::User)
            .expect("user Flatpak check");
        eprintln!("{} Flatpak user updates", items.len());
        for item in &items {
            eprintln!(
                "  {} {} -> {} ({} bytes)",
                item.name, item.current_version, item.new_version, item.download_bytes
            );
        }
        assert_eq!(
            FlatpakCli
                .check(FlatpakScope::System)
                .expect("system Flatpak check")
                .len(),
            0
        );
    }
}
