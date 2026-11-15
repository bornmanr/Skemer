use std::collections::{HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use alpm::{Alpm, SigLevel, TransFlag};
use anyhow::{Context, Result};
use skemer_core::models::{
    ApplyOutcome, ApplyRequest, HoldDecision, UpdateItem, UpdatePlan, UpdateSource,
};
use skemer_core::sources::FlatpakScope;
use skemer_core::{pacman_conf, plan, CancelHandle};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;

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

/// Progress and log events from a running apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyEvent {
    Progress { percent: u32, message: String },
    Log(String),
}

struct ChildOutcome {
    success: bool,
    cancelled: bool,
    percent: u32,
    log: String,
    error: Option<String>,
}

/// Run a child process, forwarding each line as a log event and letting the
/// caller turn selected lines into progress events.
async fn run_child<F>(
    mut command: tokio::process::Command,
    label: &str,
    start_percent: u32,
    mut classify: F,
    events: mpsc::Sender<ApplyEvent>,
    cancel: CancelHandle,
) -> Result<ChildOutcome>
where
    F: FnMut(&str, u32) -> Option<(u32, String)>,
{
    command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);

    let mut attempts = 0;
    let mut child = loop {
        match command.spawn() {
            Ok(child) => break child,
            Err(error) if error.raw_os_error() == Some(libc::ETXTBSY) && attempts < 5 => {
                // A freshly written test helper (or a pacman binary being
                // replaced) can be briefly busy. Retry for a short moment.
                attempts += 1;
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            Err(error) => return Err(error).with_context(|| format!("run {label}")),
        }
    };
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
    let mut percent = start_percent;
    while let Some(line) = line_rx.recv().await {
        let line = line.trim_end().to_string();
        if line.is_empty() {
            continue;
        }
        let _ = events.send(ApplyEvent::Log(line.clone())).await;

        if let Some((new_percent, message)) = classify(&line, percent) {
            percent = new_percent;
            let _ = events.send(ApplyEvent::Progress { percent, message }).await;
        }

        if log_tail.len() == 200 {
            log_tail.pop_front();
        }
        log_tail.push_back(line);
    }

    let status = wait.await.context("join the child wait task")??;
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

    Ok(ChildOutcome {
        success,
        cancelled,
        percent,
        log,
        error,
    })
}

fn pacman_progress(line: &str, percent: u32) -> Option<(u32, String)> {
    if let Some((new_percent, message)) = plan::parse_progress(line) {
        Some((new_percent, message))
    } else {
        line.strip_prefix(":: ")
            .map(|message| (percent, message.to_string()))
    }
}

async fn apply_system(
    request: &ApplyRequest,
    reboot_suggested: bool,
    pacman: &str,
    events: mpsc::Sender<ApplyEvent>,
    cancel: CancelHandle,
) -> Result<ApplyOutcome> {
    let holds = normalized_holds(&request.holds);
    emit(
        &events,
        ApplyEvent::Progress {
            percent: 0,
            message: "Preparing transaction".to_string(),
        },
    )
    .await;

    let mut command = tokio::process::Command::new(pacman);
    command
        .arg("-Syu")
        .arg("--noconfirm")
        .arg("--color")
        .arg("never")
        .env("LC_ALL", "C");
    if !holds.is_empty() {
        command.arg("--ignore").arg(holds.join(","));
    }

    let outcome = run_child(
        command,
        &format!("{pacman} -Syu"),
        0,
        pacman_progress,
        events.clone(),
        cancel,
    )
    .await?;

    let candidates = if outcome.success || outcome.cancelled {
        Vec::new()
    } else {
        plan::hold_candidates_from_error(&outcome.log)
    };
    let message = if outcome.success {
        "System updated"
    } else if outcome.cancelled {
        "Update cancelled"
    } else {
        "Update failed"
    };
    let _ = events
        .send(ApplyEvent::Progress {
            percent: if outcome.success { 90 } else { outcome.percent },
            message: message.to_string(),
        })
        .await;

    Ok(ApplyOutcome {
        success: outcome.success,
        cancelled: outcome.cancelled,
        held_packages: holds,
        reboot_suggested,
        log_tail: outcome.log,
        error: outcome.error,
        candidates,
    })
}

async fn apply_flatpak(
    scope: FlatpakScope,
    flatpak: &str,
    start_percent: u32,
    events: mpsc::Sender<ApplyEvent>,
    cancel: CancelHandle,
) -> Result<ApplyOutcome> {
    let label = scope.source().label();
    let mut command = tokio::process::Command::new(flatpak);
    command
        .arg(scope.flag())
        .arg("update")
        .arg("-y")
        .arg("--noninteractive")
        .env("LC_ALL", "C");

    let start = start_percent;
    let outcome = run_child(
        command,
        &format!("{flatpak} {} update", scope.flag()),
        start_percent,
        move |line: &str, percent| {
            if line.starts_with("Updating ") || line.starts_with("Looking for updates") {
                Some((percent.max(start), line.trim().to_string()))
            } else {
                None
            }
        },
        events.clone(),
        cancel,
    )
    .await?;

    let message = if outcome.success {
        format!("{label} updated")
    } else if outcome.cancelled {
        "Update cancelled".to_string()
    } else {
        "Update failed".to_string()
    };
    let _ = events
        .send(ApplyEvent::Progress {
            percent: if outcome.success { 95 } else { outcome.percent },
            message,
        })
        .await;

    Ok(ApplyOutcome {
        success: outcome.success,
        cancelled: outcome.cancelled,
        held_packages: Vec::new(),
        reboot_suggested: false,
        log_tail: outcome.log,
        error: outcome.error,
        candidates: Vec::new(),
    })
}

/// Run the selected helper-side sources: the system, then Flatpak system.
/// Flatpak user updates run in the user session and are rejected here.
pub async fn apply_streaming(
    request: &ApplyRequest,
    reboot_suggested: bool,
    pacman: &str,
    flatpak: &str,
    events: mpsc::Sender<ApplyEvent>,
    cancel: CancelHandle,
) -> Result<ApplyOutcome> {
    if request.sources.contains(&UpdateSource::FlatpakUser) {
        anyhow::bail!("Flatpak user updates run in the user session");
    }
    if request.sources.is_empty() {
        return Ok(empty_outcome());
    }

    let system_selected = request.sources.contains(&UpdateSource::System);
    let flatpak_selected = request.sources.contains(&UpdateSource::FlatpakSystem);
    let mut log = String::new();
    let mut held_packages = Vec::new();
    let mut reboot_suggested = reboot_suggested;

    if system_selected {
        let outcome = apply_system(
            request,
            reboot_suggested,
            pacman,
            events.clone(),
            cancel.clone(),
        )
        .await?;
        if !outcome.success {
            return Ok(outcome);
        }
        log = outcome.log_tail;
        held_packages = outcome.held_packages;
        reboot_suggested = outcome.reboot_suggested;
    }

    if flatpak_selected {
        let start = if system_selected { 90 } else { 50 };
        let outcome =
            apply_flatpak(FlatpakScope::System, flatpak, start, events.clone(), cancel).await?;
        log = combine_logs(log, outcome.log_tail);
        if !outcome.success {
            return Ok(ApplyOutcome {
                success: false,
                cancelled: outcome.cancelled,
                held_packages,
                reboot_suggested,
                log_tail: log,
                error: outcome.error,
                candidates: Vec::new(),
            });
        }
    }

    let _ = events
        .send(ApplyEvent::Progress {
            percent: 100,
            message: "Update complete".to_string(),
        })
        .await;

    Ok(ApplyOutcome {
        success: true,
        cancelled: false,
        held_packages,
        reboot_suggested,
        log_tail: log,
        error: None,
        candidates: Vec::new(),
    })
}

async fn emit(events: &mpsc::Sender<ApplyEvent>, event: ApplyEvent) {
    let _ = events.send(event).await;
}

fn combine_logs(first: String, second: String) -> String {
    if first.is_empty() {
        second
    } else if second.is_empty() {
        first
    } else {
        format!("{first}\n{second}")
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

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
    fn failure_message_prefers_the_last_error_line() {
        let log = "warning: something\nsome error: first\nerror: failed to commit transaction";
        assert_eq!(
            plan::failure_message(log),
            "error: failed to commit transaction"
        );
    }

    #[tokio::test]
    async fn streaming_apply_reports_progress_and_logs() {
        let script = fake_pacman(
            "#!/bin/sh\n\
             echo ':: Starting full system upgrade...'\n\
             echo '(1/2) upgrading linux'\n\
             echo '(2/2) upgrading mesa'\n\
             exit 0\n",
        );
        let (tx, mut rx) = mpsc::channel(64);
        let request = request(&[UpdateSource::System], &[]);
        let outcome = apply_streaming(
            &request,
            true,
            script.to_str().expect("script path"),
            "/bin/true",
            tx,
            CancelHandle::default(),
        )
        .await
        .expect("apply");
        let events = drain(&mut rx).await;

        assert!(outcome.success);
        assert!(!outcome.cancelled);
        assert!(outcome.reboot_suggested);
        assert!(outcome.log_tail.contains("(2/2) upgrading mesa"));
        assert!(events
            .iter()
            .any(|event| matches!(event, ApplyEvent::Progress { percent: 50, .. })));
        assert!(events
            .iter()
            .any(|event| matches!(event, ApplyEvent::Progress { percent: 100, .. })));
        assert!(events.iter().any(
            |event| matches!(event, ApplyEvent::Log(line) if line.contains("upgrading linux"))
        ));
    }

    #[tokio::test]
    async fn streaming_apply_reports_failure_candidates() {
        let script = fake_pacman(
            "#!/bin/sh\n\
             echo \"error: failed to commit transaction (conflicting files)\"\n\
             echo \"/usr/lib/foo: /usr/lib/foo exists in both 'mpv' and 'mpv-old'\"\n\
             exit 1\n",
        );
        let (tx, mut rx) = mpsc::channel(64);
        let request = request(&[UpdateSource::System], &[]);
        let outcome = apply_streaming(
            &request,
            false,
            script.to_str().expect("script path"),
            "/bin/true",
            tx,
            CancelHandle::default(),
        )
        .await
        .expect("apply");
        let _ = drain(&mut rx).await;

        assert!(!outcome.success);
        assert!(!outcome.cancelled);
        assert_eq!(
            outcome.error.as_deref(),
            Some("error: failed to commit transaction (conflicting files)")
        );
        assert_eq!(
            outcome.candidates,
            vec!["mpv".to_string(), "mpv-old".to_string()]
        );
    }

    #[tokio::test]
    async fn cancellation_stops_pacman() {
        let script = fake_pacman(
            "#!/bin/sh\n\
             trap 'exit 130' INT TERM\n\
             echo '(1/2) upgrading linux'\n\
             sleep 5\n",
        );
        let (tx, mut rx) = mpsc::channel(64);
        let request = request(&[UpdateSource::System], &[]);
        let cancel = CancelHandle::default();
        let canceller = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            canceller.cancel();
        });

        let outcome = apply_streaming(
            &request,
            false,
            script.to_str().expect("script path"),
            "/bin/true",
            tx,
            cancel,
        )
        .await
        .expect("apply");
        let _ = drain(&mut rx).await;

        assert!(!outcome.success);
        assert!(outcome.cancelled);
        assert_eq!(outcome.error.as_deref(), Some("Update cancelled."));
    }

    #[tokio::test]
    async fn flatpak_system_apply_streams_logs() {
        let script = fake_flatpak(
            "#!/bin/sh\n\
             echo 'Looking for updates...'\n\
             echo 'Updating org.gimp.GIMP'\n\
             echo 'Updating org.freedesktop.Platform'\n\
             exit 0\n",
        );
        let (tx, mut rx) = mpsc::channel(64);
        let request = request(&[UpdateSource::FlatpakSystem], &[]);
        let outcome = apply_streaming(
            &request,
            false,
            "/bin/true",
            script.to_str().expect("script path"),
            tx,
            CancelHandle::default(),
        )
        .await
        .expect("apply");
        let events = drain(&mut rx).await;

        assert!(outcome.success);
        assert!(outcome.log_tail.contains("Updating org.gimp.GIMP"));
        assert!(events
            .iter()
            .any(|event| matches!(event, ApplyEvent::Progress { percent: 95, .. })));
        assert!(events
            .iter()
            .any(|event| matches!(event, ApplyEvent::Progress { percent: 100, .. })));
    }

    #[tokio::test]
    async fn mixed_apply_runs_system_then_flatpak() {
        let pacman = fake_pacman("#!/bin/sh\necho 'system line'\nexit 0\n");
        let flatpak = fake_flatpak("#!/bin/sh\necho 'flatpak line'\nexit 0\n");
        let (tx, mut rx) = mpsc::channel(64);
        let request = request(&[UpdateSource::System, UpdateSource::FlatpakSystem], &[]);
        let outcome = apply_streaming(
            &request,
            false,
            pacman.to_str().expect("pacman path"),
            flatpak.to_str().expect("flatpak path"),
            tx,
            CancelHandle::default(),
        )
        .await
        .expect("apply");
        let _ = drain(&mut rx).await;

        assert!(outcome.success);
        let system = outcome.log_tail.find("system line").expect("system log");
        let flatpak = outcome.log_tail.find("flatpak line").expect("flatpak log");
        assert!(system < flatpak);
    }

    #[tokio::test]
    async fn flatpak_user_is_rejected_by_the_helper() {
        let (tx, _rx) = mpsc::channel(4);
        let request = request(&[UpdateSource::FlatpakUser], &[]);
        let result = apply_streaming(
            &request,
            false,
            "/bin/true",
            "/bin/true",
            tx,
            CancelHandle::default(),
        )
        .await;
        assert!(result.is_err());
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

    fn request(sources: &[UpdateSource], holds: &[&str]) -> ApplyRequest {
        ApplyRequest {
            sources: sources.to_vec(),
            holds: holds.iter().map(|hold| hold.to_string()).collect(),
        }
    }

    async fn drain(rx: &mut mpsc::Receiver<ApplyEvent>) -> Vec<ApplyEvent> {
        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            events.push(event);
        }
        events
    }

    fn fake_pacman(body: &str) -> PathBuf {
        fake_program("pacman", body)
    }

    fn fake_flatpak(body: &str) -> PathBuf {
        fake_program("flatpak", body)
    }

    fn fake_program(name: &str, body: &str) -> PathBuf {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;

        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let directory =
            std::env::temp_dir().join(format!("skemer-{name}-{}-{id}", std::process::id()));
        fs::create_dir_all(&directory).expect("temp directory");
        let path = directory.join(name);
        let mut file = fs::File::create(&path).expect("create fake program");
        file.write_all(body.as_bytes()).expect("write fake program");
        file.sync_all().expect("flush fake program");
        drop(file);
        let mut permissions = fs::metadata(&path).expect("metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).expect("chmod");
        path
    }
}
