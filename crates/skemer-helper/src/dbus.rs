use std::sync::{Arc, Mutex};

use anyhow::Result;
use skemer_core::models::{ApplyOutcome, ApplyRequest, UpdateSource};
use skemer_core::CancelHandle;
use tokio::sync::mpsc;
use zbus::message::Header;
use zbus::object_server::SignalEmitter;
use zbus::{connection, fdo, interface, Connection};

use crate::{polkit, system};

pub struct SkemerService {
    /// One libalpm transaction at a time. The lock covers check, hold
    /// validation, and apply so callers cannot race on the pacman database.
    transaction_lock: Arc<tokio::sync::Mutex<()>>,
    /// The running apply, if any.
    running: Arc<Mutex<Option<RunningJob>>>,
}

struct RunningJob {
    cancel: CancelHandle,
    uid: u32,
}

impl Default for SkemerService {
    fn default() -> Self {
        Self {
            transaction_lock: Arc::new(tokio::sync::Mutex::new(())),
            running: Arc::new(Mutex::new(None)),
        }
    }
}

/// Clear the running job when the apply method returns, including on an early
/// validation error.
struct RunningGuard {
    running: Arc<Mutex<Option<RunningJob>>>,
}

impl Drop for RunningGuard {
    fn drop(&mut self) {
        if let Ok(mut running) = self.running.lock() {
            *running = None;
        }
    }
}

#[interface(name = "org.veldmuislinux.skemer")]
impl SkemerService {
    /// Read-only plan. Allowed for local users and must never apply anything.
    async fn check(&self) -> Result<String, fdo::Error> {
        let _guard = self.transaction_lock.lock().await;
        let plan = tokio::task::spawn_blocking(system::check)
            .await
            .map_err(failed)?
            .map_err(failed)?;
        serde_json::to_string(&plan).map_err(failed)
    }

    /// Validate a proposed hold set with libalpm. This is read-only; the UI
    /// calls it before offering "hold and retry".
    async fn prepare(&self, request: String) -> Result<String, fdo::Error> {
        let request: ApplyRequest = serde_json::from_str(&request)
            .map_err(|error| fdo::Error::InvalidArgs(error.to_string()))?;
        let _guard = self.transaction_lock.lock().await;
        let decision = tokio::task::spawn_blocking(move || system::prepare(&request.holds))
            .await
            .map_err(failed)?
            .map_err(failed)?;
        serde_json::to_string(&decision).map_err(failed)
    }

    /// Apply the selected sources. Requires polkit authorization. Progress and
    /// log lines are emitted as signals while the method runs; the outcome is
    /// the method reply.
    async fn apply(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
        request: String,
    ) -> Result<String, fdo::Error> {
        let request: ApplyRequest = serde_json::from_str(&request)
            .map_err(|error| fdo::Error::InvalidArgs(error.to_string()))?;
        let sender = header
            .sender()
            .map(ToString::to_string)
            .ok_or_else(|| fdo::Error::Failed("missing D-Bus sender".to_string()))?;

        polkit::authorize(connection, &sender, polkit::APPLY_ACTION)
            .await
            .map_err(failed)?;
        let uid = connection_uid(connection, &sender).await.map_err(failed)?;

        // Register the cancel handle before waiting for the transaction lock so
        // a second apply is rejected immediately.
        let cancel = CancelHandle::default();
        {
            let mut running = self
                .running
                .lock()
                .map_err(|_| fdo::Error::Failed("the update lock is poisoned".to_string()))?;
            if running.is_some() {
                return Err(fdo::Error::Failed(
                    "an update is already running".to_string(),
                ));
            }
            *running = Some(RunningJob {
                cancel: cancel.clone(),
                uid,
            });
        }
        let _running_guard = RunningGuard {
            running: self.running.clone(),
        };

        let _guard = self.transaction_lock.lock().await;

        let wants_system = request.sources.contains(&UpdateSource::System);
        let mut reboot_suggested = false;

        if wants_system {
            // Holds are validated before pacman starts. A rejected hold is a
            // hard error; the UI asks for confirmation and calls Prepare first.
            let holds = request.holds.clone();
            let decision = tokio::task::spawn_blocking(move || system::prepare(&holds))
                .await
                .map_err(failed)?
                .map_err(failed)?;
            if !decision.safe {
                return Err(fdo::Error::Failed(decision.reason));
            }

            reboot_suggested = tokio::task::spawn_blocking(system::check)
                .await
                .map_err(failed)?
                .map(|plan| plan.reboot_suggested)
                .unwrap_or(false);
        }

        let (events_tx, mut events_rx) = mpsc::channel::<system::ApplyEvent>(256);
        let emitter = emitter.to_owned();
        let forwarder = tokio::spawn(async move {
            while let Some(event) = events_rx.recv().await {
                match event {
                    system::ApplyEvent::Progress { percent, message } => {
                        let _ = SkemerService::progress(emitter.clone(), percent, message).await;
                    }
                    system::ApplyEvent::Log(line) => {
                        let _ = SkemerService::log_line(emitter.clone(), line).await;
                    }
                }
            }
        });

        let result = system::apply_streaming(
            &request,
            reboot_suggested,
            "pacman",
            "flatpak",
            events_tx,
            cancel,
        )
        .await;

        // Flush the log/progress signals before returning the outcome.
        let _ = forwarder.await;

        let outcome = match result {
            Ok(outcome) => outcome,
            Err(error) => ApplyOutcome {
                success: false,
                cancelled: false,
                held_packages: request.holds.clone(),
                reboot_suggested,
                log_tail: String::new(),
                error: Some(error.to_string()),
                candidates: Vec::new(),
            },
        };

        serde_json::to_string(&outcome).map_err(failed)
    }

    /// Cancel the running transaction. Only the user that started the update
    /// may cancel it. SIGINT lets pacman release its lock and transaction
    /// before exiting.
    async fn cancel(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> Result<(), fdo::Error> {
        let sender = header
            .sender()
            .map(ToString::to_string)
            .ok_or_else(|| fdo::Error::Failed("missing D-Bus sender".to_string()))?;
        let uid = connection_uid(connection, &sender).await.map_err(failed)?;

        let handle = {
            let running = self
                .running
                .lock()
                .map_err(|_| fdo::Error::Failed("the update lock is poisoned".to_string()))?;
            match running.as_ref() {
                Some(job) if job.uid == uid => Some(job.cancel.clone()),
                Some(_) => {
                    return Err(fdo::Error::AccessDenied(
                        "only the user that started the update can cancel it".to_string(),
                    ));
                }
                None => None,
            }
        };

        if let Some(handle) = handle {
            handle.cancel();
        }
        Ok(())
    }

    /// Refresh the Arch mirrorlist. This lands with the mirror phase.
    async fn refresh_mirrors(&self) -> Result<(), fdo::Error> {
        Err(fdo::Error::NotSupported(
            "mirror refresh is not implemented yet".to_string(),
        ))
    }

    /// One package progressed in the running transaction.
    #[zbus(signal)]
    async fn progress(
        emitter: SignalEmitter<'_>,
        percent: u32,
        message: String,
    ) -> zbus::Result<()>;

    /// One line of pacman output.
    #[zbus(signal)]
    async fn log_line(emitter: SignalEmitter<'_>, line: String) -> zbus::Result<()>;
}

async fn connection_uid(connection: &Connection, sender: &str) -> Result<u32, zbus::Error> {
    let reply = connection
        .call_method(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"),
            "GetConnectionUnixUser",
            &(sender,),
        )
        .await?;
    reply.body().deserialize::<u32>()
}

fn failed(error: impl std::fmt::Display) -> fdo::Error {
    fdo::Error::Failed(error.to_string())
}

pub async fn serve() -> Result<()> {
    let _connection = connection::Builder::system()?
        .name("org.veldmuislinux.skemer")?
        .serve_at("/org/veldmuislinux/skemer", SkemerService::default())?
        .build()
        .await?;

    tracing::info!("skemer-helper is listening on the system bus");
    tokio::signal::ctrl_c().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use skemer_core::models::UpdatePlan;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "needs a session bus"]
    async fn check_round_trips_over_dbus() {
        let connection = connection::Builder::session()
            .expect("session bus")
            .build()
            .await
            .expect("connect");
        connection
            .object_server()
            .at("/org/veldmuislinux/skemer", SkemerService::default())
            .await
            .expect("serve the interface");

        let destination = connection.unique_name().expect("unique name").to_string();
        let proxy = zbus::Proxy::new(
            &connection,
            destination,
            "/org/veldmuislinux/skemer",
            "org.veldmuislinux.skemer",
        )
        .await
        .expect("proxy");

        let reply: String = proxy.call("Check", &()).await.expect("Check");
        let plan: UpdatePlan = serde_json::from_str(&reply).expect("plan JSON");
        assert!(!plan.checked_at.is_empty());
    }
}
