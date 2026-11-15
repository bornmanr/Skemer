use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use alpm::{Alpm, SigLevel, TransFlag};
use anyhow::{Context, Result};
use skemer_core::models::{
    ApplyOutcome, ApplyRequest, HoldDecision, UpdateItem, UpdatePlan, UpdateSource,
};
use skemer_core::{pacman_conf, plan};

const ROOT: &str = "/";
const DB_PATH: &str = "/var/lib/pacman";
const SYNC_PATH: &str = "/var/lib/pacman/sync";
const PACMAN_CONF: &str = "/etc/pacman.conf";

/// Read the pending system update plan from the pacman databases.
pub fn check() -> Result<UpdatePlan> {
    let mut handle = SystemHandle::open()?;
    let items = handle.update_items();
    let items = match items {
        Ok(items) => items,
        Err(error) => {
            // A read-only check does not need the pacman lock. If another
            // process holds it, fall back quietly instead of alarming the user.
            if error.downcast_ref::<alpm::Error>() != Some(&alpm::Error::HandleLock) {
                handle
                    .warnings
                    .push(format!("Could not build the pacman transaction: {error}"));
            }
            handle.scan_items()?
        }
    };

    let reboot_suggested = plan::reboot_suggested_now(&items);
    let mut plan = UpdatePlan::from_items(items, now());
    plan.reboot_suggested = reboot_suggested;
    plan.warnings = handle.warnings;
    Ok(plan)
}

/// Validate a proposed hold set with libalpm. A hold is only offered when the
/// remaining transaction can still be prepared.
pub fn prepare(holds: &[String]) -> Result<HoldDecision> {
    let mut handle = SystemHandle::open()?;

    if !holds.is_empty() {
        // If the plan cannot be read (for example because another process holds
        // the pacman lock), let the prepare step report the real error instead
        // of rejecting every hold as unknown.
        if let Ok(items) = handle.update_items() {
            let pending: HashSet<String> = items.into_iter().map(|item| item.name).collect();
            let unknown: Vec<String> = holds
                .iter()
                .filter(|hold| !pending.contains(*hold))
                .cloned()
                .collect();
            if !unknown.is_empty() {
                return Ok(HoldDecision::rejected(
                    format!(
                        "These packages are not pending updates: {}",
                        unknown.join(", ")
                    ),
                    unknown,
                ));
            }
        }
    }

    handle.prepare_with(holds)
}

/// Apply the system update with `pacman -Syu`, so hooks and transaction
/// behavior are exactly pacman's.
pub async fn apply(request: &ApplyRequest) -> Result<ApplyOutcome> {
    if request
        .sources
        .iter()
        .any(|source| *source != UpdateSource::System)
    {
        anyhow::bail!("Flatpak updates are not available yet");
    }

    if !request.sources.contains(&UpdateSource::System) {
        return Ok(ApplyOutcome {
            success: true,
            held_packages: Vec::new(),
            reboot_suggested: false,
            log_tail: String::new(),
            error: None,
            candidates: Vec::new(),
        });
    }

    let holds = normalized_holds(&request.holds);

    let decision = {
        let holds = holds.clone();
        tokio::task::spawn_blocking(move || prepare(&holds))
            .await
            .context("join the hold validation task")??
    };
    if !decision.safe {
        return Ok(ApplyOutcome {
            success: false,
            held_packages: holds,
            reboot_suggested: false,
            log_tail: String::new(),
            error: Some(decision.reason),
            candidates: decision.candidates,
        });
    }

    let reboot_suggested = tokio::task::spawn_blocking(check)
        .await
        .context("join the plan task")?
        .map(|plan| plan.reboot_suggested)
        .unwrap_or(false);

    let mut command = tokio::process::Command::new("pacman");
    command
        .arg("-Syu")
        .arg("--noconfirm")
        .arg("--color")
        .arg("never")
        .env("LC_ALL", "C");
    if !holds.is_empty() {
        command.arg("--ignore").arg(holds.join(","));
    }

    let output = command.output().await.context("run pacman -Syu")?;
    let log = combined_log(&output.stdout, &output.stderr);
    let success = output.status.success();

    Ok(ApplyOutcome {
        success,
        held_packages: holds,
        reboot_suggested,
        error: if success {
            None
        } else {
            Some(failure_message(&log))
        },
        candidates: if success {
            Vec::new()
        } else {
            plan::hold_candidates_from_error(&log)
        },
        log_tail: tail_lines(&log, 200),
    })
}

struct SystemHandle {
    alpm: Alpm,
    warnings: Vec<String>,
}

impl SystemHandle {
    fn open() -> Result<Self> {
        let conf = pacman_conf::load(Path::new(PACMAN_CONF)).unwrap_or_default();
        let mut alpm = Alpm::new(ROOT, DB_PATH).context("open the pacman database")?;

        if !conf.ignore_pkgs.is_empty() {
            alpm.set_ignorepkgs(conf.ignore_pkgs.iter().map(String::as_str))
                .context("apply IgnorePkg from pacman.conf")?;
        }
        if !conf.ignore_groups.is_empty() {
            alpm.set_ignoregroups(conf.ignore_groups.iter().map(String::as_str))
                .context("apply IgnoreGroup from pacman.conf")?;
        }

        let mut warnings = Vec::new();
        let repositories = if conf.repositories.is_empty() {
            scan_sync_repositories()
        } else {
            conf.repositories
        };

        for repository in repositories {
            let database = sync_database(&repository);
            if !database.exists() {
                warnings.push(format!(
                    "The package database for {repository} is missing. Run pacman -Sy."
                ));
                continue;
            }
            alpm.register_syncdb(repository.as_str(), SigLevel::USE_DEFAULT)
                .with_context(|| format!("register the {repository} package database"))?;
        }

        if alpm.syncdbs().is_empty() {
            warnings.push("No package databases were found. Run pacman -Sy.".to_string());
        }

        Ok(Self { alpm, warnings })
    }

    /// Build the update list from the transaction pacman itself would run. This
    /// includes replacements, not just same-name upgrades.
    fn update_items(&mut self) -> Result<Vec<UpdateItem>> {
        self.alpm
            .trans_init(TransFlag::NONE)
            .context("start the libalpm transaction")?;

        let result = (|| -> Result<(Vec<UpdateItem>, Vec<String>)> {
            self.alpm
                .sync_sysupgrade(false)
                .context("compute the sysupgrade")?;
            let mut items = Vec::new();
            for package in self.alpm.trans_add().iter() {
                items.push(item_for(&self.alpm, package));
            }
            items.sort_by(|left, right| left.name.cmp(&right.name));
            let removals = self
                .alpm
                .trans_remove()
                .iter()
                .map(|package| package.name().to_string())
                .collect();
            Ok((items, removals))
        })();

        // Release even when the transaction could not be built, otherwise the
        // pacman lock is left behind.
        let release = self.alpm.trans_release();
        let (items, removals) = result?;
        release.context("release the libalpm transaction")?;

        if !removals.is_empty() {
            self.warnings
                .push(format!("This update removes: {}.", removals.join(", ")));
        }

        Ok(items)
    }

    /// Fallback when the transaction cannot be built: compare local packages
    /// against the sync databases one by one.
    fn scan_items(&self) -> Result<Vec<UpdateItem>> {
        let mut items = Vec::new();
        for package in self.alpm.localdb().pkgs().iter() {
            let Some(candidate) = package.sync_new_version(self.alpm.syncdbs()) else {
                continue;
            };
            items.push(item_for(&self.alpm, candidate));
        }
        items.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(items)
    }

    fn prepare_with(&mut self, holds: &[String]) -> Result<HoldDecision> {
        for hold in holds {
            self.alpm
                .add_ignorepkg(hold.as_str())
                .with_context(|| format!("hold {hold} for this transaction"))?;
        }

        self.alpm
            .trans_init(TransFlag::NONE)
            .context("start the libalpm transaction")?;

        let result = self
            .alpm
            .sync_sysupgrade(false)
            .map_err(|error| error.to_string())
            .and_then(|()| self.alpm.trans_prepare().map_err(|error| error.to_string()));

        let release = self.alpm.trans_release();

        match result {
            Ok(()) => {
                release.context("release the libalpm transaction")?;
                let reason = if holds.is_empty() {
                    "The full transaction is valid.".to_string()
                } else {
                    format!("The transaction is valid with {} held.", holds.join(", "))
                };
                Ok(HoldDecision::safe(reason))
            }
            Err(error) => {
                let _ = release;
                Ok(HoldDecision::rejected(
                    error.clone(),
                    plan::hold_candidates_from_error(&error),
                ))
            }
        }
    }
}

fn item_for(alpm: &Alpm, package: &alpm::Package) -> UpdateItem {
    let name = package.name().to_string();
    let current_version = alpm
        .localdb()
        .pkg(name.as_str())
        .map(|local| local.version().to_string())
        .unwrap_or_else(|_| "not installed".to_string());
    let repo = package
        .db()
        .map(|database| database.name().to_string())
        .unwrap_or_default();

    UpdateItem {
        id: format!("system:{name}"),
        source: UpdateSource::System,
        name: name.clone(),
        repo,
        current_version,
        new_version: package.version().to_string(),
        download_bytes: package.size().max(0) as u64,
        installed_bytes: package.isize().max(0) as u64,
        summary: package.desc().unwrap_or_default().to_string(),
        reboot_suggested: plan::is_reboot_package(&name),
        warning: None,
    }
}

fn scan_sync_repositories() -> Vec<String> {
    let mut repositories = Vec::new();
    let Ok(entries) = fs::read_dir(SYNC_PATH) else {
        return repositories;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("db") {
            continue;
        }
        if let Some(name) = path.file_stem().and_then(|stem| stem.to_str()) {
            repositories.push(name.to_string());
        }
    }
    repositories.sort();
    repositories
}

fn sync_database(repository: &str) -> PathBuf {
    Path::new(SYNC_PATH).join(format!("{repository}.db"))
}

fn normalized_holds(holds: &[String]) -> Vec<String> {
    let mut holds: Vec<String> = holds
        .iter()
        .map(|hold| hold.trim().to_string())
        .filter(|hold| !hold.is_empty())
        .collect();
    holds.sort();
    holds.dedup();
    holds
}

fn now() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

fn combined_log(stdout: &[u8], stderr: &[u8]) -> String {
    let mut log = String::from_utf8_lossy(stdout).into_owned();
    let stderr = String::from_utf8_lossy(stderr);
    if !stderr.trim().is_empty() {
        if !log.ends_with('\n') && !log.is_empty() {
            log.push('\n');
        }
        log.push_str(&stderr);
    }
    log
}

fn tail_lines(text: &str, limit: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= limit {
        return text.to_string();
    }
    lines[lines.len() - limit..].join("\n")
}

fn failure_message(log: &str) -> String {
    log.lines()
        .rev()
        .map(str::trim)
        .find(|line| {
            let line = line.to_ascii_lowercase();
            line.contains("error:") || line.contains("failed")
        })
        .unwrap_or("pacman -Syu failed")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holds_are_trimmed_sorted_and_deduplicated() {
        assert_eq!(
            normalized_holds(&[
                " linux ".to_string(),
                "mesa".to_string(),
                "linux".to_string(),
                "".to_string(),
            ]),
            vec!["linux".to_string(), "mesa".to_string()]
        );
    }

    #[test]
    fn tail_keeps_the_last_lines() {
        let text = "one\ntwo\nthree\nfour";
        assert_eq!(tail_lines(text, 2), "three\nfour");
        assert_eq!(tail_lines(text, 10), text);
    }

    #[test]
    fn failure_message_prefers_the_last_error_line() {
        let log = "warning: something\nsome error: first\nerror: failed to commit transaction";
        assert_eq!(failure_message(log), "error: failed to commit transaction");
    }

    #[test]
    fn combined_log_includes_stderr() {
        let log = combined_log(b"upgrading\n", b"error: nope\n");
        assert_eq!(log, "upgrading\nerror: nope\n");
    }

    #[test]
    fn repositories_are_sorted_from_the_sync_directory() {
        let repositories = scan_sync_repositories();
        let mut sorted = repositories.clone();
        sorted.sort();
        assert_eq!(repositories, sorted);
    }

    #[test]
    #[ignore = "reads the host pacman databases"]
    fn reads_the_host_plan() {
        let plan = check().expect("host plan");
        assert!(!plan.checked_at.is_empty());
        eprintln!(
            "{} updates, {} bytes, reboot suggested: {}",
            plan.items.len(),
            plan.total_download_bytes,
            plan.reboot_suggested
        );
        for item in &plan.items {
            assert_eq!(item.source, UpdateSource::System);
            assert!(!item.name.is_empty());
            assert!(!item.new_version.is_empty());
            eprintln!(
                "  {}/{} {} -> {} ({})",
                item.repo, item.name, item.current_version, item.new_version, item.download_bytes
            );
        }

        if is_root() {
            let decision = prepare(&[]).expect("host prepare");
            assert!(decision.safe, "{}", decision.reason);
            eprintln!("full transaction: {}", decision.reason);

            if let Some(first) = plan.items.first() {
                let decision = prepare(std::slice::from_ref(&first.name)).expect("hold prepare");
                eprintln!(
                    "holding {}: {} ({})",
                    first.name, decision.safe, decision.reason
                );
            }

            let decision = prepare(&["definitely-not-a-package".to_string()]).expect("bad hold");
            assert!(!decision.safe);
        }
    }

    fn is_root() -> bool {
        std::fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|status| {
                status
                    .lines()
                    .find(|line| line.starts_with("Uid:"))
                    .and_then(|line| line.split_whitespace().nth(2).map(str::to_string))
            })
            .is_some_and(|uid| uid == "0")
    }
}
